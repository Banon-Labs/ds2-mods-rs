//! The one text field: a folder path typed with completion, or a new save's name.
//!
//! # The caret lives at the end
//!
//! Characters append, Backspace removes the last one, and there is no caret to move. A player
//! fixing a path in the middle deletes back to it; that is what a gamepad's on-screen keyboard
//! does anyway, and a movable caret would be a second cursor for the panel to draw and for the
//! model to keep legal, for a field that holds one short line.
//!
//! # A rejected commit keeps the text
//!
//! `FOLDER NOT FOUND` with the field wiped means retyping a whole path to fix one letter. The
//! refusal goes to the status banner and the field stays open with exactly what was typed.

use crate::autocomplete::{resolve_typed_path, suggestion_for, typed_path_is_absolute};
use crate::path::join_leaf;
use crate::reason::PickRejection;

use super::{PickerActivation, SavePickerModel};

/// What the field is being used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// A folder to browse to, with completion.
    Path,
    /// Destination mode: the name of a new save in the current folder. No completion -- a name
    /// that matches something here is either a file to overwrite, which has its own row, or a
    /// folder, which is refused.
    FileName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Edit {
    pub(super) target: EditTarget,
    pub(super) text: String,
}

/// Characters Windows refuses anywhere in a file name.
const NAME_FORBIDDEN: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

impl SavePickerModel {
    /// True while the field is taking keystrokes.
    pub fn editing(&self) -> Option<EditTarget> {
        self.edit.as_ref().map(|edit| edit.target)
    }

    /// What the field holds while it is being edited.
    pub fn edit_text(&self) -> Option<&str> {
        self.edit.as_ref().map(|edit| edit.text.as_str())
    }

    /// Start typing a folder path, prefilled with the current folder and a trailing separator so
    /// the next keystroke is already naming a child of it.
    pub fn begin_path_edit(&mut self) -> PickerActivation {
        if !self.in_files() {
            return PickerActivation::Ignored;
        }
        let dir = self.current_dir.to_string_lossy();
        let text = if dir.ends_with(['\\', '/']) || dir.is_empty() {
            dir.into_owned()
        } else {
            let separator = if dir.contains('\\') { '\\' } else { '/' };
            format!("{dir}{separator}")
        };
        self.edit = Some(Edit {
            target: EditTarget::Path,
            text,
        });
        if self.has_drive_row() {
            self.path_focused = true;
            self.cursor = 0;
        }
        PickerActivation::Repopulate
    }

    /// Start typing a new save's name, prefilled with the destination's default.
    pub(super) fn begin_name_edit(&mut self) -> PickerActivation {
        let Some(default_name) = self.default_file_name().map(str::to_owned) else {
            return PickerActivation::Ignored;
        };
        if !self.in_files() {
            return PickerActivation::Ignored;
        }
        self.edit = Some(Edit {
            target: EditTarget::FileName,
            text: default_name,
        });
        if let Some(row) = self.new_file_row() {
            self.cursor = row;
        }
        PickerActivation::Repopulate
    }

    /// Stop typing and throw the text away.
    pub fn cancel_edit(&mut self) -> PickerActivation {
        if self.edit.take().is_none() {
            return PickerActivation::Ignored;
        }
        self.clear_status_message();
        PickerActivation::Repopulate
    }

    /// Append one typed character. Control characters are not text and are dropped.
    pub fn type_char(&mut self, typed: char) -> PickerActivation {
        let Some(edit) = self.edit.as_mut() else {
            return PickerActivation::Ignored;
        };
        if typed.is_control() {
            return PickerActivation::Ignored;
        }
        edit.text.push(typed);
        PickerActivation::Repopulate
    }

    /// Remove the last character.
    pub fn delete_char(&mut self) -> PickerActivation {
        match self.edit.as_mut().and_then(|edit| edit.text.pop()) {
            Some(_) => PickerActivation::Repopulate,
            None => PickerActivation::Ignored,
        }
    }

    /// What completing the path field would add after the typed text, if anything would.
    ///
    /// Only the tail is returned: the panel draws it dimmed after the live text, and the typed
    /// part underneath stays exactly as typed (see `crate::autocomplete` on why).
    pub fn ghost_suffix(&self) -> Option<String> {
        let edit = self.edit.as_ref()?;
        if edit.target != EditTarget::Path {
            return None;
        }
        let offer = suggestion_for(&edit.text, self.posix_drive)?;
        offer.get(edit.text.len()..).map(str::to_owned)
    }

    /// Tab or Right in the path field: take the ghost.
    pub fn accept_completion(&mut self) -> PickerActivation {
        let Some(suffix) = self.ghost_suffix() else {
            return PickerActivation::Ignored;
        };
        if let Some(edit) = self.edit.as_mut() {
            edit.text.push_str(&suffix);
        }
        PickerActivation::Repopulate
    }

    /// Enter in the field: browse to the typed folder, or choose the typed name.
    pub fn commit_edit(&mut self) -> PickerActivation {
        let Some(edit) = self.edit.clone() else {
            return PickerActivation::Ignored;
        };
        match edit.target {
            EditTarget::Path => self.commit_path(&edit.text),
            EditTarget::FileName => self.commit_name(&edit.text),
        }
    }

    fn commit_path(&mut self, typed: &str) -> PickerActivation {
        if typed.trim().is_empty() {
            return self.refuse(PickRejection::PathEmpty);
        }
        if !typed_path_is_absolute(typed) {
            return self.refuse(PickRejection::PathNotAbsolute);
        }
        let candidate = resolve_typed_path(typed, self.posix_drive);
        if !candidate.is_dir() {
            let message = PickRejection::FolderNotFound
                .status_message()
                .with_second_detail(candidate.to_string_lossy());
            self.set_status_message(message);
            return PickerActivation::Ignored;
        }
        self.clear_status_message();
        self.path_focused = false;
        self.browse(candidate)
    }

    fn commit_name(&mut self, typed: &str) -> PickerActivation {
        let name = typed.trim();
        if name.is_empty() {
            return self.refuse(PickRejection::NameEmpty);
        }
        if name.contains(NAME_FORBIDDEN)
            || name.chars().any(char::is_control)
            || typed.ends_with(['.', ' '])
            || name == "."
            || name == ".."
        {
            return self.refuse(PickRejection::NameNotAllowed);
        }
        let path = ds2_save_file_core::dest::with_extension(
            &join_leaf(&self.current_dir, name),
            ds2_save_file_core::SAVE_EXTENSION,
        );
        if self.is_current_container(&path) {
            return self.refuse(PickRejection::DestinationIsLive);
        }
        if path.is_dir() {
            return self.refuse(PickRejection::NameIsFolder);
        }
        self.clear_status_message();
        if path.is_file() {
            // Typed the name of a file that is already here: the same question as picking its row.
            self.edit = None;
            if let Some(row) = self.new_file_row() {
                self.cursor = row;
            }
            return self.ask_overwrite(path);
        }
        self.edit = None;
        PickerActivation::PickedDestination {
            path,
            overwrite: false,
        }
    }
}
