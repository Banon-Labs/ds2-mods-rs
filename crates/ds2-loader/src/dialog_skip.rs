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
    /// Off, like every feature here (user directive 2026-09-25): a config that says nothing is the
    /// game as shipped. `enabled = true` turns it on, and it is wanted together with `intro_skip`
    /// -- skipping the boot screens only to stop on a message box is half a feature.
    ///
    /// Two separate switches, not one, so a run that fails to boot can have `intro_skip` and this
    /// ruled out independently by editing one line rather than by rebuilding.
    fn default() -> Self {
        Self { enabled: false }
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
        // Only an exact `true` turns it on, so a typo leaves the game as shipped.
        let enabled = parsed
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
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
