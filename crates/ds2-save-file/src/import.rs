//! **Load Character from File**: pick a save container, and have the next launch read it.
//!
//! # The row does not load anything, and that is the design rather than a shortfall
//!
//! DARK SOULS II keeps one save container per Steam account. Loading somebody else's character means
//! pointing the game's save-directory builder somewhere else, which `ds2-save-redirect` already does
//! -- and doing it mid-session does not work:
//!
//! > DS2 saves on the way out. The pause menu's Quit Game row is `FeGroupInGameReturnTitleCheck`,
//! > which persists the character before it returns to the title. A session that re-points the
//! > directory and then quits writes the CURRENT character into the staged copy, and the LOAD GAME
//! > that follows reads back the character the player was trying to replace.
//!
//! Neither ordering escapes it, because both halves are the game's: the quit saves, and the load
//! reads the same directory the quit wrote to. So this row writes the pick to
//! [`ds2_save_file_core::HANDOFF_FILE_NAME`] and `ds2-loader` arms the redirect from it on the next
//! launch, during `DLL_PROCESS_ATTACH` -- before the game has built a save path, let alone saved into
//! one. The loader deletes the file as it reads it, so the handoff is one launch and not a setting.
//!
//! # What it costs, said plainly
//!
//! **A restart.** `../er-mods-rs` does this in-session, because Elden Ring has ten save slots and a
//! profile picker to switch between them; DS2 has one container and no picker. The in-session version
//! is possible here too and is NOT built: it needs `FUN_1402e67f0`'s
//! `[saveLoadSystem+8] in {1,3,5,6}` states decoded far enough to tell a SAVE apart from a LOAD, so
//! the redirect can answer differently for each. That is a bounded static job and it is filed; until
//! it is done, a restart is the honest price and the row says so on itself.
//!
//! # The player's own save is never touched
//!
//! Nothing here writes to a save. The row writes one text file in the game directory; the staging
//! that follows on the next launch COPIES the picked file into `ds2-save-staging/` and rebinds the
//! copy, leaving both the player's container and the donor's file exactly as they were.

use std::path::{Path, PathBuf};

use ds2_save_file_core::{
    HANDOFF_FILE_NAME, SOURCE_EXTENSIONS, accepts, filter::FilterEntry, filter_string, handoff,
};

use crate::dialog::{Intent, Pick, Request};
use crate::{LOG_PREFIX, log_line};

/// The row's caption, before anything has been picked.
pub const ROW_CAPTION: &str = "Load Character from File";

/// The caption after a pick has been recorded.
///
/// Short on purpose: the caption mark beside a pause-menu row is not wide, and a truncated sentence
/// is worse than a short one. The log line carries the path and the reasoning.
pub const ROW_CAPTION_STAGED: &str = "Staged: restart to load";

/// Where the handoff file goes: next to `DarkSoulsII.exe`, beside the loader's own log.
fn handoff_path() -> Option<PathBuf> {
    ds2_game_base::log::game_directory_path().map(|dir| dir.join(HANDOFF_FILE_NAME))
}

/// The dialog's type dropdown: the four shapes staging can read, then everything.
///
/// Two lines and not five. A dropdown with one entry per extension makes the player choose which kind
/// of archive they have before they can see it, which is the opposite of helpful.
fn dialog_filter() -> Vec<u16> {
    filter_string(&[
        FilterEntry {
            label: "DARK SOULS II save or archive",
            extensions: &SOURCE_EXTENSIONS,
        },
        FilterEntry {
            label: "All files",
            extensions: &[],
        },
    ])
}

/// Where to start browsing.
///
/// The player's own save folder's PARENT, because that is where a donor save most often lands -- one
/// folder up from `<steamid>/` is `…\DarkSoulsII\`, which holds every account folder on the machine.
/// Falling back to the save directory itself, and then to nothing, which the dialog treats as "wherever
/// the shell would have started".
fn start_directory() -> Option<PathBuf> {
    let live = ds2_save_redirect::live_directory()?;
    // A directory path from the game ends in a separator, so `parent()` on it is the directory
    // itself. Trimming first is what makes the answer the folder ABOVE it.
    let text = live.to_string_lossy().into_owned();
    let trimmed = Path::new(text.trim_end_matches(['\\', '/']));
    trimmed
        .parent()
        .map(Path::to_path_buf)
        .or_else(|| Some(live.clone()))
}

/// What pressing the row does. **Game thread, inside the menu's confirm path.**
///
/// Opens the picker inline -- which blocks the game, on purpose, see [`crate::dialog`] -- checks the
/// extension, writes the handoff, and relabels itself so the player can see that it took.
pub fn load_from_file() {
    let Some(handoff_file) = handoff_path() else {
        log_line(format_args!(
            "{LOG_PREFIX} import REFUSED reason=no-game-directory -- there is nowhere to record the \
             pick"
        ));
        return;
    };

    let filter = dialog_filter();
    let start_dir = start_directory();
    let request = Request {
        intent: Intent::Open,
        title: "Load a character from a save file",
        start_dir: start_dir.as_deref(),
        filter: &filter,
        default_name: "",
    };
    // SAFETY: game thread inside the menu's confirm path, which is what `dialog::show` requires.
    let picked = match unsafe { crate::dialog::show(&request) } {
        Pick::Chosen(path) => path,
        Pick::Cancelled => {
            log_line(format_args!(
                "{LOG_PREFIX} import cancelled -- any earlier pick is left exactly as it was"
            ));
            return;
        }
        Pick::Failed(error) => {
            log_line(format_args!(
                "{LOG_PREFIX} import REFUSED reason=dialog-failed comdlg-error={error:#x} -- no file \
                 was picked"
            ));
            return;
        }
    };

    // The cheap gate, before anything is written: say "that is not a save" here rather than surfacing
    // a decompression error from inside staging on the next launch, where nobody is watching.
    let kind = match accepts(&picked) {
        Ok(kind) => kind,
        Err(rejection) => {
            log_line(format_args!(
                "{LOG_PREFIX} import REFUSED path={} reason={rejection} -- nothing was recorded",
                picked.display()
            ));
            return;
        }
    };
    // Existence is checked too, even though the dialog was opened with `OFN_FILEMUSTEXIST`: the flag
    // governs what the dialog accepts, and the thing that has to be true is that the file is there on
    // the NEXT launch. Checking now catches the ordinary case -- a typed path, a removable drive --
    // while the player can still do something about it.
    if !picked.is_file() {
        log_line(format_args!(
            "{LOG_PREFIX} import REFUSED path={} reason=not-a-file -- nothing was recorded",
            picked.display()
        ));
        return;
    }

    let contents = handoff::encode(&picked.to_string_lossy());
    if let Err(error) = std::fs::write(&handoff_file, contents) {
        log_line(format_args!(
            "{LOG_PREFIX} import FAILED reason=write error={error} file={} -- the pick was NOT \
             recorded and the next launch will load your own save",
            handoff_file.display()
        ));
        return;
    }
    log_line(format_args!(
        "{LOG_PREFIX} import recorded kind={kind} path={} file={} -- the NEXT launch loads it, and \
         this session is untouched",
        picked.display(),
        handoff_file.display()
    ));
    announce();
}

/// Relabel the row so the player can see the pick took, without reading a log file.
///
/// The row id is whichever slot the registration got; a caption written before registration would
/// land on row zero, which belongs to somebody else -- so this does nothing until [`crate::register`]
/// has succeeded.
fn announce() {
    let Some(row) = crate::registered_import_row() else {
        return;
    };
    if !ds2_menu_row::set_row_caption(row, ROW_CAPTION_STAGED) {
        return;
    }
    // SAFETY: game thread, inside a row's own confirm, with the pause menu this caption belongs to
    // still up -- which is exactly the context `refresh_row_captions` documents.
    let written = unsafe { ds2_menu_row::refresh_row_captions() };
    if written == 0 {
        log_line(format_args!(
            "{LOG_PREFIX} caption not pushed -- the row will read {ROW_CAPTION_STAGED:?} the next \
             time the menu opens"
        ));
    }
}

/// Read and CONSUME a handoff left by a previous session.
///
/// Called by the loader during attach, before anything has built a save path. Returns the path the
/// previous session picked, having already deleted the file -- so a crash between here and the
/// redirect being armed costs the handoff rather than stranding the player in someone else's save on
/// every launch from then on.
///
/// Deleting FIRST rather than after a successful arm is deliberate. The two orderings fail
/// differently: this one loses a pick, and the other one keeps loading the wrong save forever.
pub fn take_handoff() -> Option<PathBuf> {
    let file = handoff_path()?;
    let text = std::fs::read_to_string(&file).ok()?;
    let removed = std::fs::remove_file(&file);
    let picked = handoff::decode(&text);
    match (&picked, removed) {
        (Some(path), Ok(())) => log_line(format_args!(
            "{LOG_PREFIX} handoff taken path={path} file={} -- consumed, so this applies to THIS \
             launch only",
            file.display()
        )),
        (Some(path), Err(error)) => log_line(format_args!(
            "{LOG_PREFIX} handoff taken path={path} file={} BUT NOT DELETED: {error} -- delete it by \
             hand or every launch will load this save",
            file.display()
        )),
        (None, _) => log_line(format_args!(
            "{LOG_PREFIX} handoff file={} names no path -- nothing armed",
            file.display()
        )),
    }
    picked.map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialog offers every shape staging can read, on one line, plus an escape hatch.
    #[test]
    fn the_filter_offers_every_stageable_shape() {
        let filter = dialog_filter();
        let fields: Vec<String> = filter
            .split(|unit| *unit == 0)
            .map(String::from_utf16_lossy)
            .collect();
        assert_eq!(fields[1], "*.sl2;*.zip;*.7z;*.rar");
        assert_eq!(fields[3], "*.*");
    }

    /// THE DUPLICATION GATE. `ds2-save-file-core`'s extension list is `ds2-save-redirect::stage`'s
    /// four arms spelled a second time, because that crate is `cfg(windows)` in full and cannot be
    /// imported by a host-tested one. This is where the two are made to agree: a fifth arm there
    /// without a fifth entry there fails here rather than going silently unoffered.
    #[test]
    fn every_stageable_extension_is_offered() {
        assert_eq!(SOURCE_EXTENSIONS, ["sl2", "zip", "7z", "rar"]);
        // And the bare-save arm is the one the game's own file name uses, so a player who picks their
        // own container by hand is picking something staging will accept.
        assert!(
            ds2_save_redirect::SAVE_FILE_NAME
                .to_ascii_lowercase()
                .ends_with(SOURCE_EXTENSIONS[0])
        );
    }

    /// The staged caption is short enough to be a caption.
    #[test]
    fn the_staged_caption_is_short() {
        assert!(ROW_CAPTION_STAGED.len() <= ROW_CAPTION.len());
    }
}
