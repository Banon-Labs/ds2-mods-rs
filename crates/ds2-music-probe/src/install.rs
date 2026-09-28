//! The four import-slot detours, the log, and the install.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Instant;

use ds2_game_base::mem::{safe_read_u32, safe_read_usize};

use crate::fmod::{
    self, EventInfo, ORIGINAL_GET_STATE, ORIGINAL_SET_PAUSED, ORIGINAL_START, ORIGINAL_STOP,
};
use crate::{LOG_FIRST_ANY_EVENTS, LOG_PREFIX, engine, is_music_name, store};

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
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

unsafe extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn GetCurrentThreadId() -> u32;
}

const PAGE_READWRITE: u32 = 0x04;

/// Returned by a detour whose original was never published, which the publish-before-write order
/// in [`patch_import`] rules out. Any non-zero result is a failure to the game's callers.
const NOT_CALLED: i32 = -1;

type StartFn = unsafe extern "system" fn(usize) -> i32;
type BoolArgFn = unsafe extern "system" fn(usize, u8) -> i32;
type GetStateFn = unsafe extern "system" fn(usize, *mut u32) -> i32;

/// Events of any name logged so far; see [`LOG_FIRST_ANY_EVENTS`].
static ANY_LOGGED: AtomicU32 = AtomicU32::new(0);
static CLOCK_REPORTED: AtomicBool = AtomicBool::new(false);

fn origin() -> &'static Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now)
}

/// Milliseconds since install.
pub(crate) fn now_ms() -> u64 {
    origin().elapsed().as_millis() as u64
}

/// The map index the player stands in, `None` at the title and during a load.
pub(crate) fn map_index() -> Option<u32> {
    let base = ds2_game_base::mem::game_module_base().ok()?;
    // SAFETY: fault-safe reads at every hop. `GameManagerImp -> +0x38 MapManager -> +0x170` is the
    // chain `ds2-invasion-path` reads; see the two constants for the evidence.
    let index = unsafe {
        safe_read_usize(base + ds2_rva::GAME_MANAGER_IMP as usize)
            .filter(|gm| *gm != 0)
            .and_then(|gm| safe_read_usize(gm + ds2_rva::GAME_MANAGER_MAP_MANAGER_OFFSET))
            .filter(|mm| *mm != 0)
            .and_then(|mm| safe_read_u32(mm + ds2_rva::MAP_MANAGER_PLAYER_MAP_INDEX_OFFSET))
    }?;
    (index != ds2_rva::MAP_INDEX_NONE).then_some(index)
}

/// Log an event call if it is music, or one of the first few events of the run.
fn report(kind: &str, event: usize, extra: std::fmt::Arguments<'_>) -> Option<EventInfo> {
    let info = fmod::event_info(event, false);
    let music = info.as_ref().is_some_and(|i| is_music_name(&i.name));
    if !music && ANY_LOGGED.fetch_add(1, Ordering::Relaxed) >= LOG_FIRST_ANY_EVENTS {
        return info;
    }
    let map = map_index().map_or_else(|| "none".to_owned(), |m| m.to_string());
    match &info {
        Some(i) => log(format_args!(
            "{LOG_PREFIX} {kind} name={} handle=0x{event:x} music={music} map={map} project={} \
             system={} event-pos={}ms event-len={}ms{extra}",
            i.name, i.project_id, i.system_id, i.position_ms, i.length_ms
        )),
        None => log(format_args!(
            "{LOG_PREFIX} {kind} name=? handle=0x{event:x} getInfo-failed map={map}{extra}"
        )),
    }
    info
}

unsafe extern "system" fn detour_start(event: usize) -> i32 {
    let Some(original) = load::<StartFn>(&ORIGINAL_START) else {
        return NOT_CALLED;
    };
    // SAFETY: the import the Windows loader resolved, with the game's own argument.
    let rc = unsafe { original(event) };
    let info = report("start", event, format_args!(" rc={rc}"));
    engine::on_start(event, info.as_ref());
    engine::tick();
    rc
}

unsafe extern "system" fn detour_stop(event: usize, immediate: u8) -> i32 {
    let Some(original) = load::<BoolArgFn>(&ORIGINAL_STOP) else {
        return NOT_CALLED;
    };
    // Named before the call: a stopped event may no longer answer `getInfo`.
    report("stop", event, format_args!(" immediate={}", immediate != 0));
    engine::on_stop(event, immediate != 0);
    // SAFETY: the import the Windows loader resolved, with the game's own arguments.
    unsafe { original(event, immediate) }
}

unsafe extern "system" fn detour_set_paused(event: usize, paused: u8) -> i32 {
    let Some(original) = load::<BoolArgFn>(&ORIGINAL_SET_PAUSED) else {
        return NOT_CALLED;
    };
    // SAFETY: the import the Windows loader resolved, with the game's own arguments.
    let rc = unsafe { original(event, paused) };
    let info = report(
        "setPaused",
        event,
        format_args!(" paused={} rc={rc}", paused != 0),
    );
    engine::on_paused(event, paused != 0, info.as_ref());
    engine::tick();
    rc
}

unsafe extern "system" fn detour_get_state(event: usize, state: *mut u32) -> i32 {
    let Some(original) = load::<GetStateFn>(&ORIGINAL_GET_STATE) else {
        return NOT_CALLED;
    };
    // SAFETY: the import the Windows loader resolved, with the game's own arguments.
    let rc = unsafe { original(event, state) };
    if !CLOCK_REPORTED.swap(true, Ordering::Relaxed) {
        // SAFETY: takes nothing and cannot fail.
        let tid = unsafe { GetCurrentThreadId() };
        log(format_args!(
            "{LOG_PREFIX} clock getState first call tid={tid}"
        ));
    }
    engine::tick();
    rc
}

fn load<F: Copy>(slot: &AtomicUsize) -> Option<F> {
    let raw = slot.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    // SAFETY: the four original slots are only ever stored with the import the Windows loader
    // resolved for them, and `F` is the `extern "system" fn` type of that import.
    Some(unsafe { std::mem::transmute_copy::<usize, F>(&raw) })
}

/// Point one import slot at `replacement`, publishing the original first. False, logged, on
/// failure.
///
/// # Safety
///
/// `base` must be the live game module base and `rva` a pointer-sized import slot in its `.idata`.
unsafe fn patch_import(
    base: usize,
    rva: u32,
    replacement: usize,
    original: &AtomicUsize,
    name: &str,
) -> bool {
    let slot = (base + rva as usize) as *mut usize;
    let mut old_protect = 0u32;
    // SAFETY: one pointer-sized slot inside the image's own `.idata`.
    let ok = unsafe {
        VirtualProtect(
            slot.cast::<c_void>(),
            std::mem::size_of::<usize>(),
            PAGE_READWRITE,
            &raw mut old_protect,
        )
    };
    if ok == 0 {
        log(format_args!(
            "{LOG_PREFIX} import-hook-failed import={name} stage=VirtualProtect slot=0x{:016x}",
            slot as usize
        ));
        return false;
    }
    // SAFETY: the slot is now writable and holds the resolved import. The original is published
    // before the slot changes, so a thread that reaches the detour at once finds it.
    unsafe {
        original.store(slot.read(), Ordering::Release);
        slot.write(replacement);
        let mut restored = 0u32;
        VirtualProtect(
            slot.cast::<c_void>(),
            std::mem::size_of::<usize>(),
            old_protect,
            &raw mut restored,
        );
    }
    log(format_args!(
        "{LOG_PREFIX} hooked import={name} slot=0x{:016x} original=0x{:016x}",
        slot as usize,
        original.load(Ordering::Acquire)
    ));
    true
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Whether the `Event::start` slot, the one the feature cannot do without, was fronted.
    pub installed: bool,
    /// Slots fronted, of four.
    pub hooked: u32,
}

/// Read the playlists, resolve the FMOD functions, and front the four event import slots.
///
/// # Safety
///
/// Writes four pointer-sized import slots in the game image. Call once, from the loader's
/// post-Arxan callback, before the game has started any sound.
pub unsafe fn install() -> Outcome {
    let _ = origin();
    let refused = Outcome {
        installed: false,
        hooked: 0,
    };
    let Ok(base) = ds2_game_base::mem::game_module_base() else {
        log(format_args!(
            "{LOG_PREFIX} no game module base; nothing hooked"
        ));
        return refused;
    };
    // SAFETY: `base` is the live module base; only slots named in `ds2-rva` are read.
    let missing = unsafe { fmod::resolve(base) };
    log(format_args!("{LOG_PREFIX} resolved missing={missing:?}"));
    if missing.contains(&"Event::getInfo") {
        log(format_args!(
            "{LOG_PREFIX} Event::getInfo unresolved; no event could be named, nothing hooked"
        ));
        return refused;
    }
    store::load();

    let start: StartFn = detour_start;
    let stop: BoolArgFn = detour_stop;
    let set_paused: BoolArgFn = detour_set_paused;
    let get_state: GetStateFn = detour_get_state;
    // SAFETY: each RVA is the named import slot, read out of the image's import descriptors.
    let results = unsafe {
        [
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_START_IAT,
                start as usize,
                &ORIGINAL_START,
                "fmod_event64!Event::start",
            ),
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_STOP_IAT,
                stop as usize,
                &ORIGINAL_STOP,
                "fmod_event64!Event::stop",
            ),
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_SET_PAUSED_IAT,
                set_paused as usize,
                &ORIGINAL_SET_PAUSED,
                "fmod_event64!Event::setPaused",
            ),
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_GET_STATE_IAT,
                get_state as usize,
                &ORIGINAL_GET_STATE,
                "fmod_event64!Event::getState",
            ),
        ]
    };
    Outcome {
        installed: results[0],
        hooked: results.iter().filter(|ok| **ok).count() as u32,
    }
}
