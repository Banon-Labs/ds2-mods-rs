//! Reading `[soul_memory_guard]` out of `<Game>/ds2-mods.toml`: whether to judge each loaded
//! character's soul memory against its level.
//!
//! The feature lives in `ds2-soul-memory-guard`; this is only the switch, kept here for the same
//! reason every other feature's is -- the config file belongs to the loader.
//!
//! ```toml
//! [soul_memory_guard]
//! enabled = true
//! ```

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "soul_memory_guard";

/// Whether to install the detour at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[soul_memory_guard]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SoulMemoryGuardConfig {
    /// Detour the character restore and log a verdict line per load.
    ///
    /// Off by default, like every feature here: a config that says nothing is the game as shipped.
    pub enabled: bool,
}

impl SoulMemoryGuardConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::from_text(&text)
    }

    /// The same decision against a config file's text, so it can be tested without one on disk.
    pub fn from_text(text: &str) -> Self {
        let enabled = KeyValues::parse(text)
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_soul_memory_guard::LOG_PREFIX,
            self.enabled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_key_leaves_it_off() {
        for text in [
            "",
            "[soul_memory_guard]\n",
            "[save_block]\nenabled = true\n",
        ] {
            assert!(!SoulMemoryGuardConfig::from_text(text).enabled, "{text:?}");
        }
    }

    #[test]
    fn only_an_exact_true_turns_it_on() {
        for value in ["true", "\"true\"", " true "] {
            let text = format!("[soul_memory_guard]\nenabled = {value}\n");
            assert!(SoulMemoryGuardConfig::from_text(&text).enabled, "{value:?}");
        }
        for value in ["false", "ture", "1", "yes", "TRUE", ""] {
            let text = format!("[soul_memory_guard]\nenabled = {value}\n");
            assert!(
                !SoulMemoryGuardConfig::from_text(&text).enabled,
                "{value:?}"
            );
        }
    }
}
