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

/// Do not draw the "please wait" windows at all.
pub const KEY_HIDE_PROCESS_WINDOWS: &str = "hide_process_windows";

/// Cut the title screen's activation animation short.
pub const KEY_TITLE_ANIMATION: &str = "title_animation";

/// Do not wait for the title logo / prompt animation before accepting the forced press.
pub const KEY_TITLE_SEQUENCE_GATE: &str = "title_sequence_gate";

/// `[title_skip]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleSkipConfig {
    /// Detour the press-any-button poll so it always reports a press.
    pub press_any_button: bool,
    /// Zero each process window's minimum display duration as it opens.
    pub process_windows: bool,
    /// Reproduce the process window's `enter` without its show call, so the window never appears.
    /// Implies [`Self::process_windows`], which is the hook it rides on.
    pub hide_process_windows: bool,
    /// Write the title screen's terminal phase once its setup has run, skipping the flourish.
    pub title_animation: bool,
    /// Force the gate that waits for the title logo and prompt to finish animating in.
    ///
    /// Without this, [`Self::press_any_button`] skips nothing visible: the press poll is not even
    /// reached until the animation has played out on its own.
    pub title_sequence_gate: bool,
}

impl Default for TitleSkipConfig {
    /// Both **on**, matching `intro_skip` and `dialog_skip`. Getting to the menu without touching
    /// anything is the point; leaving one of the four stops in place would mean the default is
    /// still a run that has to be babysat.
    fn default() -> Self {
        Self {
            press_any_button: true,
            process_windows: true,
            hide_process_windows: true,
            title_animation: true,
            title_sequence_gate: true,
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
        let hide = read(KEY_HIDE_PROCESS_WINDOWS, defaults.hide_process_windows);
        Self {
            press_any_button: read(KEY_PRESS_ANY_BUTTON, defaults.press_any_button),
            // Hiding rides on the same detour that shortens, so asking for the stronger behaviour
            // without the hook it needs would silently do nothing. Turning `process_windows` off is
            // therefore the way to leave the wait windows completely alone.
            process_windows: read(KEY_PROCESS_WINDOWS, defaults.process_windows) || hide,
            hide_process_windows: hide,
            title_animation: read(KEY_TITLE_ANIMATION, defaults.title_animation),
            title_sequence_gate: read(KEY_TITLE_SEQUENCE_GATE, defaults.title_sequence_gate),
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_PRESS_ANY_BUTTON}={} {KEY_PROCESS_WINDOWS}={} \
             {KEY_HIDE_PROCESS_WINDOWS}={} {KEY_TITLE_ANIMATION}={} \
             {KEY_TITLE_SEQUENCE_GATE}={}",
            ds2_dialog_skip::LOG_PREFIX,
            self.press_any_button,
            self.process_windows,
            self.hide_process_windows,
            self.title_animation,
            self.title_sequence_gate
        )
    }
}
