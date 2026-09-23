//! **Save Game to File**: ask the game to persist, then copy the container the player picked a name
//! for.
//!
//! # Why this is two phases and not one
//!
//! `SaveLoadSystem::RequestSave` does not save. It writes three bytes -- a wanted-kind and two flags
//! -- and `GameManagerImp`'s master update performs the save on its own schedule, the same way the
//! shutdown byte `ds2-menu-row` writes is polled rather than obeyed. See
//! [`ds2_rva::SAVE_LOAD_REQUEST_SAVE`], which is transcribed in full there precisely because it looks
//! synchronous and is not.
//!
//! So a row press cannot copy anything: the bytes on disk at that instant are the LAST save, and the
//! one the player just asked for has not been written. The press therefore requests and arms; a
//! per-frame tick watches the file and copies when it has actually changed.
//!
//! # What it watches, and why not the interlock alone
//!
//! `[saveLoadSystem+0x08]`/`+0x0c` is the interlock every start path refuses on and every completion
//! path zeroes -- a real signal, and not sufficient on its own. It says a REQUEST finished; it does
//! not say the file was rewritten, and this row's whole promise is about the file. So the gate is
//! both: the `.sl2`'s length-and-mtime stamp has to CHANGE, and then the interlock has to be idle
//! before a byte is copied. The first says the save landed, the second says nobody is still writing.
//!
//! # The deadline copies anyway, and says so
//!
//! If the stamp never changes -- the game decided a save was unnecessary, or the request was dropped
//! -- the wait gives up after [`DEADLINE_TICKS`] and copies the file that IS there, logging that the
//! flush was never observed. The alternative is handing the player nothing after they named a
//! destination, which is worse than handing them their last autosave and saying which it is.
//!
//! # The tick only runs while the pause menu is up
//!
//! Which is where the press came from, and where the player still is when the dialog closes. If they
//! leave the menu before the copy completes, the request stays armed and finishes the next time a
//! menu is opened. Nothing is lost and nothing is written early.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use ds2_save_file_core::{Route, filter::FilterEntry, filter_string, with_extension};

use crate::dialog::{Intent, Pick, Request};
use crate::{LOG_PREFIX, log_line};

/// The row's caption.
pub const ROW_CAPTION: &str = "Save Game to File";

/// Menu frames to wait for the save to land before copying whatever is on disk.
///
/// The pause menu ticks with the game, so this is about five seconds at 60 fps -- far longer than a
/// DS2 save takes, and short enough that a player who is watching does not conclude nothing happened.
const DEADLINE_TICKS: u32 = 300;

// Long enough to be a save, short enough that a player who is watching does not conclude the row did
// nothing. Both directions are failures, so both are asserted where the number is.
const _: () = assert!(DEADLINE_TICKS >= 60, "under a second is not a save");
const _: () = assert!(DEADLINE_TICKS <= 60 * 30, "half a minute is a hang");

/// An export that has been asked for and not yet written.
struct Pending {
    /// Where the copy goes. Already extension-completed and already checked not to be the live one.
    destination: PathBuf,
    /// The container being copied -- the live save in the directory the game is actually using.
    source: PathBuf,
    /// `(length, modified)` of `source` as it was before the save was requested. `None` when the file
    /// could not be stat'd, in which case any successful stat counts as a change.
    stamp: Option<(u64, SystemTime)>,
    /// The stamp has changed: the save the press asked for has been written.
    flushed: bool,
    ticks: u32,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

/// The `.sl2` the game is reading and writing right now, redirected or not.
///
/// `ds2-save-redirect`'s detour records the directory it produced in BOTH arms, so this is the live
/// path whether or not a redirect is armed -- which is the distinction this row must not have to care
/// about. `None` means the game has not built a save path yet, which cannot be true while a pause
/// menu is up and is reported rather than guessed around.
fn live_container() -> Option<PathBuf> {
    let directory = ds2_save_redirect::live_directory()?;
    Some(directory.join(ds2_save_redirect::SAVE_FILE_NAME))
}

/// `(length, modified)` for a file, or `None` if it cannot be read.
fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// The `SaveLoadSystem` the game is using, through the two hops `ds2-rva` records.
fn save_load_system() -> Option<usize> {
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
/// answers `false`: "I do not know whether a writer is active" must never mean "go ahead and copy".
fn interlock_idle(system: usize) -> bool {
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
fn request_save(system: usize) -> bool {
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
             saw={prologue:02x?} want={expected:02x?} -- that address is not RequestSave on this build"
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

/// The dialog's type dropdown: saves, then everything, so a player who wants another name can have
/// one.
fn dialog_filter() -> Vec<u16> {
    filter_string(&[
        FilterEntry {
            label: "DARK SOULS II save",
            extensions: &[ds2_save_file_core::SAVE_EXTENSION],
        },
        FilterEntry {
            label: "All files",
            extensions: &[],
        },
    ])
}

/// What pressing the row does. **Game thread, inside the menu's confirm path.**
///
/// Opens the destination dialog inline -- which blocks the game, on purpose, see
/// [`crate::dialog`] -- validates what came back, then requests the save and arms [`tick`].
pub fn save_to_file() {
    let Some(source) = live_container() else {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=no-save-directory -- the game has not built a save \
             path in this run, so there is nothing to copy"
        ));
        return;
    };
    if !source.is_file() {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=no-container path={} -- the save directory holds no \
             {}",
            source.display(),
            ds2_save_redirect::SAVE_FILE_NAME
        ));
        return;
    }
    if PENDING.lock().map(|state| state.is_some()).unwrap_or(true) {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=already-pending -- an export is still waiting for \
             the save to land"
        ));
        return;
    }

    let filter = dialog_filter();
    let request = Request {
        intent: Intent::Save,
        title: "Save this character to a file",
        start_dir: source.parent(),
        filter: &filter,
        default_name: ds2_save_redirect::SAVE_FILE_NAME,
    };
    // SAFETY: game thread inside the menu's confirm path, which is what `dialog::show` requires.
    let picked = unsafe { crate::dialog::show(&request) };
    let picked = match picked {
        Pick::Chosen(path) => path,
        Pick::Cancelled => {
            log_line(format_args!(
                "{LOG_PREFIX} export cancelled -- nothing was requested and nothing was written"
            ));
            return;
        }
        Pick::Failed(error) => {
            log_line(format_args!(
                "{LOG_PREFIX} export REFUSED reason=dialog-failed comdlg-error={error:#x} -- no \
                 destination was named"
            ));
            return;
        }
    };

    let destination = with_extension(&picked, ds2_save_file_core::SAVE_EXTENSION);
    // THE ONE REFUSAL THAT PROTECTS A SAVE. Copying a file onto itself truncates it on most
    // platforms, and the destination dialog opens IN the save's own folder with the save's own name
    // already filled in -- so pressing Save without typing is the default path to this mistake, not
    // an exotic one.
    if ds2_save_file_core::dest::is_live_container(&destination, &source) {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=destination-is-the-live-save path={} -- copying the \
             container onto itself would truncate the save you are playing",
            destination.display()
        ));
        return;
    }
    let route = Route::of(destination.is_file());

    let Some(system) = save_load_system() else {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=no-save-load-system -- cannot ask the game to \
             persist, so nothing is copied"
        ));
        return;
    };

    let before = stamp(&source);
    if !request_save(system) {
        return;
    }
    let Ok(mut pending) = PENDING.lock() else {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=poisoned -- the save WAS requested and will happen; \
             only the copy is lost"
        ));
        return;
    };
    log_line(format_args!(
        "{LOG_PREFIX} export armed route={route:?} source={} destination={} -- waiting for the save \
         to land",
        source.display(),
        destination.display()
    ));
    *pending = Some(Pending {
        destination,
        source,
        stamp: before,
        flushed: false,
        ticks: 0,
    });
}

/// The game-thread half: watch for the save, then copy. Registered with `ds2_menu_row::add_tick`.
pub fn tick() {
    let Ok(mut guard) = PENDING.lock() else {
        return;
    };
    let Some(pending) = guard.as_mut() else {
        return;
    };
    pending.ticks += 1;

    if !pending.flushed {
        let now = stamp(&pending.source);
        if now.is_some() && now != pending.stamp {
            pending.flushed = true;
            log_line(format_args!(
                "{LOG_PREFIX} export saw the save land after {} ticks",
                pending.ticks
            ));
        }
    }
    let timed_out = pending.ticks >= DEADLINE_TICKS;
    if !pending.flushed && !timed_out {
        return;
    }
    // Idle-or-timed-out: a writer still holding the file would give a torn copy, so the interlock
    // gets the last word right up to the deadline.
    if !timed_out {
        let Some(system) = save_load_system() else {
            return;
        };
        if !interlock_idle(system) {
            return;
        }
    }
    let pending = guard.take().expect("checked above");
    drop(guard);
    finish(&pending, timed_out);
}

/// Perform the copy and say exactly what happened.
fn finish(pending: &Pending, timed_out: bool) {
    let note = if timed_out && !pending.flushed {
        " -- THE FLUSH WAS NEVER OBSERVED: this is the last save the game wrote, not the state you \
         pressed the row in"
    } else {
        ""
    };
    match std::fs::copy(&pending.source, &pending.destination) {
        Ok(bytes) => log_line(format_args!(
            "{LOG_PREFIX} exported bytes={bytes} source={} destination={} ticks={}{note}",
            pending.source.display(),
            pending.destination.display(),
            pending.ticks
        )),
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} export FAILED error={error} source={} destination={} ticks={} -- nothing \
             was written",
            pending.source.display(),
            pending.destination.display(),
            pending.ticks
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialog offers the save extension first, and an escape hatch second.
    #[test]
    fn the_filter_offers_saves_first_and_then_everything() {
        let filter = dialog_filter();
        let fields: Vec<String> = filter
            .split(|unit| *unit == 0)
            .map(String::from_utf16_lossy)
            .collect();
        assert_eq!(fields[1], "*.sl2");
        assert_eq!(fields[3], "*.*");
    }

    /// The default name is the one the game itself builds, so the two cannot drift apart.
    #[test]
    fn the_default_name_is_the_games_own() {
        assert_eq!(
            ds2_save_redirect::SAVE_FILE_NAME,
            ds2_save_file_core::SAVE_FILE_NAME
        );
    }
}
