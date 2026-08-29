//! The two detours that find the Inventory tab, the tick that reads the button, and the watcher
//! that lets the button move while the game runs.

use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};
use ds2_hotkey_config::keys::{Chord, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_SHIFT};
use ds2_hotkey_config::kv::KeyValues;
use ds2_hotkey_config::live::AtomicChord;
use ds2_hotkey_config::reload::{FileChange, HotFile};
use ds2_hotkey_config::{chord_name, parse_chord};

use crate::{CONFIG_KEY_KEYBOARD, CONFIG_KEY_PAD, CONFIG_SECTION, LOG_PREFIX};

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

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
    /// The config file to watch for the binding. `None` disables live rebinding and leaves the
    /// built-in defaults in force -- which is a degraded mode, not the normal one.
    pub config_path: Option<PathBuf>,
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Outcome {
    /// Both detours went in and the tick is registered.
    pub installed: bool,
}

/// The live `FeGroupInGameMenuInventory2`, or `0` when the Inventory tab is not open.
///
/// Written by both detours and read by the tick, all on the game thread; atomic anyway because
/// "the game thread" is an expectation this crate cannot enforce on MinHook's behalf.
static LIVE_GROUP: AtomicUsize = AtomicUsize::new(0);

/// Trampolines back to the original constructor and destructor.
static CTOR_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static DTOR_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Resolved address of [`ds2_rva::FE_INVENTORY_SORT_DIALOG_OPEN`], after its prologue was checked.
static DIALOG_OPEN: AtomicUsize = AtomicUsize::new(0);

/// Resolved address of the object vtable a cached pointer must still carry.
static GROUP_VTABLE: AtomicUsize = AtomicUsize::new(0);

/// The keyboard binding in force. Empty until [`install`] applies the config.
static KEY_BINDING: AtomicChord = AtomicChord::unset();

/// The XInput button mask in force, `0` for "no controller binding".
static PAD_BINDING: AtomicU16 = AtomicU16::new(0);

/// Whether the button was down on the previous tick. The edge detector.
static WAS_DOWN: AtomicBool = AtomicBool::new(false);

/// How many times the dialog has been opened from here. Logged for the first few only.
static OPENED: AtomicU32 = AtomicU32::new(0);

/// `XInputGetState`, resolved once out of the copy the game itself imports.
static XINPUT_GET_STATE: AtomicUsize = AtomicUsize::new(0);

/// How many presses of each kind -- opened, refused -- reach the log before it goes quiet. A fact
/// established three times is established; a fourth line is a write on the game's own frame.
const LOGGED_LINES: u32 = 3;

/// How many presses have been refused. Bounds the refusal logging, nothing else.
static REFUSED: AtomicU32 = AtomicU32::new(0);

/// The default keyboard binding. **A placeholder, not a recommendation**: the button worth having
/// here is whichever one the player's fingers already know, and no default can guess that.
const DEFAULT_KEY: &str = "F7";

/// XInput button names accepted in the config, and their `wButtons` masks.
///
/// Names are the Xbox ones because XInput is an Xbox API and its own constants are spelled this
/// way; a DualShock player reading `Y` gets Triangle, which is the same physical position.
pub const PAD_BUTTONS: [(&str, u16); 14] = [
    ("dpad_up", 0x0001),
    ("dpad_down", 0x0002),
    ("dpad_left", 0x0004),
    ("dpad_right", 0x0008),
    ("start", 0x0010),
    ("back", 0x0020),
    ("lthumb", 0x0040),
    ("rthumb", 0x0080),
    ("lb", 0x0100),
    ("rb", 0x0200),
    ("a", 0x1000),
    ("b", 0x2000),
    ("x", 0x4000),
    ("y", 0x8000),
];

/// `XINPUT_STATE`. 16 bytes, and the layout is the one the header documents.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XInputState {
    packet: u32,
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumbs: [i16; 4],
}

/// `DWORD XInputGetState(DWORD dwUserIndex, XINPUT_STATE *pState)`.
type XInputGetStateFn = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

/// The constructor: `fn(this, ?, ?) -> this`. The later two arguments are passed straight through.
type CtorFn = unsafe extern "system" fn(*mut u8, usize, usize) -> *mut u8;

/// The scalar deleting destructor: `fn(this, flags) -> this`.
type DtorFn = unsafe extern "system" fn(*mut u8, u32) -> *mut u8;

/// The sort dialog: `fn(this)`.
type DialogOpenFn = unsafe extern "system" fn(*mut u8);

/// Record the group, then let the game build it.
///
/// The record happens BEFORE the original runs and is not conditional on it: the original returns
/// `this` unchanged, and a detour that waited would be a window in which the object exists and this
/// crate does not know it.
unsafe extern "system" fn ctor_detour(this: *mut u8, second: usize, third: usize) -> *mut u8 {
    LIVE_GROUP.store(this as usize, Ordering::Release);
    let trampoline = CTOR_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return this;
    }
    // SAFETY: MinHook published this trampoline for this site, and the arguments are the caller's
    // own, forwarded unaltered.
    let original: CtorFn = unsafe { std::mem::transmute::<usize, CtorFn>(trampoline) };
    unsafe { original(this, second, third) }
}

/// Forget the group, then let the game destroy it.
///
/// **Compared before clearing.** Two objects of this class are not expected to exist at once, but a
/// destructor for some OTHER instance must not blank a record that still points at a live one.
unsafe extern "system" fn dtor_detour(this: *mut u8, flags: u32) -> *mut u8 {
    let _ = LIVE_GROUP.compare_exchange(this as usize, 0, Ordering::AcqRel, Ordering::Relaxed);
    let trampoline = DTOR_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return this;
    }
    // SAFETY: MinHook published this trampoline for this site; both arguments are the caller's own.
    let original: DtorFn = unsafe { std::mem::transmute::<usize, DtorFn>(trampoline) };
    unsafe { original(this, flags) }
}

/// Whether the foreground window belongs to THIS process.
///
/// The question is "is the player pressing this at US", and the process is the right granularity:
/// a game has several windows and under Proton there is a Wine frame in between, so comparing one
/// `HWND` would be an equality nobody checked. Copied in shape from `ds2-build-import`, where that
/// mistake was made and corrected.
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

/// Is this virtual key down right now?
fn vk_down(vk: i32) -> bool {
    // SAFETY: a plain Win32 call taking an integer. The HIGH bit is "down now"; the low bit --
    // "pressed since the last call" -- is deliberately unused, being per-caller state the game's
    // own polling would race us for.
    unsafe { GetAsyncKeyState(vk) < 0 }
}

/// Is the whole chord down, modifiers included?
fn chord_down_async(chord: Chord) -> bool {
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

/// Is the bound controller button down on any connected pad?
///
/// `XInputGetState` is resolved out of the module the GAME imports rather than a copy loaded here:
/// `DarkSoulsII.exe` has two `XINPUT1_3.dll` thunks, so it is already in the process and asking
/// Windows for it again could bind a different implementation than the one the game is reading.
/// When the game is not using a pad the module is still loaded and every index reports "not
/// connected", which is a `false` and not an error.
fn pad_down(mask: u16) -> bool {
    if mask == 0 {
        return false;
    }
    let mut raw = XINPUT_GET_STATE.load(Ordering::Acquire);
    if raw == 0 {
        // SAFETY: both are plain Win32 calls on NUL-terminated literals.
        raw = unsafe {
            let module = GetModuleHandleA(c"XINPUT1_3.dll".as_ptr().cast());
            if module.is_null() {
                0
            } else {
                GetProcAddress(module, c"XInputGetState".as_ptr().cast()) as usize
            }
        };
        if raw == 0 {
            return false;
        }
        XINPUT_GET_STATE.store(raw, Ordering::Release);
    }
    // SAFETY: `raw` came from `GetProcAddress` for this exact export, whose signature is the one
    // `XInputGetStateFn` spells.
    let get_state: XInputGetStateFn =
        unsafe { std::mem::transmute::<usize, XInputGetStateFn>(raw) };
    for index in 0..4u32 {
        let mut state = XInputState::default();
        // SAFETY: `state` is a live, correctly sized `XINPUT_STATE`.
        if unsafe { get_state(index, &raw mut state) } == 0 && state.buttons & mask != 0 {
            return true;
        }
    }
    false
}

/// Read one pointer-sized field out of the object, faulting safely.
///
/// **Not a raw `read()`, and the distinction is the whole point of the caller.** The reason to
/// check a cached pointer's vtable is that the pointer might be stale; a stale pointer's memory can
/// be unmapped, and a raw dereference to find that out crashes the game instead of reporting it.
fn read_field(group: usize, offset: usize) -> Option<usize> {
    let mut bytes = [0u8; std::mem::size_of::<usize>()];
    // SAFETY: `read_bytes` probes rather than trusting -- an unmapped address is `false`, not a
    // fault.
    if unsafe { ds2_game_base::mem::read_bytes(group + offset, &mut bytes) } {
        Some(usize::from_le_bytes(bytes))
    } else {
        None
    }
}

/// Open the sort dialog on the live group, if there is one.
///
/// # Safety
///
/// Game thread only, from the pause menu's own per-frame update. It calls into the menu's dialog
/// machinery, which is not safe to enter from a worker thread.
unsafe fn open_dialog() {
    let group = LIVE_GROUP.load(Ordering::Acquire);
    if group == 0 {
        note_refusal(format_args!(
            "{LOG_PREFIX} press ignored -- the Inventory tab is not open"
        ));
        return;
    }
    let wanted = GROUP_VTABLE.load(Ordering::Acquire);
    let Some(live) = read_field(group, 0) else {
        note_refusal(format_args!(
            "{LOG_PREFIX} press ignored -- cached group 0x{group:016x} is not readable"
        ));
        LIVE_GROUP.store(0, Ordering::Release);
        return;
    };
    if wanted != 0 && live != wanted {
        // THE RECORD IS STALE AND THE DESTRUCTOR DID NOT SAY SO. Nothing is called on it.
        note_refusal(format_args!(
            "{LOG_PREFIX} press ignored -- cached group 0x{group:016x} carries vtable \
             0x{live:016x}, wanted 0x{wanted:016x}"
        ));
        LIVE_GROUP.store(0, Ordering::Release);
        return;
    }
    let open = DIALOG_OPEN.load(Ordering::Acquire);
    if open == 0 {
        return;
    }
    let count = OPENED.fetch_add(1, Ordering::Relaxed);
    if count < LOGGED_LINES {
        let busy = read_field(group, ds2_rva::FE_INVENTORY_GROUP_BUSY_OFFSET).unwrap_or(0);
        log(format_args!(
            "{LOG_PREFIX} opening the sort dialog group=0x{group:016x} busy={busy}"
        ));
    }
    // SAFETY: the game thread, inside the pause menu's own update, with a group whose vtable was
    // just checked. The callee takes one argument and guards itself on `[this+0x58]`.
    let dialog: DialogOpenFn = unsafe { std::mem::transmute::<usize, DialogOpenFn>(open) };
    unsafe { dialog(group as *mut u8) };
}

/// Log a refused press, but only the first few.
///
/// **The tick runs on the pause menu's own frame, and the loader's log sink `sync_all`s every
/// line.** An unbounded log line on that path is not a diagnostic, it is a stall: a per-open probe
/// left armed in `ds2-menu-row` froze the pause menu for exactly this reason, and a player whose
/// button is one they press often would hit this every frame they held it.
fn note_refusal(args: std::fmt::Arguments<'_>) {
    if REFUSED.fetch_add(1, Ordering::Relaxed) < LOGGED_LINES {
        log(args);
    }
}

/// Read the button once per frame and act on the PRESS, not on the hold.
///
/// Registered with `ds2-menu-row`, so it runs on the game thread from the pause menu's own update
/// -- which is also the only time it could do anything.
fn tick() {
    let focused = game_has_focus();
    let key = KEY_BINDING.load();
    let pad = PAD_BINDING.load(Ordering::Relaxed);
    let down = focused && (key.is_some_and(chord_down_async) || pad_down(pad));
    if WAS_DOWN.swap(down, Ordering::Relaxed) || !down {
        return;
    }
    // SAFETY: this is the game thread, called from the pause menu's per-frame update.
    unsafe { open_dialog() };
}

/// Apply one config text: the keyboard name, then the pad name.
///
/// A value that does not parse leaves the binding that was already working in force and says so.
/// The alternative -- falling back to the default -- moves the button somewhere the player did not
/// ask for, which is indistinguishable from a broken feature.
fn apply_config(text: &str, first: bool) {
    let parsed = KeyValues::parse(text);
    match parsed.get(CONFIG_SECTION, CONFIG_KEY_KEYBOARD) {
        Some(raw) => {
            let value = raw.trim().trim_matches('"');
            if value.is_empty() {
                KEY_BINDING.store(Chord {
                    modifiers: 0,
                    vk: 0,
                    dik: None,
                });
                log(format_args!(
                    "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_KEYBOARD} is empty -- no keyboard \
                     binding"
                ));
            } else {
                match parse_chord(value) {
                    Ok(chord) => {
                        let before = KEY_BINDING.load();
                        if before != Some(chord) {
                            KEY_BINDING.store(chord);
                            // A MOVED BINDING RESETS THE EDGE. Without this a key held at the
                            // moment of the reload reads as a fresh press nobody made.
                            WAS_DOWN.store(false, Ordering::Relaxed);
                            log(format_args!(
                                "{LOG_PREFIX} keyboard binding = {}",
                                chord_name(chord)
                            ));
                        }
                    }
                    Err(error) => log(format_args!(
                        "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_KEYBOARD} = {value:?} not \
                         understood ({error:?}) -- keeping the binding already in force"
                    )),
                }
            }
        }
        None if first => log(format_args!(
            "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_KEYBOARD} not set -- default {DEFAULT_KEY}"
        )),
        None => {}
    }

    match parsed.get(CONFIG_SECTION, CONFIG_KEY_PAD) {
        Some(raw) => {
            let value = raw.trim().trim_matches('"');
            if value.is_empty() {
                PAD_BINDING.store(0, Ordering::Relaxed);
            } else {
                let wanted = value.to_ascii_lowercase();
                match PAD_BUTTONS.iter().find(|(name, _)| *name == wanted) {
                    Some((name, mask)) => {
                        if PAD_BINDING.swap(*mask, Ordering::Relaxed) != *mask {
                            WAS_DOWN.store(false, Ordering::Relaxed);
                            log(format_args!("{LOG_PREFIX} controller binding = {name}"));
                        }
                    }
                    None => log(format_args!(
                        "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_PAD} = {value:?} is not a \
                         button name -- keeping the binding already in force. Names: {}",
                        PAD_BUTTONS
                            .iter()
                            .map(|(name, _)| *name)
                            .collect::<Vec<_>>()
                            .join(" ")
                    )),
                }
            }
        }
        None if first => log(format_args!(
            "{LOG_PREFIX} [{CONFIG_SECTION}] {CONFIG_KEY_PAD} not set -- no controller binding"
        )),
        None => {}
    }
}

/// Watch the config file so the button can move while the game runs.
///
/// A thread rather than the tick: the tick is the game's own frame, and a filesystem read there is
/// a stall on the render path for a value that changes about as often as never. The tick only ever
/// does an atomic load of what this thread published.
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

/// How often the watcher looks at the config file.
const POLL_INTERVAL_MS: u64 = 1000;

/// Hook one site after checking the bytes it starts with.
///
/// The prologue check is not ceremony: an RVA is a number, and on a build these were not read from
/// it points into the middle of some other function that MinHook would happily patch. Refusing
/// costs one comparison and is the difference between "the mod did nothing" and "the mod corrupted
/// an unrelated function".
unsafe fn hook(
    base: usize,
    rva: u32,
    expected: &[u8],
    detour: *mut c_void,
    trampoline: &AtomicUsize,
    what: &str,
) -> bool {
    let site = base + rva as usize;
    let mut found = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(site, &mut found) };
    if !read || found != expected {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=prologue what={what} va=0x{site:016x} read={read} \
             saw={found:02x?} want={expected:02x?}"
        ));
        return false;
    }
    match unsafe { MhHook::new(site as *mut c_void, detour) } {
        Ok(handle) => {
            // Published BEFORE the site is patched, so a detour that fires immediately cannot read
            // a zero and drop the original -- which for the constructor would mean an Inventory tab
            // that was never built.
            trampoline.store(handle.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} hooked {what} rva=0x{rva:08x} va=0x{site:016x}"
                ));
                true
            } else {
                log(format_args!(
                    "{LOG_PREFIX} install-failed stage=MH_EnableHook what={what} status={status:?}"
                ));
                false
            }
        }
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=MH_CreateHook what={what} status={status:?}"
            ));
            false
        }
    }
}

/// Install both detours and register the tick.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan`, which in
/// practice means the loader's Arxan callback, and BEFORE `ds2_menu_row::install` -- that call
/// seals the tick registry as its first act.
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

    // THE FUNCTION THIS CRATE CALLS IS CHECKED LIKE THE ONES IT PATCHES. Nothing is hooked here --
    // it is called directly -- and that makes the check more important rather than less: a bad
    // address is a jump into arbitrary code on the game thread instead of a refused patch.
    let open_site = base + ds2_rva::FE_INVENTORY_SORT_DIALOG_OPEN as usize;
    let mut prologue = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(open_site, &mut prologue) };
    if !read || prologue != ds2_rva::FE_INVENTORY_SORT_DIALOG_OPEN_PROLOGUE {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=prologue what=sort-dialog va=0x{open_site:016x} \
             read={read} saw={prologue:02x?} want={:02x?}",
            ds2_rva::FE_INVENTORY_SORT_DIALOG_OPEN_PROLOGUE
        ));
        return Outcome::default();
    }
    DIALOG_OPEN.store(open_site, Ordering::Release);
    GROUP_VTABLE.store(
        base + ds2_rva::FE_INVENTORY_GROUP_VTABLE as usize,
        Ordering::Release,
    );

    // MinHook is statically linked into this DLL, so ALREADY_INITIALIZED can only mean this ran
    // twice. Treat it as success.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome::default();
    }

    let ctor = unsafe {
        hook(
            base,
            ds2_rva::FE_INVENTORY_GROUP_CTOR,
            &ds2_rva::FE_INVENTORY_GROUP_CTOR_PROLOGUE,
            ctor_detour as *mut c_void,
            &CTOR_TRAMPOLINE,
            "inventory-group-ctor",
        )
    };
    if !ctor {
        return Outcome::default();
    }
    // THE DESTRUCTOR IS NOT OPTIONAL. Without it the record survives the tab closing and the next
    // press calls a shipped function on freed memory. If it refuses, the constructor hook stays in
    // (MinHook has no removal here) but the record is never populated, so nothing is ever called.
    let dtor = unsafe {
        hook(
            base,
            ds2_rva::FE_INVENTORY_GROUP_DTOR,
            &ds2_rva::FE_INVENTORY_GROUP_DTOR_PROLOGUE,
            dtor_detour as *mut c_void,
            &DTOR_TRAMPOLINE,
            "inventory-group-dtor",
        )
    };
    if !dtor {
        DIALOG_OPEN.store(0, Ordering::Release);
        log(format_args!(
            "{LOG_PREFIX} disarmed -- no destructor hook means a stale group pointer, and a stale \
             group pointer is a call into freed memory"
        ));
        return Outcome::default();
    }

    // The default is in force before the file is read, so a missing or unreadable config still
    // leaves a working button rather than a silent feature.
    if let Ok(chord) = parse_chord(DEFAULT_KEY) {
        KEY_BINDING.store(chord);
    }
    if let Some(path) = request.config_path.clone() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            apply_config(&text, true);
        }
        std::thread::spawn(move || watch(path));
    } else {
        log(format_args!(
            "{LOG_PREFIX} no config path -- the binding is {DEFAULT_KEY} and cannot be changed \
             without a restart"
        ));
    }

    if !ds2_menu_row::add_tick(tick) {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=add-tick -- the button would never be read"
        ));
        return Outcome::default();
    }

    log(format_args!(
        "{LOG_PREFIX} armed -- the shipped ① Sort prompt still works and is untouched"
    ));
    Outcome { installed: true }
}
