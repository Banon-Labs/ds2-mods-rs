//! Reading `[menu_row]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature itself lives in `ds2-menu-row`; this is only the switch that decides whether to
//! turn it on, kept here for the same reason `intro_skip`'s config is: the config file belongs to
//! the loader, and the feature crate should not have to know where the game directory is.
//!
//! Its own section rather than a key under `[title_menu]`, even though both are menus. That one
//! restyles rows the TITLE menu already draws; this one appends an entry to the PAUSE menu's item
//! vector, on different functions, at a different time in the run. A run that misbehaves has to be
//! attributable to one of them by editing one line.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "menu_row";

/// Whether to append the extra row at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[menu_row]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuRowConfig {
    /// Append one entry to the pause menu tab that carries the quit item.
    pub enabled: bool,
}

impl Default for MenuRowConfig {
    /// **Off**, and unlike the other defaults in this loader that is not a judgement about which
    /// behaviour is better -- it is that this feature has no behaviour yet.
    ///
    /// It exists to answer one question that could not be answered by reading the executable:
    /// whether a fourth entry in a tab's item vector becomes a fourth visible ROW, and what
    /// caption it carries if it does. The row count is set from the vector's count by the game's
    /// own per-tab init, so it should; the captions are not in the executable at all, so it might
    /// come up blank. Until someone has looked, shipping it on would put an unexplained row in
    /// every player's pause menu to satisfy this repo's curiosity.
    ///
    /// Turn it on for a run, read `ds2-menu-row: appended ...` in the loader log, open the pause
    /// menu's last tab, and the question is closed.
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl MenuRowConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        let enabled = match parsed.get(CONFIG_SECTION, KEY_ENABLED) {
            None => Self::default().enabled,
            // Only an exact `true` turns it on, so a typo leaves the feature OFF -- the harmless
            // direction when off is the default, and the opposite polarity from the features that
            // default on. A probe that switched itself on because a value was misspelled would be
            // the one failure mode a probe cannot have.
            Some(raw) => matches!(raw.trim().trim_matches('"'), "true"),
        };
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_menu_row::LOG_PREFIX,
            self.enabled
        )
    }
}
