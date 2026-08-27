//! Reading `[title_menu]` out of `<Game>/ds2-mods.toml`.
//!
//! Its own section rather than a key under `[title_skip]`, because it is not a skip. Everything in
//! `[title_skip]` removes a wait; this changes what the title menu draws once the waits are gone,
//! on different functions, and a run that fails to boot has to be attributable to one of them by
//! editing one line.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "title_menu";

/// Draw the menu rows the game would hide, instead of hiding them.
pub const KEY_SHOW_UNAVAILABLE: &str = "show_unavailable";

/// `[title_menu]`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleMenuConfig {
    /// Style unavailable rows as drawn, and put their cell state back so they stay unselectable.
    pub show_unavailable: bool,
}

impl Default for TitleMenuConfig {
    /// **Off, and this default was flipped by a control run rather than by preference.**
    ///
    /// The feature was written on the premise that the game removes an unavailable row from the
    /// screen, so the menu's shape changes with the network state and with whether a save exists.
    /// A run with this key set to `false` -- the game's own title menu, with none of our writes in
    /// it -- shows that premise is wrong on this build: every row is present, an unavailable
    /// Continue is DIMMED rather than absent, and INFORMATION and GO ONLINE swap inside one shared
    /// screen slot with no gap left behind. The game already does what this was built to do.
    ///
    /// With it on, the mod forces the enable byte, lets the game's pass draw the row bright with
    /// `0x67`, and then stamps [`ds2_rva::FE_TOP_MENU_SEQUENCE_FADED`] over it -- which is the
    /// segment leading into the row's removal, so the row poses itself invisible. Every visual
    /// defect this feature was blamed for (a blank INFORMATION row, a gap between rows, both halves
    /// of the exclusive pair on screen at once) was produced by the feature itself.
    ///
    /// Kept as a switch rather than deleted because the hooks and the measurements around them are
    /// worth having, and because "off" is a claim about THIS build's layout resource that a future
    /// build could falsify. Turning it on is opting into overriding a look the game gets right.
    fn default() -> Self {
        Self {
            show_unavailable: false,
        }
    }
}

impl TitleMenuConfig {
    /// Read the section. A missing file or a missing key means [`Default`].
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let parsed = KeyValues::parse(&text);
        let show_unavailable = match parsed.get(CONFIG_SECTION, KEY_SHOW_UNAVAILABLE) {
            None => Self::default().show_unavailable,
            // Only an exact `false` turns it off, so a typo leaves the feature ON -- the harmless
            // direction when on is the default.
            Some(raw) => !matches!(raw.trim().trim_matches('"'), "false"),
        };
        Self { show_unavailable }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        format!(
            "{} config [{CONFIG_SECTION}] {KEY_SHOW_UNAVAILABLE}={}",
            ds2_dialog_skip::LOG_PREFIX,
            self.show_unavailable
        )
    }
}
