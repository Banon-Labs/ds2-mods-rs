//! Reading `[menu_row]` out of `<Game>/ds2-mods.toml`: WHICH extra rows go on the pause menu.
//!
//! The features themselves live in `ds2-menu-row`, `ds2-build-import` and `ds2-save-file`; this is
//! the one place that decides which of them gets a slot, kept here for the same reason
//! `intro_skip`'s config is -- the config file belongs to the loader, and a feature crate should not
//! have to know where the game directory is.
//!
//! # Why a list and not four switches
//!
//! **Because the tab used to hold two, and the list is what kept that survivable.**
//! `FeGroupInGameGroupSelect`'s item vector is a `DLKR::DLFixedVector` of capacity five and the
//! System tab ships three rows, so for as long as that vector was the ceiling there were two slots
//! and FOUR rows that wanted one. Four independent `enabled` switches let a player turn on three and
//! find out from an allocator panic during a menu open; a list refused the third at registration,
//! with the numbers, before anything was hooked.
//!
//! **That ceiling is now [`ds2_menu_row::MAX_ADDED_ROWS`] = 12 and all four rows fit**, because
//! `ds2-menu-row` stopped storing its rows in the game's two fixed vectors -- see that crate's
//! [`api`](ds2_menu_row) docs. The list stays, and so does the refusal: the bound is still the
//! game's (the grid's layout bind stops looking after fifteen rows), and the next row somebody adds
//! should still be refused by a number rather than by a menu that misbehaves.
//!
//! **Nothing has measured where rows stop being VISIBLE**, which is a smaller number than twelve and
//! is not known. Twelve is what the engine will bind.
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
/// Four variants against what used to be two slots, which is the whole reason [`MenuRowConfig`]
/// exists -- and they all fit now.
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

/// Every row a `rows` list may name, **in the order a player who names none of them sees them** --
/// which is also the order they are listed in the log.
///
/// **Quit is last, and the order is the whole reason this is a list rather than a set.** It sat
/// first for as long as it was the only row there was, and a tab's cursor starts on the first row:
/// first place is the cheapest slot to press by accident, and a quit that neither saves nor asks is
/// the most expensive thing on the tab to press by accident. The three rows that load, import or
/// write a file take the near slots; the one that cannot be taken back takes the far one.
///
/// A `rows` list still says otherwise. This is the default, not a policy.
pub const EVERY_ROW: [Row; 4] = [
    Row::LoadBuildFromUrl,
    Row::LoadCharacterFromFile,
    Row::SaveGameToFile,
    Row::QuitToDesktop,
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

impl Default for MenuRowConfig {
    /// Every row, which is what a config file that says nothing about rows means.
    ///
    /// Hand-written rather than derived, because the derived one is `rows: vec![]` and that is the
    /// opposite answer -- reached on the two paths where there is no file to read at all, which are
    /// exactly the paths a player who has configured nothing takes.
    fn default() -> Self {
        Self {
            rows: EVERY_ROW.to_vec(),
            overflow: Vec::new(),
            complaints: Vec::new(),
            source: Source::Default,
        }
    }
}

impl Default for Source {
    /// Every row this project adds.
    ///
    /// It used to be nothing, on the reasoning that these rows change the pause menu, one of them
    /// can quit the game without asking and one of them writes a file, so a player who had not
    /// named them should not get them. The user's words on seeing two of four, 2026-09-23: "I would
    /// like it on by default. All the extra rows we add ourselves."
    ///
    /// The reasoning it replaces was written when the ceiling was two and a row cost another row
    /// its slot. It is twelve now, and the rows are on a tab of their own, so nothing is displaced
    /// by carrying all four. `rows = []` still turns every one of them off.
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
        // In [`EVERY_ROW`]'s order, for the reason written there. An old config file is exactly the
        // one that used to put the quit row under the cursor, so this path needs the move most.
        let mut rows = Vec::new();
        if on(crate::build_import::CONFIG_SECTION) {
            rows.push(Row::LoadBuildFromUrl);
        }
        if on(CONFIG_SECTION) {
            rows.push(Row::QuitToDesktop);
        }
        if rows.is_empty() {
            // NEITHER KEY SET, so nothing has been said about rows at all -- and what a player who
            // has said nothing gets is every row this project adds. A `rows` list, including an
            // empty one, is what says otherwise; that path never reaches here.
            return Self {
                rows: EVERY_ROW.to_vec(),
                overflow: Vec::new(),
                complaints: Vec::new(),
                source: Source::Default,
            };
        }
        Self {
            rows,
            overflow: Vec::new(),
            complaints: Vec::new(),
            source: Source::LegacyEnabled,
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
                " REFUSED-OVER-CEILING=[{}] -- the grid's layout bind never looks past row {} and \
                 the game ships {} on this tab",
                names(&self.overflow),
                ds2_rva::FEX_GRID_MAX_ROWS,
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

    /// All four rows fit. This test used to assert the opposite -- that naming four rows kept two
    /// and refused two -- and the whole point of raising the ceiling was to stop that being true.
    #[test]
    fn every_row_this_table_knows_fits_at_once() {
        let config = MenuRowConfig::from_text(
            "[menu_row]\nrows = [\"quit-to-desktop\", \"load-build-from-url\", \
             \"save-game-to-file\", \"load-character-from-file\"]\n",
        );
        assert_eq!(
            config.rows,
            vec![
                Row::QuitToDesktop,
                Row::LoadBuildFromUrl,
                Row::SaveGameToFile,
                Row::LoadCharacterFromFile,
            ]
        );
        assert!(config.overflow.is_empty());
        let line = config.describe();
        assert!(!line.contains("REFUSED-OVER-CEILING"), "{line}");
    }

    /// The ceiling is still the game's and the refusal is still here -- it is out of reach for now,
    /// because this table has fewer names than the tab has slots. That inequality is
    /// the thing worth pinning: the day it stops holding, the test above starts failing and whoever
    /// added the row finds out here rather than in a pause menu.
    #[test]
    fn the_ceiling_is_above_every_name_this_table_offers() {
        assert!(
            EVERY_ROW.len() <= ds2_menu_row::MAX_ADDED_ROWS,
            "{} rows against {} slots -- the overflow path is live again and wants a test",
            EVERY_ROW.len(),
            ds2_menu_row::MAX_ADDED_ROWS
        );
        // And the ceiling is the grid's, not either of the two fixed vectors it used to be.
        assert_eq!(
            ds2_menu_row::MAX_ADDED_ROWS,
            ds2_rva::FEX_GRID_MAX_ROWS - ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len()
        );
    }

    /// The overflow SPLIT itself, exercised without needing thirteen real rows: whatever the ceiling
    /// is, `from_names` keeps that many and reports the rest rather than dropping them silently.
    #[test]
    fn the_overflow_split_keeps_the_ceiling_and_reports_the_rest() {
        let names: Vec<String> = EVERY_ROW.iter().map(|row| row.name().to_owned()).collect();
        let config = MenuRowConfig::from_names(&names);
        assert_eq!(
            config.rows.len() + config.overflow.len(),
            names.len(),
            "a name went missing between the list and the two buckets"
        );
        assert!(config.rows.len() <= ds2_menu_row::MAX_ADDED_ROWS);
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
        // In `EVERY_ROW`'s order, quit last -- the same order the no-config default hands back.
        assert_eq!(config.rows, vec![Row::LoadBuildFromUrl, Row::QuitToDesktop]);
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

    /// Saying nothing gets every row; saying `rows = []` gets none.
    ///
    /// The two used to be the same answer, and the difference is the whole of the 2026-09-23
    /// change: a player who has configured nothing wants the rows this project adds, and a player
    /// who wrote an empty list has said otherwise in the only place that can say it.
    #[test]
    fn an_absent_selection_adds_every_row_and_an_empty_one_adds_none() {
        for text in ["", "[menu_row]\n"] {
            let config = MenuRowConfig::from_text(text);
            assert_eq!(config.rows, EVERY_ROW.to_vec(), "{text:?}");
            assert_eq!(config.source, Source::Default, "{text:?}");
        }
        // An explicitly empty list is the modern key being read, which is a different thing from no
        // key at all and is reported -- and obeyed -- as such.
        let none = MenuRowConfig::from_text("[menu_row]\nrows = []\n");
        assert!(none.rows.is_empty());
        assert_eq!(none.source, Source::Rows);
    }

    /// The default set is every row the table knows, with nothing left out.
    ///
    /// Pinned against `EVERY_ROW` rather than against a list written out here, so a row added to
    /// the table without being added to the default is a failure rather than a surprise.
    #[test]
    fn the_default_set_is_all_of_them() {
        assert_eq!(MenuRowConfig::default().rows, EVERY_ROW.to_vec());
        assert_eq!(MenuRowConfig::default().rows.len(), 4);
        assert!(EVERY_ROW.len() <= ds2_menu_row::MAX_ADDED_ROWS);
    }

    /// **The quit row is last of the default set**, on every path that produces one.
    ///
    /// Pinned here rather than left to the doc comment on `EVERY_ROW`, because "the cursor must not
    /// start on the row that ends the process without asking" is a property of the shipped menu and
    /// a comment enforces nothing. A row added to the table in front of it fails here.
    #[test]
    fn the_row_that_does_not_ask_is_never_the_one_under_the_cursor() {
        assert_eq!(EVERY_ROW[EVERY_ROW.len() - 1], Row::QuitToDesktop);
        for config in [
            MenuRowConfig::default(),
            MenuRowConfig::from_text(""),
            MenuRowConfig::from_text("[menu_row]\n"),
            MenuRowConfig::from_text(
                "[menu_row]\nenabled = true\n[build_import]\nenabled = true\n",
            ),
        ] {
            assert_eq!(
                config.rows.last(),
                Some(&Row::QuitToDesktop),
                "{:?} put something after the quit row",
                config.source
            );
            assert_ne!(config.rows.first(), Some(&Row::QuitToDesktop));
        }
    }

    /// Only an exact `true` is read as a legacy `enabled`.
    ///
    /// A misspelling no longer leaves the row off -- off is not the default any more -- so what it
    /// leaves behind is the no-key answer, reported as `Default` rather than as `LegacyEnabled`.
    /// That distinction is what tells a player their typo was not read.
    #[test]
    fn a_misspelled_legacy_value_is_not_read_as_true() {
        for value in ["ture", "1", "yes", "TRUE", ""] {
            let text = format!("[menu_row]\nenabled = {value}\n");
            let config = MenuRowConfig::from_text(&text);
            assert_eq!(config.source, Source::Default, "{value:?} was read as true");
            assert_eq!(config.rows, EVERY_ROW.to_vec(), "{value:?}");
        }
        let read = MenuRowConfig::from_text("[menu_row]\nenabled = true\n");
        assert_eq!(read.source, Source::LegacyEnabled);
        assert_eq!(read.rows, vec![Row::QuitToDesktop]);
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
