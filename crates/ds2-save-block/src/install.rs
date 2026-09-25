//! The one detour, and the five writes it makes.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ds2_game_base::mem::{game_rva, read_bytes};
use ds2_hook::{MH_ApplyQueued, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::policy::{self, Decision, Frame};

/// `SaveLoadSystem::update(this, f32 delta)`.
///
/// The delta is a `float` in `xmm1`; see [`ds2_rva::SAVE_LOAD_SYSTEM_UPDATE`]. Declaring it as an
/// integer would compile and would hand the engine whatever that volatile register held, which is
/// the frame delta the save timer and every cooldown in this system are measured in.
type SaveLoadUpdate = unsafe extern "system" fn(usize, f32);

/// Trampoline back to the real update, published before the site is patched so a detour that fires
/// on the next frame cannot read a zero and drop the engine's own save tick on the floor.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Whether the detour has run at least once, so the log can say so exactly once.
static FIRST_TICK: AtomicBool = AtomicBool::new(false);

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

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Whether the detour is live. When this is `false` the game saves exactly as it always did.
    pub installed: bool,
}

/// Detour `SaveLoadSystem::update` so the game stops asking itself to save.
///
/// # Safety
///
/// Installs a native code detour. Call once, from the loader's install position, in the game
/// process.
pub unsafe fn install() -> Outcome {
    let refused = Outcome { installed: false };

    let site = match game_rva(ds2_rva::SAVE_LOAD_SYSTEM_UPDATE) {
        Ok(site) => site,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} not installed reason=no-module-base -- {error}. The game saves as it \
                 always did."
            ));
            return refused;
        }
    };
    // An RVA is a number until something checks it. On a build these offsets were not read from,
    // this address is some other function, and a detour there would clear five fields of whatever
    // object happened to be in rcx every frame.
    let expected = ds2_rva::SAVE_LOAD_SYSTEM_UPDATE_PROLOGUE;
    let mut prologue = [0u8; ds2_rva::SAVE_LOAD_SYSTEM_UPDATE_PROLOGUE.len()];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` reports an unmapped page
    // rather than faulting on it.
    let read = unsafe { read_bytes(site, &mut prologue) };
    if !read || prologue != expected {
        log(format_args!(
            "{LOG_PREFIX} not installed reason=prologue va=0x{site:016x} read={read} \
             saw={prologue:02x?} want={expected:02x?} -- that address is not SaveLoadSystem::update \
             on this build, so nothing was patched"
        ));
        return refused;
    }
    // SAFETY: MinHook's own initialiser; idempotent, and other features in this DLL may have run it.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} not installed: MH_Initialize said {status:?}"
        ));
        return refused;
    }
    // Not `ds2_hook::register_union_hook`. The union's shared handler signature is four `usize`, and
    // this target's second argument is a float in `xmm1` -- a volatile register the dispatcher is
    // entitled to clobber. Nothing else in this workspace hooks this address, so there is no
    // collision for the union to arbitrate. `ds2-invasion-path` hooks its own per-frame target the
    // same way and for the same reason.
    //
    // SAFETY: `site` has been proven to hold this function's own prologue, and `save_load_update`
    // matches the ABI recorded with that prologue. The trampoline is stored before the hook is
    // enabled, so the detour can never run without one.
    let hook = match unsafe { MhHook::new(site as *mut c_void, save_load_update as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} not installed: MH_CreateHook said {status:?}"
            ));
            return refused;
        }
    };
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: enabling the hook created immediately above.
    if let Err(status) = unsafe { hook.queue_enable() } {
        log(format_args!(
            "{LOG_PREFIX} not installed: queue_enable said {status:?}"
        ));
        return refused;
    }
    // SAFETY: applies this DLL's queued hooks; other features queue theirs the same way.
    if unsafe { MH_ApplyQueued() } != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} not installed: MH_ApplyQueued refused"
        ));
        return refused;
    }
    log(format_args!(
        "{LOG_PREFIX} installed at 0x{site:016x} -- this run does not save by itself: no autosave \
         every {}s, nothing on quit to menu, nothing at a bonfire. Only the menu rows save.",
        ds2_rva::SAVE_LOAD_SYSTEM_AUTOSAVE_SECONDS
    ));
    Outcome { installed: true }
}

/// The detour. Erases whatever asked for a save, then runs the engine's own tick.
///
/// Before the original, and that ordering is the whole mechanism: the request fields are read and
/// acted on inside the original, so a frame's request has to be gone by the time it starts.
///
/// Never panics and never propagates: this is the game's simulation step, and an unwind through an
/// `extern "system"` boundary into engine code is undefined behaviour rather than an error message.
unsafe extern "system" fn save_load_update(system: usize, delta: f32) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: `system` is the `SaveLoadSystem` the engine passed in rcx, and every access is a
        // fault-safe read of a field recorded in `ds2-rva` before anything is written.
        unsafe { suppress(system) };
    }));
    let raw = TRAMPOLINE.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is MinHook's trampoline for this exact function, stored before the hook was
        // enabled, and the signature is the one the prologue check proved.
        let original: SaveLoadUpdate = unsafe { std::mem::transmute::<usize, SaveLoadUpdate>(raw) };
        unsafe { original(system, delta) };
    }
}

/// The span of `SaveLoadSystem` this feature touches: from the autosave accumulator at `+0x64` to
/// one past kind 14's flag at `+0x1a9`.
///
/// Read in one go rather than field by field, for two reasons. It is one `ReadProcessMemory` per
/// frame instead of four. And it proves every byte this function is about to write is mapped --
/// including `+0x1a3` and `+0x64`, which are written and never read, and which a set of four
/// single-field reads would not cover if a page boundary fell between them.
const WINDOW_START: usize = ds2_rva::SAVE_LOAD_SYSTEM_AUTOSAVE_ELAPSED_OFFSET;
const WINDOW_END: usize = ds2_rva::SAVE_LOAD_SYSTEM_SAVE_DEFERRED_OFFSET + 1;
const WINDOW_LEN: usize = WINDOW_END - WINDOW_START;

// Every field this file reads or writes has to be inside that window, or the read proves nothing
// about it. Asserted here so adding a seventh field cannot quietly escape the check.
const _: () = assert!(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_WANTED_OFFSET < WINDOW_END);
const _: () = assert!(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_KIND2_OFFSET < WINDOW_END);
const _: () = assert!(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_IN_FLIGHT_OFFSET < WINDOW_END);
const _: () = assert!(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_KIND_OFFSET >= WINDOW_START);
const _: () = assert!(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_KIND_OFFSET + 4 <= WINDOW_END);

/// Read the six fields, decide, write. One frame.
///
/// # Safety
///
/// `system` must be the live `SaveLoadSystem` the engine passed to the update.
unsafe fn suppress(system: usize) {
    // Read before write, always: a successful fault-safe read is what proves the object is there,
    // and it is the only thing standing between a wrong `system` and five writes into somebody
    // else's memory.
    let mut window = [0u8; WINDOW_LEN];
    // SAFETY: a bulk read through the fault-safe reader, which reports an unmapped range rather
    // than faulting on it. Nothing is written when it fails.
    if !unsafe { read_bytes(system + WINDOW_START, &mut window) } {
        return;
    }
    let mut kind_bytes = [0u8; 4];
    kind_bytes.copy_from_slice(&window[field(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_KIND_OFFSET)..][..4]);
    let frame = Frame {
        wanted: window[field(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_WANTED_OFFSET)] != 0,
        deferred: window[field(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_DEFERRED_OFFSET)] != 0,
        in_flight: window[field(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_IN_FLIGHT_OFFSET)] != 0,
        kind: i32::from_le_bytes(kind_bytes),
    };

    // One line the first time the detour runs, because "installed" and "on the live path" are
    // different claims and only the second one means the game's saves are being refused. The update
    // is reached from two game-state handlers, so a run that never gets into a game never calls it --
    // and without this line that run looks exactly like a working one.
    if !FIRST_TICK.swap(true, Ordering::AcqRel) {
        let mut elapsed = [0u8; 4];
        elapsed.copy_from_slice(
            &window[field(ds2_rva::SAVE_LOAD_SYSTEM_AUTOSAVE_ELAPSED_OFFSET)..][..4],
        );
        log(format_args!(
            "{LOG_PREFIX} the save tick is live system=0x{system:016x} wanted={} deferred={} \
             in-flight={} kind={} autosave-elapsed={:.3}s of {}s",
            frame.wanted,
            frame.deferred,
            frame.in_flight,
            frame.kind,
            f32::from_le_bytes(elapsed),
            ds2_rva::SAVE_LOAD_SYSTEM_AUTOSAVE_SECONDS
        ));
    }

    let wanted_at = system + ds2_rva::SAVE_LOAD_SYSTEM_SAVE_WANTED_OFFSET;
    let kind2_at = system + ds2_rva::SAVE_LOAD_SYSTEM_SAVE_KIND2_OFFSET;
    let deferred_at = system + ds2_rva::SAVE_LOAD_SYSTEM_SAVE_DEFERRED_OFFSET;
    let kind_at = system + ds2_rva::SAVE_LOAD_SYSTEM_SAVE_KIND_OFFSET;
    let elapsed_at = system + ds2_rva::SAVE_LOAD_SYSTEM_AUTOSAVE_ELAPSED_OFFSET;

    match policy::step(frame) {
        Decision::Pass => {}
        Decision::HoldTimer => {
            // SAFETY: the read above proved this span is mapped, and this field is the `f32`
            // accumulator recorded at `SAVE_LOAD_SYSTEM_AUTOSAVE_ELAPSED_OFFSET`. Written on the
            // game thread, from inside the only function that reads it.
            unsafe { (elapsed_at as *mut f32).write_volatile(0.0) };
        }
        Decision::Erase {
            kind: requested,
            deferred: was_deferred,
        } => {
            // The same four writes the game performs itself once it has started a save, plus the
            // accumulator. See `ds2_rva::SAVE_LOAD_SYSTEM_UPDATE` for that tail transcribed.
            // SAFETY: as above -- fields recorded in `ds2-rva`, inside a span the read proved is
            // mapped, on the game thread, before the function that consumes them runs.
            unsafe {
                (wanted_at as *mut u8).write_volatile(0);
                (kind2_at as *mut u8).write_volatile(0);
                (deferred_at as *mut u8).write_volatile(0);
                (kind_at as *mut i32).write_volatile(ds2_rva::SAVE_LOAD_SYSTEM_SAVE_KIND_NONE);
                (elapsed_at as *mut f32).write_volatile(0.0);
            }
            let dropped = policy::dropped_requests();
            if policy::should_log(dropped) {
                log(format_args!(
                    "{LOG_PREFIX} refused a save kind={requested} deferred={was_deferred} \
                     total={dropped} -- nothing was written; use the Save Game to File row"
                ));
            }
        }
    }
}

/// Where a `SaveLoadSystem` field offset lands in the window [`suppress`] read.
const fn field(offset: usize) -> usize {
    offset - WINDOW_START
}
