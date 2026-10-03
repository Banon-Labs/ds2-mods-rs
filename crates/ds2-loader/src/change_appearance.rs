//! Reading `[change_appearance]` out of `<Game>/ds2-mods.toml`: whether to add the bonfire menu's
//! "Change Appearance" row.
//!
//! The feature lives in `ds2-change-appearance`; this is only the switch, kept here for the same
//! reason every other feature's is -- the config file belongs to the loader.
//!
//! ```toml
//! [change_appearance]
//! enabled = true
//! ```

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "change_appearance";

/// Whether to install the row at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[change_appearance]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChangeAppearanceConfig {
    /// Add the row and the hooks behind it.
    ///
    /// Off by default, like every feature here: a config that says nothing is the game as shipped.
    pub enabled: bool,
}

impl ChangeAppearanceConfig {
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
        Self {
            enabled: values
                .get(CONFIG_SECTION, KEY_ENABLED)
                .is_some_and(|raw| raw.trim().trim_matches('"') == "true"),
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_change_appearance::LOG_PREFIX,
            self.enabled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_key_leaves_it_off() {
        for text in ["", "[change_appearance]\n", "[estus_max]\nenabled = true\n"] {
            assert!(!ChangeAppearanceConfig::from_text(text).enabled, "{text:?}");
        }
    }

    #[test]
    fn a_release_leaves_it_off() {
        let shipped = include_str!("../../../.github/dist-ds2-mods.toml");
        assert!(!ChangeAppearanceConfig::from_text(shipped).enabled);
    }

    #[test]
    fn only_an_exact_true_turns_it_on() {
        assert!(ChangeAppearanceConfig::from_text("[change_appearance]\nenabled = true\n").enabled);
        for value in ["false", "1", "yes", "TRUE", ""] {
            let text = format!("[change_appearance]\nenabled = {value}\n");
            assert!(
                !ChangeAppearanceConfig::from_text(&text).enabled,
                "{value:?}"
            );
        }
    }
}
