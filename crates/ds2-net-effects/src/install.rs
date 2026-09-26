//! The frame consumer that reads the key and makes the call, and the watcher that lets the key and
//! the id move while the game runs.

use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

use ds2_hotkey_config::chord_name;
use ds2_hotkey_config::keys::Chord;
use ds2_hotkey_config::live::AtomicChord;
use ds2_hotkey_config::reload::{FileChange, HotFile};

use crate::{
    CONFIG_KEY_EFFECT, CONFIG_KEY_KEYBOARD, CONFIG_SECTION, DEFAULT_EFFECT, DEFAULT_KEY,
    EffectSetting, KeySetting, LOG_PREFIX, chord_held, default_chord, effect_setting, is_press,
    key_setting, request_bytes, sp_effect_ctrl,
};

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
    fn GetCurrentProcessId() -> u32;
}

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

/// Apply the configured effect to the local player.
///
/// # Safety
///
/// Game thread only: called from the `Present` clock's consumer.
unsafe fn apply_effect() {
    let apply = APPLY.load(Ordering::Acquire);
    let global = GAME_MANAGER.load(Ordering::Acquire);
    if apply == 0 || global == 0 {
        return;
    }
    let id = EFFECT.load(Ordering::Relaxed);
    let ctrl = match sp_effect_ctrl(global, |addr| {
        // SAFETY: a fault-tolerant read; an unmapped address is `None`, not a crash.
        unsafe { ds2_game_base::mem::safe_read_usize(addr) }
    }) {
        Ok(ctrl) => ctrl,
        Err(link) => {
            if !MISSING_LOGGED.swap(true, Ordering::Relaxed) {
                log(format_args!(
                    "{LOG_PREFIX} press ignored -- {link:?} is null (no character loaded?); \
                     later presses like it are not logged"
                ));
            }
            return;
        }
    };
    let request = RequestBuf(request_bytes(id));
    // SAFETY: `apply` is `ds2_rva::SP_EFFECT_APPLY`, checked against its recorded first bytes at
    // install. It takes the controller and a pointer to the sixteen-byte request, exactly what the
    // game's own callers pass, and this is the game thread, which is where they call it from.
    // `request` outlives the call.
    let returned = unsafe {
        let apply: ApplySpEffect = std::mem::transmute::<usize, ApplySpEffect>(apply);
        apply(ctrl, request.0.as_ptr())
    };
    log(format_args!(
        "{LOG_PREFIX} applied effect={id} ctrl=0x{ctrl:016x} returned=0x{returned:x}"
    ));
}

/// The `Present` consumer. Reads the key and applies the effect on a fresh press.
fn on_frame() {
    let down = game_has_focus()
        && KEY_BINDING
            .load()
            .is_some_and(|chord| chord_held(chord, vk_down));
    if is_press(WAS_DOWN.swap(down, Ordering::Relaxed), down) {
        // SAFETY: the `Present` clock runs its consumers on the game thread.
        unsafe { apply_effect() };
    }
}

/// Apply one config text.
///
/// A value that does not parse leaves the one already in force and says so; falling back to the
/// default would move the key, or change the effect, to something the player did not ask for.
fn apply_config(text: &str, first: bool) {
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

fn watch(path: PathBuf) {
    let mut hot = HotFile::with_interval(path, POLL_INTERVAL_MS);
    loop {
        match hot.poll() {
            Some(FileChange::Text(text)) => apply_config(&text, false),
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
/// Patches nothing. The consumer only runs while `ds2-invasion-path`'s `Present` detour is
/// installed; the loader reports when it is not.
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
    APPLY.store(apply, Ordering::Release);
    GAME_MANAGER.store(base + ds2_rva::GAME_MANAGER_IMP as usize, Ordering::Release);

    // The defaults are in force before the file is read, so a missing file still leaves a key.
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
            "{LOG_PREFIX} no config path -- key {DEFAULT_KEY}, effect {DEFAULT_EFFECT} for this \
             session"
        ));
    }

    if !ds2_invasion_path::frame_hook::add_frame_hook(on_frame) {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=frame-hook -- every Present frame-hook slot is \
             taken"
        ));
        return Outcome::default();
    }

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
