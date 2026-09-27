//! The frame consumer that reads the key and makes the call, and the watcher that lets the key and
//! the id move while the game runs.

use core::ffi::c_void;
use core::mem::size_of;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};
use ds2_hotkey_config::chord_name;
use ds2_hotkey_config::keys::Chord;
use ds2_hotkey_config::live::AtomicChord;
use ds2_hotkey_config::reload::{FileChange, HotFile};

use crate::catalog::{CATALOG_FILE_NAME, CatalogEntry, EMBEDDED_CATALOG, parse_catalog};
use crate::marked;
use crate::overlay;
use crate::selector::{ACTIONS, Bindings, Keeper, Selector, Step, keep_reason, view};
use crate::{
    ApplyReason, CONFIG_KEY_EFFECT, CONFIG_KEY_KEYBOARD, CONFIG_KEY_NETWORK, CONFIG_SECTION,
    DEFAULT_EFFECT, DEFAULT_KEY, EffectSetting, KeySetting, LOG_PREFIX, Toggle, chord_held,
    default_chord, effect_setting, is_press, key_setting, network_setting, request_bytes,
    sp_effect_active, sp_effect_ctrl, sp_effect_ids,
};

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
    fn GetCurrentProcessId() -> u32;
}

// The same `winmm` declaration and flags `ds2-voice-chat` plays its clips with. Kept here rather
// than shared because the crates that could hold one shared copy are not this change's to edit.
#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const c_void, module: *mut c_void, flags: u32) -> i32;
}

/// `SND_ASYNC | SND_NODEFAULT | SND_MEMORY`: return at once, never fall back to the system beep,
/// and read the WAV from the pointer. A new clip cuts off one still playing.
const PLAY_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004;

/// Say the toggle's new state out loud and log whether `PlaySoundW` took the clip.
fn announce(on: bool) {
    let clip = crate::glyph::clip(on);
    // SAFETY: `clip` is a `'static` WAV compiled into this DLL, so it outlives the async play.
    let played = unsafe { PlaySoundW(clip.as_ptr().cast(), core::ptr::null_mut(), PLAY_FLAGS) };
    log(format_args!(
        "{LOG_PREFIX} voice clip \"Net effects {}.\" played={} ({} bytes)",
        if on { "on" } else { "off" },
        played != 0,
        clip.len()
    ));
}

/// Times the key was seen held while another window had focus, so each such press is logged
/// without a held key filling the log.
static UNFOCUSED_HELD: AtomicU64 = AtomicU64::new(0);

/// Whether the key was held without focus last frame.
static UNFOCUSED_WAS_DOWN: AtomicBool = AtomicBool::new(false);

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
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
    /// The config file to watch for the key and the id. `None` leaves [`DEFAULT_KEY`] and
    /// [`DEFAULT_EFFECT`] in force for the whole session.
    pub config_path: Option<PathBuf>,
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Outcome {
    /// The apply function matched its recorded first bytes and the frame consumer is registered.
    pub installed: bool,
}

/// `applySpEffect(ChrSpEffectCtrl*, const Request*) -> pointer`.
type ApplySpEffect = unsafe extern "system" fn(usize, *const u8) -> usize;

/// Resolved address of the apply function, or `0` before install.
static APPLY: AtomicUsize = AtomicUsize::new(0);

/// Resolved address of `GameManagerImp`'s global, or `0` before install.
static GAME_MANAGER: AtomicUsize = AtomicUsize::new(0);

/// The binding the consumer reads. Unset means unbound.
static KEY_BINDING: AtomicChord = AtomicChord::unset();

/// The id a press applies.
static EFFECT: AtomicI32 = AtomicI32::new(DEFAULT_EFFECT);

/// Whether the chord was down last frame, so a hold is one press.
static WAS_DOWN: AtomicBool = AtomicBool::new(false);

/// Whether a press has already found the chain broken. Logged once, so a player mashing the key on
/// the title screen does not fill the log.
static MISSING_LOGGED: AtomicBool = AtomicBool::new(false);

/// `[net_effects] network`: whether this crate's own applies may reach the session.
static NETWORK: AtomicBool = AtomicBool::new(false);

/// Set only for the length of this crate's own apply call when [`NETWORK`] is off: the send detour
/// withholds the `SpEffect` packet while it is set. Everything else the game sends goes out as before.
static WITHHOLD: AtomicBool = AtomicBool::new(false);

/// Packets withheld so far, reported with each press.
static WITHHELD: AtomicU64 = AtomicU64::new(0);

/// Trampoline to the original packet builder, or `0` when the detour is not in.
static SEND_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// `ds2_rva::SP_EFFECT_SEND`'s shape for pass-through: eight integer-class arguments. The first four
/// are in registers and are integers or pointers; the rest sit in stack slots, where a float (the
/// duration) is passed through bit for bit.
type SendFn = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize);

/// The packet builder's detour: drop the packet while [`WITHHOLD`] is set, else do nothing new.
unsafe extern "system" fn send_detour(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) {
    if WITHHOLD.load(Ordering::Relaxed) {
        WITHHELD.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let original = SEND_ORIGINAL.load(Ordering::Acquire);
    if original != 0 {
        // SAFETY: the trampoline MinHook made for this site, called with the arguments this detour
        // was given, unchanged.
        unsafe {
            let original: SendFn = std::mem::transmute::<usize, SendFn>(original);
            original(a, b, c, d, e, f, g, h);
        }
    }
}

/// Detour the packet builder, after checking its first bytes. `false` leaves sending as the game
/// does it, which is logged, and the feature refuses to arm with `network = false` in that case.
///
/// # Safety
///
/// Patches the game image; must run after `neuter_arxan`.
unsafe fn install_send_detour(base: usize) -> bool {
    let site = base + ds2_rva::SP_EFFECT_SEND as usize;
    let expected = ds2_rva::SP_EFFECT_SEND_PROLOGUE;
    let mut found = [0u8; 8];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(site, &mut found) };
    if !read || found != expected {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=prologue what=sp-effect-send va=0x{site:016x} \
             read={read} saw={found:02x?} want={expected:02x?}"
        ));
        return false;
    }
    // SAFETY: MinHook's own initialiser, no arguments. Already initialised by another crate in this
    // DLL is fine.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return false;
    }
    // SAFETY: the site matched its recorded prologue, and the detour has the same ABI.
    match unsafe { MhHook::new(site as *mut c_void, send_detour as *mut c_void) } {
        Ok(handle) => {
            SEND_ORIGINAL.store(handle.trampoline() as usize, Ordering::Release);
            // SAFETY: the address `MhHook::new` just registered.
            let status = unsafe { MH_EnableHook(site as *mut c_void) };
            if status != MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} install-failed stage=MH_EnableHook what=sp-effect-send \
                     status={status:?}"
                ));
                return false;
            }
            true
        }
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=MH_CreateHook what=sp-effect-send \
                 status={status:?}"
            ));
            false
        }
    }
}

/// How often the watcher looks at the config file.
const POLL_INTERVAL_MS: u64 = 1000;

/// The request, aligned the way a stack local in the game's own callers is.
#[repr(C, align(16))]
struct RequestBuf([u8; ds2_rva::SP_EFFECT_REQUEST_SIZE]);

/// Whether the foreground window belongs to this process, so a key typed into another window is
/// not a press. Same shape as `ds2-voice-chat`'s check.
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

fn read_usize(addr: usize) -> Option<usize> {
    // SAFETY: a fault-tolerant read; an unmapped address is `None`, not a crash.
    unsafe { ds2_game_base::mem::safe_read_usize(addr) }
}

/// `n` consecutive pointer-sized values, in one read.
fn read_words(addr: usize, n: usize) -> Option<Vec<usize>> {
    let mut bytes = vec![0u8; n * size_of::<usize>()];
    // SAFETY: `read_bytes` faults safely; `bytes` is a live buffer of the length it is given.
    if !unsafe { ds2_game_base::mem::read_bytes(addr, &mut bytes) } {
        return None;
    }
    let (words, _) = bytes.as_chunks::<{ size_of::<usize>() }>();
    Some(
        words
            .iter()
            .map(|word| usize::from_le_bytes(*word))
            .collect(),
    )
}

fn read_i32(addr: usize) -> Option<i32> {
    // SAFETY: a fault-tolerant read; an unmapped address is `None`, not a crash.
    unsafe { ds2_game_base::mem::safe_read_i32(addr) }
}

/// The toggle. Only the `Present` consumer touches it, on the game thread; the lock is for Rust,
/// not for a second thread.
static TOGGLE: Mutex<Toggle> = Mutex::new(Toggle::new());

/// Re-applies since the toggle last went on, reported with each one.
static REAPPLIES: AtomicU64 = AtomicU64::new(0);

/// Apply `id` to the controller `ctrl`.
///
/// # Safety
///
/// Game thread only: called from the `Present` clock's consumer, with a controller it just walked.
unsafe fn apply_effect(ctrl: usize, id: i32, reason: ApplyReason) {
    let why = match reason {
        ApplyReason::Enabled => "toggled-on".to_string(),
        ApplyReason::Expired => {
            format!(
                "re-apply #{}",
                REAPPLIES.fetch_add(1, Ordering::Relaxed) + 1
            )
        }
    };
    // SAFETY: this function's own contract, passed on.
    unsafe { apply_effect_because(ctrl, id, &why) };
}

/// Apply `id` to the controller `ctrl` and log it with `why`.
///
/// # Safety
///
/// Game thread only: called from the `Present` clock's consumer, with a controller it just walked.
unsafe fn apply_effect_because(ctrl: usize, id: i32, why: &str) {
    let apply = APPLY.load(Ordering::Acquire);
    if apply == 0 {
        return;
    }
    let request = RequestBuf(request_bytes(id));
    let network = NETWORK.load(Ordering::Relaxed);
    let withheld_before = WITHHELD.load(Ordering::Relaxed);
    // The game sends a local player's applied effect to the session (see `ds2_rva::SP_EFFECT_SEND`);
    // with `network` off the send detour drops that packet for exactly the length of this call.
    WITHHOLD.store(!network, Ordering::Relaxed);
    // SAFETY: `apply` is `ds2_rva::SP_EFFECT_APPLY`, checked against its recorded first bytes at
    // install. It takes the controller and a pointer to the sixteen-byte request, exactly what the
    // game's own callers pass, and this is the game thread, which is where they call it from.
    // `request` outlives the call.
    let returned = unsafe {
        let apply: ApplySpEffect = std::mem::transmute::<usize, ApplySpEffect>(apply);
        apply(ctrl, request.0.as_ptr())
    };
    WITHHOLD.store(false, Ordering::Relaxed);
    let withheld = WITHHELD.load(Ordering::Relaxed) - withheld_before;
    log(format_args!(
        "{LOG_PREFIX} applied effect={id} reason={why} ctrl=0x{ctrl:016x} returned=0x{returned:x} \
         network={network} packets-withheld={withheld}"
    ));
}

/// The `Present` consumer: the selector and its kept effects, then the F9 toggle.
fn on_frame() {
    selector_frame();
    toggle_frame();
}

/// The selector's state. Only the `Present` consumer touches it.
struct SelectorState {
    selector: Selector,
    bindings: Bindings,
    catalog: Vec<CatalogEntry>,
    marked: Vec<i32>,
    keeper: Keeper,
    was_down: [bool; ACTIONS.len()],
    marked_path: Option<PathBuf>,
    ignored_logged: u32,
    missing_logged: bool,
}

static SELECTOR: Mutex<Option<SelectorState>> = Mutex::new(None);

/// Selector keys a watcher thread read out of the config, waiting for the frame consumer.
static PENDING_BINDINGS: Mutex<Option<Bindings>> = Mutex::new(None);

fn write_marked(state: &SelectorState) {
    let Some(path) = &state.marked_path else {
        return;
    };
    let text = marked::render(state.marked.iter().map(|id| {
        let name = state
            .catalog
            .iter()
            .find(|e| e.id == *id)
            .map_or("", |e| e.name.as_str());
        (*id, name)
    }));
    match std::fs::write(path, text) {
        Ok(()) => log(format_args!(
            "{LOG_PREFIX} selector: kept list written ({} ids) to {}",
            state.marked.len(),
            path.display()
        )),
        Err(error) => log(format_args!(
            "{LOG_PREFIX} selector: could not write {}: {error}",
            path.display()
        )),
    }
}

/// One frame of the selector: its keys, the kept effects, and the lines for the overlay.
fn selector_frame() {
    let mut guard = SELECTOR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(state) = guard.as_mut() else {
        return;
    };
    if let Some(bindings) = PENDING_BINDINGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        state.bindings = bindings;
        // A moved binding resets the edges, or a key held during the reload reads as a press.
        state.was_down = [false; ACTIONS.len()];
    }
    let focus = game_has_focus();
    let mut presses = Vec::new();
    for (index, action) in ACTIONS.iter().enumerate() {
        let down = focus
            && state
                .bindings
                .chords(*action)
                .iter()
                .any(|chord| chord_held(*chord, vk_down));
        if is_press(state.was_down[index], down) {
            presses.push(*action);
        }
        state.was_down[index] = down;
    }

    let global = GAME_MANAGER.load(Ordering::Acquire);
    let ctrl = (global != 0)
        .then(|| sp_effect_ctrl(global, read_usize).ok())
        .flatten();
    let on_player = ctrl.and_then(|ctrl| sp_effect_ids(ctrl, read_usize, read_words, read_i32));

    let len = state.catalog.len();
    for action in presses {
        let outcome = state.selector.act(action, len);
        let entry = |row: usize| state.catalog.get(row).map(|e| (e.id, e.name.clone()));
        match outcome {
            Step::Ignored => {
                if state.ignored_logged < 8 {
                    state.ignored_logged += 1;
                    log(format_args!(
                        "{LOG_PREFIX} selector: {action:?} ignored -- the bar is not open \
                         (Alt+9 expands it)"
                    ));
                }
            }
            Step::Moved(row) => {
                if let Some((id, name)) = entry(row) {
                    log(format_args!(
                        "{LOG_PREFIX} selector: cursor {}/{len} id={id} {name}",
                        row + 1
                    ));
                }
            }
            Step::Expanded(on) => log(format_args!(
                "{LOG_PREFIX} selector: bar {}",
                if on { "expanded" } else { "collapsed" }
            )),
            Step::Shown(on) => log(format_args!(
                "{LOG_PREFIX} selector: bar {}",
                if on { "shown" } else { "hidden" }
            )),
            Step::Apply(row) => {
                let Some((id, name)) = entry(row) else {
                    continue;
                };
                match ctrl {
                    // SAFETY: the `Present` clock runs its consumers on the game thread, and
                    // `ctrl` was walked on this frame.
                    Some(ctrl) => unsafe { apply_effect_because(ctrl, id, "selector-preview") },
                    None => log(format_args!(
                        "{LOG_PREFIX} selector: apply id={id} ({name}) -- no character loaded, \
                         nothing applied"
                    )),
                }
            }
            Step::Mark(row) | Step::KeepAdd(row) | Step::KeepRemove(row) => {
                let Some((id, name)) = entry(row) else {
                    continue;
                };
                let changed = match outcome {
                    Step::Mark(_) => {
                        marked::toggle(&mut state.marked, id);
                        true
                    }
                    Step::KeepAdd(_) => marked::add(&mut state.marked, id),
                    _ => marked::remove(&mut state.marked, id),
                };
                let kept = state.marked.contains(&id);
                log(format_args!(
                    "{LOG_PREFIX} selector: {} id={id} ({name}) -- {} kept",
                    match (changed, kept) {
                        (true, true) => "marked (kept applied from now)",
                        (true, false) => "unmarked (no longer re-applied)",
                        (false, true) => "already marked",
                        (false, false) => "was not marked",
                    },
                    state.marked.len()
                ));
                if changed {
                    state.keeper.sync(&state.marked);
                    write_marked(state);
                }
            }
        }
    }

    if !state.keeper.is_empty() {
        if ctrl.is_none() && !state.missing_logged {
            state.missing_logged = true;
            log(format_args!(
                "{LOG_PREFIX} selector: {} kept effects wait for a character",
                state.marked.len()
            ));
        }
        for (id, frame) in state.keeper.frame(on_player.as_deref()) {
            if frame.seen {
                log(format_args!(
                    "{LOG_PREFIX} kept effect={id} seen on the player"
                ));
            }
            if frame.expired {
                log(format_args!(
                    "{LOG_PREFIX} kept effect={id} expired -- re-applying"
                ));
            }
            if frame.never_seen {
                log(format_args!(
                    "{LOG_PREFIX} kept effect={id} was applied but never showed up within {} \
                     frames -- not re-applying until it is marked again or the game restarts",
                    crate::SIGHTING_FRAMES
                ));
            }
            if let (Some(reason), Some(ctrl)) = (frame.apply, ctrl) {
                // SAFETY: the `Present` clock runs its consumers on the game thread, and `ctrl`
                // was walked on this frame.
                unsafe { apply_effect_because(ctrl, id, keep_reason(reason)) };
                state.keeper.applied(id);
            }
        }
    }

    overlay::publish(view(
        &state.selector,
        &state.catalog,
        &state.marked,
        on_player.as_deref().unwrap_or_default(),
    ));
}

/// Load the catalog (the file beside the game in place of the embedded one) and the kept list,
/// and arm the selector. `game_dir` is where both files live; `None` uses the embedded catalog
/// and keeps nothing across launches.
fn arm_selector(game_dir: Option<&std::path::Path>, config_text: Option<&str>) {
    let override_path = game_dir.map(|dir| dir.join(CATALOG_FILE_NAME));
    let from_file = override_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok().map(|text| (path, text)));
    let catalog = match from_file {
        Some((path, text)) => match parse_catalog(&text) {
            Ok(entries) => {
                log(format_args!(
                    "{LOG_PREFIX} selector: catalog {} entries from {}",
                    entries.len(),
                    path.display()
                ));
                entries
            }
            Err(error) => {
                log(format_args!(
                    "{LOG_PREFIX} selector: {} refused ({error:?}) -- using the built-in catalog",
                    path.display()
                ));
                parse_catalog(EMBEDDED_CATALOG).unwrap_or_default()
            }
        },
        None => {
            let entries = parse_catalog(EMBEDDED_CATALOG).unwrap_or_default();
            log(format_args!(
                "{LOG_PREFIX} selector: catalog {} entries (built in)",
                entries.len()
            ));
            entries
        }
    };
    let marked_path = game_dir.map(|dir| dir.join(marked::MARKED_FILE_NAME));
    let marked = marked_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| marked::parse(&text))
        .unwrap_or_default();
    if let Some(path) = &marked_path {
        log(format_args!(
            "{LOG_PREFIX} selector: {} kept ids read from {} {:?}",
            marked.len(),
            path.display(),
            marked
        ));
    }
    let mut bindings = Bindings::default();
    if let Some(text) = config_text {
        let mut messages = Vec::new();
        bindings.apply_config(text, &mut messages);
        for message in messages {
            log(format_args!("{LOG_PREFIX} selector: {message}"));
        }
    }
    let mut keeper = Keeper::default();
    keeper.sync(&marked);
    *SELECTOR
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(SelectorState {
        selector: Selector::default(),
        bindings,
        catalog,
        marked,
        keeper,
        was_down: [false; ACTIONS.len()],
        marked_path,
        ignored_logged: 0,
        missing_logged: false,
    });
}

/// The F9 toggle. Reads the key, flips the toggle on a fresh press, and while the toggle is on
/// looks for the effect on the player and applies it again when it has run out.
fn toggle_frame() {
    let held = KEY_BINDING
        .load()
        .is_some_and(|chord| chord_held(chord, vk_down));
    let down = held && game_has_focus();
    // A run once logged no toggle for F9 presses the player reported. The key read and the focus
    // check are the only two gates between a held key and the TOGGLE line, so the log says which
    // one closed: this line means the key was read held but another window had focus; neither
    // line means `GetAsyncKeyState` never reported the key held on the game thread.
    let held_elsewhere = held && !down;
    if held_elsewhere && !UNFOCUSED_WAS_DOWN.swap(true, Ordering::Relaxed) {
        let count = UNFOCUSED_HELD.fetch_add(1, Ordering::Relaxed) + 1;
        if count <= 8 {
            log(format_args!(
                "{LOG_PREFIX} key held but the game window does not have focus -- not a press \
                 (#{count})"
            ));
        }
    } else if !held_elsewhere {
        UNFOCUSED_WAS_DOWN.store(false, Ordering::Relaxed);
    }
    let pressed = is_press(WAS_DOWN.swap(down, Ordering::Relaxed), down);
    let mut toggle = TOGGLE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !pressed && !toggle.enabled() {
        return;
    }
    let global = GAME_MANAGER.load(Ordering::Acquire);
    if global == 0 {
        return;
    }
    let id = EFFECT.load(Ordering::Relaxed);
    let ctrl = sp_effect_ctrl(global, read_usize);
    if let Err(link) = ctrl
        && pressed
        && !MISSING_LOGGED.swap(true, Ordering::Relaxed)
    {
        log(format_args!(
            "{LOG_PREFIX} {link:?} is null (no character loaded?) -- the toggle still flips, and \
             the effect is applied once a character is; later presses like it are not logged"
        ));
    }
    let active = ctrl
        .ok()
        .and_then(|ctrl| sp_effect_active(ctrl, id, read_usize, read_words, read_i32));
    let frame = toggle.frame(pressed, active);
    if let Some(on) = frame.toggled {
        overlay::set_glyph(on);
        announce(on);
    }
    match frame.toggled {
        Some(true) => {
            REAPPLIES.store(0, Ordering::Relaxed);
            log(format_args!(
                "{LOG_PREFIX} ===== TOGGLE ON ===== effect={id} -- applying it now and again each \
                 time it runs out, until the key is pressed again"
            ));
        }
        Some(false) => log(format_args!(
            "{LOG_PREFIX} ===== TOGGLE OFF ===== effect={id} -- no further re-apply ({} re-applies \
             this time on)",
            REAPPLIES.load(Ordering::Relaxed)
        )),
        None => {}
    }
    if frame.seen {
        log(format_args!("{LOG_PREFIX} effect={id} seen on the player"));
    }
    if frame.expired {
        log(format_args!(
            "{LOG_PREFIX} effect={id} expired -- no longer on the player"
        ));
    }
    if frame.never_seen {
        log(format_args!(
            "{LOG_PREFIX} effect={id} was applied but never showed up on the player within {} \
             frames -- not re-applying until the key is pressed again",
            crate::SIGHTING_FRAMES
        ));
    }
    if let (Some(reason), Ok(ctrl)) = (frame.apply, ctrl) {
        // SAFETY: the `Present` clock runs its consumers on the game thread, and `ctrl` was walked
        // on this frame.
        unsafe { apply_effect(ctrl, id, reason) };
        toggle.applied();
    }
}

/// Apply one config text.
///
/// A value that does not parse leaves the one already in force and says so; falling back to the
/// default would move the key, or change the effect, to something the player did not ask for.
fn apply_config(text: &str, first: bool) {
    let network = network_setting(text);
    if NETWORK.swap(network, Ordering::Relaxed) != network || first {
        log(format_args!(
            "{LOG_PREFIX} {CONFIG_KEY_NETWORK} = {network} -- {}",
            if network {
                "an applied effect is sent to the session as the game sends it"
            } else {
                "an applied effect stays on this machine"
            }
        ));
    }
    match effect_setting(text) {
        EffectSetting::NotSet => {
            if first {
                log(format_args!(
                    "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_EFFECT} not set -- default \
                     {DEFAULT_EFFECT}"
                ));
            }
        }
        EffectSetting::Set(id) => {
            if EFFECT.swap(id, Ordering::Relaxed) != id || first {
                log(format_args!("{LOG_PREFIX} effect = {id}"));
            }
        }
        EffectSetting::Invalid(value) => log(format_args!(
            "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_EFFECT} = {value:?} is not a positive \
             id -- keeping the effect already in force"
        )),
    }
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

/// The selector keys a config text binds, from the defaults.
fn selector_bindings(text: &str) -> (Bindings, Vec<String>) {
    let mut bindings = Bindings::default();
    let mut messages = Vec::new();
    bindings.apply_config(text, &mut messages);
    (bindings, messages)
}

fn watch(path: PathBuf, initial: Option<String>) {
    let mut hot = HotFile::with_interval(path, POLL_INTERVAL_MS);
    let mut bindings = selector_bindings(initial.as_deref().unwrap_or_default()).0;
    loop {
        match hot.poll() {
            Some(FileChange::Text(text)) => {
                apply_config(&text, false);
                let (fresh, messages) = selector_bindings(&text);
                if fresh != bindings {
                    for message in messages {
                        log(format_args!("{LOG_PREFIX} selector: {message}"));
                    }
                    bindings = fresh.clone();
                    *PENDING_BINDINGS
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(fresh);
                }
            }
            Some(FileChange::Missing) => log(format_args!(
                "{LOG_PREFIX} config file disappeared -- keeping the key and effect already in \
                 force"
            )),
            None => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
    }
}

/// Check the apply function's first bytes, arm the binding, and register the frame consumer.
///
/// Patches nothing. The consumer runs from `ds2-overlay`'s `Present` detour, which this starts.
///
/// # Safety
///
/// Reads the loaded game image and, from then on, calls into it on every press. Must run after
/// `neuter_arxan`, which in practice means the loader's Arxan callback.
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

    // The function this crate calls is checked like a patched one would be: a bad address here is
    // a jump into arbitrary code on the game thread.
    let apply = base + ds2_rva::SP_EFFECT_APPLY as usize;
    let expected = ds2_rva::SP_EFFECT_APPLY_PROLOGUE;
    let mut found = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(apply, &mut found) };
    if !read || found != expected {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=prologue what=apply-sp-effect va=0x{apply:016x} \
             read={read} saw={found:02x?} want={expected:02x?}"
        ));
        return Outcome::default();
    }
    // Without the send detour the game would share every press with the session, which is the one
    // thing this feature promises not to do by default -- so no detour, no key.
    // SAFETY: after neuter_arxan, per this function's contract.
    if !unsafe { install_send_detour(base) } {
        log(format_args!(
            "{LOG_PREFIX} not armed -- the packet builder could not be detoured, so an applied \
             effect could not be kept off the network"
        ));
        return Outcome::default();
    }
    APPLY.store(apply, Ordering::Release);
    GAME_MANAGER.store(base + ds2_rva::GAME_MANAGER_IMP as usize, Ordering::Release);

    // The defaults are in force before the file is read, so a missing file still leaves a key.
    if let Some(chord) = default_chord() {
        KEY_BINDING.store(chord);
    }
    let config_text = request
        .config_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    arm_selector(
        request.config_path.as_ref().and_then(|path| path.parent()),
        config_text.as_deref(),
    );
    if let Some(path) = request.config_path.clone() {
        if let Some(text) = &config_text {
            apply_config(text, true);
        }
        let initial = config_text.clone();
        std::thread::spawn(move || watch(path, initial));
    } else {
        log(format_args!(
            "{LOG_PREFIX} no config path -- key {DEFAULT_KEY}, effect {DEFAULT_EFFECT} for this \
             session"
        ));
    }

    if !ds2_overlay::frame_hook::add_frame_hook(on_frame) {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=frame-hook -- every Present frame-hook slot is \
             taken"
        ));
        return Outcome::default();
    }
    if !ds2_overlay::install() {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=present -- ds2-overlay could not start its Present \
             watcher, so the clock never ticks"
        ));
        return Outcome::default();
    }
    // The bar is optional: without it the keys, the preview and the kept effects still work, and
    // the log says why nothing is drawn.
    overlay::install();

    let key = KEY_BINDING
        .load()
        .filter(|chord| chord.vk != 0)
        .map_or_else(|| "none".to_string(), chord_name);
    log(format_args!(
        "{LOG_PREFIX} armed key={key} effect={} apply=0x{apply:016x} -- read on the Present clock",
        EFFECT.load(Ordering::Relaxed)
    ));
    Outcome { installed: true }
}
