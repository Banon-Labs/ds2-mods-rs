//! Reading `[intro_skip]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature itself lives in `ds2-intro-skip`; this is only the switch that decides whether to
//! turn it on, kept here for the same reason `crash_logging`'s config is: the config file belongs
//! to the loader, and the feature crate should not have to know where the game directory is.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "intro_skip";

/// Whether to skip the boot screens at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[intro_skip]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntroSkipConfig {
    /// Detour the three boot screens' `enter` so each reports itself finished immediately.
    pub enabled: bool,
}

impl Default for IntroSkipConfig {
    /// **Off.** This patches executable memory in three places to change what the player sees at
    /// boot, and a mod that does that without being asked is a mod that gets blamed for the next
    /// unrelated startup problem. `scripts/ds2-run.py` writes the key explicitly on every launch.
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl IntroSkipConfig {
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
            // Strict, and deliberately so: anything that is not exactly `true` or `false` is a
            // typo, and a typo that reads as `true` would turn the feature on by accident.
            Some(raw) => matches!(raw.trim().trim_matches('"'), "true"),
        };
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_intro_skip::LOG_PREFIX,
            self.enabled
        )
    }
}
