//! Reading `[weapon_sync]` out of `<Game>/ds2-mods.toml`: whether to cap our weapon levels to the
//! other players' in multiplayer, and the key that turns it on and off in game.
//!
//! The feature lives in `ds2-weapon-sync`; this is only the switch, kept here for the same reason
//! every other feature's is -- the config file belongs to the loader.
//!
//! ```toml
//! [weapon_sync]
//! enabled = true
//! # Turns it off and on in game. Absent means F6; "none" leaves it unbound. Re-read live.
//! key = "F6"
//! # Optional. A pretend remote player at this level, so the cap can be tested solo.
//! # Re-read once a second while the game runs; delete the line to make the player leave.
//! test_cap = 3
//! ```

use ds2_hotkey_config::keys::{Chord, parse_chord};
use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "weapon_sync";

/// Whether to install the detours at all.
pub const KEY_ENABLED: &str = "enabled";

/// A pretend remote player's level, `0..=10`. Absent or anything else means none.
pub const KEY_TEST_CAP: &str = "test_cap";

/// The on/off key, a chord such as `"F6"` or `"ctrl+F6"`, or `"none"`.
pub const KEY_KEY: &str = "key";

/// `[weapon_sync]`, resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponSyncConfig {
    /// Install the clamp and the per-frame check. Off by default.
    pub enabled: bool,
    /// A pretend remote player at this level. Only for testing the mechanism alone.
    pub test_cap: Option<u8>,
    /// The on/off key. `None` is unbound.
    pub key: Option<Chord>,
    /// Why the configured key was not used, when it was not. The default is used instead.
    pub key_error: Option<String>,
}

impl Default for WeaponSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            test_cap: None,
            key: default_key(),
            key_error: None,
        }
    }
}

fn default_key() -> Option<Chord> {
    parse_chord(ds2_weapon_sync::DEFAULT_KEY).ok()
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
        let (key, key_error) = match values
            .get(CONFIG_SECTION, KEY_KEY)
            .map(|raw| raw.trim().trim_matches('"').trim())
        {
            None => (default_key(), None),
            Some(raw) if raw.is_empty() || raw.eq_ignore_ascii_case("none") => (None, None),
            Some(raw) => match parse_chord(raw) {
                Ok(chord) => (Some(chord), None),
                Err(error) => (
                    default_key(),
                    Some(format!(
                        "{KEY_KEY} = {raw:?} is not a key ({error:?}); using {}",
                        ds2_weapon_sync::DEFAULT_KEY
                    )),
                ),
            },
        };
        Self {
            enabled,
            test_cap,
            key,
            key_error,
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {KEY_KEY}={} {KEY_TEST_CAP}={}{}",
            ds2_weapon_sync::LOG_PREFIX,
            self.enabled,
            self.key
                .map_or_else(|| "none".to_string(), ds2_hotkey_config::chord_name),
            self.test_cap
                .map_or_else(|| "none".to_string(), |level| level.to_string()),
            self.key_error
                .as_ref()
                .map_or_else(String::new, |error| format!(" -- {error}"))
        )
    }
}

/// How often [`watch_live`] re-reads the file.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Re-read `test_cap` and `key` once a second for the life of the process and hand every change to
/// the crate. `enabled` is not re-read: the detours are installed once, at startup.
pub fn watch_live() {
    let first = WeaponSyncConfig::load();
    let (mut test_cap, mut key) = (first.test_cap, first.key);
    loop {
        std::thread::sleep(WATCH_INTERVAL);
        let now = WeaponSyncConfig::load();
        if now.test_cap != test_cap {
            ds2_weapon_sync::set_test_cap(now.test_cap);
            test_cap = now.test_cap;
        }
        if now.key != key {
            ds2_weapon_sync::set_key(now.key);
            key = now.key;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_section_leaves_it_off_with_the_default_key() {
        for text in ["", "[weapon_sync]\n", "[save_block]\nenabled = true\n"] {
            let config = WeaponSyncConfig::from_text(text);
            assert_eq!(config, WeaponSyncConfig::default());
            assert_eq!(config.key, parse_chord("F6").ok());
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
            WeaponSyncConfig::from_text(&format!(
                "[weapon_sync]\nenabled = true\ntest_cap = {value}\n"
            ))
            .test_cap
        };
        assert_eq!(read("3"), Some(3));
        assert_eq!(read("0"), Some(0));
        assert_eq!(read("10"), Some(10));
        assert_eq!(read("11"), None);
        assert_eq!(read("-1"), None);
        assert_eq!(read("none"), None);
    }

    #[test]
    fn the_key_is_a_chord_none_or_the_default_with_a_reason() {
        let read =
            |value: &str| WeaponSyncConfig::from_text(&format!("[weapon_sync]\nkey = {value}\n"));
        assert_eq!(read("\"F10\"").key, parse_chord("F10").ok());
        assert_eq!(read("ctrl+F6").key, parse_chord("ctrl+F6").ok());
        assert_eq!(read("\"none\"").key, None);
        let bad = read("\"not-a-key\"");
        assert_eq!(bad.key, parse_chord("F6").ok());
        assert!(bad.key_error.is_some());
        assert!(bad.describe().contains("not a key"));
    }
}
