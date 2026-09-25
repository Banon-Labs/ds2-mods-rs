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
    /// Off, like every feature here (user directive 2026-09-25): a config that says nothing is the
    /// game as shipped, boot screens included. `enabled = true` turns it on.
    ///
    /// This patches executable memory in three places during startup, so if a run fails to boot,
    /// the first question is whether this is why -- and one line answers it, with no rebuild.
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
            ds2_intro_skip::LOG_PREFIX,
            self.enabled
        )
    }
}
