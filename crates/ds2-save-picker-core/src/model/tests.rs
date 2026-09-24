use std::time::Duration;

use ds2_sl2_core::SlotState;

use super::*;

/// Everything a test path in here is rooted at: a WINDOWS path, on a Linux host, deliberately.
const DIR: &str = r"Z:\home\banon\saves";

/// A model over a listing a test chose, with no filesystem underneath it.
fn files_model(dir: &str, entries: Vec<PickerEntry>) -> SavePickerModel {
    let mut model = SavePickerModel {
        current_dir: PathBuf::from(dir),
        entries,
        scroll_offset: 0,
        cursor: 0,
        row_capacity: DEFAULT_ROW_CAPACITY,
        status_message: None,
        stage: Stage::Files,
    };
    model.cursor = model.first_selectable_row();
    model
}

fn save_entry(dir: &str, index: usize) -> PickerEntry {
    PickerEntry::File {
        name: format!("save{index}.sl2"),
        path: PathBuf::from(format!(r"{dir}\save{index}.sl2")),
        // Later index, later write: the newest-first order should put the LAST one on top.
        modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(index as u64 * 60)),
    }
}

fn dir_entry(dir: &str, name: &str) -> PickerEntry {
    PickerEntry::Dir {
        name: name.to_owned(),
        path: PathBuf::from(format!(r"{dir}\{name}")),
    }
}

/// `count` saves in `dir`, already in the order [`order_entries`] would put them.
fn listing(dir: &str, count: usize) -> Vec<PickerEntry> {
    order_entries(Vec::new(), (0..count).map(|i| save_entry(dir, i)).collect())
}

fn slot(index: usize, state: SlotState) -> SaveSlot {
    let (name, stats) = match state {
        SlotState::Empty => (String::new(), [0i16; 9]),
        SlotState::Blank => (format!("blank{index}"), [1i16; 9]),
        SlotState::Occupied => (format!("char{index}"), [10i16 + index as i16; 9]),
    };
    SaveSlot {
        slot: index,
        state,
        name,
        stats,
    }
}

/// Ten slots, occupied at exactly the given indexes and empty everywhere else.
fn ten_slots(occupied: &[usize]) -> Vec<SaveSlot> {
    (0..SLOT_COUNT)
        .map(|index| {
            slot(
                index,
                if occupied.contains(&index) {
                    SlotState::Occupied
                } else {
                    SlotState::Empty
                },
            )
        })
        .collect()
}

const CONTAINER: &str = r"Z:\home\banon\saves\DS2SOFS0000.sl2";

/// A model already in the character stage over a container with these slots occupied.
fn character_model(occupied: &[usize]) -> SavePickerModel {
    let mut model = files_model(DIR, listing(DIR, 1));
    model
        .show_characters(Path::new(CONTAINER), ten_slots(occupied))
        .expect("a container with an occupied slot is showable");
    model
}

fn rows(model: &SavePickerModel) -> Vec<PickerRow> {
    (0..model.visible_row_count())
        .map(|row| model.row_meaning(row))
        .collect()
}

// ---------------------------------------------------------------------------------------
// The listing.
// ---------------------------------------------------------------------------------------

/// Folders above saves, folders by name, saves newest-first.
#[test]
fn folders_come_first_by_name_and_saves_follow_newest_first() {
    let ordered = order_entries(
        vec![
            dir_entry(DIR, "zeta"),
            dir_entry(DIR, "Alpha"),
            dir_entry(DIR, "beta"),
        ],
        vec![save_entry(DIR, 0), save_entry(DIR, 2), save_entry(DIR, 1)],
    );
    assert_eq!(
        ordered.iter().map(PickerEntry::name).collect::<Vec<_>>(),
        vec![
            "Alpha",
            "beta",
            "zeta",
            "save2.sl2",
            "save1.sl2",
            "save0.sl2"
        ],
        "folders sort case-insensitively by name; saves sort by when they were written"
    );
}

/// A file whose timestamp could not be read is still offered -- it sorts last, it does not vanish.
#[test]
fn a_save_with_no_timestamp_sinks_instead_of_disappearing() {
    let undated = PickerEntry::File {
        name: "mystery.sl2".to_owned(),
        path: PathBuf::from(format!(r"{DIR}\mystery.sl2")),
        modified: None,
    };
    let ordered = order_entries(Vec::new(), vec![undated, save_entry(DIR, 0)]);
    assert_eq!(
        ordered.iter().map(PickerEntry::name).collect::<Vec<_>>(),
        vec!["save0.sl2", "mystery.sl2"]
    );
}

/// Two saves written in the same second still have a total order, so a refresh of an unchanged
/// folder cannot shuffle them.
#[test]
fn saves_written_at_the_same_moment_fall_back_to_their_names() {
    let same = |leaf: &str| PickerEntry::File {
        name: leaf.to_owned(),
        path: PathBuf::from(format!(r"{DIR}\{leaf}")),
        modified: Some(SystemTime::UNIX_EPOCH),
    };
    let ordered = order_entries(Vec::new(), vec![same("b.sl2"), same("a.sl2")]);
    assert_eq!(
        ordered.iter().map(PickerEntry::name).collect::<Vec<_>>(),
        vec!["a.sl2", "b.sl2"]
    );
}

/// The real thing, against a real directory: the cheap gate and nothing else decides what shows.
#[test]
fn a_real_directory_is_filtered_by_extension_and_nothing_more() {
    let dir = crate::picker_scratch_dir("model-listing");
    std::fs::create_dir_all(dir.join("donors")).expect("scratch dir must be creatable");
    std::fs::create_dir_all(dir.join(".hidden")).expect("scratch dir must be creatable");
    for (leaf, when) in [
        ("DS2SOFS0000.sl2", 2_000u64),
        ("donor.zip", 3_000),
        ("screenshot.png", 4_000),
        ("notes.txt", 5_000),
        (".secret.sl2", 6_000),
    ] {
        let path = dir.join(leaf);
        std::fs::write(&path, b"contents are not read by the listing")
            .expect("scratch file must be writable");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("scratch file must be reopenable")
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(when))
            .expect("scratch file timestamp must be settable");
    }

    let model = SavePickerModel::open(&dir);
    assert_eq!(
        model
            .entries()
            .iter()
            .map(PickerEntry::name)
            .collect::<Vec<_>>(),
        vec!["donors", "donor.zip", "DS2SOFS0000.sl2"],
        "a folder, then the two shapes the staging step can read, newest first"
    );
}

/// An unreadable folder leaves the picker navigable rather than stuck.
#[test]
fn a_folder_that_cannot_be_read_still_has_a_way_out() {
    let dir = crate::picker_scratch_dir("model-missing").join("no-such-folder");
    let model = SavePickerModel::open(&dir);
    assert_eq!(model.entry_count(), 0);
    assert_eq!(model.row_meaning(0), PickerRow::ParentDir);
    assert_eq!(
        model.cursor(),
        0,
        "the only row worth pressing is the way up"
    );
}

// ---------------------------------------------------------------------------------------
// The file stage's rows.
// ---------------------------------------------------------------------------------------

/// The parent row shifts the entries by one, and the shift is applied in exactly one place -- so
/// the row that says `save0.sl2` is the row that picks `save0.sl2`.
#[test]
fn the_parent_row_shifts_the_entries_and_activation_follows_the_shift() {
    let model = files_model(DIR, listing(DIR, 3));
    assert_eq!(model.parent_row(), Some(0));
    assert_eq!(model.entry_row_base(), 1);
    assert_eq!(
        rows(&model),
        vec![
            PickerRow::ParentDir,
            PickerRow::File(PathBuf::from(format!(r"{DIR}\save2.sl2"))),
            PickerRow::File(PathBuf::from(format!(r"{DIR}\save1.sl2"))),
            PickerRow::File(PathBuf::from(format!(r"{DIR}\save0.sl2"))),
        ]
    );
}

/// At a drive root there is no parent row, so the entries start at row 0.
#[test]
fn a_drive_root_has_no_way_up_and_starts_its_entries_at_the_top() {
    let model = files_model(r"Z:\", listing(r"Z:", 2));
    assert_eq!(model.parent_row(), None);
    assert_eq!(model.entry_row_base(), 0);
    assert!(matches!(model.row_meaning(0), PickerRow::File(_)));
}

/// An empty drive root has nothing above it and nothing in it. It still gets one row, because a
/// listing with no rows is a listing a player cannot tell from a hang.
#[test]
fn an_empty_root_names_its_dead_end() {
    let mut model = files_model(r"Z:\", Vec::new());
    assert_eq!(model.visible_row_count(), 1);
    assert_eq!(model.row_meaning(0), PickerRow::AtRoot);
    assert_eq!(model.activate(0), PickerActivation::Ignored);
}

/// A fresh listing puts the cursor on something worth pressing, not on `[..]`.
#[test]
fn the_cursor_starts_on_a_save_rather_than_on_the_way_up() {
    let model = files_model(DIR, listing(DIR, 3));
    assert_eq!(model.cursor(), 1);
    assert!(matches!(
        model.row_meaning(model.cursor()),
        PickerRow::File(_)
    ));

    // With nothing to list, the only selectable row IS the way up.
    let empty = files_model(DIR, Vec::new());
    assert_eq!(empty.cursor(), 0);
    assert_eq!(empty.row_meaning(0), PickerRow::ParentDir);
}

// ---------------------------------------------------------------------------------------
// The scroll window.
// ---------------------------------------------------------------------------------------

/// More saves than rows: the window slides under a cursor that stays on the edge row.
#[test]
fn a_long_listing_scrolls_under_a_cursor_that_holds_the_edge() {
    let mut model = files_model(DIR, listing(DIR, 30));
    model.set_row_capacity(5);
    // Row 0 is `[..]`; four entry rows below it.
    assert_eq!(model.entry_row_base(), 1);
    assert_eq!(model.visible_row_count(), 5);
    assert_eq!(model.scroll_max(), 30 - 4);

    model.set_cursor(4);
    model.move_cursor(true);
    assert_eq!(model.cursor(), 4, "the cursor held the bottom row");
    assert_eq!(model.scroll_offset(), 1, "the listing moved instead");

    model.move_cursor(false);
    assert_eq!(
        model.cursor(),
        3,
        "away from the edge, it is ordinary movement"
    );
    assert_eq!(model.scroll_offset(), 1);

    model.set_cursor(1);
    model.move_cursor(false);
    assert_eq!(model.cursor(), 1, "the top entry row holds too");
    assert_eq!(model.scroll_offset(), 0, "and the window came back");
}

/// The window stops at the ends, and only then does the cursor wrap.
#[test]
fn the_cursor_wraps_only_once_the_window_has_nowhere_left_to_go() {
    let mut model = files_model(DIR, listing(DIR, 6));
    model.set_row_capacity(4);
    let scroll_max = model.scroll_max();
    assert_eq!(scroll_max, 3);

    model.set_cursor(3);
    for _ in 0..scroll_max {
        model.move_cursor(true);
    }
    assert_eq!(model.scroll_offset(), scroll_max);
    assert_eq!(model.cursor(), 3);

    // One more press: nothing to scroll, so the selection wraps to the top of the rows.
    model.move_cursor(true);
    assert_eq!(
        model.scroll_offset(),
        scroll_max,
        "the window is at its end"
    );
    assert_eq!(model.cursor(), 0);
    assert_eq!(model.row_meaning(0), PickerRow::ParentDir);
}

/// Rows past the listing are not drawn and cannot be reached.
#[test]
fn rows_past_the_listing_hold_nothing() {
    let model = files_model(DIR, listing(DIR, 2));
    assert_eq!(model.visible_row_count(), 3);
    for row in model.visible_row_count()..model.row_capacity() + 4 {
        assert_eq!(model.row_meaning(row), PickerRow::Empty, "row {row}");
        assert!(!model.row_selectable(row), "row {row}");
    }
}

/// A surface that shrinks re-clamps the window and the cursor instead of addressing rows it can
/// no longer draw.
#[test]
fn shrinking_the_surface_reclamps_the_window_and_the_cursor() {
    let mut model = files_model(DIR, listing(DIR, 12));
    model.set_row_capacity(10);
    model.set_cursor(9);
    model.scroll_window_one(true);
    model.scroll_window_one(true);
    assert_eq!(model.scroll_offset(), 2);

    model.set_row_capacity(3);
    assert!(
        model.cursor() < 3,
        "cursor {} is off the surface",
        model.cursor()
    );
    assert!(model.scroll_offset() <= model.scroll_max());
    assert!(model.row_selectable(model.cursor()));
}

// ---------------------------------------------------------------------------------------
// The character stage. The invariant this whole crate exists for.
// ---------------------------------------------------------------------------------------

/// ROW INDEX IS SLOT NUMBER. Ten rows for ten slots, in order, gaps included.
#[test]
fn a_character_rows_index_is_the_slot_number_the_game_means() {
    let model = character_model(&[0, 3, 7]);
    for index in 0..SLOT_COUNT {
        assert_eq!(
            model.row_meaning(index),
            PickerRow::Character(index),
            "row {index} must be slot {index}"
        );
    }
    assert_eq!(model.row_meaning(CHARACTER_BACK_ROW), PickerRow::Back);
    assert_eq!(model.row_meaning(CHARACTER_BACK_ROW + 1), PickerRow::Empty);
    assert_eq!(model.visible_row_count(), SLOT_COUNT + 1);
}

/// The back row is BELOW the slots. Above them it would shift every slot by one, which is the
/// exact renumbering the invariant forbids.
#[test]
fn the_back_row_sits_after_the_slots_so_it_shifts_nothing() {
    let model = character_model(&[0]);
    assert_eq!(CHARACTER_BACK_ROW, SLOT_COUNT);
    assert_eq!(model.row_meaning(0), PickerRow::Character(0));
    assert_eq!(model.entry_row_base(), 0, "nothing sits above slot 0");
}

/// An empty slot keeps its row, draws, and refuses the press.
#[test]
fn an_empty_slot_keeps_its_row_and_refuses_to_be_pressed() {
    let mut model = character_model(&[5]);
    assert_eq!(model.row_meaning(2), PickerRow::Character(2));
    assert!(!model.row_selectable(2));
    assert_eq!(model.activate(2), PickerActivation::Ignored);
    let expected = PickRejection::NoLoadableCharacter.status_message();
    assert_eq!(
        model.status_message().map(PickerStatusMessage::headline),
        Some(expected.headline()),
        "a row that does nothing when pressed has to say why"
    );
}

/// The cursor steps over the gaps without the gaps moving.
#[test]
fn the_cursor_steps_over_empty_slots_without_renumbering_them() {
    let mut model = character_model(&[1, 4, 9]);
    assert_eq!(
        model.cursor(),
        1,
        "the first loadable slot, not the first row"
    );

    model.move_cursor(true);
    assert_eq!(model.cursor(), 4);
    model.move_cursor(true);
    assert_eq!(model.cursor(), 9);
    // Past the last character sits the back row, then it wraps.
    model.move_cursor(true);
    assert_eq!(model.cursor(), CHARACTER_BACK_ROW);
    model.move_cursor(true);
    assert_eq!(model.cursor(), 1);
    model.move_cursor(false);
    assert_eq!(model.cursor(), CHARACTER_BACK_ROW);

    // And the rows themselves never moved.
    assert_eq!(model.row_meaning(4), PickerRow::Character(4));
    assert_eq!(model.row_meaning(9), PickerRow::Character(9));
}

/// A blank slot -- named, nothing rolled -- is a character the game loads, so the picker offers it.
#[test]
fn a_blank_slot_is_offered_because_the_game_will_load_it() {
    let mut model = files_model(DIR, listing(DIR, 1));
    let mut slots = ten_slots(&[]);
    slots[6] = slot(6, SlotState::Blank);
    model
        .show_characters(Path::new(CONTAINER), slots)
        .expect("a blank slot is loadable");
    assert!(model.row_selectable(6));
    assert_eq!(model.cursor(), 6);
    assert_eq!(
        model.activate(6),
        PickerActivation::PickedCharacter {
            path: PathBuf::from(CONTAINER),
            slot: 6,
        }
    );
}

/// Choosing a character names the container AND the slot. That pair is the whole feature.
#[test]
fn choosing_a_character_names_the_container_and_the_slot() {
    let mut model = character_model(&[0, 3, 7]);
    assert_eq!(
        model.activate(7),
        PickerActivation::PickedCharacter {
            path: PathBuf::from(CONTAINER),
            slot: 7,
        }
    );
    assert_eq!(model.picked_container(), Some(Path::new(CONTAINER)));
    assert_eq!(model.character_slot(7).map(|found| found.slot), Some(7));
}

/// A container with ten empty slots never becomes a character stage at all.
#[test]
fn a_container_with_nothing_in_it_is_refused_rather_than_shown() {
    let mut model = files_model(DIR, listing(DIR, 1));
    assert_eq!(
        model.show_characters(Path::new(CONTAINER), ten_slots(&[])),
        Err(PickRejection::NoLoadableCharacter)
    );
    assert!(
        !model.choosing_character(),
        "the listing is still the listing"
    );
}

/// Back returns to the folder the player was browsing, not to some remembered other place.
#[test]
fn going_back_returns_to_the_folder_the_player_left() {
    let dir = crate::picker_scratch_dir("model-back");
    std::fs::write(dir.join("DS2SOFS0000.sl2"), b"not a container")
        .expect("scratch file must be writable");
    let mut model = SavePickerModel::open(&dir);
    let listed = model.entry_count();
    model
        .show_characters(Path::new(CONTAINER), ten_slots(&[2]))
        .expect("one occupied slot is showable");
    assert!(model.choosing_character());

    assert_eq!(
        model.activate(CHARACTER_BACK_ROW),
        PickerActivation::Repopulate
    );
    assert!(!model.choosing_character());
    assert_eq!(model.current_dir(), dir.as_path());
    assert_eq!(model.entry_count(), listed);
}

/// The surface's row budget does not get to renumber the slots.
#[test]
fn a_short_surface_cannot_shrink_the_slot_numbering() {
    let mut model = character_model(&[9]);
    model.set_row_capacity(4);
    assert_eq!(model.row_meaning(9), PickerRow::Character(9));
    assert_eq!(model.row_meaning(CHARACTER_BACK_ROW), PickerRow::Back);
    assert_eq!(model.visible_row_count(), SLOT_COUNT + 1);
    assert_eq!(
        model.cursor(),
        9,
        "the one loadable slot is still reachable"
    );
}

// ---------------------------------------------------------------------------------------
// Picking a file.
// ---------------------------------------------------------------------------------------

/// A file that is not a container leaves the listing exactly where it was, and says why.
#[test]
fn a_refused_pick_leaves_the_listing_alone_and_says_why() {
    let dir = crate::picker_scratch_dir("model-refuse");
    std::fs::write(dir.join("DS2SOFS0000.sl2"), b"this is not a BND4")
        .expect("scratch file must be writable");
    let mut model = SavePickerModel::open(&dir);
    let before = model.entries().to_vec();
    let row = model.cursor();
    assert!(matches!(model.row_meaning(row), PickerRow::File(_)));

    assert_eq!(model.activate(row), PickerActivation::Ignored);
    assert!(!model.choosing_character());
    assert_eq!(model.entries(), before.as_slice());
    let expected = PickRejection::NotBnd4.status_message();
    assert_eq!(
        model.status_message().map(PickerStatusMessage::headline),
        Some(expected.headline())
    );
}

/// An archive's characters are unknowable until it is unwrapped, so the pick ends at the file.
#[test]
fn an_archive_pick_ends_at_the_file() {
    let dir = crate::picker_scratch_dir("model-archive");
    let archive = dir.join("donor.zip");
    std::fs::write(&archive, b"PK\x03\x04 not really").expect("scratch file must be writable");
    let mut model = SavePickerModel::open(&dir);
    let row = model.cursor();

    assert_eq!(model.activate(row), PickerActivation::PickedFile(archive));
    assert!(
        !model.choosing_character(),
        "nothing can name the characters inside an archive yet"
    );
}

/// Navigating clears a stale refusal, so an error never follows the player into another folder.
#[test]
fn a_refusal_does_not_follow_the_player_out_of_the_folder() {
    let dir = crate::picker_scratch_dir("model-clear");
    std::fs::create_dir_all(dir.join("donors")).expect("scratch dir must be creatable");
    std::fs::write(dir.join("DS2SOFS0000.sl2"), b"this is not a BND4")
        .expect("scratch file must be writable");
    let mut model = SavePickerModel::open(&dir);

    let save_row = (0..model.visible_row_count())
        .find(|&row| matches!(model.row_meaning(row), PickerRow::File(_)))
        .expect("the save is listed");
    assert_eq!(model.activate(save_row), PickerActivation::Ignored);
    assert!(model.status_message().is_some());

    let folder_row = (0..model.visible_row_count())
        .find(|&row| matches!(model.row_meaning(row), PickerRow::Dir(_)))
        .expect("the folder is listed");
    assert_eq!(model.activate(folder_row), PickerActivation::Repopulate);
    assert!(model.status_message().is_none());
    let donors = dir.join("donors");
    assert_eq!(model.current_dir(), donors.as_path());
}

/// Walking up out of a folder is the same decision `crate::path::parent` makes, and it works on a
/// real host directory as well as on the Windows paths the game hands over.
#[test]
fn the_way_up_goes_where_the_parent_row_says_it_goes() {
    let dir = crate::picker_scratch_dir("model-up");
    let child = dir.join("donors");
    std::fs::create_dir_all(&child).expect("scratch dir must be creatable");
    let mut model = SavePickerModel::open(&child);
    assert_eq!(model.row_meaning(0), PickerRow::ParentDir);
    assert_eq!(model.activate(0), PickerActivation::Repopulate);
    assert_eq!(model.current_dir(), dir.as_path());
}

/// Every row a surface would draw is a row the model can explain, in both stages.
#[test]
fn no_drawn_row_is_ever_a_mystery() {
    let browsing = files_model(DIR, listing(DIR, 4));
    let choosing = character_model(&[2, 6]);
    for model in [&browsing, &choosing] {
        for row in 0..model.visible_row_count() {
            assert_ne!(
                model.row_meaning(row),
                PickerRow::Empty,
                "row {row} is drawn and means nothing"
            );
        }
    }
}
