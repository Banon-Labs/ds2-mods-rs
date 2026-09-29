//! The parts ported for the in-game panel: drive strip, path field, file summaries, destination
//! mode, back, paging, and the single input and view entry points.
//!
//! Tests that walk real folders build them under a scratch root and turn the `/` translation off,
//! because on this host a scratch folder is `/tmp/...` and Wine's `Z:` does not exist. The ones
//! about the translation itself use paths no host has, and assert on the refusal's text.

use std::time::Duration;

use ds2_sl2_core::SlotState;

use super::tests::{files_model, listing};
use super::*;
use crate::text::{CURRENT_CONTAINER_MARKER, NEW_FILE_ROW_TEXT, row_text};

const DIR: &str = r"Z:\home\banon\saves";

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn touch(path: &Path, seconds: u64) {
    std::fs::write(path, b"contents are not a container").expect("scratch file must be writable");
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("scratch file must be reopenable")
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("scratch file timestamp must be settable");
}

/// Two "drives" that are real folders, `c` and `z`, each with a `saves` folder in it.
fn two_drives(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = crate::picker_scratch_dir(tag);
    let c = root.join("c");
    let z = root.join("z");
    for dir in [c.join("saves"), c.join("deep"), z.join("saves")] {
        std::fs::create_dir_all(dir).expect("scratch dir must be creatable");
    }
    (root, c, z)
}

fn host_model(dir: &Path) -> SavePickerModel {
    let mut model = SavePickerModel::open(dir);
    model.set_posix_drive(None);
    model
}

fn headline(model: &SavePickerModel) -> Option<String> {
    model
        .status_message()
        .map(|message| message.headline().to_owned())
}

fn expected_headline(rejection: PickRejection) -> Option<String> {
    Some(rejection.status_message().headline().to_owned())
}

fn row_of(model: &SavePickerModel, wanted: impl Fn(&PickerRow) -> bool) -> usize {
    (0..model.visible_row_count())
        .find(|&row| wanted(&model.row_meaning(row)))
        .expect("the row is on screen")
}

fn type_text(model: &mut SavePickerModel, typed: &str) {
    for character in typed.chars() {
        model.apply(PickerInput::Char(character));
    }
}

fn clear_field(model: &mut SavePickerModel) {
    while model.apply(PickerInput::Backspace) != PickerActivation::Ignored {}
}

// ---------------------------------------------------------------------------------------
// The drive strip.
// ---------------------------------------------------------------------------------------

/// The strip is row 0 once drives exist, and every other fixed row moves down by one -- in the
/// one place the shift is decided.
#[test]
fn the_drive_strip_sits_above_the_way_up_and_shifts_everything_once() {
    let mut model = files_model(DIR, listing(DIR, 2));
    assert_eq!(model.drive_row(), None);
    assert_eq!(model.entry_row_base(), 1);

    model.set_drives(vec![r"C:\".into(), r"Z:\".into()]);
    assert_eq!(model.drive_row(), Some(0));
    assert_eq!(model.row_meaning(0), PickerRow::DriveStrip);
    assert_eq!(model.parent_row(), Some(1));
    assert_eq!(model.entry_row_base(), 2);
    assert!(matches!(model.row_meaning(2), PickerRow::File(_)));
    assert_eq!(model.cursor(), 2, "the cursor still starts on a save");

    model.set_drives(Vec::new());
    assert_eq!(model.drive_row(), None);
    assert_eq!(model.parent_row(), Some(0));
}

/// The current drive is found without regard to case or separator, as Windows finds it.
#[test]
fn the_current_drive_is_the_root_the_folder_lies_under() {
    let mut model = files_model(r"z:/home/banon", Vec::new());
    model.set_drives(vec![r"C:\".into(), r"Z:\".into()]);
    assert_eq!(model.current_drive(), Some(1));
    let view = model.view();
    let current: Vec<&str> = view
        .drives
        .iter()
        .filter(|cell| cell.current)
        .map(|cell| cell.label.as_str())
        .collect();
    assert_eq!(current, vec!["Z:"]);
    assert!(crate::text::drive_strip_text(&model).contains(">Z:<"));
}

/// Left and right walk a ring of drives and the path field, and wrap at both ends.
#[test]
fn left_and_right_walk_a_ring_that_passes_through_the_path_field() {
    let (_root, c, z) = two_drives("parity-ring");
    let mut model = host_model(&z.join("saves"));
    model.set_drives(vec![text(&c), text(&z)]);
    model.set_cursor(0);
    assert_eq!(model.current_drive(), Some(1));

    assert_eq!(
        model.apply(PickerInput::Right),
        PickerActivation::Repopulate
    );
    assert!(
        model.path_field_focused(),
        "right off the last drive is the path field"
    );
    assert_eq!(model.cursor(), 0);

    model.apply(PickerInput::Right);
    assert!(!model.path_field_focused());
    assert_eq!(
        model.current_drive(),
        Some(0),
        "and right again wraps to the first drive"
    );
    assert_eq!(
        model.cursor(),
        0,
        "the cursor stays on the strip while stepping"
    );

    model.apply(PickerInput::Left);
    assert!(
        model.path_field_focused(),
        "left off the first drive is the path field too"
    );
    model.apply(PickerInput::Left);
    assert_eq!(model.current_drive(), Some(1));
}

/// Coming back to a drive lands in the folder last browsed on it.
#[test]
fn each_drive_remembers_where_the_player_left_it() {
    let (_root, c, z) = two_drives("parity-remember");
    let saves = z.join("saves");
    let mut model = host_model(&saves);
    model.set_drives(vec![text(&c), text(&z)]);

    assert_eq!(model.switch_to_drive(0), PickerActivation::Repopulate);
    assert_eq!(
        model.current_dir(),
        c.as_path(),
        "a drive never visited opens at its root"
    );
    let deep = row_of(
        &model,
        |row| matches!(row, PickerRow::Dir(path) if path.ends_with("deep")),
    );
    model.activate(deep);
    assert_eq!(model.current_dir(), c.join("deep").as_path());

    model.switch_to_drive(1);
    assert_eq!(model.current_dir(), saves.as_path());
    model.switch_to_drive(0);
    assert_eq!(model.current_dir(), c.join("deep").as_path());
    assert_eq!(model.remembered_dir(1), Some(saves.as_path()));
}

/// A remembered folder that has gone since is not browsed; the drive opens at its root.
#[test]
fn a_remembered_folder_that_vanished_falls_back_to_the_root() {
    let (_root, c, z) = two_drives("parity-vanished");
    let mut model = host_model(&c.join("deep"));
    model.set_drives(vec![text(&c), text(&z)]);
    model.switch_to_drive(1);
    std::fs::remove_dir_all(c.join("deep")).expect("scratch dir must be removable");
    model.switch_to_drive(0);
    assert_eq!(model.current_dir(), c.as_path());
}

/// Enter on a drive opens its root; Enter on the path field starts typing.
#[test]
fn enter_on_the_strip_opens_the_root_or_starts_typing() {
    let (_root, c, z) = two_drives("parity-strip-enter");
    let mut model = host_model(&z.join("saves"));
    model.set_drives(vec![text(&c), text(&z)]);
    model.set_cursor(0);

    assert_eq!(
        model.apply(PickerInput::Confirm),
        PickerActivation::Repopulate
    );
    assert_eq!(model.current_dir(), z.as_path());
    assert_eq!(model.cursor(), 0);

    model.apply(PickerInput::Right);
    assert!(model.path_field_focused());
    model.apply(PickerInput::Confirm);
    assert_eq!(model.editing(), Some(EditTarget::Path));
}

/// Past seven cells the strip shows a window, and every drive is visible when it is current.
#[test]
fn a_long_drive_list_shows_a_window_that_always_holds_the_current_drive() {
    let letters: Vec<char> = ('A'..='N').collect();
    let drives: Vec<String> = letters
        .iter()
        .map(|letter| format!(r"{letter}:\"))
        .collect();
    for (index, letter) in letters.iter().enumerate() {
        let mut model = files_model(&format!(r"{letter}:\saves"), Vec::new());
        model.set_drives(drives.clone());
        let cells = model.drive_strip_cells();
        assert!(cells.len() <= DRIVE_STRIP_MAX_CELLS, "{letter}: {cells:?}");
        assert!(
            cells.contains(&DriveCellKind::Drive(index)),
            "{letter}: {cells:?}"
        );
        assert_eq!(
            cells.first() == Some(&DriveCellKind::MoreLeft),
            !cells.contains(&DriveCellKind::Drive(0)),
            "{letter}: a left marker exactly when the first drive is hidden"
        );
        assert_eq!(
            cells.last() == Some(&DriveCellKind::MoreRight),
            !cells.contains(&DriveCellKind::Drive(letters.len() - 1)),
            "{letter}: a right marker exactly when the last drive is hidden"
        );
    }
}

#[test]
fn a_short_drive_list_has_no_markers() {
    let mut model = files_model(DIR, Vec::new());
    model.set_drives(vec![r"C:\".into(), r"D:\".into(), r"Z:\".into()]);
    assert_eq!(
        model.drive_strip_cells(),
        vec![
            DriveCellKind::Drive(0),
            DriveCellKind::Drive(1),
            DriveCellKind::Drive(2)
        ]
    );
}

/// The right marker steps to the first hidden drive, and clicking a drive cell switches to it.
#[test]
fn clicking_the_strip_switches_drives() {
    let drives: Vec<String> = ('A'..='J').map(|letter| format!(r"{letter}:\")).collect();
    let mut model = files_model(r"A:\", Vec::new());
    model.set_drives(drives);
    let cells = model.drive_strip_cells();
    let marker = cells
        .iter()
        .position(|cell| *cell == DriveCellKind::MoreRight)
        .expect("ten drives need a right marker");
    model.apply(PickerInput::ClickDriveCell(marker));
    assert_eq!(model.current_dir(), Path::new(r"G:\"));

    let first = model
        .drive_strip_cells()
        .iter()
        .position(|cell| *cell == DriveCellKind::Drive(5))
        .expect("drive five is next to drive six");
    model.apply(PickerInput::ClickDriveCell(first));
    assert_eq!(model.current_dir(), Path::new(r"F:\"));
}

// ---------------------------------------------------------------------------------------
// Paging and back.
// ---------------------------------------------------------------------------------------

/// Off the strip, left and right turn a page; the cursor keeps its row.
#[test]
fn left_and_right_off_the_strip_turn_a_page() {
    let mut model = files_model(DIR, listing(DIR, 30));
    model.set_row_capacity(5);
    model.set_cursor(2);
    assert_eq!(
        model.apply(PickerInput::Right),
        PickerActivation::Repopulate
    );
    assert_eq!(model.scroll_offset(), 4, "one window of four entries");
    assert_eq!(model.cursor(), 2);
    model.apply(PickerInput::PageDown);
    assert_eq!(model.scroll_offset(), 8);
    model.apply(PickerInput::Left);
    model.apply(PickerInput::PageUp);
    assert_eq!(model.scroll_offset(), 0);
    assert_eq!(model.apply(PickerInput::PageUp), PickerActivation::Ignored);
}

/// The last page is short: the cursor drops onto the last row that holds something.
#[test]
fn a_short_last_page_pulls_the_cursor_onto_something() {
    let mut model = files_model(DIR, listing(DIR, 6));
    model.set_row_capacity(5);
    model.set_cursor(4);
    model.apply(PickerInput::PageDown);
    assert_eq!(model.scroll_offset(), model.scroll_max());
    assert!(model.row_selectable(model.cursor()));
}

#[test]
fn back_goes_up_a_folder_and_stops_at_a_root() {
    let mut model = files_model(DIR, Vec::new());
    model.set_posix_drive(None);
    assert_eq!(model.apply(PickerInput::Back), PickerActivation::Repopulate);
    assert_eq!(model.current_dir(), Path::new(r"Z:\home\banon"));
    model.apply(PickerInput::Backspace);
    model.apply(PickerInput::Back);
    assert_eq!(model.current_dir(), Path::new(r"Z:\"));
    assert_eq!(model.apply(PickerInput::Back), PickerActivation::Ignored);
    assert_eq!(model.current_dir(), Path::new(r"Z:\"));
}

#[test]
fn back_from_the_characters_returns_to_the_files() {
    let mut model = files_model(DIR, Vec::new());
    let slots: Vec<SaveSlot> = (0..SLOT_COUNT)
        .map(|index| SaveSlot {
            slot: index,
            state: if index == 2 {
                SlotState::Occupied
            } else {
                SlotState::Empty
            },
            name: "Vendrick".into(),
            stats: if index == 2 { [20; 9] } else { [0; 9] },
        })
        .collect();
    model
        .show_characters(Path::new(r"Z:\home\banon\saves\a.sl2"), slots)
        .expect("one occupied slot is showable");
    assert_eq!(model.apply(PickerInput::Back), PickerActivation::Repopulate);
    assert!(!model.choosing_character());
    assert_eq!(model.current_dir(), Path::new(DIR));
}

// ---------------------------------------------------------------------------------------
// The path field.
// ---------------------------------------------------------------------------------------

/// Tab opens the field on the current folder; typing, completing and committing browse there.
///
/// The folders are lower case because the completion keeps the case the player typed, which is
/// the same folder to Windows and to Wine's drives but not to this host's filesystem.
/// `crate::autocomplete` holds the case rule to its own test.
#[test]
fn a_typed_path_completes_and_commits() {
    let root = crate::picker_scratch_dir("parity-path");
    std::fs::create_dir_all(root.join("hangar").join("inner")).expect("scratch dir");
    std::fs::create_dir_all(root.join("Harbour")).expect("scratch dir");
    std::fs::write(root.join("hat.sl2"), b"x").expect("scratch file");
    let mut model = host_model(&root);

    assert_eq!(model.apply(PickerInput::Tab), PickerActivation::Repopulate);
    let separator = std::path::MAIN_SEPARATOR;
    assert_eq!(
        model.edit_text(),
        Some(format!("{}{separator}", root.display()).as_str()),
        "prefilled with the current folder, ready for a child's name"
    );

    type_text(&mut model, "ha");
    assert_eq!(
        model.ghost_suffix().as_deref(),
        Some("ngar"),
        "folders only, first by name"
    );
    let view = model.view();
    let field = view.path_field.expect("the file stage has a path field");
    assert!(field.editing);
    assert_eq!(field.ghost.as_deref(), Some("ngar"));

    assert_eq!(
        model.apply(PickerInput::Right),
        PickerActivation::Repopulate
    );
    assert!(
        model
            .edit_text()
            .is_some_and(|typed| typed.ends_with("hangar"))
    );
    assert_eq!(model.ghost_suffix(), None, "nothing more to add");
    model.apply(PickerInput::Backspace);
    model.apply(PickerInput::Tab);
    assert!(
        model
            .edit_text()
            .is_some_and(|typed| typed.ends_with("hangar"))
    );

    assert_eq!(
        model.apply(PickerInput::Confirm),
        PickerActivation::Repopulate
    );
    assert_eq!(model.editing(), None);
    assert_eq!(
        text(model.current_dir()).to_lowercase(),
        text(&root.join("hangar")).to_lowercase()
    );
}

/// Each way a typed path can be refused says so in its own words, and keeps the text.
#[test]
fn a_refused_path_says_why_and_keeps_what_was_typed() {
    let root = crate::picker_scratch_dir("parity-path-refused");
    let mut model = host_model(&root);
    model.apply(PickerInput::Tab);

    clear_field(&mut model);
    assert_eq!(model.edit_text(), Some(""));
    assert_eq!(model.apply(PickerInput::Confirm), PickerActivation::Ignored);
    assert_eq!(
        headline(&model),
        expected_headline(PickRejection::PathEmpty)
    );

    type_text(&mut model, r"saves\x");
    model.apply(PickerInput::Confirm);
    assert_eq!(
        headline(&model),
        expected_headline(PickRejection::PathNotAbsolute)
    );
    assert_eq!(
        model.edit_text(),
        Some(r"saves\x"),
        "the field keeps the text"
    );

    clear_field(&mut model);
    let missing = text(&root.join("no-such-folder"));
    type_text(&mut model, &missing);
    model.apply(PickerInput::Confirm);
    assert_eq!(
        headline(&model),
        expected_headline(PickRejection::FolderNotFound)
    );
    assert_eq!(model.edit_text(), Some(missing.as_str()));
    assert_eq!(model.editing(), Some(EditTarget::Path));
    assert_eq!(model.current_dir(), root.as_path(), "nothing moved");

    // A file is not a folder either.
    clear_field(&mut model);
    let file = root.join("a.sl2");
    std::fs::write(&file, b"x").expect("scratch file");
    type_text(&mut model, &text(&file));
    model.apply(PickerInput::Confirm);
    assert_eq!(
        headline(&model),
        expected_headline(PickRejection::FolderNotFound)
    );
}

/// A Linux path is the Wine drive the model was told about. On this host that folder does not
/// exist, and the refusal names the translated path so the player can see what was tried.
#[test]
fn a_linux_path_is_tried_on_the_wine_drive() {
    let mut model = files_model(DIR, Vec::new());
    model.apply(PickerInput::Tab);
    clear_field(&mut model);
    type_text(&mut model, "/definitely/not/here");
    model.apply(PickerInput::Confirm);
    let message = model.status_message().expect("refused");
    assert_eq!(message.headline(), "Folder Not Found");
    assert_eq!(message.second_detail(), Some(r"Z:\definitely\not\here"));
}

/// Back cancels the edit and discards the text; Backspace only deletes.
#[test]
fn back_cancels_typing_and_backspace_deletes() {
    let mut model = files_model(DIR, Vec::new());
    model.apply(PickerInput::Tab);
    type_text(&mut model, "xy");
    model.apply(PickerInput::Backspace);
    assert!(model.edit_text().is_some_and(|typed| typed.ends_with('x')));
    assert_eq!(model.apply(PickerInput::Back), PickerActivation::Repopulate);
    assert_eq!(model.editing(), None);
    assert_eq!(
        model.current_dir(),
        Path::new(DIR),
        "cancelling goes nowhere"
    );
    assert_eq!(
        model.apply(PickerInput::Char('q')),
        PickerActivation::Ignored
    );
}

/// Up and down do nothing while typing: they would move a highlight the player cannot see
/// behind the field.
#[test]
fn arrows_other_than_right_do_nothing_while_typing() {
    let mut model = files_model(DIR, listing(DIR, 3));
    let cursor = model.cursor();
    model.apply(PickerInput::Tab);
    for input in [PickerInput::Up, PickerInput::Down, PickerInput::Left] {
        assert_eq!(model.apply(input), PickerActivation::Ignored);
    }
    assert_eq!(model.cursor(), cursor);
}

// ---------------------------------------------------------------------------------------
// Rows: summary, time, current marker.
// ---------------------------------------------------------------------------------------

#[test]
fn a_file_row_carries_its_time_and_its_summary() {
    let root = crate::picker_scratch_dir("parity-summary");
    touch(&root.join("junk.sl2"), 1_790_496_180);
    touch(&root.join("donor.zip"), 1_000);
    let mut model = host_model(&root);
    let view = model.view();
    let junk = view
        .rows
        .iter()
        .find(|row| row.text == "junk.sl2")
        .expect("the save is listed");
    assert_eq!(junk.kind, RowKind::File);
    assert_eq!(junk.modified.as_deref(), Some("2026-09-27 08:03"));
    assert_eq!(
        junk.detail.as_deref(),
        Some(crate::summary::SUMMARY_UNREADABLE_TEXT)
    );
    let zip = view
        .rows
        .iter()
        .find(|row| row.text == "donor.zip")
        .expect("the archive is listed");
    assert_eq!(zip.detail, None, "an archive's characters are not claimed");
    assert_eq!(zip.modified.as_deref(), Some("1970-01-01 00:16"));
}

/// The summary is read once per listing: a cached one is used as is, and a new listing drops it.
#[test]
fn a_summary_is_cached_until_the_next_listing() {
    let root = crate::picker_scratch_dir("parity-summary-cache");
    let save = root.join("DS2SOFS0000.sl2");
    touch(&save, 5_000);
    let mut model = host_model(&root);
    let named: Vec<SaveSlot> = (0..SLOT_COUNT)
        .map(|index| SaveSlot {
            slot: index,
            state: if index == 0 {
                SlotState::Occupied
            } else {
                SlotState::Empty
            },
            name: "Lucatiel".into(),
            stats: if index == 0 { [10; 9] } else { [0; 9] },
        })
        .collect();
    model
        .summaries
        .insert(save.clone(), FileSummary::from_slots(&named));
    let detail =
        |model: &mut SavePickerModel| model.view().rows.into_iter().find_map(|row| row.detail);
    assert_eq!(detail(&mut model).as_deref(), Some("Lucatiel SL37"));
    model.refresh();
    assert_eq!(
        detail(&mut model).as_deref(),
        Some(crate::summary::SUMMARY_UNREADABLE_TEXT),
        "a fresh listing reads the file again"
    );
}

#[test]
fn the_live_container_is_starred() {
    let root = crate::picker_scratch_dir("parity-current");
    let live = root.join("DS2SOFS0000.sl2");
    touch(&live, 5_000);
    touch(&root.join("other.sl2"), 4_000);
    let mut model = host_model(&root);
    model.set_current_container(Some(PathBuf::from(text(&live).to_uppercase())));
    let row = row_of(
        &model,
        |row| matches!(row, PickerRow::File(path) if *path == live),
    );
    assert_eq!(
        row_text(&model, row),
        format!("{CURRENT_CONTAINER_MARKER} DS2SOFS0000.sl2"),
        "matched without regard to case, as Windows matches it"
    );
    let view = model.view();
    let starred: Vec<bool> = view
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::File)
        .map(|row| row.current)
        .collect();
    assert_eq!(starred, vec![true, false]);
}

// ---------------------------------------------------------------------------------------
// Destination mode.
// ---------------------------------------------------------------------------------------

fn destination(tag: &str) -> (PathBuf, SavePickerModel) {
    let root = crate::picker_scratch_dir(tag);
    touch(&root.join("existing.sl2"), 5_000);
    touch(&root.join("donor.zip"), 6_000);
    std::fs::create_dir_all(root.join("folder.sl2")).expect("scratch dir");
    let mut model = SavePickerModel::open_for_destination(&root, "export");
    model.set_posix_drive(None);
    (root, model)
}

#[test]
fn a_destination_lists_containers_under_a_new_row() {
    let (_root, model) = destination("parity-dest-list");
    assert!(model.is_destination());
    assert_eq!(model.open_reason(), PickerOpenReason::SaveToFile);
    assert_eq!(model.parent_row(), Some(0));
    assert_eq!(model.new_file_row(), Some(1));
    assert_eq!(model.row_meaning(1), PickerRow::NewFile);
    assert_eq!(row_text(&model, 1), format!("{NEW_FILE_ROW_TEXT} export"));
    let names: Vec<&str> = model.entries().iter().map(PickerEntry::name).collect();
    assert_eq!(
        names,
        vec!["folder.sl2", "existing.sl2"],
        "no archive is a destination"
    );
}

#[test]
fn a_new_name_becomes_a_new_save_with_its_extension() {
    let (root, mut model) = destination("parity-dest-new");
    model.set_cursor(1);
    assert_eq!(
        model.apply(PickerInput::Confirm),
        PickerActivation::Repopulate
    );
    assert_eq!(model.editing(), Some(EditTarget::FileName));
    assert_eq!(model.edit_text(), Some("export"));
    assert_eq!(model.ghost_suffix(), None, "names are not completed");
    let view = model.view();
    assert_eq!(
        view.name_field.map(|field| field.text).as_deref(),
        Some("export")
    );
    model.apply(PickerInput::Char('2'));
    assert_eq!(
        model.apply(PickerInput::Confirm),
        PickerActivation::PickedDestination {
            path: crate::path::join_leaf(&root, "export2.sl2"),
            overwrite: false,
        }
    );
}

#[test]
fn an_existing_file_is_asked_about_before_it_is_replaced() {
    let (root, mut model) = destination("parity-dest-confirm");
    let existing = root.join("existing.sl2");
    let row = row_of(
        &model,
        |row| matches!(row, PickerRow::File(path) if *path == existing),
    );
    assert_eq!(
        model.apply(PickerInput::ClickRow(row)),
        PickerActivation::Repopulate
    );
    assert_eq!(model.confirming_overwrite(), Some(existing.as_path()));
    assert_eq!(
        model.cursor(),
        CONFIRM_KEEP_ROW,
        "a stray press keeps the file"
    );

    let view = model.view();
    assert_eq!(view.stage, PickerStage::ConfirmOverwrite);
    assert_eq!(
        view.rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![RowKind::Overwrite, RowKind::KeepFile]
    );
    assert!(
        view.status
            .is_some_and(|banner| banner.headline().contains("Overwrite"))
    );

    assert_eq!(model.apply(PickerInput::Back), PickerActivation::Repopulate);
    assert_eq!(model.confirming_overwrite(), None);
    assert_eq!(model.cursor(), row, "no puts the player back on the file");

    model.apply(PickerInput::Confirm);
    model.apply(PickerInput::Up);
    assert_eq!(model.cursor(), CONFIRM_OVERWRITE_ROW);
    assert_eq!(
        model.apply(PickerInput::Confirm),
        PickerActivation::PickedDestination {
            path: existing,
            overwrite: true,
        }
    );
}

/// Typing the name of a file already there is the same question as picking its row.
#[test]
fn typing_an_existing_name_asks_the_same_question() {
    let (root, mut model) = destination("parity-dest-typed-existing");
    model.activate(1);
    clear_field(&mut model);
    type_text(&mut model, "existing");
    assert_eq!(
        model.apply(PickerInput::Confirm),
        PickerActivation::Repopulate
    );
    assert_eq!(
        model.confirming_overwrite(),
        Some(crate::path::join_leaf(&root, "existing.sl2").as_path())
    );
}

/// The container the game is playing from is a destination like any other file, by row or by
/// name: saving onto it is an ordinary save. It used to be refused, which stopped the player
/// saving over their own Seamless Co-op `.co2` (2026-09-27).
#[test]
fn the_live_container_is_asked_about_like_any_existing_file() {
    let (root, mut model) = destination("parity-dest-live");
    let live = root.join("existing.sl2");
    model.set_current_container(Some(live.clone()));
    let row = row_of(
        &model,
        |row| matches!(row, PickerRow::File(path) if *path == live),
    );
    assert_eq!(model.activate(row), PickerActivation::Repopulate);
    assert_eq!(model.confirming_overwrite(), Some(live.as_path()));

    let (root, mut model) = destination("parity-dest-live-typed");
    model.set_current_container(Some(crate::path::join_leaf(&root, "existing.sl2")));
    model.activate(1);
    clear_field(&mut model);
    type_text(&mut model, "existing.sl2");
    assert_eq!(
        model.apply(PickerInput::Confirm),
        PickerActivation::Repopulate
    );
    assert!(model.confirming_overwrite().is_some());
}

#[test]
fn a_bad_name_is_refused_in_its_own_words() {
    let (_root, mut model) = destination("parity-dest-bad-name");
    model.activate(1);
    for (typed, rejection) in [
        ("   ", PickRejection::NameEmpty),
        ("a/b", PickRejection::NameNotAllowed),
        (r"a\b", PickRejection::NameNotAllowed),
        ("what?", PickRejection::NameNotAllowed),
        ("trailing.", PickRejection::NameNotAllowed),
        ("folder.sl2", PickRejection::NameIsFolder),
    ] {
        clear_field(&mut model);
        type_text(&mut model, typed);
        assert_eq!(
            model.apply(PickerInput::Confirm),
            PickerActivation::Ignored,
            "{typed}"
        );
        assert_eq!(headline(&model), expected_headline(rejection), "{typed}");
    }
}

/// With nothing in the folder, the cursor starts on the row that can still make a file.
#[test]
fn an_empty_destination_starts_on_the_new_row() {
    let root = crate::picker_scratch_dir("parity-dest-empty");
    let model = SavePickerModel::open_for_destination(&root, "export");
    assert_eq!(model.row_meaning(model.cursor()), PickerRow::NewFile);
}

// ---------------------------------------------------------------------------------------
// The single entry points.
// ---------------------------------------------------------------------------------------

#[test]
fn a_click_selects_and_presses_and_a_click_on_nothing_does_nothing() {
    let mut model = files_model(DIR, listing(DIR, 3));
    assert_eq!(
        model.apply(PickerInput::ClickRow(40)),
        PickerActivation::Ignored
    );
    // Row 0 is the way up; clicking it goes there.
    assert_eq!(
        model.apply(PickerInput::ClickRow(0)),
        PickerActivation::Repopulate
    );
    assert_eq!(model.current_dir(), Path::new(r"Z:\home\banon"));
}

#[test]
fn up_and_down_report_whether_anything_moved() {
    let mut model = files_model(DIR, listing(DIR, 3));
    assert_eq!(model.apply(PickerInput::Down), PickerActivation::Repopulate);
    let lone = &mut files_model(r"Z:\", Vec::new());
    assert_eq!(lone.apply(PickerInput::Down), PickerActivation::Ignored);
}

/// The view is enough to draw the panel: a title, where it is, rows with kinds, and a hint that
/// changes with what the keys would do.
#[test]
fn the_view_carries_everything_the_panel_draws() {
    let mut model = files_model(DIR, listing(DIR, 2));
    model.set_drives(vec![r"C:\".into(), r"Z:\".into()]);
    let view = model.view();
    assert_eq!(view.title, PickerOpenReason::LoadCharacter.title());
    assert_eq!(view.subtitle, DIR);
    assert_eq!(view.stage, PickerStage::Files);
    assert_eq!(view.drives.len(), 2);
    assert_eq!(
        view.rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            RowKind::DriveStrip,
            RowKind::Parent,
            RowKind::File,
            RowKind::File
        ]
    );
    assert_eq!(view.rows.iter().filter(|row| row.highlighted).count(), 1);
    assert!(view.status.is_none());
    let browsing = view.hint;

    model.set_cursor(0);
    let on_strip = model.view();
    assert_ne!(
        on_strip.hint, browsing,
        "the strip's keys are different keys"
    );
    assert!(on_strip.drives.iter().any(|cell| cell.focused));

    model.apply(PickerInput::Tab);
    assert_ne!(model.view().hint, on_strip.hint);
}

/// Every press in every state returns an answer rather than panicking, and leaves the cursor on
/// something the player can press.
#[test]
fn no_press_in_any_state_strands_the_cursor() {
    let inputs = [
        PickerInput::Up,
        PickerInput::Down,
        PickerInput::Left,
        PickerInput::Right,
        PickerInput::PageUp,
        PickerInput::PageDown,
        PickerInput::Tab,
        PickerInput::Char('a'),
        PickerInput::Backspace,
        PickerInput::ClickRow(3),
        PickerInput::ClickDriveCell(1),
        PickerInput::ClickPathField,
        PickerInput::Back,
        PickerInput::Confirm,
    ];
    let (_root, mut model) = destination("parity-fuzz");
    model.set_drives(vec![r"C:\".into(), r"Z:\".into()]);
    for round in 0..3 {
        for input in inputs {
            model.apply(input);
            if model.editing().is_none() {
                assert!(
                    model.row_selectable(model.cursor())
                        || model.row_meaning(model.cursor()) == PickerRow::AtRoot,
                    "round {round}, after {input:?}: cursor {} on {:?}",
                    model.cursor(),
                    model.row_meaning(model.cursor())
                );
            }
            let _ = model.view();
        }
    }
}
