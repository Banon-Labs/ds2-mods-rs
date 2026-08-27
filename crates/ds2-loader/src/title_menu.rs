//! Reading `[title_menu]` out of `<Game>/ds2-mods.toml`.
//!
//! Its own section rather than a key under `[title_skip]`, because it is not a skip. Everything in
//! `[title_skip]` removes a wait; this changes what the title menu draws once the waits are gone,
//! on different functions, and a run that fails to boot has to be attributable to one of them by
//! editing one line.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "title_menu";

/// Draw the menu rows the game would hide, instead of hiding them.
pub const KEY_SHOW_UNAVAILABLE: &str = "show_unavailable";

/// `[title_menu]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleMenuConfig {
    /// Style unavailable rows as drawn, and put their cell state back so they stay unselectable.
    pub show_unavailable: bool,
}

impl Default for TitleMenuConfig {
    /// **On.** The shipped behaviour removes a row from the screen entirely when it is
    /// unavailable, so the menu's shape changes with the machine's network state and with whether
    /// a save exists. Drawing all six always is the requested behaviour and is the more legible
    /// one; it stays a switch for the reason every switch here is one, since it patches executable
    /// memory during startup.
    fn default() -> Self {
        Self {
            show_unavailable: true,
        }
    }
}

impl TitleMenuConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        let show_unavailable = match parsed.get(CONFIG_SECTION, KEY_SHOW_UNAVAILABLE) {
            None => Self::default().show_unavailable,
            // Only an exact `false` turns it off, so a typo leaves the feature ON -- the harmless
            // direction when on is the default.
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
        };
        Self { show_unavailable }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_SHOW_UNAVAILABLE}={}",
            ds2_dialog_skip::LOG_PREFIX,
            self.show_unavailable
        )
    }
}
