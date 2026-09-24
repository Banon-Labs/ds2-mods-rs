//! Reading `[item_warn]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-item-warn`; this is the switch, kept here for the reason every other
//! feature's config is -- the config file belongs to the loader, and a feature crate should not
//! have to know where the game directory is.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "item_warn";

/// Whether to mark weapons the player's stats cannot meet.
pub const KEY_ENABLED: &str = "enabled";

/// `[item_warn]`, resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemWarnConfig {
    /// Put a red badge on the icon of any weapon whose requirements the player fails.
    pub enabled: bool,
}

impl ItemWarnConfig {
    /// Read the section. A missing file or a missing key means off.
    ///
    /// **Off by default, and the default is not taste.** This feature has never been in front of a
    /// running game: it patches two functions in the frontend's layout builder and its cell bind,
    /// and the argument that it is safe is an argument from static reading alone. `inventory_sort`
    /// defaults on because three runs put its dialog on screen; this one has no such line to point
    /// at, so it is opt-in until it does.
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        // Only an exact `true` turns it on, which is the harmless direction while the default is
        // off: a typo leaves the feature where it already was. `describe` prints what was resolved
        // so a typo shows up as `enabled=false` next to a config file that says otherwise.
        let enabled = KeyValues::parse(&text)
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_item_warn::LOG_PREFIX,
            self.enabled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_off() {
        assert!(!ItemWarnConfig::default().enabled);
    }

    #[test]
    fn the_line_names_the_section_and_the_value() {
        let line = ItemWarnConfig { enabled: true }.describe();
        assert!(line.contains("[item_warn]"));
        assert!(line.contains("enabled=true"));
        assert!(line.starts_with(ds2_item_warn::LOG_PREFIX));
    }
}
