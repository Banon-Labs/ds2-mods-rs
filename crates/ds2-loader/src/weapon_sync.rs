//! Reading `[weapon_sync]` and `[armor_sync]` out of `<Game>/ds2-mods.toml`.
//!
//! Whether to cap our weapon levels, and our armour levels, to the other players' in multiplayer,
//! and the key that turns each on and off in game. The two sections are read the same way and are
//! independent: either feature can be on without the other.
//!
//! Both features live in `ds2-weapon-sync`; this is only the switch, kept here for the same reason
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
//! # Optional. A pretend invader at this level: while we host, the cap follows the invaders
//! # alone. Re-read live like test_cap.
//! test_invader = 2
//!
//! [armor_sync]
//! enabled = true
//! # Absent means F5.
//! key = "F5"
//! test_cap = 3
//! ```

use ds2_hotkey_config::keys::{Chord, parse_chord};
use ds2_hotkey_config::kv::KeyValues;
use ds2_weapon_sync::Kind;

use crate::crash_logging::config_file_path;

/// The weapon section. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "weapon_sync";

/// The armour section. Mirrored in `scripts/ds2-run.py`.
pub const ARMOR_CONFIG_SECTION: &str = "armor_sync";

/// Whether to install the detours at all.
pub const KEY_ENABLED: &str = "enabled";

/// A pretend remote player's level, `0..=10`. Absent or anything else means none.
pub const KEY_TEST_CAP: &str = "test_cap";

/// A pretend invader's level, `0..=10`. Absent or anything else means none.
pub const KEY_TEST_INVADER: &str = "test_invader";

/// The on/off key, a chord such as `"F6"` or `"ctrl+F6"`, or `"none"`.
pub const KEY_KEY: &str = "key";

/// Which feature's section, default key and log prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    /// The feature in `ds2-weapon-sync`.
    pub kind: Kind,
    /// The TOML section.
    pub name: &'static str,
    /// The key used when the section does not name one.
    pub default_key: &'static str,
    /// The feature's log prefix.
    pub prefix: &'static str,
}

/// `[weapon_sync]`.
pub const WEAPONS: Section = Section {
    kind: Kind::Weapon,
    name: CONFIG_SECTION,
    default_key: ds2_weapon_sync::DEFAULT_KEY,
    prefix: ds2_weapon_sync::LOG_PREFIX,
};

/// `[armor_sync]`.
pub const ARMOR: Section = Section {
    kind: Kind::Armor,
    name: ARMOR_CONFIG_SECTION,
    default_key: ds2_weapon_sync::ARMOR_DEFAULT_KEY,
    prefix: ds2_weapon_sync::ARMOR_LOG_PREFIX,
};

/// One section, resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponSyncConfig {
    /// Which section this was read from.
    pub section: Section,
    /// Install the clamp and the per-frame check. Off by default.
    pub enabled: bool,
    /// A pretend remote player at this level. Only for testing the mechanism alone.
    pub test_cap: Option<u8>,
    /// A pretend invader at this level. Only for testing the mechanism alone.
    pub test_invader: Option<u8>,
    /// The on/off key. `None` is unbound.
    pub key: Option<Chord>,
    /// Why the configured key was not used, when it was not. The default is used instead.
    pub key_error: Option<String>,
}

impl WeaponSyncConfig {
    /// A section with nothing in it: off, with the default key.
    pub fn default_for(section: Section) -> Self {
        Self {
            section,
            enabled: false,
            test_cap: None,
            test_invader: None,
            key: parse_chord(section.default_key).ok(),
            key_error: None,
        }
    }

    /// Read one section. A missing file or a missing key means [`WeaponSyncConfig::default_for`].
    pub fn load(section: Section) -> Self {
        let Some(path) = config_file_path() else {
            return Self::default_for(section);
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default_for(section);
        };
        Self::from_text(section, &text)
    }

    /// The same decision against a config file's text, so it can be tested without one on disk.
    pub fn from_text(section: Section, text: &str) -> Self {
        let values = KeyValues::parse(text);
        let enabled = values
            .get(section.name, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        let level = |key: &str| {
            values
                .get(section.name, key)
                .and_then(|raw| raw.trim().trim_matches('"').parse::<u8>().ok())
                .filter(|level| *level <= section.kind.level_max())
        };
        let test_cap = level(KEY_TEST_CAP);
        let test_invader = level(KEY_TEST_INVADER);
        let default_key = || parse_chord(section.default_key).ok();
        let (key, key_error) = match values
            .get(section.name, KEY_KEY)
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
                        section.default_key
                    )),
                ),
            },
        };
        Self {
            section,
            enabled,
            test_cap,
            test_invader,
            key,
            key_error,
        }
    }

    /// The settings `ds2_weapon_sync::install` takes, or `None` when the section is off.
    #[cfg(windows)]
    pub fn settings(&self) -> Option<ds2_weapon_sync::Settings> {
        self.enabled.then_some(ds2_weapon_sync::Settings {
            test_cap: self.test_cap,
            test_invader: self.test_invader,
            key: self.key,
        })
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{}] {KEY_ENABLED}={} {KEY_KEY}={} {KEY_TEST_CAP}={} {KEY_TEST_INVADER}={}{}",
            self.section.prefix,
            self.section.name,
            self.enabled,
            self.key
                .map_or_else(|| "none".to_string(), ds2_hotkey_config::chord_name),
            self.test_cap
                .map_or_else(|| "none".to_string(), |level| level.to_string()),
            self.test_invader
                .map_or_else(|| "none".to_string(), |level| level.to_string()),
            self.key_error
                .as_ref()
                .map_or_else(String::new, |error| format!(" -- {error}"))
        )
    }
}

/// How often [`watch_live`] re-reads the file.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Re-read one section's `test_cap` and `key` once a second for the life of the process and hand
/// every change to the crate. `enabled` is not re-read: the detours are installed once, at startup.
#[cfg(windows)]
pub fn watch_live(section: Section) {
    let first = WeaponSyncConfig::load(section);
    let (mut test_cap, mut test_invader, mut key) = (first.test_cap, first.test_invader, first.key);
    loop {
        std::thread::sleep(WATCH_INTERVAL);
        let now = WeaponSyncConfig::load(section);
        if now.test_cap != test_cap {
            ds2_weapon_sync::set_test_cap(section.kind, now.test_cap);
            test_cap = now.test_cap;
        }
        if now.test_invader != test_invader {
            ds2_weapon_sync::set_test_invader(section.kind, now.test_invader);
            test_invader = now.test_invader;
        }
        if now.key != key {
            ds2_weapon_sync::set_key(section.kind, now.key);
            key = now.key;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weapons(text: &str) -> WeaponSyncConfig {
        WeaponSyncConfig::from_text(WEAPONS, text)
    }

    #[test]
    fn an_absent_section_leaves_it_off_with_the_default_key() {
        for text in ["", "[weapon_sync]\n", "[save_block]\nenabled = true\n"] {
            let config = weapons(text);
            assert_eq!(config, WeaponSyncConfig::default_for(WEAPONS));
            assert_eq!(config.key, parse_chord("F6").ok());
        }
        let armor = WeaponSyncConfig::from_text(ARMOR, "");
        assert!(!armor.enabled);
        assert_eq!(armor.key, parse_chord("F5").ok());
    }

    #[test]
    fn only_an_exact_true_turns_it_on() {
        assert!(weapons("[weapon_sync]\nenabled = true\n").enabled);
        for value in ["false", "1", "yes", "TRUE", ""] {
            let text = format!("[weapon_sync]\nenabled = {value}\n");
            assert!(!weapons(&text).enabled, "{value:?}");
        }
    }

    #[test]
    fn the_two_sections_are_read_independently() {
        let text = "[weapon_sync]\nenabled = true\nkey = \"F6\"\ntest_cap = 2\n\
                    [armor_sync]\nenabled = false\nkey = \"ctrl+F5\"\ntest_cap = 7\n";
        let w = WeaponSyncConfig::from_text(WEAPONS, text);
        let a = WeaponSyncConfig::from_text(ARMOR, text);
        assert!(w.enabled && !a.enabled);
        assert_eq!((w.test_cap, a.test_cap), (Some(2), Some(7)));
        assert_eq!(a.key, parse_chord("ctrl+F5").ok());
        let only_armor = "[armor_sync]\nenabled = true\n";
        assert!(!WeaponSyncConfig::from_text(WEAPONS, only_armor).enabled);
        assert!(WeaponSyncConfig::from_text(ARMOR, only_armor).enabled);
        assert!(
            WeaponSyncConfig::from_text(ARMOR, only_armor)
                .describe()
                .starts_with("ds2-armor-sync: config [armor_sync] enabled=true key=F5")
        );
    }

    #[test]
    fn test_cap_is_a_level_or_nothing() {
        let read = |value: &str| {
            weapons(&format!(
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
    fn test_invader_is_a_level_or_nothing_and_independent_of_test_cap() {
        let config = weapons("[weapon_sync]\nenabled = true\ntest_cap = 9\ntest_invader = 3\n");
        assert_eq!((config.test_cap, config.test_invader), (Some(9), Some(3)));
        assert!(config.describe().contains("test_cap=9 test_invader=3"));
        let armor = WeaponSyncConfig::from_text(ARMOR, "[armor_sync]\ntest_invader = 11\n");
        assert_eq!(armor.test_invader, None);
        assert_eq!(weapons("[weapon_sync]\ntest_cap = 2\n").test_invader, None);
    }

    #[test]
    fn the_key_is_a_chord_none_or_the_default_with_a_reason() {
        let read = |value: &str| weapons(&format!("[weapon_sync]\nkey = {value}\n"));
        assert_eq!(read("\"F10\"").key, parse_chord("F10").ok());
        assert_eq!(read("ctrl+F6").key, parse_chord("ctrl+F6").ok());
        assert_eq!(read("\"none\"").key, None);
        let bad = read("\"not-a-key\"");
        assert_eq!(bad.key, parse_chord("F6").ok());
        assert!(bad.key_error.is_some());
        assert!(bad.describe().contains("not a key"));
    }
}
