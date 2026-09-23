//! A save picker that lets a player choose a CHARACTER, not just a file.
//!
//! # The gap this fills
//!
//! `ds2-save-file`'s Load Character from File row opens a common file dialog, checks what comes
//! back, and hands the whole container to the redirect. That is a file picker wearing a
//! character picker's name: DARK SOULS II keeps ten characters per container, and the row asks
//! about none of them. A player with two saves and six characters between them cannot say which
//! one they want.
//!
//! This crate is the model behind the picker that can. Browse a folder, choose a container,
//! choose one of its ten slots. It is a port of `../er-mods-rs`'s `er-save-picker-core` ROW MODEL
//! -- and only that. The overlay it draws itself, its CPU compositor, its comdlg32 surface, its
//! no-save boot flow and its path autocomplete all stay there, because those are Elden Ring's
//! menus and Elden Ring's boot. The surfaces here will be different; the rows are the same
//! problem.
//!
//! # What it is made of
//!
//! | module | what it decides |
//! |---|---|
//! | [`model`] | what each row means, where the cursor is, what pressing a row does |
//! | [`reason`] | why a pick was refused, in words a player can act on |
//! | [`text`] | what a row says, kept apart from what it means |
//! | [`path`] | the leaf and the parent, found by hand rather than through `Path` |
//!
//! # It borrows the two gates instead of rebuilding them
//!
//! `ds2_save_file_core::accepts` already decides which extensions a pick may carry, and
//! `ds2_sl2_core::slots` already reads the ten character records out of a container. Neither one
//! is re-implemented here. A second copy of either would be a second answer to a question the
//! player is standing in front of, and the two copies would drift on the day one of them was
//! fixed.
//!
//! # Windows paths, Linux tests
//!
//! Every path this handles is a Windows path (`Z:\home\...`, `S:\steamapps\...`) and every test
//! runs on Linux, where `Path` cannot see a backslash. So nothing here decides anything from
//! `Path::extension` or `Path::file_name` -- see [`path`] for what happens when it does. Judging
//! the pick the same way under test as in the game is the whole reason the judging is testable
//! at all.
//!
//! # No game, no Windows, no unsafe
//!
//! Enforced at the root below rather than promised in a paragraph. The only handles this crate
//! opens are a directory it was asked to list and a file the player just chose.

#![forbid(unsafe_code)]

pub mod model;
pub mod path;
pub mod reason;
pub mod text;

pub use model::{
    CHARACTER_BACK_ROW, DEFAULT_ROW_CAPACITY, PickerActivation, PickerEntry, PickerRow,
    SavePickerModel,
};
pub use reason::{PickRejection, PickedSource, PickerStatusMessage, accepts_pick};
pub use text::{character_text, row_text};

/// A scratch directory this test process owns alone.
///
/// The process id is in the name and it is load-bearing: `cargo test` runs test functions on
/// several threads and a developer may have two copies of the binary running at once, and a
/// fixed name means one test's `remove_dir_all` deletes the directory another test is midway
/// through writing into. That failure is intermittent, blames the wrong test, and reproduces
/// about one run in eight -- which is long enough to waste an afternoon on.
///
/// It lives at crate scope so every module shares one implementation. Private per-module copies
/// are exactly how the fixed names got into the sibling repo in the first place: three modules
/// each grew their own, so there was nowhere a single fix could land.
#[cfg(test)]
pub(crate) fn picker_scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ds2-save-picker-{tag}-p{pid}",
        pid = std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir must be creatable");
    dir
}
