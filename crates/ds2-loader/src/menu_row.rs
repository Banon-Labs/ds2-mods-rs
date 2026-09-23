//! Reading `[menu_row]` out of `<Game>/ds2-mods.toml`: WHICH extra rows go on the pause menu.
//!
//! The features themselves live in `ds2-menu-row`, `ds2-build-import` and `ds2-save-file`; this is
//! the one place that decides which of them gets a slot, kept here for the same reason
//! `intro_skip`'s config is -- the config file belongs to the loader, and a feature crate should not
//! have to know where the game directory is.
//!
//! # Why a list and not four switches
//!
//! **Because the tab holds two.** `FeGroupInGameGroupSelect`'s item vector is a
//! `DLKR::DLFixedVector` of capacity five, spelled by the builders as
//! `if (5 < newCount) panic("out of memory.")`, and the System tab ships three rows -- so
//! [`ds2_menu_row::MAX_ADDED_ROWS`] is two and there are FOUR rows that want one. Four independent
//! `enabled` switches let a player turn on three and find out from an allocator panic during a menu
//! open. A list refuses the third at registration, with the numbers, before anything is hooked.
//!
//! This is the shape `../er-mods-rs` arrived at as well, for the same reason and after the same
//! mistake: `er-quit-menu-core::row_config` replaced three separate DLLs that each armed a different
//! row set, because *"a file beside the game executable is the one form the player can change and the
//! installer does not have to predict"*.
//!
//! ```toml
//! [menu_row]
//! rows = ["quit-to-desktop", "save-game-to-file"]
//! ```
//!
//! # A name this table does not know arms NOTHING
//!
//! It is recorded in [`MenuRowConfig::complaints`] and logged. That direction is deliberate, and it
//! is ER's too: a typo must lose a row the player asked for, never gain one they did not, because an
//! unasked row is a press that reaches a flow nobody expected to be reachable.
//!
//! # The old keys still work
//!
//! A file with no `rows` key falls back to what it used to mean -- `[menu_row] enabled` for the
//! quit-to-desktop row and `[build_import] enabled` for Load from URL -- so an existing config keeps
//! the menu it had. [`MenuRowConfig::describe`] says which of the two paths a run took, because
//! "my rows key did nothing" and "my enabled key did nothing" are different problems.

use ds2_hotkey_config::kv::KeyValues;

use crate::crash_logging::config_file_path;

/// The section this module reads. Mirrored in `scripts/ds2-run.py`.
pub const CONFIG_SECTION: &str = "menu_row";

/// Which rows to add, as a list of names. The modern key.
pub const KEY_ROWS: &str = "rows";

/// Whether to add the quit-to-desktop row. The legacy key, honoured only when [`KEY_ROWS`] is absent.
pub const KEY_ENABLED: &str = "enabled";

/// A row that can be put on the pause menu's System tab.
///
/// Four variants against two slots, which is the whole reason [`MenuRowConfig`] exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Quit straight to the desktop, without a confirmation and without saving.
    QuitToDesktop,
    /// Take a soulsplanner link and put that build on the live character.
    LoadBuildFromUrl,
    /// Pick a save container for the NEXT launch to load. See `ds2_save_file::import`.
    LoadCharacterFromFile,
    /// Ask the game to save, then copy the container to a file the player names.
    SaveGameToFile,
}

impl Row {
    /// The name a config file spells this row with.
    pub const fn name(self) -> &'static str {
        match self {
            Self::QuitToDesktop => "quit-to-desktop",
            Self::LoadBuildFromUrl => "load-build-from-url",
            Self::LoadCharacterFromFile => "load-character-from-file",
            Self::SaveGameToFile => "save-game-to-file",
        }
    }
}

/// Every row a `rows` list may name, in the order they are listed in the log.
pub const EVERY_ROW: [Row; 4] = [
    Row::QuitToDesktop,
    Row::LoadBuildFromUrl,
    Row::LoadCharacterFromFile,
    Row::SaveGameToFile,
];

/// Where a run's selection came from, so the log can say which key was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// No config file, or neither key present. Nothing is added.
    Default,
    /// A `rows` list. The legacy keys were not consulted.
    Rows,
    /// No `rows` key, so `[menu_row] enabled` and `[build_import] enabled` decided.
    LegacyEnabled,
}

/// `[menu_row]`, resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuRowConfig {
    /// The rows to register, in order, already truncated to what the tab can hold.
    pub rows: Vec<Row>,
    /// Rows named past the tab's ceiling. Logged and NOT registered -- see the module docs.
    pub overflow: Vec<Row>,
    /// Names the table did not recognise, in the order they appeared. Each one arms nothing.
    pub complaints: Vec<String>,
    /// Which key was read.
    pub source: Source,
}

impl Default for Source {
    /// **Nothing**, and unlike the other defaults in this loader that is not a judgement about which
    /// behaviour is better. These rows change the pause menu, one of them can quit the game without
    /// asking and one of them writes a file; a player who has not named them should not get them.
    fn default() -> Self {
        Self::Default
    }
}

/// Split a TOML-ish list value into its items.
///
/// The config parser is deliberately a `key = value` reader with no array support -- see
/// `ds2_hotkey_config::kv`, which explains why a TOML crate is not here -- so the brackets arrive as
/// part of the value and are stripped here. The accepted forms are the same text TOML would accept,
/// so the file stays a valid TOML file:
///
/// ```text
/// rows = ["a", "b"]
/// rows = [a, b]
/// rows = a
/// ```
///
/// A trailing comma is tolerated rather than reported: it produces an empty item, and an empty item
/// is not a name somebody meant.
fn split_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

impl MenuRowConfig {
    /// Read the section. A missing file, a missing key or an empty list all mean nothing is added.
    pub fn load() -> Self {
        let Some(path) = config_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::from_text(&text)
    }

    /// The same decision against a config file's text, so it can be tested without one on disk.
    pub fn from_text(text: &str) -> Self {
        let parsed = KeyValues::parse(text);
        match parsed.get(CONFIG_SECTION, KEY_ROWS) {
            Some(raw) => Self::from_names(&split_list(raw)),
            None => Self::from_legacy_keys(&parsed),
        }
    }

    /// The modern path: a list of names, resolved against the table.
    fn from_names(names: &[String]) -> Self {
        let mut wanted = Vec::new();
        let mut complaints = Vec::new();
        for name in names {
            match EVERY_ROW.iter().find(|row| row.name() == name) {
                // A row named twice is registered once. Two entries would spend both of the tab's
                // slots on one row, which is a way to lose a row to a duplicated line.
                Some(row) if wanted.contains(row) => {}
                Some(row) => wanted.push(*row),
                None => complaints.push(name.clone()),
            }
        }
        let ceiling = ds2_menu_row::MAX_ADDED_ROWS.min(wanted.len());
        let overflow = wanted.split_off(ceiling);
        Self {
            rows: wanted,
            overflow,
            complaints,
            source: Source::Rows,
        }
    }

    /// The compatibility path: the two `enabled` keys this used to be spelled with.
    fn from_legacy_keys(parsed: &KeyValues) -> Self {
        let on = |section: &str| {
            // Only an exact `true` turns a row on, so a typo leaves it OFF -- the harmless direction
            // when off is the default.
            matches!(
                parsed
                    .get(section, KEY_ENABLED)
                    .map(|raw| raw.trim().trim_matches('"')),
                Some("true")
            )
        };
        let mut rows = Vec::new();
        if on(CONFIG_SECTION) {
            rows.push(Row::QuitToDesktop);
        }
        if on(crate::build_import::CONFIG_SECTION) {
            rows.push(Row::LoadBuildFromUrl);
        }
        let source = if rows.is_empty() {
            Source::Default
        } else {
            Source::LegacyEnabled
        };
        Self {
            rows,
            overflow: Vec::new(),
            complaints: Vec::new(),
            source,
        }
    }

    /// One line for the attach log, written before anything acts on it.
    pub fn describe(&self) -> String {
        let names = |rows: &[Row]| {
            rows.iter()
                .map(|row| row.name())
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut line = format!(
            "{} config [{CONFIG_SECTION}] source={:?} rows=[{}] of {} slots",
            ds2_menu_row::LOG_PREFIX,
            self.source,
            names(&self.rows),
            ds2_menu_row::MAX_ADDED_ROWS
        );
        if !self.overflow.is_empty() {
            line.push_str(&format!(
                " REFUSED-OVER-CEILING=[{}] -- the System tab's item vector holds {} and the game \
                 ships {}",
                names(&self.overflow),
                ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY,
                ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len()
            ));
        }
        if !self.complaints.is_empty() {
            line.push_str(&format!(
                " UNKNOWN=[{}] -- armed nothing; known names are [{}]",
                self.complaints.join(","),
                names(&EVERY_ROW)
            ));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The modern key names rows in order, and that order is the order they appear on screen.
    #[test]
    fn a_rows_list_is_read_in_order() {
        let config = MenuRowConfig::from_text(
            "[menu_row]\nrows = [\"save-game-to-file\", \"quit-to-desktop\"]\n",
        );
        assert_eq!(config.rows, vec![Row::SaveGameToFile, Row::QuitToDesktop]);
        assert_eq!(config.source, Source::Rows);
        assert!(config.complaints.is_empty());
        assert!(config.overflow.is_empty());
    }

    /// Unquoted and single-item forms are the same text TOML accepts, and mean the same thing.
    #[test]
    fn the_accepted_spellings_agree() {
        for text in [
            "[menu_row]\nrows = [\"quit-to-desktop\"]\n",
            "[menu_row]\nrows = [quit-to-desktop]\n",
            "[menu_row]\nrows = quit-to-desktop\n",
            "[menu_row]\nrows = [\"quit-to-desktop\",]\n",
        ] {
            let config = MenuRowConfig::from_text(text);
            assert_eq!(config.rows, vec![Row::QuitToDesktop], "{text:?}");
        }
    }

    /// THE CEILING IS THE GAME'S. A third row is refused here, not by the game's allocator.
    #[test]
    fn a_third_row_is_refused_and_named() {
        let config = MenuRowConfig::from_text(
            "[menu_row]\nrows = [\"quit-to-desktop\", \"load-build-from-url\", \
             \"save-game-to-file\", \"load-character-from-file\"]\n",
        );
        assert_eq!(config.rows.len(), ds2_menu_row::MAX_ADDED_ROWS);
        assert_eq!(config.rows, vec![Row::QuitToDesktop, Row::LoadBuildFromUrl]);
        assert_eq!(
            config.overflow,
            vec![Row::SaveGameToFile, Row::LoadCharacterFromFile]
        );
        // And the log line says so, with the two numbers that explain why.
        let line = config.describe();
        assert!(line.contains("REFUSED-OVER-CEILING"), "{line}");
        assert!(line.contains("save-game-to-file"), "{line}");
    }

    /// A typo loses a row; it never gains one.
    #[test]
    fn an_unknown_name_arms_nothing_and_is_reported() {
        let config = MenuRowConfig::from_text(
            "[menu_row]\nrows = [\"quit-to-deskotp\", \"save-game-to-file\"]\n",
        );
        assert_eq!(config.rows, vec![Row::SaveGameToFile]);
        assert_eq!(config.complaints, vec!["quit-to-deskotp".to_owned()]);
        let line = config.describe();
        assert!(line.contains("UNKNOWN=[quit-to-deskotp]"), "{line}");
        // The refusal names every spelling that would have worked.
        for row in EVERY_ROW {
            assert!(line.contains(row.name()), "{line} omits {}", row.name());
        }
    }

    /// A row named twice spends one slot, not two.
    #[test]
    fn a_duplicate_name_is_registered_once() {
        let config = MenuRowConfig::from_text(
            "[menu_row]\nrows = [\"quit-to-desktop\", \"quit-to-desktop\"]\n",
        );
        assert_eq!(config.rows, vec![Row::QuitToDesktop]);
        assert!(config.overflow.is_empty());
    }

    /// An existing config with the old keys keeps the menu it had.
    #[test]
    fn the_legacy_keys_still_arm_their_rows() {
        let config = MenuRowConfig::from_text(
            "[menu_row]\nenabled = true\n[build_import]\nenabled = true\n",
        );
        assert_eq!(config.source, Source::LegacyEnabled);
        assert_eq!(config.rows, vec![Row::QuitToDesktop, Row::LoadBuildFromUrl]);
    }

    /// `rows` wins outright when present, so there is one source of truth per run.
    #[test]
    fn rows_overrides_the_legacy_keys() {
        let config = MenuRowConfig::from_text(
            "[menu_row]\nenabled = true\nrows = [\"save-game-to-file\"]\n[build_import]\nenabled = \
             true\n",
        );
        assert_eq!(config.source, Source::Rows);
        assert_eq!(config.rows, vec![Row::SaveGameToFile]);
    }

    /// Nothing named means nothing added, and the line says `Default` rather than looking broken.
    #[test]
    fn an_empty_or_absent_selection_adds_nothing() {
        for text in ["", "[menu_row]\n", "[menu_row]\nrows = []\n"] {
            let config = MenuRowConfig::from_text(text);
            assert!(config.rows.is_empty(), "{text:?}");
        }
        assert_eq!(MenuRowConfig::from_text("").source, Source::Default);
        // An explicitly empty list is still the modern key being read, which is a different thing
        // from no key at all and is reported as such.
        assert_eq!(
            MenuRowConfig::from_text("[menu_row]\nrows = []\n").source,
            Source::Rows
        );
    }

    /// Only an exact `true` arms a legacy row.
    #[test]
    fn a_misspelled_legacy_value_leaves_the_row_off() {
        for value in ["ture", "1", "yes", "TRUE", ""] {
            let text = format!("[menu_row]\nenabled = {value}\n");
            assert!(
                MenuRowConfig::from_text(&text).rows.is_empty(),
                "{value:?} armed a row"
            );
        }
    }

    /// Every name is distinct, or two rows would answer to one spelling.
    #[test]
    fn the_names_are_unique() {
        for (index, row) in EVERY_ROW.iter().enumerate() {
            for other in &EVERY_ROW[index + 1..] {
                assert_ne!(row.name(), other.name());
            }
        }
    }
}
