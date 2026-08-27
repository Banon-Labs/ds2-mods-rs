//! Reading `[dialog_skip]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature itself lives in `ds2-dialog-skip`; this is only the switch, kept here for the same
//! reason `intro_skip`'s is -- the config file belongs to the loader, and the feature crate should
//! not have to know where the game directory is.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "dialog_skip";

/// Whether to answer the title-flow message boxes at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[dialog_skip]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogSkipConfig {
    /// Detour the shared dialog update so each allowlisted message box answers itself.
    pub enabled: bool,
}

impl Default for DialogSkipConfig {
    /// **On**, matching `intro_skip`. Skipping the boot screens only to stop on a message box is
    /// half a feature, and the two are wanted together or not at all.
    ///
    /// It stays a key for the reason that one does: this patches executable memory during startup,
    /// so a run that fails to boot has to be attributable to one feature by editing one line
    /// rather than by rebuilding. Two separate switches, not one, precisely so `intro_skip` and
    /// this can be ruled out independently.
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl DialogSkipConfig {
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
            // Only an exact `false` turns it off, so a typo leaves the feature ON -- the harmless
            // direction when on is the default.
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
        };
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_dialog_skip::LOG_PREFIX,
            self.enabled
        )
    }
}
