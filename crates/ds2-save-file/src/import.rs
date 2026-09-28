//! **Load Character from File**: pick a save container, and the row does the rest -- including the
//! restart.
//!
//! # Two things this row is not allowed to do, both learned from a live run
//!
//! **It does not accept a file because of its name.** The first version offered an "All files (*.*)"
//! line in the dialog and gated the pick on its EXTENSION, which accepts `holiday.jpg` renamed to
//! `save.sl2` -- and the player finds out one launch later, when the game shows no LOAD GAME row,
//! which is also exactly what a correct redirect to an empty folder looks like. Now the dialog offers
//! only what can be loaded, and the pick is checked by what the file IS:
//! [`ds2_save_redirect::validate_source`] unwraps the archive if there is one, requires exactly one
//! `DS2SOFS0000.sl2` inside it, and structurally validates the BND4 container that comes out.
//!
//! **It does not ask the player to restart the game.** The earlier version wrote the pick, relabelled
//! itself `Staged: restart to load`, and left the restart to whoever was holding the controller. That
//! is handing the user a chore the mod created. The row now performs the whole sequence itself.
//!
//! # What one press does now
//!
//! ```text
//! pick -> validate -> stage the copy -> return to the title -> the game's own character list
//!      -> the player chooses -> playing it
//! ```
//!
//! [`crate::swap`] owns everything after the validation and documents why each step is where it is.
//! Nothing in that sequence restarts the process and nothing writes a file for a later launch to
//! find.
//!
//! # The route below it, kept because a hook can fail to install
//!
//! ```text
//! pick -> validate -> record the handoff -> refuse saves -> quit
//! ```
//!
//! This runs only when [`crate::swap::begin`] refuses -- which in practice means the title flow is
//! not hooked, so nothing can drive the character list. A row that loads a save slowly beats a row
//! that reports a missing detour, and the log says which of the two ran.
//!
//! It does not save the character being left, and neither does the swap. The player asked for that
//! and accepted the cost: progress since the game last saved is gone. The quit is the game's own
//! one-byte shutdown -- `FeSubStateTitleShutdown`'s write, polled by the master update -- which
//! asks nothing itself, but a measured run saw a save requested on the way out
//! (`ds2-save-block: refused a save kind=10`), so `ds2_save_block::refuse_saves` is armed first and
//! drops it.
//!
//! # Why this used to be the only route, and what changed
//!
//! DS2 keeps one save container per Steam account and saves on the way out of a game, so a session
//! that re-points [`ds2_rva::SAVE_DIR_BUILD`] and then quits writes the current character into the
//! staged copy, and the LOAD GAME that follows reads back the character the player was replacing.
//! Every word of that is still true. What it does not survive is the save side and the load side
//! being separable, which they are: `SLSaveSession` and `SLLoadSession` each override the directory
//! virtual, each override is reached only through its own vtable, and
//! `ds2_save_redirect::session_dir` arms them independently. The character being left is written by a
//! save side nothing has touched; the list and the load read a staged copy. See [`crate::swap`].
//!
//! Two earlier plans for that split are worth not repeating. One claimed `FUN_1402e67f0` calls
//! `SAVE_DIR_BUILD` per request and that its `{1,3,5,6}` states separate saves from loads; those
//! states are an early-out guard (`6 < state || (0x6a >> (state & 31) & 1) == 0`) and every state
//! surviving it reaches the same switch, whose only `SAVE_DIR_BUILD` arm is session setup. The other
//! aimed at the request object, where `+0x3c` is a mode (`0` load, `1` save) beside the directory at
//! `+0x48` -- correct, and still unnecessary, because the class that asks for the directory already
//! answers the same question without a flag having to be read.

use std::path::{Path, PathBuf};

use ds2_save_file_core::{
    HANDOFF_FILE_NAME, accepts_with, filter::FilterEntry, filter_string, handoff,
};

use crate::dialog::{Intent, Pick, Request};
use crate::swap;
use crate::{LOG_PREFIX, log_line};

/// The row's caption.
pub const ROW_CAPTION: &str = "Load Character from File";

/// The caption once the in-session swap has asked the game to leave.
///
/// The game is leaving for the title during the frames this is on screen, so it answers whether the
/// row did anything.
pub const ROW_CAPTION_LEAVING: &str = "Returning to the title to pick a character...";

/// Where the handoff file goes: next to `DarkSoulsII.exe`, beside the loader's own log.
fn handoff_path() -> Option<PathBuf> {
    ds2_game_base::log::game_directory_path().map(|dir| dir.join(HANDOFF_FILE_NAME))
}

/// The dialog's type dropdown: exactly what staging can read, and nothing else.
///
/// **No "All files" line.** An escape hatch on a LOAD dialog is an invitation to pick something that
/// is not a save, and the gate behind it used to be an extension check. The player who has renamed
/// their save can rename it back; every other use of that line was a mistake waiting to be made.
fn dialog_filter() -> Vec<u16> {
    // The running session's own container extension leads the list. With Seamless Co-op loaded the
    // game opens `DS2SOFS0000.co2`, and a dropdown offering only the static four would hide the
    // player's own character behind a filter -- with no "All files" line to escape through.
    let extensions = ds2_save_file_core::extensions_with(Some(session_extension()));
    filter_string(&[FilterEntry {
        label: "DARK SOULS II save or archive",
        extensions: &extensions,
    }])
}

/// The extension the running game is using for its save container.
///
/// `sl2` unmodded. See `ds2_save_redirect::active_save_file_name`, which resolves it once from
/// both mods' config files.
fn session_extension() -> &'static str {
    ds2_save_redirect::active_save_file_name()
        .rsplit_once('.')
        .map_or("sl2", |(_, extension)| extension)
}

/// The player's downloads folder, in the Windows spelling the dialog wants.
///
/// A donor container arrives as a download and is opened from where it landed, so this is the
/// folder the row opens in. It was the parent of the player's own save directory, which is where
/// the per-account folders live -- a sensible-sounding place that nobody's downloads are in, and so
/// a browse away from the file every time.
///
/// Wine maps `Z:` to `/`, which is how a Linux path reaches a dialog running inside the prefix.
/// The dialog ignores a directory that does not exist and falls back to the shell's own default, so
/// a machine with no `~/Downloads` loses nothing.
const DOWNLOADS_WINDOWS_PREFIX: &str = "Z:\\";

/// Where to start browsing. Downloads first, then the folder above the player's own save.
///
/// Shared with the export row, which is the point: the two dialogs are one round trip to the
/// player -- save a character out, browse back to it later -- and a save dialog that opens
/// somewhere the load dialog never goes makes them hunt for the file they just wrote. It used to
/// open in the live container's own folder, which after a swap is `ds2-swapped-save` inside the
/// game install.
pub(crate) fn start_directory() -> Option<PathBuf> {
    if let Some(downloads) = downloads_directory() {
        return Some(downloads);
    }
    let live = ds2_save_redirect::live_directory()?;
    let text = live.to_string_lossy().into_owned();
    let trimmed = Path::new(text.trim_end_matches(['\\', '/']));
    trimmed
        .parent()
        .map(Path::to_path_buf)
        .or_else(|| Some(live.clone()))
}

/// Where the prefix's own user profile lives. Its `Downloads` is not the player's.
///
/// Measured on this machine: `C:\users\steamuser\Downloads` is a real, empty directory that Proton
/// created, not a link to the host's. So the shell's own Downloads known folder is the wrong answer
/// here and is deliberately not asked for.
const PREFIX_HOME_DRIVE: &str = "Z:\\home";

/// `~/Downloads` as the prefix sees it, or `None` when no such folder can be found.
///
/// Two sources, in order, because the first one is not always there. `HOME` is the Linux home and
/// is the exact answer when it survives into this process -- but the game is started by an
/// already-running Steam client, through the Steam Linux Runtime's container, and one live run
/// measured the dialog opening in the fallback, which is what a missing `HOME` looks like from
/// here. So the second source is to look: `Z:\home` is the host's `/home`, and a directory under it
/// holding a `Downloads` is a player's downloads folder.
///
/// `USERPROFILE` is not a source. Inside the prefix it is `C:\users\steamuser`, whose `Downloads`
/// is [`PREFIX_HOME_DRIVE`]'s empty impostor.
fn downloads_directory() -> Option<PathBuf> {
    // TESTED IN THE PREFIX'S SPELLING, NOT THE HOST'S, and that distinction was a bug of its own.
    // `std::fs` inside this DLL is the Win32 API: `Path::new("/home/you/Downloads")` is a rooted
    // path with no drive, so it is asked about as `C:\home\you\Downloads`, which does not exist --
    // and the check said "no downloads folder" on a machine that has one. The `Z:` form is the same
    // directory as Wine maps it, and is also the form handed to the dialog, so what is tested is
    // what is used.
    let readable = |path: String| Path::new(&path).is_dir().then_some(path);

    if let Some(from_env) = std::env::var("HOME")
        .ok()
        .and_then(|home| downloads_windows_path(&home))
        .and_then(readable)
    {
        return Some(PathBuf::from(from_env));
    }

    // One level of `Z:\home`, which is a handful of entries on any real machine. The first one
    // holding a `Downloads` wins; a box with several users gets one of them rather than nothing,
    // and the player can browse from there.
    let mut homes: Vec<PathBuf> = std::fs::read_dir(PREFIX_HOME_DRIVE)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    // Sorted so a machine with more than one user picks the same one every launch. An order that
    // came out of the filesystem would make this open somewhere different on a whim.
    homes.sort();
    homes
        .into_iter()
        .map(|home| home.join("Downloads"))
        .find(|candidate| candidate.is_dir())
}

/// `~/Downloads` as the prefix spells it, from a Unix home directory. Pure, so it is testable.
fn downloads_windows_path(home: &str) -> Option<String> {
    let home = home.trim_end_matches('/');
    if home.is_empty() || !home.starts_with('/') {
        return None;
    }
    let body = format!("{}/Downloads", home.trim_start_matches('/')).replace('/', "\\");
    Some(format!("{DOWNLOADS_WINDOWS_PREFIX}{body}"))
}

/// What pressing the row does. **Game thread, inside the menu's confirm path.**
pub fn load_from_file() {
    if handoff_path().is_none() {
        log_line(format_args!(
            "{LOG_PREFIX} import REFUSED reason=no-game-directory -- there is nowhere to record the \
             pick"
        ));
        return;
    }
    // The in-game picker unless the player asked for the OS dialog. It answers through
    // [`accept`] from [`tick`], a frame or more later, and this press is done.
    if crate::picker::open_for_load() {
        return;
    }

    let filter = dialog_filter();
    let start_dir = start_directory();
    // Where it opened, said out loud. A dialog that quietly fell back to somewhere else looks
    // identical to one that was never told anything, and that is exactly how the first version of
    // this shipped: its folder test ran as a Win32 call against a Unix path and always said no.
    log_line(format_args!(
        "{LOG_PREFIX} import dialog opening in {}",
        start_dir
            .as_deref()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<the shell's own default>".to_owned())
    ));
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
    // Every refusal is already in the log; the dialog has nowhere else to show one.
    let _ = accept(&picked, None);
}

/// Why [`accept`] refused a pick, in words for the picker's banner. The log line has the detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refused {
    /// One short line.
    pub(crate) headline: &'static str,
    /// What was wrong, or what to do about it.
    pub(crate) detail: String,
}

/// Load `picked` -- and, when the picker chose one, the character in `slot` -- or say why not.
/// **Game thread, from the row's press or from the pause menu's tick.**
///
/// The two gates, then the in-session swap, then the restart route when the swap cannot run.
pub(crate) fn accept(picked: &Path, slot: Option<usize>) -> Result<(), Refused> {
    let Some(handoff_file) = handoff_path() else {
        log_line(format_args!(
            "{LOG_PREFIX} import REFUSED reason=no-game-directory -- there is nowhere to record the \
             pick"
        ));
        return Err(Refused {
            headline: "CANNOT LOAD NOW",
            detail: String::from("the game directory could not be found"),
        });
    };
    // GATE ONE: the name. Cheap, and it is what makes the log say "that is not a save" rather than
    // surfacing a decompression error from three layers down.
    if let Err(rejection) = accepts_with(picked, Some(session_extension())) {
        log_line(format_args!(
            "{LOG_PREFIX} import REFUSED path={} reason={rejection} -- expected one of {}; \
             nothing was recorded",
            picked.display(),
            ds2_save_file_core::offered_with(Some(session_extension()))
        ));
        return Err(Refused {
            headline: "WRONG FILE TYPE",
            detail: format!(
                "expected one of {}",
                ds2_save_file_core::offered_with(Some(session_extension()))
            ),
        });
    }
    // GATE TWO: the CONTENT, which is the one that matters. Unwraps the archive, demands exactly one
    // save inside it, and structurally validates the container. A file that passes gate one and
    // fails here is precisely the renamed-jpeg case, and it is refused while the player is still
    // looking at the menu rather than one launch later.
    let (kind, entries) = match ds2_save_redirect::validate_source(picked) {
        Ok(found) => found,
        Err(error) => {
            log_line(format_args!(
                "{LOG_PREFIX} import REFUSED path={} reason={error} -- nothing was recorded",
                picked.display()
            ));
            return Err(Refused {
                headline: "SAVE UNREADABLE",
                detail: error.to_string(),
            });
        }
    };

    // The in-session route first, and it is the whole feature: [`crate::swap`] stages the pick,
    // asks the game to return to the title as its Quit Game row's "yes" does, but with no confirm
    // box and no save of the character being left, points the loads at
    // the staged copy, and hands the player the game's own character list for it. No restart, and
    // no handoff file.
    //
    // The fallback below is kept rather than deleted because the in-session route needs the title
    // flow hooked, and a build where that hook did not install should still be able to load a save
    // -- slowly, through a restart -- instead of doing nothing at all. Which one ran is in the log.
    match swap::begin(picked, slot) {
        Ok(()) => {
            log_line(format_args!(
                "{LOG_PREFIX} import in-session kind={kind} entries={entries} path={} slot={} -- \
                 leaving the game to load it",
                picked.display(),
                slot.map_or_else(|| String::from("<list>"), |slot| slot.to_string())
            ));
            announce(ROW_CAPTION_LEAVING);
            return Ok(());
        }
        Err(reason) => log_line(format_args!(
            "{LOG_PREFIX} import in-session unavailable reason={reason} -- falling back to the \
             route that records the pick and restarts the game"
        )),
    }

    let contents = handoff::encode(&picked.to_string_lossy());
    if let Err(error) = std::fs::write(&handoff_file, contents) {
        log_line(format_args!(
            "{LOG_PREFIX} import FAILED reason=write error={error} file={} -- the pick was NOT \
             recorded and nothing will change",
            handoff_file.display()
        ));
        return Err(Refused {
            headline: "COULD NOT RECORD THE PICK",
            detail: error.to_string(),
        });
    }
    log_line(format_args!(
        "{LOG_PREFIX} import recorded kind={kind} entries={entries} path={} file={}",
        picked.display(),
        handoff_file.display()
    ));

    // No save of the character being left: the player asked to leave it unsaved. The refusal is
    // armed before the quit so a save the shutdown path asks for is dropped too, and it is never
    // lifted -- the process is ending.
    let refusing = ds2_save_block::refuse_saves();
    log_line(format_args!(
        "{LOG_PREFIX} import quitting without saving -- detour-installed={refusing}; the next \
         launch loads the file you picked"
    ));
    quit();
    Ok(())
}

/// The pause-menu tick. Registered with `ds2_menu_row::add_tick`.
///
/// It carries the in-session swap's pause-menu half, because both want the same place and the tick
/// registry has a slot count. What that half watches for is this tick continuing to run: the pause
/// menu updating is what calls it, so a swap that asked the game to leave and is still being ticked
/// is a swap the game never acted on.
pub fn tick() {
    crate::picker::collect();
    crate::swap::pause_tick();
}

/// Quit to desktop through the game's own shutdown, which the master update polls.
fn quit() {
    ds2_menu_row::quit_to_desktop();
}

/// Relabel the row so the player can see the press took, without reading a log file.
fn announce(caption: &'static str) {
    let Some(row) = crate::registered_import_row() else {
        return;
    };
    if !ds2_menu_row::set_row_caption(row, caption) {
        return;
    }
    // SAFETY: game thread, inside a row's own confirm, with the pause menu this caption belongs to
    // still up -- which is exactly the context `refresh_row_captions` documents.
    let _ = unsafe { ds2_menu_row::refresh_row_captions() };
}

/// Give the label back, for every path that ends a flow which borrowed it.
///
/// Deliberately does not push: [`announce`] can, because it runs inside the row's own confirm with
/// the menu up, and most callers of this one are at the title where there is no pause menu to write
/// into. Rewriting the buffer is enough -- the menu's own per-frame push takes it if the menu is up,
/// and the next caption bind takes it if it is not.
pub(crate) fn restore() {
    let Some(row) = crate::registered_import_row() else {
        return;
    };
    ds2_menu_row::reset_row_caption(row);
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

    /// THE DIALOG OFFERS ONLY WHAT CAN BE LOADED. An "All files" line on a load dialog is how a
    /// player comes to pick something that is not a save; it was there once and is not coming back.
    #[test]
    fn the_filter_offers_no_escape_hatch() {
        let filter = dialog_filter();
        let fields: Vec<String> = filter
            .split(|unit| *unit == 0)
            .map(String::from_utf16_lossy)
            .collect();
        assert_eq!(
            fields[0],
            "DARK SOULS II save or archive (*.sl2;*.zip;*.7z;*.rar)"
        );
        assert_eq!(fields[1], "*.sl2;*.zip;*.7z;*.rar");
        // Pair, pair, terminator -- and no second pair offering everything.
        assert_eq!(fields[2], "");
        assert!(
            !fields.iter().any(|field| field.contains("*.*")),
            "a load dialog must not offer every file: {fields:?}"
        );
    }

    /// THE DUPLICATION GATE between the host-tested extension list and the staging step's arms.
    #[test]
    fn every_stageable_extension_is_offered() {
        assert_eq!(
            ds2_save_file_core::SOURCE_EXTENSIONS,
            ["sl2", "zip", "7z", "rar"]
        );
        assert!(
            ds2_save_redirect::SAVE_FILE_NAME
                .to_ascii_lowercase()
                .ends_with(ds2_save_file_core::SOURCE_EXTENSIONS[0])
        );
    }

    /// The picker opens in the prefix's spelling of `~/Downloads`, with no forward slashes left.
    ///
    /// A path the dialog cannot resolve is not refused by it -- it silently falls back to the
    /// shell's own folder, which is indistinguishable from never having been told anything. So the
    /// spelling is the whole contract, and one forward slash is enough to lose it.
    #[test]
    fn downloads_is_handed_over_in_the_prefixs_own_spelling() {
        assert_eq!(
            downloads_windows_path("/home/you").as_deref(),
            Some("Z:\\home\\you\\Downloads")
        );
        // A trailing separator on HOME must not double up in the middle of the path.
        assert_eq!(
            downloads_windows_path("/home/you/").as_deref(),
            Some("Z:\\home\\you\\Downloads")
        );
        let produced = downloads_windows_path("/var/lib/steam").expect("an absolute home");
        assert!(!produced.contains('/'), "{produced}");
        assert!(produced.starts_with(DOWNLOADS_WINDOWS_PREFIX), "{produced}");
    }

    /// A home directory that is not an absolute Unix path is refused rather than mangled.
    ///
    /// `Z:` maps to `/`, so the mapping only means anything for a path that starts there. Anything
    /// else -- an empty variable, a Windows `USERPROFILE` that leaked in -- would produce a path
    /// that resolves to nothing, and the fallback is the better answer.
    #[test]
    fn a_home_that_is_not_an_absolute_unix_path_is_refused() {
        assert_eq!(downloads_windows_path(""), None);
        assert_eq!(downloads_windows_path("/"), None);
        assert_eq!(downloads_windows_path("C:\\users\\steamuser"), None);
        assert_eq!(downloads_windows_path("relative/home"), None);
    }
}
