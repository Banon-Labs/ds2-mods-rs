//! Reading `[title_skip]` out of `<Game>/ds2-mods.toml`.
//!
//! Separate from `[dialog_skip]` because these are separate mechanisms on separate functions. The
//! notice boxes wait for a button and are suppressed outright; these two do not, and neither is
//! suppressed -- the press gate is forced so the game's own path runs, and the process windows keep
//! every wait they had and lose only an artificial minimum display time.
//!
//! Two keys rather than one, for the reason every switch in this repo is its own switch: a run that
//! fails to boot has to be attributable to one hook by editing one line.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "title_skip";

/// Force the "press any button" gate.
pub const KEY_PRESS_ANY_BUTTON: &str = "press_any_button";

/// Clear the "please wait" windows' minimum display time.
pub const KEY_PROCESS_WINDOWS: &str = "process_windows";

/// `[title_skip]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleSkipConfig {
    /// Detour the press-any-button poll so it always reports a press.
    pub press_any_button: bool,
    /// Zero each process window's minimum display duration as it opens.
    pub process_windows: bool,
}

impl Default for TitleSkipConfig {
    /// Both **on**, matching `intro_skip` and `dialog_skip`. Getting to the menu without touching
    /// anything is the point; leaving one of the four stops in place would mean the default is
    /// still a run that has to be babysat.
    fn default() -> Self {
        Self {
            press_any_button: true,
            process_windows: true,
        }
    }
}

impl TitleSkipConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        // Only an exact `false` turns a key off, so a typo leaves the feature ON -- the harmless
        // direction when on is the default.
        let read = |key: &str, fallback: bool| match parsed.get(CONFIG_SECTION, key) {
            None => fallback,
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
        };
        let defaults = Self::default();
        Self {
            press_any_button: read(KEY_PRESS_ANY_BUTTON, defaults.press_any_button),
            process_windows: read(KEY_PROCESS_WINDOWS, defaults.process_windows),
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_PRESS_ANY_BUTTON}={} {KEY_PROCESS_WINDOWS}={}",
            ds2_dialog_skip::LOG_PREFIX,
            self.press_any_button,
            self.process_windows
        )
    }
}
