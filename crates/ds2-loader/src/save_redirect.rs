//! Reading `[save_redirect]` out of `<Game>/ds2-mods.toml`.
//!
//! The redirect itself lives in `ds2-save-redirect`; this is only the switch, kept here for the
//! same reason `dialog_skip`'s is -- the config file belongs to the loader, and the feature crate
//! should not have to know where the game directory is.
//!
//! # The key this is not
//!
//! `[save_redirect] path = <a .sl2>` existed once and was deleted. A file cannot be played in
//! place by a game that builds its own container name, so that key copied it into a staging folder
//! and rewrote the copy from the same source on the next launch. Every session started that way
//! threw away its own progress while the help string said the save had been loaded.
//!
//! [`KEY_DIRECTORY`] names a folder instead, which is the whole difference: the game's directory
//! builder is answered with it, the game opens its own container name inside it, and reads and
//! writes go to that one file for the rest of the session. Nothing is copied, so nothing can be
//! overwritten by the next launch.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "save_redirect";

/// The folder to play out of, as a Windows path -- this DLL runs inside the Proton prefix, so a
/// Linux path reaches it as `Z:\home\you\...`.
pub const KEY_DIRECTORY: &str = "directory";

/// `[save_redirect]`, resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveRedirectConfig {
    /// The folder, or `None` for the game's own.
    pub directory: Option<String>,
}

impl SaveRedirectConfig {
    /// Read the section. A missing file, a missing key or an empty value means the game's own
    /// directory, which is the only default that can be right: a save location guessed on the
    /// player's behalf is a save location they did not choose.
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        let directory = parsed
            .get(CONFIG_SECTION, KEY_DIRECTORY)
            .map(|raw| raw.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty());
        Self { directory }
    }

    /// One line for the attach log, written before anything acts on it.
    ///
    /// The unset case is a line rather than silence. A player who set the key and misspelled the
    /// section gets `<the game's own>` here, which is the difference between "the redirect did not
    /// work" and "the redirect was never read".
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_DIRECTORY}={}",
            ds2_save_redirect::LOG_PREFIX,
            self.directory.as_deref().unwrap_or("<the game's own>")
        )
    }
}
