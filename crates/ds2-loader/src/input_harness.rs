//! Reading `[input_harness]` out of `<Game>/ds2-mods.toml`.
//!
//! The feature lives in `ds2-input-harness`; this is the switch, kept here for the reason every
//! other feature's config is -- the config file belongs to the loader, and a feature crate
//! should not have to know where the game directory is.
//!
//! **Only `enabled` is read here.** Everything else the harness does is asked for at runtime
//! through `<Game>/ds2-input-harness-cmd.txt`, because the point of the thing is to be driven
//! while the game is already running.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads.
pub const CONFIG_SECTION: &str = "input_harness";

/// Whether to detour the three device polls at all.
pub const KEY_ENABLED: &str = "enabled";

/// `[input_harness]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputHarnessConfig {
    /// Detour `DLUID::PadDevice`, `MouseDevice` and `KeyboardDevice`'s polls.
    pub enabled: bool,
}

impl Default for InputHarnessConfig {
    /// **Off**, and for a blunter reason than any other feature in this workspace.
    ///
    /// This is the only thing here that can take a player's controller away from them. Its whole
    /// job is to write the input the engine reads and, on request, to blank what the hardware
    /// produced -- which is exactly right for an unattended measurement and exactly wrong to
    /// arrive uninvited in somebody's playthrough. It also detours three functions on the
    /// engine's input path, which is not a hook to install on a run that never asked for it.
    ///
    /// Every command it can be given is frame-bounded and the block has a hard cap, so the
    /// failure mode of a harness that is on and wedges is "the stick lets go after a while"
    /// rather than "the game stops responding". That is a reason it is safe to turn ON
    /// deliberately, not a reason to default it on.
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl InputHarnessConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        // Three-way, matching `[invasion_path]`: an exact `false` turns it off, an exact `true`
        // turns it on, and anything else falls back to the default. `describe` prints what was
        // resolved, so a typo shows up as `enabled=false` next to a config file that says
        // `ture`, which is the discrepancy that tells the player where to look.
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
            ds2_input_harness::LOG_PREFIX,
            self.enabled
        )
    }
}
