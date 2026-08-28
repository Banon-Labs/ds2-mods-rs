//! Reading `[build_import]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature itself lives in `ds2-build-import`; this is only the switch that decides whether to
//! turn it on, kept here for the same reason `menu_row`'s config is: the config file belongs to the
//! loader, and the feature crate should not have to know where the game directory is.
//!
//! Its own section rather than a key under `[menu_row]`, even though both put a row on the same
//! tab. That one adds a row whose whole purpose was to measure whether a fourth row draws; this one
//! adds a row that opens a Steam overlay and talks to the network. A run that misbehaves has to be
//! attributable to one of them by editing one line.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "build_import";

/// Whether to add the Load from URL row at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[build_import]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildImportConfig {
    /// Add a row to the pause menu that opens a prefilled Steam text field.
    pub enabled: bool,
}

impl Default for BuildImportConfig {
    /// **Off**, and this one is a holding position rather than a verdict.
    ///
    /// The feature reaches outside the process in two ways the rest of this loader does not: it
    /// asks Steam to draw an overlay, and it writes the game's own `SoftwareKeyboardManagerImpl`
    /// state field to interlock against character naming. Both are believed correct from static
    /// reading and NEITHER has been run. Until a run says otherwise, a player who has not asked for
    /// this should not get it.
    ///
    /// It flips to on once a run shows the field opening prefilled and the game's own name entry
    /// still working afterwards.
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl BuildImportConfig {
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
            // direction while the default is off.
            Some(raw) => matches!(raw.trim().trim_matches('"'), "true"),
        };
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_build_import::LOG_PREFIX,
            self.enabled
        )
    }
}
