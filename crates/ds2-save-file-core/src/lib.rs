//! Two pause-menu rows that move a save between the game and a file the player picks.
//!
//! This crate is the half that needs no DARK SOULS II: the strings a common file dialog takes, the
//! rules about what a picked path is allowed to be, and the one decision that decides whether a
//! player loses a save. The half that opens the dialog and talks to the game is `ds2-save-file`.
//!
//! # The two rows, and why they are not symmetric
//!
//! | row | direction | what it risks |
//! |---|---|---|
//! | Load Character from File | file -> game | nothing on disk; the game reads a staged COPY |
//! | Save Game to File | game -> file | the player's own file, if the destination already exists |
//!
//! Only one of them can destroy something, and it is the one whose name sounds safe. So the
//! asymmetry is deliberate everywhere below: the load side refuses unfamiliar input and moves on,
//! and the save side refuses to be quiet about an overwrite.
//!
//! # What "load from a file" actually does, said plainly
//!
//! It does not read the file into the running character. DARK SOULS II keeps one save container per
//! Steam account in one directory, and the mechanism this repo already has -- `ds2-save-redirect` --
//! points the game's own save-directory builder at a staged copy of whatever file you name.
//!
//! **And that swap cannot happen in the session that asks for it.** DS2 saves on the way out of a
//! game: the pause menu's own Quit Game row persists the character before returning to the title, so
//! a session that re-points the directory and then quits writes the CURRENT character over the staged
//! copy, and LOAD GAME reads back the character the player was replacing. So the row records the pick
//! and the LOADER applies it on the next launch, before the game has built a save path at all. See
//! [`handoff`].
//!
//! That is why [`source`] accepts archives as well as `.sl2`: the staging path this feeds already
//! unwraps `.zip`, `.7z` and `.rar`, because that is the shape a save arrives in when someone
//! sends you one.
//!
//! # Provenance
//!
//! The dialog shapes are ported from `../er-mods-rs`'s `er-save-picker-core::os_dialog`, which is
//! the same comdlg32 mechanism against the same kind of file. What is NOT ported is that crate's
//! in-game browser (three thousand lines of Elden Ring menu), because DS2's menus are a different
//! engine generation and the OS dialog is the part that carries no game coupling at all.

pub mod dest;
pub mod filter;
pub mod handoff;
pub mod source;

pub use dest::{Route, with_extension};
pub use filter::{filter_string, wide_nul};
pub use handoff::HANDOFF_FILE_NAME;
pub use source::{SOURCE_EXTENSIONS, SourceRejection, accepts};

/// The extension every DARK SOULS II save container carries, without its dot.
///
/// Also the extension a Save-to-File destination is given when the player types a name with none:
/// the file this writes IS a `.sl2` container, byte for byte, and a copy under another extension is
/// a copy nothing will ever open.
pub const SAVE_EXTENSION: &str = "sl2";

/// The only save file name SOTFS builds, and therefore the default a Save-to-File dialog offers.
///
/// Held here rather than imported from `ds2-save-redirect` so this crate keeps its zero
/// dependencies; `ds2-save-file`'s tests assert the two spellings agree, which is the check that
/// makes the duplication safe rather than merely small.
pub const SAVE_FILE_NAME: &str = "DS2SOFS0000.sl2";
