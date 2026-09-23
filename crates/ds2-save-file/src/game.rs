//! The three things both rows need from the running game: the save system, its interlock, and a way
//! to ask it to persist.
//!
//! Shared rather than duplicated because both rows have the same shape underneath -- *ask the game to
//! save, wait until it has, then do the thing* -- and the waiting is the part that is easy to get
//! subtly wrong in two different ways.

use std::path::Path;
use std::time::SystemTime;

use crate::{LOG_PREFIX, log_line};

/// `(length, modified)` for a file, or `None` if it cannot be read.
///
/// The pair, not either alone: a save that rewrites to the same length is common, and a filesystem
/// whose mtime has one-second resolution -- which a Proton prefix has -- can stamp two writes
/// identically. Two weak signals disagree less often than one.
pub fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// The `SaveLoadSystem` the game is using, through the two hops `ds2-rva` records.
pub fn save_load_system() -> Option<usize> {
    let address = ds2_game_base::mem::game_rva(ds2_rva::GAME_MANAGER_IMP).ok()?;
    // SAFETY: a resolved RVA in the loaded image read through the fault-safe reader, which reports a
    // bad address rather than faulting on it.
    let manager = unsafe { ds2_game_base::mem::safe_read_usize(address)? };
    if manager == 0 {
        return None;
    }
    // SAFETY: as above, one hop further in.
    let system =
        unsafe { ds2_game_base::mem::safe_read_usize(manager + ds2_rva::SAVE_LOAD_SYSTEM_OFFSET)? };
    (system != 0).then_some(system)
}

/// Whether no save or load request is in flight.
///
/// Both halves of the interlock, because both start paths test both
/// (`if ([this+0x08] != 0 || [this+0x0c] != 0) return false`). A read that cannot be performed
/// answers `false`: "I do not know whether a writer is active" must never mean "go ahead".
pub fn interlock_idle(system: usize) -> bool {
    // SAFETY: two `u32` reads inside a live `SaveLoadSystem`, through the fault-safe reader. `u32`
    // and not `usize`: the game's own tests are `cmp DWORD PTR [rcx+8],0`, and reading eight bytes
    // would fold the neighbouring field into the answer.
    unsafe {
        let state =
            ds2_game_base::mem::safe_read_u32(system + ds2_rva::SAVE_LOAD_SYSTEM_STATE_OFFSET);
        let substate =
            ds2_game_base::mem::safe_read_u32(system + ds2_rva::SAVE_LOAD_SYSTEM_SUBSTATE_OFFSET);
        matches!((state, substate), (Some(0), Some(0)))
    }
}

/// `void RequestSave(SaveLoadSystem*, u32 kind)` -- three byte writes, no return value.
type RequestSaveFn = unsafe extern "system" fn(usize, u32);

/// Ask the game to persist the character. Returns whether the request was made.
///
/// Refuses on a prologue that is not the recorded one. `RequestSave` is not Arxan-redirected, but an
/// RVA is a number: on a build these offsets were not read from, this address is some other function
/// that would accept the call and leave a log line claiming a save was requested.
pub fn request_save(system: usize) -> bool {
    let Ok(address) = ds2_game_base::mem::game_rva(ds2_rva::SAVE_LOAD_REQUEST_SAVE) else {
        log_line(format_args!(
            "{LOG_PREFIX} save REFUSED reason=no-module-base -- nothing was requested"
        ));
        return false;
    };
    let expected = ds2_rva::SAVE_LOAD_REQUEST_SAVE_PROLOGUE;
    let mut prologue = [0u8; 3];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(address, &mut prologue) };
    if !read || prologue != expected {
        log_line(format_args!(
            "{LOG_PREFIX} save REFUSED reason=prologue va=0x{address:016x} read={read} \
             saw={prologue:02x?} want={expected:02x?} -- that address is not RequestSave on this \
             build"
        ));
        return false;
    }
    // SAFETY: the prologue matches the function `ds2-rva` transcribed, the signature is the one its
    // disassembly implements (pointer in RCX, kind in EDX, no return), and `system` is a live
    // `SaveLoadSystem` reached through two recorded hops. Called on the game thread from the menu's
    // own confirm path, which is where the game's own save rows call it from.
    let request: RequestSaveFn = unsafe { std::mem::transmute::<usize, RequestSaveFn>(address) };
    unsafe { request(system, ds2_rva::SAVE_LOAD_REQUEST_KIND_CHARACTER) };
    log_line(format_args!(
        "{LOG_PREFIX} save requested system=0x{system:016x} kind={} -- the game writes it on a \
         later frame",
        ds2_rva::SAVE_LOAD_REQUEST_KIND_CHARACTER
    ));
    true
}

/// `bool loadSystemData(SaveLoadSystem*)` -- see [`ds2_rva::SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA`].
///
/// `u8` back rather than `bool`, for the reason the gate predicate in `ds2-menu-row` is: a `bool`
/// whose byte is neither 0 nor 1 is undefined behaviour, and an address that turned out to be the
/// wrong function is exactly how that byte arrives.
type LoadSystemDataFn = unsafe extern "system" fn(usize) -> u8;

/// Ask the game to re-read the container's system data: the ten character records.
///
/// Returns whether the request was accepted. `false` covers both refusals -- a prologue that is not
/// the recorded one, and the game's own interlock saying a request is already in flight -- and both
/// mean nothing has changed, so a caller can simply try again next frame.
///
/// # Why a character list needs this at all
///
/// `FUN_1400f0f60`, the vector the list's `enter` measures, is built out of
/// `GameManagerImp->GameDataManager->savedata__` alone. Pointing the load side at another container
/// changes what a READ of the file answers; it does not touch a block that was filled before the
/// redirect was armed. So the sequence is arm, then this, then the list -- in that order, or the
/// list describes the container the player is leaving.
pub fn load_system_data(system: usize) -> bool {
    let Ok(address) = ds2_game_base::mem::game_rva(ds2_rva::SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA)
    else {
        log_line(format_args!(
            "{LOG_PREFIX} re-read REFUSED reason=no-module-base -- the list still describes the \
             old container"
        ));
        return false;
    };
    let expected = ds2_rva::SAVE_LOAD_SYSTEM_LOAD_SYSTEM_DATA_PROLOGUE;
    let mut prologue = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(address, &mut prologue) };
    if !read || prologue != expected {
        log_line(format_args!(
            "{LOG_PREFIX} re-read REFUSED reason=prologue va=0x{address:016x} read={read} \
             saw={prologue:02x?} want={expected:02x?} -- that address is not the system-data load \
             on this build"
        ));
        return false;
    }
    // SAFETY: the prologue matches the function `ds2-rva` transcribed, the signature is the one its
    // decompilation implements (the system in RCX, a byte back), and `system` is a live
    // `SaveLoadSystem` reached through two recorded hops. The function's own first act is to test
    // the interlock and return `false`, so calling it at a bad moment is a no-op rather than a
    // race.
    let load: LoadSystemDataFn = unsafe { std::mem::transmute::<usize, LoadSystemDataFn>(address) };
    let accepted = unsafe { load(system) } != 0;
    log_line(format_args!(
        "{LOG_PREFIX} re-read requested system=0x{system:016x} accepted={accepted}"
    ));
    accepted
}

/// How a wait for "the save has landed" ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// Still waiting. Nothing to do this frame.
    Waiting,
    /// The file changed and no writer is active. Safe to act.
    Yes,
    /// The deadline passed without the file changing.
    TimedOut,
}

/// One frame of the wait: has the save the row asked for actually been written?
///
/// **Two signals, and neither is sufficient alone.** The `.sl2`'s stamp changing says a save landed,
/// which is the row's actual promise; the interlock going idle says nobody is still writing, which is
/// what stops a torn copy. `flushed` is the caller's latch for the first, so a stamp that changes and
/// changes back cannot un-observe the save.
pub fn poll_landed(
    source: &Path,
    before: Option<(u64, SystemTime)>,
    flushed: &mut bool,
    ticks: u32,
    deadline: u32,
) -> Landed {
    if !*flushed {
        let now = stamp(source);
        if now.is_some() && now != before {
            *flushed = true;
        }
    }
    if ticks >= deadline {
        return Landed::TimedOut;
    }
    if !*flushed {
        return Landed::Waiting;
    }
    match save_load_system() {
        Some(system) if interlock_idle(system) => Landed::Yes,
        _ => Landed::Waiting,
    }
}
