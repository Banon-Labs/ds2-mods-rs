//! Reading `[invasion_path]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-invasion-path`; this is the switch and the path, kept here for the
//! reason every other feature's config is -- the config file belongs to the loader, and a feature
//! crate should not have to know where the game directory is.
//!
//! **Only `enabled` is read here.** The key binding, the distances and the target count are all
//! live settings the feature re-reads while the game runs, so the crate that uses them is the
//! crate that watches the file for them. All this passes over is where that file is.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `ds2-invasion-path`.
pub const CONFIG_SECTION: &str = "invasion_path";

/// Whether to install the overlay at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[invasion_path]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvasionPathConfig {
    /// Install the `Present` detour and draw the overlay.
    pub enabled: bool,
}

impl Default for InvasionPathConfig {
    /// **Off**, and unlike `[inventory_sort]` that is not a placeholder waiting for confidence.
    ///
    /// Two reasons, both structural rather than cautious:
    ///
    /// 1. This is the only feature in the workspace that detours a **rendering** function. Every
    ///    other one hooks a menu method or reads memory. A mistake in a menu hook is a menu that
    ///    misbehaves; a mistake in `Present` is the frame. Until a real run has shown the overlay
    ///    drawing and the game still rendering correctly afterwards, a player who installed this
    ///    DLL for the save redirect should not have their renderer hooked as a side effect.
    /// 2. It draws through the player's screen during multiplayer. That is a thing to opt into,
    ///    not to discover.
    ///
    /// Turning it on costs one line in `ds2-mods.toml`, and the toggle key still governs whether
    /// anything is actually drawn once it is on.
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl InvasionPathConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        // Three-way, matching `[inventory_sort]`: an exact `false` turns it off, an exact `true`
        // turns it on, and anything else falls back to the default. `describe` prints what was
        // resolved, so a typo shows up as `enabled=false` next to a config file that says `ture`,
        // which is the discrepancy that tells the player where to look.
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
            ds2_invasion_path::LOG_PREFIX,
            self.enabled
        )
    }
}
