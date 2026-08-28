//! Reading `[offline]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-offline`; this is the switch that decides whether to turn it on, kept
//! here for the same reason every other feature's config is -- the config file belongs to the
//! loader, and a feature crate should not have to know where the game directory is.
//!
//! # Four keys, and why they are not one
//!
//! `enabled` is the master. Under it are three layers that answer different questions, and the
//! whole point of separating them is that a run can tell you which one did the work:
//!
//! * `pin_flag` -- the primary. `NetService::setOnline` becomes `ret`, so the online flag keeps
//!   the zero its own constructor wrote.
//! * `report_offline` -- the backstop. `NetService::isOnline` returns zero to all 34 of its
//!   readers whatever the byte holds.
//! * `block_sockets` -- the guarantee. The game's own `WS2_32` imports refuse anything that is not
//!   loopback.
//!
//! The third one is not redundant with the first two, and the disassembly is what says so:
//! `FeSubStateTitleGameServerLogin`'s work starter does not read the flag. A run with
//! `block_sockets = true` and the other two false is the arm that measures how much of the network
//! traffic the flag layer never reaches.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "offline";

/// Master switch for the whole feature.
pub const KEY_ENABLED: &str = "enabled";

/// Neuter `NetService::setOnline` so the flag keeps its constructed zero.
pub const KEY_PIN_FLAG: &str = "pin_flag";

/// Force `NetService::isOnline` to report offline.
pub const KEY_REPORT_OFFLINE: &str = "report_offline";

/// Refuse the game's non-loopback `connect`/`sendto`/name lookups.
pub const KEY_BLOCK_SOCKETS: &str = "block_sockets";

/// `[offline]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfflineConfig {
    /// Whether to install anything at all.
    pub enabled: bool,
    /// Make the online flag's setter inert.
    pub pin_flag: bool,
    /// Make the online flag's getter report zero.
    pub report_offline: bool,
    /// Front the outbound `WS2_32` imports.
    pub block_sockets: bool,
}

impl Default for OfflineConfig {
    /// **All on.** This workspace patches `.text` in a running copy of DARK SOULS II for a living,
    /// and that is what FromSoftware's servers are watching for. Shipping "online unless you
    /// remember to say otherwise" would make the default configuration the one that risks the
    /// player's account.
    ///
    /// Every layer is still a key rather than a constant, for the reason `intro_skip` gives: a run
    /// that fails has to be attributable to one change by editing one line, with no rebuild and
    /// nothing to re-stage.
    fn default() -> Self {
        Self {
            enabled: true,
            pin_flag: true,
            report_offline: true,
            block_sockets: true,
        }
    }
}

impl OfflineConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        // Only an exact `false` turns a key off, so a typo leaves the feature ON. That is the
        // harmless direction for every other feature in this loader and it is the *safe* direction
        // for this one: a misspelled value here means the player stays offline rather than
        // silently going online.
        let read = |key: &str, fallback: bool| match parsed.get(CONFIG_SECTION, key) {
            None => fallback,
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
        };
        let defaults = Self::default();
        Self {
            enabled: read(KEY_ENABLED, defaults.enabled),
            pin_flag: read(KEY_PIN_FLAG, defaults.pin_flag),
            report_offline: read(KEY_REPORT_OFFLINE, defaults.report_offline),
            block_sockets: read(KEY_BLOCK_SOCKETS, defaults.block_sockets),
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_ENABLED}={} {KEY_PIN_FLAG}={} \
             {KEY_REPORT_OFFLINE}={} {KEY_BLOCK_SOCKETS}={}",
            ds2_offline::LOG_PREFIX,
            self.enabled,
            self.pin_flag,
            self.report_offline,
            self.block_sockets
        )
    }
}
