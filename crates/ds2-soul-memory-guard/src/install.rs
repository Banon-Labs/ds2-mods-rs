//! The one detour, on the function that restores a character, and the read after it returns.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::{game_rva, read_bytes, safe_read_u16, safe_read_u32, safe_read_usize};
use ds2_hook::{MH_ApplyQueued, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// `PlayerParam::RestoreFromRecord(param, record)`. See [`ds2_rva::PLAYER_PARAM_RESTORE_FROM_RECORD`].
///
/// The return is declared as the whole of RAX: the function sets only AL, and passing the register
/// through untouched is what leaves the caller seeing exactly what it would have seen.
type RestoreFromRecord = unsafe extern "system" fn(usize, usize) -> usize;

/// [`ds2_rva::PLAYER_LEVEL_UP_SOULS_COST`]: the level in ECX, the price in EAX.
type LevelUpCost = unsafe extern "system" fn(u32) -> i32;

/// Trampoline back to the real restore, published before the site is patched.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The resolved address of the cost function, set once its prologue has been checked.
static COST_FN: AtomicUsize = AtomicUsize::new(0);

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
    /// Whether the detour is live. When this is `false` nothing is judged and nothing is patched.
    pub installed: bool,
}

/// Whether the bytes at `rva` are `expected`. Logs the mismatch and returns the address when they
/// are.
fn checked_site(rva: u32, expected: &[u8], name: &str) -> Option<usize> {
    let site = match game_rva(rva) {
        Ok(site) => site,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} not installed reason=no-module-base -- {error}"
            ));
            return None;
        }
    };
    let mut seen = vec![0u8; expected.len()];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` reports an unmapped page
    // rather than faulting on it.
    let read = unsafe { read_bytes(site, &mut seen) };
    if !read || seen != expected {
        log(format_args!(
            "{LOG_PREFIX} not installed reason=prologue fn={name} va=0x{site:016x} read={read} \
             saw={seen:02x?} want={expected:02x?} -- that address is not {name} on this build, so \
             nothing was patched"
        ));
        return None;
    }
    Some(site)
}

/// Detour `PlayerParam::RestoreFromRecord` so every character load ends in a verdict line.
///
/// # Safety
///
/// Installs a native code detour. Call once, from the loader's install position, in the game
/// process.
pub unsafe fn install() -> Outcome {
    let refused = Outcome { installed: false };

    // The cost function is checked before anything is patched: a detour with nothing to call would
    // only ever log that it could not judge.
    let Some(cost) = checked_site(
        ds2_rva::PLAYER_LEVEL_UP_SOULS_COST,
        &ds2_rva::PLAYER_LEVEL_UP_SOULS_COST_PROLOGUE,
        "LevelUpSoulsCost",
    ) else {
        return refused;
    };
    let Some(site) = checked_site(
        ds2_rva::PLAYER_PARAM_RESTORE_FROM_RECORD,
        &ds2_rva::PLAYER_PARAM_RESTORE_FROM_RECORD_PROLOGUE,
        "PlayerParam::RestoreFromRecord",
    ) else {
        return refused;
    };
    COST_FN.store(cost, Ordering::Release);

    // SAFETY: `MH_Initialize` takes no arguments and is documented as safe to call again on an
    // already-initialised library, which the status below distinguishes.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} not installed: MH_Initialize said {status:?}"
        ));
        return refused;
    }
    // SAFETY: the target is an RVA this crate validated against the prologue it expects, and the
    // detour is a `'static` fn item of the ABI that prologue's function implements: two pointers
    // in, RAX out. The trampoline is stored before the hook is enabled, so the detour can never
    // run without one.
    let hook = match unsafe { MhHook::new(site as *mut c_void, restore_from_record as *mut c_void) }
    {
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
        "{LOG_PREFIX} installed at 0x{site:016x} cost_fn=0x{cost:016x} from_level={} -- each \
         character load logs a verdict line; nothing is refused or changed",
        crate::HIGHEST_CLASS_START_LEVEL
    ));
    Outcome { installed: true }
}

/// The detour. Runs the real restore first, then judges what it left behind.
///
/// Never panics and never propagates: this is the world-entry spawn, and an unwind through an
/// `extern "system"` boundary into engine code is undefined behaviour rather than an error message.
unsafe extern "system" fn restore_from_record(param: usize, record: usize) -> usize {
    let raw = TRAMPOLINE.load(Ordering::Acquire);
    if raw == 0 {
        return 0;
    }
    // SAFETY: `raw` is MinHook's trampoline for this exact function, stored before the hook was
    // enabled, and the signature is the one the prologue check proved.
    let original: RestoreFromRecord =
        unsafe { std::mem::transmute::<usize, RestoreFromRecord>(raw) };
    // SAFETY: the engine's own arguments, passed through untouched.
    let returned = unsafe { original(param, record) };
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: `param` is the `PlayerParam` the engine just restored; every read is fault-safe.
        unsafe { report(param) };
    });
    returned
}

/// Read the level and both counters, judge them, and log the line.
///
/// # Safety
///
/// `param` must be the `PlayerParam` the engine passed to the restore, on the thread that ran it.
unsafe fn report(param: usize) {
    // SAFETY: fault-safe reads of fields recorded in `ds2-rva`.
    let (level, first, second) = unsafe {
        (
            safe_read_u32(param + ds2_rva::PLAYER_PARAM_SOUL_LEVEL_OFFSET),
            safe_read_u32(param + ds2_rva::PLAYER_PARAM_SOUL_MEMORY_OFFSETS[0]),
            safe_read_u32(param + ds2_rva::PLAYER_PARAM_SOUL_MEMORY_OFFSETS[1]),
        )
    };
    let (Some(level), Some(first), Some(second)) = (level, first, second) else {
        log(format_args!(
            "{LOG_PREFIX} verdict=unreadable param=0x{param:016x} -- the restored character's \
             level or soul memory could not be read"
        ));
        return;
    };
    let callable = cost_is_callable();
    let check = crate::judge(level, [first, second], |step| {
        if !callable {
            return None;
        }
        // SAFETY: see `cost`.
        unsafe { cost(step) }
    });
    log(format_args!("{LOG_PREFIX} {check} param=0x{param:016x}"));
}

/// Whether every pointer [`ds2_rva::PLAYER_LEVEL_UP_SOULS_COST`] dereferences without a null test
/// is there, and whether its search is sure to end.
///
/// The function checks none of this itself. The chain is `[GameManagerImp] + 0x18` then `+ 0x580`;
/// the file under that container has to exist; and row 0 has to be a row whose level is at or below
/// every level asked about, or the halving loop would sit at index zero forever. Only the wide
/// row-index layout the live table uses is accepted.
fn cost_is_callable() -> bool {
    let Ok(global) = game_rva(ds2_rva::GAME_MANAGER_IMP) else {
        return false;
    };
    let hop = |at: usize| -> Option<usize> {
        // SAFETY: fault-safe read; a null answer is refused below.
        unsafe { safe_read_usize(at) }.filter(|pointer| *pointer != 0)
    };
    let row0_level = || -> Option<u16> {
        let manager = hop(global)?;
        let characters = hop(manager + ds2_rva::GAME_MANAGER_CHARACTER_MANAGER_OFFSET)?;
        let container = hop(characters + ds2_rva::CHARACTER_MANAGER_LEVEL_UP_SOULS_PARAM_OFFSET)?;
        let file = hop(container + ds2_rva::PARAM_FILE_RESOURCE_FILE_OFFSET)?;
        // SAFETY: fault-safe reads inside the param file the container points at.
        let (rows, shape) = unsafe {
            (
                safe_read_u16(file + ds2_rva::PARAM_ROW_COUNT_OFFSET)?,
                ds2_game_base::mem::safe_read_u8(file + ds2_rva::PARAM_FILE_TABLE_SHAPE_OFFSET)?,
            )
        };
        if rows == 0 || shape == 0 {
            return None;
        }
        let entry = file + ds2_rva::PARAM_ROW_INDEX_OFFSET;
        let offset = hop(entry + ds2_rva::PARAM_ROW_DATA_OFFSET)?;
        // SAFETY: fault-safe read of the first row's level.
        unsafe { safe_read_u16(file + offset + ds2_rva::PLAYER_LEVEL_UP_SOULS_LEVEL_OFFSET) }
    };
    matches!(row0_level(), Some(level) if u32::from(level) <= crate::HIGHEST_CLASS_START_LEVEL)
}

/// Ask the game what `step -> step + 1` costs.
///
/// # Safety
///
/// Only after [`cost_is_callable`] said yes, with `step` at or above the start level it checked
/// row 0 against, on the game thread.
unsafe fn cost(step: u32) -> Option<i32> {
    let raw = COST_FN.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    // SAFETY: `raw` is the cost function's address, stored only after its prologue matched the
    // bytes `ds2-rva` recorded; the signature is the one its disassembly implements.
    let cost: LevelUpCost = unsafe { std::mem::transmute::<usize, LevelUpCost>(raw) };
    // SAFETY: the caller has checked every pointer the function dereferences and that its search
    // ends for this level.
    Some(unsafe { cost(step) })
}
