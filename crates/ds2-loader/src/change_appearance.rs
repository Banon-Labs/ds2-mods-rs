//! The switches for the bonfire menu's three added rows, out of `<Game>/ds2-mods.toml`.
//!
//! `[change_appearance]`, `[rename_character]` and `[reallocate_stats]` each add one row: "Change
//! Appearance", "Rename Character" and "Reallocate Stats".
//!
//! The feature lives in `ds2-change-appearance`; this is only the switch, kept here for the same
//! reason every other feature's is -- the config file belongs to the loader.
//!
//! ```toml
//! [change_appearance]
//! enabled = true
//!
//! [rename_character]
//! enabled = true
//!
//! [reallocate_stats]
//! enabled = true
//! ```

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "change_appearance";

/// The rename row's section. Mirrored in `scripts/ds2-run.py`.
pub const RENAME_SECTION: &str = "rename_character";

/// The reallocate row's section. Mirrored in `scripts/ds2-run.py`.
pub const REALLOCATE_SECTION: &str = "reallocate_stats";

/// Whether to add the section's row. The same key in both sections.
pub const KEY_ENABLED: &str = "enabled";

/// `[change_appearance]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChangeAppearanceConfig {
    /// Add the row and the hooks behind it.
    ///
    /// Off by default, like every feature here: a config that says nothing is the game as shipped.
    pub enabled: bool,
    /// `[rename_character] enabled`: add the "Rename Character" row. Off by default as well.
    pub rename: bool,
    /// `[reallocate_stats] enabled`: add the "Reallocate Stats" row. Off by default as well.
    pub reallocate: bool,
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
        let on = |section| {
            values
                .get(section, KEY_ENABLED)
                .is_some_and(|raw| raw.trim().trim_matches('"') == "true")
        };
        Self {
            enabled: on(CONFIG_SECTION),
            rename: on(RENAME_SECTION),
            reallocate: on(REALLOCATE_SECTION),
        }
    }

    /// The rows to ask `ds2-change-appearance` for.
    pub const fn rows(&self) -> ds2_change_appearance::Rows {
        ds2_change_appearance::Rows {
            change_appearance: self.enabled,
            rename: self.rename,
            reallocate: self.reallocate,
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} [{RENAME_SECTION}] {KEY_ENABLED}={} \
             [{REALLOCATE_SECTION}] {KEY_ENABLED}={}",
            ds2_change_appearance::LOG_PREFIX,
            self.enabled,
            self.rename,
            self.reallocate
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
        assert!(!ChangeAppearanceConfig::from_text(shipped).rows().any());
    }

    #[test]
    fn each_section_switches_its_own_row() {
        let rename_only = ChangeAppearanceConfig::from_text("[rename_character]\nenabled = true\n");
        assert_eq!(
            rename_only.rows(),
            ds2_change_appearance::Rows {
                change_appearance: false,
                rename: true,
                reallocate: false
            }
        );
        let reallocate_only =
            ChangeAppearanceConfig::from_text("[reallocate_stats]\nenabled = true\n");
        assert_eq!(
            reallocate_only.rows(),
            ds2_change_appearance::Rows {
                change_appearance: false,
                rename: false,
                reallocate: true
            }
        );
        let both = ChangeAppearanceConfig::from_text(
            "[change_appearance]\nenabled = true\n[rename_character]\nenabled = true\n",
        );
        assert!(both.enabled && both.rename);
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
