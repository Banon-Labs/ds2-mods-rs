//! Reading `[continue]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-continue`; this is only the switch, kept here for the same reason
//! every other feature's config is: the config file belongs to the loader, and a feature crate
//! should not have to know where the game directory is.
//!
//! The module is `continue_flow` and not `continue` because the latter is a Rust keyword. The
//! config **section** is `[continue]`, which is what the user types, and that is the name that
//! matters.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "continue";

/// Whether to record which save slot a load used.
pub const KEY_RECORD: &str = "record";

/// The slot to select when the character list opens. Negative disables it.
pub const KEY_SLOT: &str = "slot";

/// Whether to hold the master channel group at zero for the length of the shortcut.
pub const KEY_SILENCE: &str = "silence";

/// Whether to cover the title flow with the game's own NOW LOADING screen.
pub const KEY_LOADING_SCREEN: &str = "loading_screen";

/// `[continue]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContinueConfig {
    /// Detour `FeSubStateTitleLoadDataList::v3` and log the slot, the action and the phase it
    /// decides. Records only -- nothing here writes into the game.
    pub record: bool,
    /// Slot to pre-select when the character list opens, or negative to leave the game's own
    /// selection alone.
    pub slot: i32,
    /// Mute FMOD's master channel group from install until `FeSubStateTitleStartIngame`, then
    /// restore the volume the game itself had applied.
    pub silence: bool,
    /// Raise `FeOperatorNowLoading` over the title flow, and drop it at
    /// `FeSubStateTitleStartIngame`.
    pub loading_screen: bool,
}

impl Default for ContinueConfig {
    /// **The feature is off.** Same reasoning as `[boot_timeline]`: this is an instrument as much
    /// as a feature, and the harmless direction for an instrument is "did not measure", never
    /// "patched a site in a run that was not supposed to be instrumented".
    ///
    /// `silence` is the exception and defaults on, because it cannot act unless `slot` has already
    /// turned the shortcut on -- so it changes nothing about which runs patch which sites.
    fn default() -> Self {
        Self {
            record: false,
            slot: -1,
            // ON, unlike the other two, and for the opposite reason. `record` and `slot` are
            // inert until asked for; this one only ever does anything when `slot` has already
            // turned the shortcut on, and a shortcut that plays the confirm sound for a button
            // nobody pressed is the surprising behaviour, not the quiet one. Set it to `false` to
            // hear the title as the game plays it.
            silence: true,
            // ON, for the same reason as `silence` and with the same guard: it cannot act unless
            // `slot` has already turned the shortcut on. Set it to `false` to watch the title
            // flow drive itself.
            loading_screen: true,
        }
    }
}

impl ContinueConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        let record = match parsed.get(CONFIG_SECTION, KEY_RECORD) {
            None => Self::default().record,
            // Only an exact `true` turns it on, mirroring `[boot_timeline]` rather than the
            // skips: a typo must leave an instrument OFF.
            Some(raw) => matches!(raw.trim().trim_matches('"'), "true"),
        };
        let slot = parsed
            .get(CONFIG_SECTION, KEY_SLOT)
            .and_then(|raw| raw.trim().trim_matches('"').parse::<i32>().ok())
            .unwrap_or(Self::default().slot);
        // Defaults ON, so the exact-match test is inverted from the other two: only a literal
        // `false` turns it off, and a typo leaves it on.
        let silence = match parsed.get(CONFIG_SECTION, KEY_SILENCE) {
            None => Self::default().silence,
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
        };
        // Defaults ON, so like `silence` only a literal `false` turns it off.
        let loading_screen = match parsed.get(CONFIG_SECTION, KEY_LOADING_SCREEN) {
            None => Self::default().loading_screen,
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
        };
        Self {
            record,
            slot,
            silence,
            loading_screen,
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_RECORD}={} {KEY_SLOT}={} {KEY_SILENCE}={} \
             {KEY_LOADING_SCREEN}={}",
            ds2_continue::LOG_PREFIX,
            self.record,
            self.slot,
            self.silence,
            self.loading_screen
        )
    }
}
