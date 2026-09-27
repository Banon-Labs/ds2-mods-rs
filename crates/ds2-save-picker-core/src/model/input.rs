//! Every press the panel can deliver, and what each one does in each state.
//!
//! The panel maps raw keys, pad buttons and mouse events onto [`PickerInput`] and hands them to
//! [`SavePickerModel::apply`]; it decides nothing else. So this table is the whole input
//! behaviour of the in-game picker, and it is tested here, on Linux:
//!
//! | input | typing | overwrite question / characters | file stage |
//! |---|---|---|---|
//! | Up, Down | -- | move | move (scrolls at the window's edge) |
//! | Left, Right | Right takes the ghost | -- | strip row: step drives; elsewhere: page |
//! | `PageUp`, `PageDown` | -- | -- | page |
//! | Confirm | commit | press the row | press the row |
//! | Back | cancel | back | up one folder |
//! | Backspace | delete a character | back | up one folder |
//! | Tab | take the ghost | -- | start typing a path |
//! | `Char` | append | -- | -- |
//! | `ClickRow` | leave the field, then click | press the row | press the row |

use super::{PickerActivation, SavePickerModel, Stage};

/// One press, already translated from whatever device made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerInput {
    /// Cursor up.
    Up,
    /// Cursor down.
    Down,
    /// Left: along the drive strip, or a page up.
    Left,
    /// Right: along the drive strip, a page down, or take the completion while typing.
    Right,
    /// Enter, the pad's A: press the highlighted row, or commit the field.
    Confirm,
    /// The pad's B, a right click, Escape where the panel has no other use for it: one step back.
    Back,
    /// The Backspace key: delete while typing, one step back otherwise.
    Backspace,
    /// Tab: take the completion while typing, start typing a path otherwise.
    Tab,
    /// A typed character.
    Char(char),
    /// A left click on row `n`, counted the way [`SavePickerModel::row_meaning`] counts.
    ClickRow(usize),
    /// A left click on strip cell `n`, counted the way [`SavePickerModel::drive_strip_cells`]
    /// counts.
    ClickDriveCell(usize),
    /// A left click on the path field.
    ClickPathField,
    /// Page Up, or a wheel notch the panel wants to be a page.
    PageUp,
    /// Page Down, likewise.
    PageDown,
}

impl SavePickerModel {
    /// Do what `input` means right now. The single entry point the panel routes through.
    pub fn apply(&mut self, input: PickerInput) -> PickerActivation {
        if self.edit.is_some() {
            return self.apply_while_editing(input);
        }
        match self.stage {
            Stage::Files => self.apply_in_files(input),
            Stage::Characters { .. } | Stage::ConfirmOverwrite { .. } => self.apply_in_list(input),
        }
    }

    fn apply_while_editing(&mut self, input: PickerInput) -> PickerActivation {
        match input {
            PickerInput::Char(typed) => self.type_char(typed),
            PickerInput::Backspace => self.delete_char(),
            PickerInput::Tab | PickerInput::Right => self.accept_completion(),
            PickerInput::Confirm => self.commit_edit(),
            PickerInput::Back => self.cancel_edit(),
            // A click elsewhere is the player leaving the field: drop the edit, then do what the
            // click would have done had they not been typing.
            PickerInput::ClickRow(_) | PickerInput::ClickDriveCell(_) => {
                self.edit = None;
                match self.apply(input) {
                    // The field closing is itself a change the panel has to draw.
                    PickerActivation::Ignored => PickerActivation::Repopulate,
                    done => done,
                }
            }
            PickerInput::ClickPathField => {
                if self.editing() == Some(super::EditTarget::Path) {
                    PickerActivation::Ignored
                } else {
                    self.edit = None;
                    self.begin_path_edit()
                }
            }
            PickerInput::Up
            | PickerInput::Down
            | PickerInput::Left
            | PickerInput::PageUp
            | PickerInput::PageDown => PickerActivation::Ignored,
        }
    }

    fn apply_in_files(&mut self, input: PickerInput) -> PickerActivation {
        let on_strip = self.drive_row() == Some(self.cursor);
        match input {
            PickerInput::Up => self.moved(|model| model.move_cursor(false)),
            PickerInput::Down => self.moved(|model| model.move_cursor(true)),
            PickerInput::Left | PickerInput::Right if on_strip => {
                self.step_drive_strip(input == PickerInput::Right)
            }
            PickerInput::Left | PickerInput::PageUp => self.paged(false),
            PickerInput::Right | PickerInput::PageDown => self.paged(true),
            PickerInput::Confirm => self.activate_cursor(),
            PickerInput::Back | PickerInput::Backspace => self.back(),
            PickerInput::Tab | PickerInput::ClickPathField => self.begin_path_edit(),
            PickerInput::Char(_) => PickerActivation::Ignored,
            PickerInput::ClickRow(row) => self.click_row(row),
            PickerInput::ClickDriveCell(cell) => self.click_drive_cell(cell),
        }
    }

    fn apply_in_list(&mut self, input: PickerInput) -> PickerActivation {
        match input {
            PickerInput::Up => self.moved(|model| model.move_cursor(false)),
            PickerInput::Down => self.moved(|model| model.move_cursor(true)),
            PickerInput::Confirm => self.activate_cursor(),
            PickerInput::Back | PickerInput::Backspace => self.back(),
            PickerInput::ClickRow(row) => self.click_row(row),
            PickerInput::Left
            | PickerInput::Right
            | PickerInput::PageUp
            | PickerInput::PageDown
            | PickerInput::Tab
            | PickerInput::Char(_)
            | PickerInput::ClickDriveCell(_)
            | PickerInput::ClickPathField => PickerActivation::Ignored,
        }
    }

    /// A click selects and presses in one go, the way a mouse user expects a list to behave. A
    /// click on the strip row itself (not on a cell) only moves the highlight there.
    fn click_row(&mut self, row: usize) -> PickerActivation {
        if !self.row_selectable(row) {
            return PickerActivation::Ignored;
        }
        self.set_cursor(row);
        if self.drive_row() == Some(row) {
            return PickerActivation::Repopulate;
        }
        self.activate(row)
    }

    fn moved(&mut self, step: impl FnOnce(&mut Self)) -> PickerActivation {
        let before = (self.cursor, self.scroll_offset);
        step(self);
        if (self.cursor, self.scroll_offset) == before {
            PickerActivation::Ignored
        } else {
            PickerActivation::Repopulate
        }
    }

    fn paged(&mut self, down: bool) -> PickerActivation {
        if self.page(down) {
            PickerActivation::Repopulate
        } else {
            PickerActivation::Ignored
        }
    }
}
