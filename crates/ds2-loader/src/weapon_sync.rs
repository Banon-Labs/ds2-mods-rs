//! Reading `[weapon_sync]` out of `<Game>/ds2-mods.toml`: whether to cap our weapon levels to the
//! other players' in multiplayer.
//!
//! The feature lives in `ds2-weapon-sync`; this is only the switch, kept here for the same reason
//! every other feature's is -- the config file belongs to the loader.
//!
//! ```toml
//! [weapon_sync]
//! enabled = true
//! # Optional. A pretend remote player at this level, so the cap can be tested solo.
//! test_cap = 3
//! ```

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "weapon_sync";

/// Whether to install the detours at all.
pub const KEY_ENABLED: &str = "enabled";

/// A pretend remote player's level, `0..=10`. Absent or anything else means none.
pub const KEY_TEST_CAP: &str = "test_cap";

/// `[weapon_sync]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WeaponSyncConfig {
    /// Install the clamp and the per-frame check. Off by default.
    pub enabled: bool,
    /// A pretend remote player at this level. Only for testing the mechanism alone.
    pub test_cap: Option<u8>,
}

impl WeaponSyncConfig {
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
        let enabled = values
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        let test_cap = values
            .get(CONFIG_SECTION, KEY_TEST_CAP)
            .and_then(|raw| raw.trim().trim_matches('"').parse::<u8>().ok())
            .filter(|level| *level <= ds2_rva::WEAPON_LEVEL_MAX);
        Self { enabled, test_cap }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {KEY_TEST_CAP}={}",
            ds2_weapon_sync::LOG_PREFIX,
            self.enabled,
            self.test_cap
                .map_or_else(|| "none".to_string(), |level| level.to_string())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_section_leaves_it_off() {
        for text in ["", "[weapon_sync]\n", "[save_block]\nenabled = true\n"] {
            assert_eq!(WeaponSyncConfig::from_text(text), WeaponSyncConfig::default());
        }
    }

    #[test]
    fn only_an_exact_true_turns_it_on() {
        assert!(WeaponSyncConfig::from_text("[weapon_sync]\nenabled = true\n").enabled);
        for value in ["false", "1", "yes", "TRUE", ""] {
            let text = format!("[weapon_sync]\nenabled = {value}\n");
            assert!(!WeaponSyncConfig::from_text(&text).enabled, "{value:?}");
        }
    }

    #[test]
    fn test_cap_is_a_level_or_nothing() {
        let read = |value: &str| {
            WeaponSyncConfig::from_text(&format!("[weapon_sync]\nenabled = true\ntest_cap = {value}\n"))
                .test_cap
        };
        assert_eq!(read("3"), Some(3));
        assert_eq!(read("0"), Some(0));
        assert_eq!(read("10"), Some(10));
        assert_eq!(read("11"), None);
        assert_eq!(read("-1"), None);
        assert_eq!(read("none"), None);
    }
}
