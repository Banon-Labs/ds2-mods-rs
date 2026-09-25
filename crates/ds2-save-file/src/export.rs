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
use crate::game::{self, Landed};
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
/// Every seam that can move the container is resolved by
/// [`ds2_save_redirect::live_container`], and asking it rather than the directory builder is what
/// this row got wrong. A swap arms nothing the builder knows about, so the builder went on naming
/// the player's own folder and the export copied a container the game had not written to since the
/// session began: the player got the save they started with, named after the character they had
/// been playing. Reported 2026-09-24, one export to `DS2 Saves\new` that loaded back as the
/// original character.
fn live_container() -> Option<PathBuf> {
    ds2_save_redirect::live_container()
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
            ds2_save_redirect::active_save_file_name()
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
    // The same folder the load row browses in, and not the container's own. A player saves a
    // character out so they can load it back, so the two dialogs opening in different places means
    // hunting for a file they had just written. The container's folder was the wrong answer twice
    // over: it is inside the Proton prefix, and after a swap it is `ds2-swapped-save` in the game
    // install, which is a directory this mod owns and nobody should be filing saves into.
    let start_dir =
        crate::import::start_directory().or_else(|| source.parent().map(Path::to_path_buf));
    log_line(format_args!(
        "{LOG_PREFIX} export dialog opening in {}",
        start_dir
            .as_deref()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<the shell's own default>".to_owned())
    ));
    let request = Request {
        intent: Intent::Save,
        title: "Save this character to a file",
        start_dir: start_dir.as_deref(),
        filter: &filter,
        // The vanilla name even inside a co-op session, and deliberately. What is written is a
        // save container; `.sl2` is the spelling every other DS2 tool and every unmodded game
        // recognises, and the load side here accepts both. Exporting under `.co2` would hand the
        // player a file only their own session can read.
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
    // platforms, and the dialog opens with the container's own name already filled in -- so a
    // player who browses to the save's folder and presses Save without typing lands here. It used
    // to be worse than that: the dialog opened in that folder too, which made this the default
    // path to the mistake rather than a reachable one.
    if ds2_save_file_core::dest::is_live_container(&destination, &source) {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=destination-is-the-live-save path={} -- copying the \
             container onto itself would truncate the save you are playing",
            destination.display()
        ));
        return;
    }
    let route = Route::of(destination.is_file());

    let Some(system) = game::save_load_system() else {
        log_line(format_args!(
            "{LOG_PREFIX} export REFUSED reason=no-save-load-system -- cannot ask the game to \
             persist, so nothing is copied"
        ));
        return;
    };

    let before = game::stamp(&source);
    if !game::request_save(system) {
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
    // Both signals, and neither alone: the stamp changing says the save landed, which is this row's
    // actual promise, and the interlock going idle says nobody is still writing, which is what stops
    // a torn copy. `game::poll_landed` owns that pair for both rows.
    let landed = game::poll_landed(
        &pending.source,
        pending.stamp,
        &mut pending.flushed,
        pending.ticks,
        DEADLINE_TICKS,
    );
    if landed == Landed::Waiting {
        return;
    }
    let pending = guard.take().expect("checked above");
    drop(guard);
    finish(&pending, landed == Landed::TimedOut);
}

/// Perform the copy and say exactly what happened.
fn finish(pending: &Pending, timed_out: bool) {
    let note = if timed_out && !pending.flushed {
        " -- THE FLUSH WAS NEVER OBSERVED: this is the last save the game wrote, not the state you \
         pressed the row in"
    } else {
        ""
    };
    // The copy must not go through the swap's window. That window is armed on the path the game's
    // directory builder produced, and with `[save_redirect] directory` pointing at the folder the
    // player exports into, the destination is that same path -- so the destination open was
    // diverted to the source and `std::fs::copy` truncated the save to nothing. Measured
    // 2026-09-24: `exported bytes=0`, staged container left empty behind it.
    //
    // The refusal above cannot catch this. It compares two paths that genuinely differ, and what
    // makes them one file is a detour underneath both.
    match ds2_save_redirect::open_redirect::bypass(|| {
        std::fs::copy(&pending.source, &pending.destination)
    }) {
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
