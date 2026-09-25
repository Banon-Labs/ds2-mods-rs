//! The detour on the net session update that reads the key, the call into the Game tab's own
//! commit, the watcher that lets the key move while the game runs, and the detour on the HUD voice
//! chat icon's update that shows the byte.

use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};
use ds2_hotkey_config::chord_name;
use ds2_hotkey_config::keys::{Chord, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_SHIFT};
use ds2_hotkey_config::live::AtomicChord;
use ds2_hotkey_config::reload::{FileChange, HotFile};

use crate::{
    Announce, AnnounceSetting, CONFIG_KEY_ANNOUNCE, CONFIG_KEY_KEYBOARD, CONFIG_SECTION,
    ComponentMemory, DEFAULT_ANNOUNCE, DEFAULT_KEY, ICON_PLAYBACK_RATE, KeySetting, LOG_PREFIX,
    announce_setting, default_chord, icon_sprites, icon_visibility, key_setting, toggled,
    voice_chat_on,
};

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const c_void, module: *mut c_void, flags: u32) -> i32;
}

/// `SND_ASYNC | SND_NODEFAULT | SND_MEMORY`: return at once, never fall back to the system beep,
/// and read the WAV from the pointer. A new clip cuts off one still playing.
const PLAY_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004;

/// `VK_CONTROL`, `VK_MENU`, `VK_SHIFT` -- the three modifiers a [`Chord`] can carry.
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_SHIFT: i32 = 0x10;

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// What the loader asks for.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// The config file to watch for the binding. `None` leaves [`DEFAULT_KEY`] in force for the
    /// whole session.
    pub config_path: Option<PathBuf>,
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Outcome {
    /// The detour is in and the commit routine matched its prologue.
    pub installed: bool,
    /// The HUD icon detour is in. Independent of `installed`: a refused icon leaves the key working.
    pub hud_icon: bool,
}

/// `NET_SESSION_UPDATE(this, f32 delta)`. The delta is a float in `xmm1`; declaring it as an
/// integer would compile and hand the original whatever that register held.
type NetSessionUpdate = unsafe extern "system" fn(usize, f32);

/// `GAME_OPTION_GAME_TAB_APPLY(options, working_copy)`.
type GameTabApply = unsafe extern "system" fn(*mut u8, *const u8);

/// Trampoline back to the real net session update.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Resolved address of the Game tab's commit, or `0` before install.
static APPLY: AtomicUsize = AtomicUsize::new(0);

/// Resolved address of `GameManagerImp`'s global, or `0` before install.
static GAME_MANAGER: AtomicUsize = AtomicUsize::new(0);

/// The binding the detour reads. Unset means unbound.
static KEY_BINDING: AtomicChord = AtomicChord::unset();

/// The announcement language, as `Announce as u8`.
static ANNOUNCE: AtomicU8 = AtomicU8::new(DEFAULT_ANNOUNCE as u8);

/// Whether the chord was down last frame, so a hold is one press.
static WAS_DOWN: AtomicBool = AtomicBool::new(false);

/// Presses logged so far. The detour runs every frame and the loader's sink syncs every line, so
/// the log is capped rather than written per press forever.
static LOGGED: AtomicU32 = AtomicU32::new(0);

/// How many press lines the log gets before it goes quiet.
const LOGGED_LINES: u32 = 32;

/// How often the watcher looks at the config file.
const POLL_INTERVAL_MS: u64 = 1000;

fn log_capped(args: std::fmt::Arguments<'_>) {
    if LOGGED.fetch_add(1, Ordering::Relaxed) < LOGGED_LINES {
        log(args);
    }
}

/// Whether the foreground window belongs to this process, so a key typed into another window is
/// not a press. Same shape as `ds2-inventory-sort`'s check.
fn game_has_focus() -> bool {
    // SAFETY: a plain Win32 call with no arguments.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return false;
    }
    let mut owner = 0u32;
    // SAFETY: `foreground` is a handle Win32 just returned and `owner` is a live `u32`.
    unsafe { GetWindowThreadProcessId(foreground, &raw mut owner) };
    // SAFETY: a plain Win32 call with no arguments.
    owner != 0 && owner == unsafe { GetCurrentProcessId() }
}

fn vk_down(vk: i32) -> bool {
    // SAFETY: a plain Win32 call taking an integer. Only the high bit ("down now") is used.
    unsafe { GetAsyncKeyState(vk) < 0 }
}

fn chord_down(chord: Chord) -> bool {
    if chord.vk == 0 {
        return false;
    }
    if chord.modifiers & MODIFIER_CTRL != 0 && !vk_down(VK_CONTROL) {
        return false;
    }
    if chord.modifiers & MODIFIER_ALT != 0 && !vk_down(VK_MENU) {
        return false;
    }
    if chord.modifiers & MODIFIER_SHIFT != 0 && !vk_down(VK_SHIFT) {
        return false;
    }
    vk_down(chord.vk as i32)
}

/// The live options block, or `None` while it does not exist (title screen, loads).
fn options_block() -> Option<usize> {
    let global = GAME_MANAGER.load(Ordering::Acquire);
    if global == 0 {
        return None;
    }
    // SAFETY: fault-tolerant reads; a null or unmapped link is `None`, not a crash.
    let manager = unsafe { ds2_game_base::mem::safe_read_usize(global) }.filter(|p| *p != 0)?;
    // SAFETY: as above.
    let data =
        unsafe { ds2_game_base::mem::safe_read_usize(manager + ds2_rva::GAME_DATA_MANAGER_OFFSET) }
            .filter(|p| *p != 0)?;
    // SAFETY: as above.
    unsafe { ds2_game_base::mem::safe_read_usize(data + ds2_rva::GAME_DATA_MANAGER_OPTIONS_OFFSET) }
        .filter(|p| *p != 0)
}

/// Flip Voice chat through the Game tab's commit.
///
/// # Safety
///
/// Game thread only: called from the net session update's own detour, before the original.
unsafe fn toggle_voice_chat() {
    let Some(options) = options_block() else {
        log_capped(format_args!(
            "{LOG_PREFIX} press ignored -- the options block does not exist yet"
        ));
        return;
    };
    let apply = APPLY.load(Ordering::Acquire);
    if apply == 0 {
        return;
    }
    let mut copy = [0u8; ds2_rva::GAME_OPTION_GAME_TAB_LEN];
    // SAFETY: fault-tolerant read into a local of exactly the length asked for.
    if !unsafe { ds2_game_base::mem::read_bytes(options, &mut copy) } {
        log_capped(format_args!(
            "{LOG_PREFIX} press ignored -- options block 0x{options:016x} is not readable"
        ));
        return;
    }
    let was = copy[ds2_rva::GAME_OPTION_VOICE_CHAT_OFFSET];
    let want = toggled(was);
    copy[ds2_rva::GAME_OPTION_VOICE_CHAT_OFFSET] = want;
    // SAFETY: `apply` is the Game tab's commit, checked against its prologue at install. It takes
    // the live block and a sixteen-byte copy -- exactly what the options menu passes it -- and this
    // is the game thread, where the menu calls it from.
    let commit: GameTabApply = unsafe { std::mem::transmute::<usize, GameTabApply>(apply) };
    // SAFETY: see above; `copy` outlives the call.
    unsafe { commit(options as *mut u8, copy.as_ptr()) };
    let mut now = [0u8; 1];
    // SAFETY: fault-tolerant read of one byte just written by the game.
    let read = unsafe {
        ds2_game_base::mem::read_bytes(options + ds2_rva::GAME_OPTION_VOICE_CHAT_OFFSET, &mut now)
    };
    let on = voice_chat_on(now[0]);
    let state = if on { "on" } else { "off" };
    // Only the state read back from the game is announced; a failed read says nothing rather than
    // guess.
    if read && let Some(clip) = Announce::from_u8(ANNOUNCE.load(Ordering::Relaxed)).clip(on) {
        // SAFETY: `clip` is a `'static` WAV compiled into this DLL, so it outlives the async play.
        unsafe { PlaySoundW(clip.as_ptr().cast(), core::ptr::null_mut(), PLAY_FLAGS) };
    }
    log_capped(format_args!(
        "{LOG_PREFIX} voice chat {state} (byte 0x{was:02x} -> 0x{:02x}, wanted 0x{want:02x}, \
         read={read})",
        now[0]
    ));
}

/// The detour. Reads the key, toggles on a fresh press, then runs the original -- which picks the
/// new byte up in this same frame.
unsafe extern "system" fn net_session_update_detour(this: usize, delta: f32) {
    let down = game_has_focus() && KEY_BINDING.load().is_some_and(chord_down);
    if !WAS_DOWN.swap(down, Ordering::Relaxed) && down {
        // SAFETY: this is the game thread, inside the net session update.
        unsafe { toggle_voice_chat() };
    }
    let original = ORIGINAL.load(Ordering::Acquire);
    if original != 0 {
        // SAFETY: the trampoline MinHook returned for this exact site and ABI.
        let original: NetSessionUpdate =
            unsafe { std::mem::transmute::<usize, NetSessionUpdate>(original) };
        // SAFETY: forwarding the arguments the game passed.
        unsafe { original(this, delta) };
    }
}

// ---------------------------------------------------------------------------------------------
// The HUD icon.
// ---------------------------------------------------------------------------------------------

/// `FE_VOICE_CHAT_ICON_UPDATE(this)`.
type IconUpdate = unsafe extern "system" fn(usize);

/// `FE_BIND_SCENE_OBJ_PROXY(layout_proxy, out, path) -> out`.
type BindProxyFn = unsafe extern "system" fn(usize, *mut u8, *const u8) -> *mut u8;

/// One of the icon's three child getters: `(this, out) -> out`.
type ChildFn = unsafe extern "system" fn(usize, *mut u8) -> *mut u8;

/// `FE_ELEMENT_SET_VISIBLE(accessor + 8, shown)`.
type SetVisibleFn = unsafe extern "system" fn(*mut u8, u8);

/// Trampoline back to the real icon update.
static ICON_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Resolved addresses, in the order the update uses them: bind, set-visible, then the three
/// children for bytes 1..3. All zero until every prologue has matched.
static ICON_CALLS: [AtomicUsize; 5] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

/// The last state word applied, logged on change only -- the detour runs every frame.
static ICON_LAST_WORD: AtomicU32 = AtomicU32::new(u32::MAX);

/// A one-component element id path, in the shape [`ds2_rva::FE_ELEMENT_RESOLVE`] reads. Aligned so
/// the game's `base + (-base & 3)` idiom lands on `base`.
#[repr(C, align(16))]
struct IdPath([u8; ds2_rva::FE_ELEMENT_PATH_SIZE]);

/// One element accessor. Self-referential once bound (see `ds2-item-warn`'s `resolve_element`), so
/// it is built in place and never moved; the game never destroys its own either.
#[repr(C, align(16))]
struct Accessor([u8; ds2_rva::FE_ELEMENT_ACCESSOR_SIZE]);

/// Apply four show/hide bytes the way [`ds2_rva::FE_VOICE_CHAT_ICON_UPDATE`] does: bind the root
/// into one accessor, set it, then let each child getter overwrite that accessor and set the child.
///
/// # Safety
///
/// Game thread, from the icon update's own detour, with `this` the icon the game passed.
unsafe fn apply_icon(this: usize, calls: [usize; 5], bytes: [u8; 4]) {
    let [bind, set_visible, child_3e0, child_3e1, child_3e2] = calls;
    let mut path = IdPath([0; ds2_rva::FE_ELEMENT_PATH_SIZE]);
    let start = (path.0.as_ptr() as usize).wrapping_neg() & 3;
    path.0[start..][..4].copy_from_slice(&ds2_rva::FE_VOICE_CHAT_ICON_ROOT_ELEMENT.to_le_bytes());
    path.0[ds2_rva::FE_ELEMENT_PATH_COUNT_OFFSET..][..8].copy_from_slice(&1u64.to_le_bytes());
    let mut accessor = Accessor([0; ds2_rva::FE_ELEMENT_ACCESSOR_SIZE]);
    let out = accessor.0.as_mut_ptr();

    // SAFETY: every address matched its recorded prologue at install, and the argument shapes are
    // the ones `0x14050a685`..`0x14050a6ec` pass: the icon's layout proxy, a 0x90-byte accessor, a
    // one-id path; then `(this, out)` for each child, and `out + 8` for every set.
    unsafe {
        let bind: BindProxyFn = std::mem::transmute::<usize, BindProxyFn>(bind);
        let set_visible: SetVisibleFn = std::mem::transmute::<usize, SetVisibleFn>(set_visible);
        bind(
            this + ds2_rva::FE_VOICE_CHAT_ICON_LAYOUT_PROXY_OFFSET,
            out,
            path.0.as_ptr(),
        );
        set_visible(
            out.add(ds2_rva::FE_ELEMENT_ACCESSOR_VISIBLE_OFFSET),
            bytes[0],
        );
        for (child, byte) in [child_3e0, child_3e1, child_3e2]
            .into_iter()
            .zip(&bytes[1..])
        {
            let child: ChildFn = std::mem::transmute::<usize, ChildFn>(child);
            let accessor = child(this, out);
            set_visible(
                accessor.add(ds2_rva::FE_ELEMENT_ACCESSOR_VISIBLE_OFFSET),
                *byte,
            );
        }
    }
}

/// The Voice chat byte, or `None` while the options block does not exist.
fn voice_chat_byte() -> Option<u8> {
    let options = options_block()?;
    let mut byte = [0u8; 1];
    // SAFETY: fault-tolerant read of one byte inside the live options block.
    unsafe {
        ds2_game_base::mem::read_bytes(options + ds2_rva::GAME_OPTION_VOICE_CHAT_OFFSET, &mut byte)
    }
    .then_some(byte[0])
}

/// Live game memory for [`icon_sprites`]: every read fault-tolerant.
struct GameMemory;

impl ComponentMemory for GameMemory {
    fn read_usize(&self, addr: usize) -> Option<usize> {
        // SAFETY: fault-tolerant read; an unmapped address is `None`.
        unsafe { ds2_game_base::mem::safe_read_usize(addr) }
    }
    fn read_u16(&self, addr: usize) -> Option<u16> {
        // SAFETY: as above.
        unsafe { ds2_game_base::mem::safe_read_u16(addr) }
    }
}

/// Module base, for the vtable tests in the icon walk. `0` before install.
static ICON_BASE: AtomicUsize = AtomicUsize::new(0);

/// Sprites slowed on the last pass, logged on change only -- the detour runs every frame.
static ICON_SLOWED: AtomicUsize = AtomicUsize::new(usize::MAX);

/// The root component of the icon's own layout scene, reached the way the game's empty-path
/// resolve does: proxy slot 1 (checked to be [`ds2_rva::FEX_SCENE_GET_SCENE`], whose body is
/// `mov rax,[rcx-0x10]`), then `[[scene+0x28]+0x30]` (`0x140afdaf0`).
fn icon_root(this: usize, base: usize) -> Option<usize> {
    let memory = GameMemory;
    let proxy = this + ds2_rva::FE_VOICE_CHAT_ICON_LAYOUT_PROXY_OFFSET;
    let vtable = memory.read_usize(proxy)?;
    let get_scene = memory.read_usize(vtable + ds2_rva::FE_SCENE_PROXY_GET_SCENE_SLOT)?;
    if get_scene != base + ds2_rva::FEX_SCENE_GET_SCENE as usize {
        return None;
    }
    let scene = memory
        .read_usize(proxy - ds2_rva::FEX_SCENE_GET_SCENE_BACK_OFFSET)
        .filter(|p| *p != 0)?;
    let holder = memory
        .read_usize(scene + ds2_rva::FE_SCENE_ROOT_HOLDER_OFFSET)
        .filter(|p| *p != 0)?;
    memory
        .read_usize(holder + ds2_rva::FE_SCENE_ROOT_OFFSET)
        .filter(|p| *p != 0)
}

/// Set [`ICON_PLAYBACK_RATE`] on every sprite of the icon's scene. Only this icon's scene is
/// walked, so no other HUD element changes speed.
///
/// # Safety
///
/// Game thread, from the icon's own update, with `this` the icon the game passed.
unsafe fn slow_icon(this: usize) {
    let base = ICON_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let Some(root) = icon_root(this, base) else {
        return;
    };
    let sprites = icon_sprites(root, base, &GameMemory);
    for &sprite in &sprites {
        let rate = sprite + ds2_rva::FE_SPRITE_RATE_OFFSET;
        // SAFETY: fault-tolerant read.
        let now = unsafe { ds2_game_base::mem::safe_read_f32(rate) };
        if now.is_some_and(|r| r != ICON_PLAYBACK_RATE) {
            // SAFETY: `sprite` carries `FeComponentSprite`'s vtable (checked by the walk) and was
            // just read at this field; `+0x60` is its rate, a plain `f32` only its tick reads.
            unsafe { (rate as *mut f32).write(ICON_PLAYBACK_RATE) };
        }
    }
    if ICON_SLOWED.swap(sprites.len(), Ordering::Relaxed) != sprites.len() {
        log_capped(format_args!(
            "{LOG_PREFIX} hud icon playback x{ICON_PLAYBACK_RATE} on {} sprite(s) under root \
             0x{root:016x}",
            sprites.len()
        ));
    }
}

/// The icon detour. With the byte known, it applies the game's own state for it and skips the
/// original; otherwise the original runs untouched. Either way the icon plays at
/// [`ICON_PLAYBACK_RATE`].
unsafe extern "system" fn icon_update_detour(this: usize) {
    if this != 0 {
        // SAFETY: game thread, inside the icon's own update, with the icon the game passed.
        unsafe { slow_icon(this) };
    }
    let calls: [usize; 5] = core::array::from_fn(|i| ICON_CALLS[i].load(Ordering::Acquire));
    if this != 0
        && !calls.contains(&0)
        && let Some(byte) = voice_chat_byte()
    {
        let bytes = icon_visibility(byte);
        let word = u32::from_le_bytes(bytes);
        if ICON_LAST_WORD.swap(word, Ordering::Relaxed) != word {
            log_capped(format_args!(
                "{LOG_PREFIX} hud icon {} (byte 0x{byte:02x}, word 0x{word:08x})",
                if voice_chat_on(byte) { "on" } else { "off" }
            ));
        }
        // SAFETY: game thread, inside the icon's own update, with the icon the game passed.
        unsafe { apply_icon(this, calls, bytes) };
        return;
    }
    let original = ICON_ORIGINAL.load(Ordering::Acquire);
    if original != 0 {
        // SAFETY: the trampoline MinHook returned for this exact site and ABI.
        let original: IconUpdate = unsafe { std::mem::transmute::<usize, IconUpdate>(original) };
        // SAFETY: forwarding the argument the game passed.
        unsafe { original(this) };
    }
}

/// Check the icon update and every function the detour calls, then hook the update. MinHook must
/// already be initialised.
fn install_icon(base: usize) -> bool {
    let bind = base + ds2_rva::FE_BIND_SCENE_OBJ_PROXY as usize;
    let set_visible = base + ds2_rva::FE_ELEMENT_SET_VISIBLE as usize;
    let children = [
        (ds2_rva::FE_VOICE_CHAT_ICON_CHILD_3E0, "icon-child-3e0"),
        (ds2_rva::FE_VOICE_CHAT_ICON_CHILD_3E1, "icon-child-3e1"),
        (ds2_rva::FE_VOICE_CHAT_ICON_CHILD_3E2, "icon-child-3e2"),
    ]
    .map(|(rva, what)| (base + rva as usize, what));
    let site = base + ds2_rva::FE_VOICE_CHAT_ICON_UPDATE as usize;
    if !prologue_matches(
        bind,
        &ds2_rva::FE_BIND_SCENE_OBJ_PROXY_PROLOGUE,
        "bind-proxy",
    ) || !prologue_matches(
        set_visible,
        &ds2_rva::FE_ELEMENT_SET_VISIBLE_PROLOGUE,
        "set-visible",
    ) || !children
        .iter()
        .all(|&(at, what)| prologue_matches(at, &ds2_rva::FE_VOICE_CHAT_ICON_CHILD_PROLOGUE, what))
        || !prologue_matches(
            site,
            &ds2_rva::FE_VOICE_CHAT_ICON_UPDATE_PROLOGUE,
            "icon-update",
        )
    {
        return false;
    }
    for (slot, address) in ICON_CALLS.iter().zip([
        bind,
        set_visible,
        children[0].0,
        children[1].0,
        children[2].0,
    ]) {
        slot.store(address, Ordering::Release);
    }
    ICON_BASE.store(base, Ordering::Release);
    // SAFETY: the site matched its recorded prologue above, and the detour is a `'static` fn of the
    // same ABI (`this` in rcx, nothing returned).
    match unsafe { MhHook::new(site as *mut c_void, icon_update_detour as *mut c_void) } {
        Ok(handle) => {
            ICON_ORIGINAL.store(handle.trampoline() as usize, Ordering::Release);
            // SAFETY: the address `MhHook::new` just registered.
            let status = unsafe { MH_EnableHook(site as *mut c_void) };
            if status != MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} hud-icon install-failed stage=MH_EnableHook status={status:?}"
                ));
                return false;
            }
        }
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} hud-icon install-failed stage=MH_CreateHook status={status:?}"
            ));
            return false;
        }
    }
    log(format_args!(
        "{LOG_PREFIX} hud icon armed site=0x{site:016x} -- on shows the game's state 2, off hides it"
    ));
    true
}

/// Apply one config text.
///
/// A value that does not parse leaves the binding already in force and says so; falling back to
/// the default would move the key somewhere the player did not ask for.
fn apply_config(text: &str, first: bool) {
    apply_announce(text, first);
    match key_setting(text) {
        KeySetting::NotSet => {
            if first {
                log(format_args!(
                    "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_KEYBOARD} not set -- default \
                     {DEFAULT_KEY}"
                ));
            }
        }
        KeySetting::Unbound => {
            if KEY_BINDING.load().is_some_and(|chord| chord.vk != 0) || first {
                log(format_args!(
                    "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_KEYBOARD} is empty -- no binding"
                ));
            }
            KEY_BINDING.store(Chord {
                modifiers: 0,
                vk: 0,
                dik: None,
            });
            WAS_DOWN.store(false, Ordering::Relaxed);
        }
        KeySetting::Bound(chord) => {
            if KEY_BINDING.load() != Some(chord) {
                KEY_BINDING.store(chord);
                // A moved binding resets the edge, or a key held during the reload reads as a
                // press nobody made.
                WAS_DOWN.store(false, Ordering::Relaxed);
                log(format_args!(
                    "{LOG_PREFIX} keyboard binding = {}",
                    chord_name(chord)
                ));
            }
        }
        KeySetting::Invalid { value, error } => log(format_args!(
            "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_KEYBOARD} = {value:?} not understood \
             ({error:?}) -- keeping the binding already in force"
        )),
    }
}

fn apply_announce(text: &str, first: bool) {
    match announce_setting(text) {
        AnnounceSetting::NotSet => {}
        AnnounceSetting::Set(lang) => {
            if ANNOUNCE.swap(lang as u8, Ordering::Relaxed) != lang as u8 || first {
                log(format_args!("{LOG_PREFIX} announce = {lang:?}"));
            }
        }
        AnnounceSetting::Invalid(value) => log(format_args!(
            "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_ANNOUNCE} = {value:?} not understood \
             (en, pl, or \"\") -- keeping the language already in force"
        )),
    }
}

fn watch(path: PathBuf) {
    let mut hot = HotFile::with_interval(path, POLL_INTERVAL_MS);
    loop {
        match hot.poll() {
            Some(FileChange::Text(text)) => apply_config(&text, false),
            Some(FileChange::Missing) => log(format_args!(
                "{LOG_PREFIX} config file disappeared -- keeping the binding already in force"
            )),
            None => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
    }
}

/// Check a site's first bytes against what `ds2-rva` recorded for it.
fn prologue_matches(site: usize, expected: &[u8], what: &str) -> bool {
    let mut found = vec![0u8; expected.len()];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(site, &mut found) };
    if !read || found != expected {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=prologue what={what} va=0x{site:016x} read={read} \
             saw={found:02x?} want={expected:02x?}"
        ));
        return false;
    }
    true
}

/// Check both sites, arm the binding, and put the detour in.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan`, which in
/// practice means the loader's Arxan callback.
pub unsafe fn install(request: &Request) -> Outcome {
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome::default();
        }
    };

    // The function this crate calls is checked like the one it patches: a bad address here is a
    // jump into arbitrary code on the game thread, not a refused patch.
    let apply = base + ds2_rva::GAME_OPTION_GAME_TAB_APPLY as usize;
    if !prologue_matches(
        apply,
        &ds2_rva::GAME_OPTION_GAME_TAB_APPLY_PROLOGUE,
        "game-tab-apply",
    ) {
        return Outcome::default();
    }
    let site = base + ds2_rva::NET_SESSION_UPDATE as usize;
    if !prologue_matches(
        site,
        &ds2_rva::NET_SESSION_UPDATE_PROLOGUE,
        "net-session-update",
    ) {
        return Outcome::default();
    }
    APPLY.store(apply, Ordering::Release);
    GAME_MANAGER.store(base + ds2_rva::GAME_MANAGER_IMP as usize, Ordering::Release);

    // The default is in force before the file is read, so a missing file still leaves a key.
    if let Some(chord) = default_chord() {
        KEY_BINDING.store(chord);
    }
    if let Some(path) = request.config_path.clone() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            apply_config(&text, true);
        }
        std::thread::spawn(move || watch(path));
    } else {
        log(format_args!(
            "{LOG_PREFIX} no config path -- the binding is {DEFAULT_KEY} for this session"
        ));
    }

    // MinHook is statically linked into this DLL, so ALREADY_INITIALIZED only means another crate
    // in it got there first.
    // SAFETY: MinHook's own initialiser, no arguments.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome::default();
    }
    // SAFETY: the site matched its recorded prologue above, and the detour is a `'static` fn of
    // the same ABI (`this` in rcx, the float delta in xmm1).
    match unsafe {
        MhHook::new(
            site as *mut c_void,
            net_session_update_detour as *mut c_void,
        )
    } {
        Ok(handle) => {
            // Published before the site is patched, so a detour that fires at once has somewhere
            // to go.
            ORIGINAL.store(handle.trampoline() as usize, Ordering::Release);
            // SAFETY: the address `MhHook::new` just registered.
            let status = unsafe { MH_EnableHook(site as *mut c_void) };
            if status != MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} install-failed stage=MH_EnableHook status={status:?}"
                ));
                return Outcome::default();
            }
        }
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=MH_CreateHook status={status:?}"
            ));
            return Outcome::default();
        }
    }

    let key = KEY_BINDING
        .load()
        .filter(|chord| chord.vk != 0)
        .map_or_else(|| "none".to_string(), chord_name);
    log(format_args!(
        "{LOG_PREFIX} armed key={key} site=0x{site:016x} -- the options menu's own Voice chat row \
         still works and is untouched"
    ));
    let hud_icon = install_icon(base);
    if !hud_icon {
        log(format_args!(
            "{LOG_PREFIX} hud icon NOT INSTALLED -- the key still works, the HUD shows the game's \
             own voice state"
        ));
    }
    Outcome {
        installed: true,
        hud_icon,
    }
}
