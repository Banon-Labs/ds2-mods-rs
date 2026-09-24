//! What a row SAYS, kept apart from what a row MEANS.
//!
//! # Why this is not a method on the model
//!
//! A surface has opinions the model has no business holding: how wide a row is, whether its font
//! has lowercase, whether it draws one line or two. `er-save-picker-core` ended up with
//! `row_label_utf16` for a native menu whose name field holds sixteen UTF-16 units and
//! `row_label_ascii` for an overlay that rasterises its own glyphs -- two renderings of the same
//! rows, both living on the model, neither one right for the other surface.
//!
//! So the rendering is free functions over the model instead. A DS2 surface that wants
//! uppercase, or a column layout, or a truncation to its own budget, writes its own on top of
//! [`character_text`] and the slot fields, and does not have to argue with the model about it.
//!
//! # What a character row says
//!
//! A name, a soul level, and a marker when the slot is not an ordinary played character.
//!
//! The level is `ds2_sl2_core::SaveSlot::soul_level`, which is the game's own arithmetic read
//! out of the disassembly -- the nine stats less a fixed starting total, floored at one -- and
//! therefore the same number the game's character list prints. It is deliberately not the raw
//! stat total, which was what this row showed while that crate still believed a level needed the
//! starting class to compute. A player picking between two characters recognises "SL 136"; they
//! do not recognise its stat sum. The sum is still on the slot for any surface that wants it.

use ds2_sl2_core::{SaveSlot, SlotState};

use crate::model::{PickerRow, SavePickerModel};
use crate::path::{leaf, parent};

/// Shown in place of a character on a slot nothing has ever been written to.
pub const EMPTY_SLOT_TEXT: &str = "[ empty ]";

/// Marks a slot the game will load that has nothing rolled in it yet -- nine stats of `1`.
pub const BLANK_SLOT_MARKER: &str = "[ new ]";

/// Shown where a loadable character's name would be when the record carries none.
pub const UNNAMED_CHARACTER_TEXT: &str = "[ unnamed ]";

/// What the `[..]` row says when its destination cannot be named.
pub const PARENT_ROW_TEXT: &str = "[..]";

/// What the character stage's last row says.
pub const BACK_ROW_TEXT: &str = "[ back ]";

/// What a listing with nothing above it and nothing in it says.
pub const ROOT_ROW_TEXT: &str = "[ root ]";

/// How a soul level is introduced on a row, the way the game's own menus abbreviate it.
pub const SOUL_LEVEL_PREFIX: &str = "SL";

/// One character slot as a line of text: the name, the soul level, and a marker where one is due.
///
/// An empty slot is ONLY the marker. It carries no name and no stats, and its level would floor
/// to one -- so a row that printed it would show an empty slot as a fresh character.
pub fn character_text(slot: &SaveSlot) -> String {
    match slot.state {
        SlotState::Empty => EMPTY_SLOT_TEXT.to_owned(),
        SlotState::Blank => format!(
            "{}  {BLANK_SLOT_MARKER}  {SOUL_LEVEL_PREFIX} {}",
            character_name(slot),
            slot.soul_level()
        ),
        SlotState::Occupied => format!(
            "{}  {SOUL_LEVEL_PREFIX} {}",
            character_name(slot),
            slot.soul_level()
        ),
    }
}

/// The name to put on a loadable slot's row. Never empty -- a blank row cannot be pressed with
/// any confidence.
pub fn character_name(slot: &SaveSlot) -> &str {
    if slot.name.trim().is_empty() {
        UNNAMED_CHARACTER_TEXT
    } else {
        &slot.name
    }
}

/// What a row says, in whatever stage the picker is in.
///
/// Empty string for a row past the end of the listing, which is a row nothing draws.
///
/// Folders carry a trailing `/` so a folder called `DS2SOFS0000.sl2` cannot be mistaken for a
/// save -- which is not a hypothetical: the thing a downloaded save is inside is very often a
/// folder named after the save.
pub fn row_text(model: &SavePickerModel, row: usize) -> String {
    match model.row_meaning(row) {
        // Name the destination, not just the direction: a row that says where it goes does not
        // have to be pressed to find out.
        PickerRow::ParentDir => {
            let up = parent(model.current_dir());
            match up.as_deref().and_then(leaf) {
                Some(name) => format!("{PARENT_ROW_TEXT} {name}"),
                // The parent is a drive root, which has no name of its own.
                None => PARENT_ROW_TEXT.to_owned(),
            }
        }
        PickerRow::AtRoot => ROOT_ROW_TEXT.to_owned(),
        PickerRow::Dir(path) => match leaf(&path) {
            Some(name) => format!("{name}/"),
            // A drive root has no leaf; show the root itself so the row still names where it goes.
            None => path.display().to_string(),
        },
        PickerRow::File(path) => match leaf(&path) {
            Some(name) => name.to_owned(),
            // Unreachable from a listing, which only offers paths with a leaf. Showing the whole
            // path is the harmless answer; an empty row is not.
            None => path.display().to_string(),
        },
        PickerRow::Character(slot) => match model.character_slot(slot) {
            Some(found) => character_text(found),
            None => EMPTY_SLOT_TEXT.to_owned(),
        },
        PickerRow::Back => BACK_ROW_TEXT.to_owned(),
        PickerRow::Empty => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CHARACTER_BACK_ROW, SavePickerModel};
    use std::path::Path;

    fn slot(name: &str, state: SlotState, stats: [i16; 9]) -> SaveSlot {
        SaveSlot {
            slot: 0,
            state,
            name: name.to_owned(),
            stats,
        }
    }

    /// The row shows the number the game's own character list shows, not the raw stat sum.
    #[test]
    fn a_played_character_shows_its_name_and_the_level_the_game_prints() {
        let vendrick = slot(
            "Vendrick",
            SlotState::Occupied,
            [30, 20, 15, 10, 25, 18, 12, 9, 7],
        );
        let text = character_text(&vendrick);
        assert!(text.contains("Vendrick"), "{text}");
        assert!(
            text.contains(&format!("{SOUL_LEVEL_PREFIX} {}", vendrick.soul_level())),
            "{text} omits the soul level"
        );
        assert_ne!(
            vendrick.soul_level(),
            vendrick.stat_total(),
            "this fixture would not tell the two numbers apart"
        );
        assert!(
            !text.contains(&vendrick.stat_total().to_string()),
            "{text} shows the stat sum, which is not what a player recognises"
        );
    }

    /// The empty marker and nothing else. A zero total on an empty row reads as a ruined save.
    #[test]
    fn an_empty_slot_says_only_that_it_is_empty() {
        assert_eq!(
            character_text(&slot("", SlotState::Empty, [0; 9])),
            EMPTY_SLOT_TEXT
        );
    }

    /// A blank slot is loadable, so it gets a real row -- marked, so nobody mistakes it for a
    /// character they have played. Its level floors at one, which is what a fresh character is.
    #[test]
    fn a_blank_slot_is_marked_and_reads_as_level_one() {
        let fresh = slot("Fresh", SlotState::Blank, [1; 9]);
        let text = character_text(&fresh);
        assert!(text.contains("Fresh"), "{text}");
        assert!(text.contains(BLANK_SLOT_MARKER), "{text}");
        assert_eq!(fresh.soul_level(), 1);
        assert!(text.contains(&format!("{SOUL_LEVEL_PREFIX} 1")), "{text}");
    }

    /// A loadable slot with no name still gets something to read.
    #[test]
    fn an_unnamed_character_still_has_a_row_worth_reading() {
        let text = character_text(&slot("   ", SlotState::Occupied, [5; 9]));
        assert!(text.starts_with(UNNAMED_CHARACTER_TEXT), "{text}");
        assert!(!text.trim().is_empty());
    }

    /// A save row shows the file's LEAF, through the same hand-parse a Windows path goes through
    /// (see `crate::path` for what `Path::file_name` does to one of those on this host).
    #[test]
    fn a_save_row_shows_its_leaf() {
        let dir = crate::picker_scratch_dir("text-leaf");
        std::fs::write(dir.join("DS2SOFS0000.sl2"), b"contents are not read here")
            .expect("scratch file must be writable");
        let model = SavePickerModel::open(&dir);
        let row = model.cursor();
        assert_eq!(row_text(&model, row), "DS2SOFS0000.sl2");
    }

    /// A folder keeps its slash, because a folder named like a save is common and confusing.
    #[test]
    fn a_folder_row_wears_a_slash() {
        let dir = crate::picker_scratch_dir("text-folder");
        std::fs::create_dir_all(dir.join("DS2SOFS0000.sl2"))
            .expect("scratch dir must be creatable");
        let model = SavePickerModel::open(&dir);
        let row = (0..model.visible_row_count())
            .find(|&row| matches!(model.row_meaning(row), PickerRow::Dir(_)))
            .expect("the folder is listed");
        assert_eq!(row_text(&model, row), "DS2SOFS0000.sl2/");
    }

    /// The way up names where it goes.
    #[test]
    fn the_way_up_names_the_folder_it_goes_to() {
        let dir = crate::picker_scratch_dir("text-up");
        let child = dir.join("donors");
        std::fs::create_dir_all(&child).expect("scratch dir must be creatable");
        let model = SavePickerModel::open(&child);
        let text = row_text(&model, 0);
        assert!(text.starts_with(PARENT_ROW_TEXT), "{text}");
        let up = leaf(&dir).expect("the scratch folder has a name");
        assert!(text.ends_with(up), "{text} does not name {up}");
    }

    /// Every row a surface would draw has something to put in it, in both stages.
    #[test]
    fn no_drawn_row_is_ever_blank() {
        let dir = crate::picker_scratch_dir("text-every-row");
        std::fs::create_dir_all(dir.join("donors")).expect("scratch dir must be creatable");
        std::fs::write(dir.join("DS2SOFS0000.sl2"), b"contents are not read here")
            .expect("scratch file must be writable");
        let mut model = SavePickerModel::open(&dir);
        for row in 0..model.visible_row_count() {
            assert!(
                !row_text(&model, row).is_empty(),
                "browse row {row} is blank"
            );
        }

        let slots: Vec<SaveSlot> = (0..10)
            .map(|index| SaveSlot {
                slot: index,
                state: if index == 3 {
                    SlotState::Occupied
                } else {
                    SlotState::Empty
                },
                name: if index == 3 {
                    "Vendrick".into()
                } else {
                    String::new()
                },
                stats: if index == 3 { [20; 9] } else { [0; 9] },
            })
            .collect();
        model
            .show_characters(Path::new(r"Z:\home\banon\saves\DS2SOFS0000.sl2"), slots)
            .expect("one occupied slot is showable");
        for row in 0..model.visible_row_count() {
            assert!(
                !row_text(&model, row).is_empty(),
                "character row {row} is blank"
            );
        }
        assert_eq!(row_text(&model, CHARACTER_BACK_ROW), BACK_ROW_TEXT);
        assert_eq!(row_text(&model, 0), EMPTY_SLOT_TEXT);
        assert!(row_text(&model, 3).contains("Vendrick"));
    }

    /// A row past the listing draws nothing, and says nothing.
    #[test]
    fn a_row_past_the_listing_has_no_text() {
        let dir = crate::picker_scratch_dir("text-past-end");
        let model = SavePickerModel::open(&dir);
        assert_eq!(row_text(&model, model.row_capacity() + 1), "");
    }
}
