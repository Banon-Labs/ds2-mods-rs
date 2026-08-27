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
    /// **On.** Removing the boot screens is what this mod is for; making that conditional on a
    /// flag means the default experience is the one nobody wanted.
    ///
    /// It is still a key rather than a constant, and that is the part that matters. This patches
    /// executable memory in three places during startup, so if a future run fails to boot, the
    /// first question is whether this is why -- and `enabled = false` answers it by editing one
    /// line, with no rebuild and no rebuilt DLL to stage. A default that cannot be turned off is
    /// a default that cannot be ruled out.
    fn default() -> Self {
        Self { enabled: true }
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
            // Only an exact `false` turns it off. A typo therefore leaves the feature ON, which
            // is the harmless direction now that on is the default: the failure mode of a
            // misspelled value is "the mod still works", not "the mod silently stopped".
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
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
