//! Reading `[inventory_sort]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-inventory-sort`; this is the switch and the path, kept here for the
//! reason every other feature's config is -- the config file belongs to the loader, and a feature
//! crate should not have to know where the game directory is.
//!
//! **The BINDING is not read here**, unlike `enabled`. It is a hotkey, and a hotkey in this repo
//! moves without restarting the game, so the crate that reads the button also watches the file for
//! it. All this passes over is where that file is.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py` and in `ds2-inventory-sort`.
pub const CONFIG_SECTION: &str = "inventory_sort";

/// Whether to bind a button to the sort dialog at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[inventory_sort]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventorySortConfig {
    /// Open the shipped inventory sort dialog from a configurable key or controller button.
    pub enabled: bool,
}

impl Default for InventorySortConfig {
    /// **On**, and every part of that was pressed in game before the switch moved.
    ///
    /// Runs on 2026-08-29 logged `opening the sort dialog` on the Inventory tab, then `on=equip`
    /// for the equip picker, then three consecutive opens from the controller with the keyboard
    /// binding cleared. The dialog appeared each time and the game was alive after every call.
    ///
    /// The default binding is `lthumb` -- L3 -- which is where ELDEN RING puts Sort, and mirroring
    /// that is the entire point of the feature. So the surprise this could spring on a player who
    /// has not asked for it is the sort dialog they already had, on the button their hands already
    /// know, in a menu where the game itself offers no other way to reach it.
    ///
    /// The refusal path is the game's own: the shipped entry tests `[this+0x58]` and returns having
    /// done nothing while another dialog is up, and with no item list on screen there is no
    /// recorded group and the press is dropped here.
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl InventorySortConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        // THREE-WAY, and it stopped being two-way the moment the default flipped to ON. The old
        // rule was "only an exact `true` turns it on", which was the harmless direction while the
        // default was off: a typo left the feature off, which is where it already was. With the
        // default on, that same rule makes `enabled = ture` silently DELETE a working feature, and
        // the player has no way to tell a typo from a mod that stopped working.
        //
        // So an exact `false` turns it off, an exact `true` turns it on, and anything else is not
        // an answer -- it falls back to the default. `describe` prints what was resolved, so a
        // typo shows up as `enabled=true` next to a config file that says otherwise, which is the
        // discrepancy that tells the player where to look.
        let enabled = match parsed.get(CONFIG_SECTION, KEY_ENABLED) {
            None => Self::default().enabled,
            Some(raw) => match raw.trim().trim_matches('"') {
                "false" => false,
                "true" => true,
                _ => Self::default().enabled,
            },
        };
        Self { enabled }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={}",
            ds2_inventory_sort::LOG_PREFIX,
            self.enabled
        )
    }
}
