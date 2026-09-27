//! Reading `[save_picker]` out of `<Game>/ds2-mods.toml`: how the two save-file rows choose a file.
//!
//! The rows live in `ds2-save-file` and are switched on by `[menu_row] rows`; this section only
//! decides what their press opens. The same three keys `../er-mods-rs`'s picker reads, renamed to
//! this file's spelling:
//!
//! ```toml
//! [save_picker]
//! os_native = false        # true: the Windows file dialog instead of the in-game panel
//! start_dir = ""           # where the panel opens; empty means ~/Downloads
//! remember_dir = true      # open next time where the last pick was made
//! ```

use ds2_hotkey_config::kv::KeyValues;
use ds2_save_file::PickerSettings;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "save_picker";

/// `true` opens the OS file dialog instead of the in-game panel.
pub const KEY_OS_NATIVE: &str = "os_native";

/// The folder the panel opens in. A Linux path is accepted and spelled through Wine's `Z:`.
pub const KEY_START_DIR: &str = "start_dir";

/// Whether a pick moves where the panel opens next time.
pub const KEY_REMEMBER_DIR: &str = "remember_dir";

/// Read the section. A missing file or key means [`PickerSettings::default`]: the in-game panel,
/// opening in `~/Downloads`, remembering the last folder.
pub fn load() -> PickerSettings {
    config_file_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map_or_else(PickerSettings::default, |text| parse(&text))
}

/// Resolve the section out of a config file's text.
pub fn parse(text: &str) -> PickerSettings {
    let parsed = KeyValues::parse(text);
    let value = |key: &str| {
        parsed
            .get(CONFIG_SECTION, key)
            .map(|raw| raw.trim().trim_matches('"').to_owned())
    };
    let defaults = PickerSettings::default();
    PickerSettings {
        // Only an exact `true` leaves the game's own screen for a Windows dialog.
        os_native: value(KEY_OS_NATIVE).is_some_and(|raw| raw == "true"),
        start_dir: value(KEY_START_DIR).filter(|raw| !raw.is_empty()),
        // Only an exact `false` stops the folder following the player.
        remember_dir: value(KEY_REMEMBER_DIR).map_or(defaults.remember_dir, |raw| raw != "false"),
    }
}

/// One line for the attach log, written before anything acts on it.
pub fn describe(settings: &PickerSettings) -> String {
    format!(
        "{} config [{CONFIG_SECTION}] {KEY_OS_NATIVE}={} {KEY_START_DIR}={:?} {KEY_REMEMBER_DIR}={}",
        ds2_save_file::LOG_PREFIX,
        settings.os_native,
        settings.start_dir,
        settings.remember_dir
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_in_game_panel_remembering_the_folder() {
        let settings = parse("");
        assert!(!settings.os_native);
        assert_eq!(settings.start_dir, None);
        assert!(settings.remember_dir);
    }

    #[test]
    fn every_key_is_read() {
        let settings = parse(
            "[save_picker]\nos_native = true\nstart_dir = \"/home/you/saves\"\nremember_dir = false\n",
        );
        assert!(settings.os_native);
        assert_eq!(settings.start_dir.as_deref(), Some("/home/you/saves"));
        assert!(!settings.remember_dir);
    }

    #[test]
    fn a_typo_keeps_the_default() {
        let settings =
            parse("[save_picker]\nos_native = yes\nremember_dir = nope\nstart_dir = \"\"\n");
        assert!(!settings.os_native);
        assert!(settings.remember_dir);
        assert_eq!(settings.start_dir, None);
    }
}
