//! Reading `[estus_max]` out of `<Game>/ds2-mods.toml`: whether to keep the loaded character's
//! Estus Flask at the game's maximum uses and effect levels.
//!
//! The feature lives in `ds2-estus-max`; this is only the switch, kept here for the same
//! reason every other feature's is -- the config file belongs to the loader.
//!
//! ```toml
//! [estus_max]
//! enabled = true
//! ```

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "estus_max";

/// Whether to register the tick at all.
pub const KEY_ENABLED: &str = "enabled";

/// Whether to run the one-shot in-process reload test. Only meaningful with `enabled`.
pub const KEY_RELOAD_TEST: &str = "reload_test";

/// `[estus_max]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EstusMaxConfig {
    /// Raise both flask levels to the maximum on every load and log what the game read back.
    ///
    /// Off by default, like every feature here: a config that says nothing is the game as shipped.
    pub enabled: bool,
    /// A test instrument: after the first load reaches max, return to the title once and let
    /// `ds2-continue` load the same slot once more, so a second load is read inside one process.
    pub reload_test: bool,
}

impl EstusMaxConfig {
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
        let values = KeyValues::parse(text);
        let flag = |key| {
            values
                .get(CONFIG_SECTION, key)
                .is_some_and(|raw| raw.trim().trim_matches('"') == "true")
        };
        Self {
            enabled: flag(KEY_ENABLED),
            reload_test: flag(KEY_RELOAD_TEST),
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {KEY_RELOAD_TEST}={}",
            ds2_estus_max::LOG_PREFIX,
            self.enabled,
            self.reload_test
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_key_leaves_it_off() {
        for text in ["", "[estus_max]\n", "[save_block]\nenabled = true\n"] {
            assert!(!EstusMaxConfig::from_text(text).enabled, "{text:?}");
        }
    }

    #[test]
    fn the_reload_test_is_its_own_key_and_off_by_default() {
        let on = EstusMaxConfig::from_text("[estus_max]\nenabled = true\n");
        assert!(on.enabled && !on.reload_test);
        let test = EstusMaxConfig::from_text("[estus_max]\nenabled = true\nreload_test = true\n");
        assert!(test.enabled && test.reload_test);
    }

    #[test]
    fn only_an_exact_true_turns_it_on() {
        for value in ["true", "\"true\"", " true "] {
            let text = format!("[estus_max]\nenabled = {value}\n");
            assert!(EstusMaxConfig::from_text(&text).enabled, "{value:?}");
        }
        for value in ["false", "ture", "1", "yes", "TRUE", ""] {
            let text = format!("[estus_max]\nenabled = {value}\n");
            assert!(!EstusMaxConfig::from_text(&text).enabled, "{value:?}");
        }
    }
}
