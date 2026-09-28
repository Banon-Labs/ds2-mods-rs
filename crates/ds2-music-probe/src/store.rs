//! The playlists in memory, and `ds2-music-playlist.toml` beside the game that keeps them.
//!
//! Read once at install, written whole on every edit (to a temporary file, then renamed over), so
//! a quit-out or a crash between edits loses nothing that was already on screen.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::install::log;
use crate::playlist::Playlists;
use crate::{LOG_PREFIX, PLAYLIST_FILE_NAME};

/// Every region's playlist. The panel edits it; the sound thread reads it.
pub(crate) static PLAYLISTS: Mutex<Playlists> = Mutex::new(Playlists::new());

fn path() -> Option<PathBuf> {
    ds2_game_base::log::game_directory_path().map(|dir| dir.join(PLAYLIST_FILE_NAME))
}

/// Read the file into [`PLAYLISTS`], naming every line that could not be used.
pub(crate) fn load() {
    let Some(path) = path() else {
        log(format_args!("{LOG_PREFIX} playlists: no game directory"));
        return;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} playlists: {} not read ({error}) -- every region plays as shipped",
                path.display()
            ));
            return;
        }
    };
    let (lists, problems) = Playlists::parse(&text);
    for problem in &problems {
        log(format_args!("{LOG_PREFIX} playlists: {problem}"));
    }
    for region in &lists.regions {
        let tracks: Vec<String> = region.tracks.iter().map(ToString::to_string).collect();
        log(format_args!(
            "{LOG_PREFIX} playlists: region={} map={} include_default={} repeat={} tracks=[{}]",
            region.default,
            region.map.map_or_else(|| "-".to_owned(), |m| m.to_string()),
            region.include_default,
            region.repeat,
            tracks.join(", ")
        ));
    }
    log(format_args!(
        "{LOG_PREFIX} playlists: read {} region(s) from {}",
        lists.regions.len(),
        path.display()
    ));
    if let Ok(mut guard) = PLAYLISTS.lock() {
        *guard = lists;
    }
}

/// Write `lists` to the file. Logged either way.
pub(crate) fn save(lists: &Playlists) {
    let Some(path) = path() else {
        return;
    };
    let temporary = path.with_extension("toml.tmp");
    let result = std::fs::write(&temporary, lists.to_text())
        .and_then(|()| std::fs::rename(&temporary, &path));
    match result {
        Ok(()) => log(format_args!(
            "{LOG_PREFIX} playlists: saved {} region(s) to {}",
            lists.regions.len(),
            path.display()
        )),
        Err(error) => log(format_args!(
            "{LOG_PREFIX} playlists: NOT saved to {} ({error}) -- the edit lasts until the game \
             closes",
            path.display()
        )),
    }
}
