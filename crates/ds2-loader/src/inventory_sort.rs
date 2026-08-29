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
    /// **Off**, and it is a holding position rather than a verdict.
    ///
    /// The feature patches two functions in the loaded image and then CALLS a third on the game
    /// thread, and none of that has been run yet. It also has a default key that is a placeholder
    /// -- nobody's muscle memory is on `F7` -- so a player who has not asked for this and has not
    /// chosen a button gets nothing rather than a surprise dialog.
    ///
    /// It flips to on once a run shows the dialog opening from the bound button and the shipped ①
    /// prompt still working afterwards.
    fn default() -> Self {
        Self { enabled: false }
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
        let enabled = match parsed.get(CONFIG_SECTION, KEY_ENABLED) {
            None => Self::default().enabled,
            // Only an exact `true` turns it on, so a typo leaves the feature OFF -- the harmless
            // direction while the default is off.
            Some(raw) => matches!(raw.trim().trim_matches('"'), "true"),
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
