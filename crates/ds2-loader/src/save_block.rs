//! Reading `[save_block]` out of `<Game>/ds2-mods.toml`: whether to stop the game saving by itself.
//!
//! The feature lives in `ds2-save-block`; this is only the switch, kept here for the same reason
//! `intro_skip`'s config is -- the config file belongs to the loader.
//!
//! It used to have no key: a `save-game-to-file` row on the menu was the switch, so the default menu
//! turned the game's own saving off. The user, 2026-09-25: "We want default saving from vanilla to be
//! on by default, with the ability to turn it off in the toml instead of always having it off."
//!
//! ```toml
//! [save_block]
//! enabled = true
//! ```
//!
//! `true` is honoured only in a run whose `save-game-to-file` row registered -- see
//! `install_menu_row` -- because a run with neither would have no way to save at all.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads.
pub const CONFIG_SECTION: &str = "save_block";

/// Whether to refuse the game's own saves.
pub const KEY_ENABLED: &str = "enabled";

/// `[save_block]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SaveBlockConfig {
    /// Refuse the autosave, the bonfire's and the one on the way out to the title.
    ///
    /// Off by default, like every feature here: a config that says nothing is the game as shipped.
    /// On, nothing gained since the last press of the save row survives the game closing, which is
    /// a cost a player has to have asked for.
    pub enabled: bool,
}

impl SaveBlockConfig {
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
        // Only an exact `true` turns the game's saving off. A typo leaves it on, because the failure
        // mode of the other direction is a lost character.
        let enabled = KeyValues::parse(text)
            .get(CONFIG_SECTION, KEY_ENABLED)
            .is_some_and(|raw| raw.trim().trim_matches('"') == "true");
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_save_block::LOG_PREFIX,
            self.enabled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saying nothing leaves the game saving.
    #[test]
    fn an_absent_key_leaves_the_game_saving() {
        for text in ["", "[save_block]\n", "[menu_row]\nenabled = true\n"] {
            assert!(!SaveBlockConfig::from_text(text).enabled, "{text:?}");
        }
    }

    /// Exactly `true` turns it on; anything else, typos included, leaves it off.
    #[test]
    fn only_an_exact_true_turns_it_on() {
        for value in ["true", "\"true\"", " true "] {
            let text = format!("[save_block]\nenabled = {value}\n");
            assert!(SaveBlockConfig::from_text(&text).enabled, "{value:?}");
        }
        for value in ["false", "ture", "1", "yes", "TRUE", ""] {
            let text = format!("[save_block]\nenabled = {value}\n");
            assert!(!SaveBlockConfig::from_text(&text).enabled, "{value:?}");
        }
    }
}
