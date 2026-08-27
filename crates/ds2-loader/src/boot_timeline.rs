//! Reading `[boot_timeline]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-boot-timeline`; this is only the switch, kept here for the same
//! reason every other feature's config is: the config file belongs to the loader, and a feature
//! crate should not have to know where the game directory is.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "boot_timeline";

/// Whether to instrument the title flow at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[boot_timeline]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootTimelineConfig {
    /// Detour `FeStateFlow::update` and the shared `v6` and log every substate entered and left.
    pub enabled: bool,
}

impl Default for BootTimelineConfig {
    /// **Off**, and it is the only feature in this repo that defaults off.
    ///
    /// The others default on because removing boot screens is what the mod is for. This one is a
    /// measuring instrument: it patches two more sites during startup and writes a burst of lines
    /// nobody playing the game asked for. Turning it on is a deliberate act taken to answer a
    /// question, and the run that answers the question is the run that turns it on.
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl BootTimelineConfig {
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
            // Only an exact `true` turns it on -- the MIRROR of every other feature here, and
            // deliberately so. Elsewhere a typo leaves the feature on, which is harmless because
            // on is the default and wanted. Here a typo must leave it OFF, because the harmless
            // direction for an instrument is "did not measure", never "patched two extra sites in
            // a run that was not supposed to be instrumented".
            Some(raw) => matches!(raw.trim().trim_matches('"'), "true"),
        };
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_boot_timeline::LOG_PREFIX,
            self.enabled
        )
    }
}
