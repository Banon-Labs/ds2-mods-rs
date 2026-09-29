//! Everything the panel draws, in one snapshot.
//!
//! [`SavePickerModel::view`] is the other half of [`SavePickerModel::apply`]: the panel draws what
//! this returns and routes presses through that, and holds no picker state of its own. Every
//! string in here is final -- the panel lays it out and does not compose, translate or decide.
//!
//! It takes `&mut self` for one reason: a file row's summary is read the first time the row is
//! shown and cached (see `crate::summary`), so the first view of a folder does the reading and
//! every later one is free.

use super::{DriveCellKind, EditTarget, PickerRow, SavePickerModel, Stage};
use crate::reason::PickerStatusMessage;
use crate::summary::format_modified;
use crate::text::row_text;

/// Which stage the snapshot is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerStage {
    /// Browsing folders.
    Files,
    /// Choosing one of a container's characters.
    Characters,
    /// Asking before overwriting a file.
    ConfirmOverwrite,
}

/// What kind of thing a row is, so the panel can colour or iconify it without parsing text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// The drive strip; its cells are in [`PickerView::drives`] and its field in
    /// [`PickerView::path_field`].
    DriveStrip,
    /// `[..]`.
    Parent,
    /// The destination's `[ new ]`.
    NewFile,
    /// `[ root ]`, the dead end.
    AtRoot,
    /// A folder.
    Dir,
    /// A save file or archive.
    File,
    /// A loadable character slot.
    Character,
    /// A character slot with nothing loadable in it. Drawn, never selectable.
    EmptySlot,
    /// `[ back ]`.
    Back,
    /// Yes, overwrite.
    Overwrite,
    /// No, keep the file.
    KeepFile,
}

/// One row to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowView {
    /// The index [`super::PickerInput::ClickRow`] takes for this row.
    pub row: usize,
    /// What it is.
    pub kind: RowKind,
    /// What it says.
    pub text: String,
    /// Whether the cursor is on it.
    pub highlighted: bool,
    /// Whether the cursor can be on it.
    pub selectable: bool,
    /// Whether it is the container the game is playing from; the text already carries the `*`.
    pub current: bool,
    /// A save file's summary line (`Vendrick SL136 / Lucatiel SL40 +2`). `None` for any other
    /// row, and for an archive.
    pub detail: Option<String>,
    /// A file's last-written time, `YYYY-MM-DD HH:MM` UTC.
    pub modified: Option<String>,
}

/// One cell of the drive strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveCellView {
    /// What pressing it does.
    pub kind: DriveCellKind,
    /// `C:`, `[<]`, `[>]`.
    pub label: String,
    /// The drive the current folder is on.
    pub current: bool,
    /// Where the strip's focus is, when the cursor is on the strip row.
    pub focused: bool,
}

/// The text field: the path beside the strip, or a new file's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldView {
    /// What the field is for.
    pub target: EditTarget,
    /// What it shows: the typed text while editing, the current folder otherwise.
    pub text: String,
    /// The completion to draw dimmed after the text, while editing a path.
    pub ghost: Option<String>,
    /// Whether keystrokes are going into it.
    pub editing: bool,
    /// Whether the strip's focus is on it.
    pub focused: bool,
    /// Whether Ctrl+A selected the whole text, so the next keystroke replaces it.
    pub selected: bool,
}

/// The whole panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerView {
    /// Why the picker is open.
    pub title: String,
    /// Where it is: the current folder, or the file the stage is about.
    pub subtitle: String,
    /// Which stage.
    pub stage: PickerStage,
    /// The strip's cells, empty when no drives were given or outside the file stage.
    pub drives: Vec<DriveCellView>,
    /// The path field, in the file stage.
    pub path_field: Option<FieldView>,
    /// The new file's name, while it is being typed.
    pub name_field: Option<FieldView>,
    /// The rows that hold something, in order.
    pub rows: Vec<RowView>,
    /// A headline and one or two detail lines, when there is something to say.
    pub status: Option<PickerStatusMessage>,
    /// The key help for the bottom of the panel: one button and one verb each.
    pub hint: &'static [Hint],
}

/// Which control a hint names. The panel shows the button for the device the player is on, so the
/// model names the job, not the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKey {
    /// Enter, or the pad's A.
    Confirm,
    /// Backspace, or the pad's B.
    Back,
    /// Left and Right, which page the list and move along the drive strip.
    LeftRight,
    /// Tab, or the pad's Y.
    Tab,
}

/// One entry of the key help: a control and what it does, in a word or two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hint {
    /// The control.
    pub key: HintKey,
    /// What it does here.
    pub verb: &'static str,
}

const fn hint(key: HintKey, verb: &'static str) -> Hint {
    Hint { key, verb }
}

impl SavePickerModel {
    /// Everything the panel draws right now.
    pub fn view(&mut self) -> PickerView {
        let stage = match self.stage {
            Stage::Files => PickerStage::Files,
            Stage::Characters { .. } => PickerStage::Characters,
            Stage::ConfirmOverwrite { .. } => PickerStage::ConfirmOverwrite,
        };
        let subtitle = match &self.stage {
            Stage::Files => self.current_dir.to_string_lossy().into_owned(),
            Stage::Characters { path, .. } | Stage::ConfirmOverwrite { path, .. } => {
                path.to_string_lossy().into_owned()
            }
        };
        let rows = (0..self.visible_row_count())
            .filter_map(|row| self.row_view(row))
            .collect();
        PickerView {
            title: self.open_reason().title().to_owned(),
            subtitle,
            stage,
            drives: self.drive_cell_views(),
            path_field: self.path_field_view(),
            name_field: self.name_field_view(),
            rows,
            status: self.status_view(),
            hint: self.hint(),
        }
    }

    fn row_view(&mut self, row: usize) -> Option<RowView> {
        let meaning = self.row_meaning(row);
        let kind = match &meaning {
            PickerRow::DriveStrip => RowKind::DriveStrip,
            PickerRow::ParentDir => RowKind::Parent,
            PickerRow::NewFile => RowKind::NewFile,
            PickerRow::AtRoot => RowKind::AtRoot,
            PickerRow::Dir(_) => RowKind::Dir,
            PickerRow::File(_) => RowKind::File,
            PickerRow::Character(_) if self.row_selectable(row) => RowKind::Character,
            PickerRow::Character(_) => RowKind::EmptySlot,
            PickerRow::Back => RowKind::Back,
            PickerRow::Overwrite(_) => RowKind::Overwrite,
            PickerRow::KeepFile => RowKind::KeepFile,
            PickerRow::Empty => return None,
        };
        let (current, detail, modified) = match &meaning {
            PickerRow::File(path) => {
                let modified = self.row_entry(row).and_then(|entry| match entry {
                    super::PickerEntry::File { modified, .. } => modified.and_then(format_modified),
                    super::PickerEntry::Dir { .. } => None,
                });
                let current = self.is_current_container(path);
                let detail = self.file_summary(path).map(|summary| summary.text());
                (current, detail, modified)
            }
            _ => (false, None, None),
        };
        Some(RowView {
            row,
            kind,
            text: row_text(self, row),
            highlighted: row == self.cursor,
            selectable: self.row_selectable(row),
            current,
            detail,
            modified,
        })
    }

    fn drive_cell_views(&self) -> Vec<DriveCellView> {
        if !self.has_drive_row() {
            return Vec::new();
        }
        let current = self.current_drive();
        let strip_focused = self.drive_row() == Some(self.cursor) && !self.path_focused;
        self.drive_strip_cells()
            .into_iter()
            .map(|kind| match kind {
                DriveCellKind::Drive(index) => DriveCellView {
                    kind,
                    label: self.drive_label(index).unwrap_or_default(),
                    current: current == Some(index),
                    focused: strip_focused && current == Some(index),
                },
                DriveCellKind::MoreLeft => DriveCellView {
                    kind,
                    label: "[<]".to_owned(),
                    current: false,
                    focused: false,
                },
                DriveCellKind::MoreRight => DriveCellView {
                    kind,
                    label: "[>]".to_owned(),
                    current: false,
                    focused: false,
                },
            })
            .collect()
    }

    fn path_field_view(&self) -> Option<FieldView> {
        if !self.in_files() {
            return None;
        }
        let editing = self.editing() == Some(EditTarget::Path);
        let text = match (&self.edit, editing) {
            (Some(edit), true) => edit.text.clone(),
            _ => self.current_dir.to_string_lossy().into_owned(),
        };
        Some(FieldView {
            target: EditTarget::Path,
            text,
            ghost: if editing { self.ghost_suffix() } else { None },
            editing,
            focused: editing || (self.path_focused && self.drive_row() == Some(self.cursor)),
            selected: editing && self.all_selected(),
        })
    }

    fn name_field_view(&self) -> Option<FieldView> {
        let edit = self.edit.as_ref()?;
        (edit.target == EditTarget::FileName).then(|| FieldView {
            target: EditTarget::FileName,
            text: edit.text.clone(),
            ghost: None,
            editing: true,
            focused: true,
            selected: edit.all_selected,
        })
    }

    /// The banner: a refusal when there is one, the overwrite question while it is asked.
    fn status_view(&self) -> Option<PickerStatusMessage> {
        if let Some(message) = &self.status_message {
            return Some(message.clone());
        }
        let Stage::ConfirmOverwrite { path, .. } = &self.stage else {
            return None;
        };
        let name = crate::path::leaf(path).unwrap_or_default();
        Some(
            PickerStatusMessage::new(
                "Overwrite This Save?",
                format!("{name} is already there. Its characters are replaced by yours."),
            )
            .with_second_detail(path.to_string_lossy()),
        )
    }

    /// The key help for where the player is.
    pub fn hint(&self) -> &'static [Hint] {
        use HintKey::{Back, Confirm, LeftRight, Tab};
        const EDIT_PATH: &[Hint] = &[
            hint(Tab, "Complete"),
            hint(Confirm, "Go"),
            hint(Back, "Cancel"),
        ];
        const EDIT_NAME: &[Hint] = &[hint(Confirm, "Save"), hint(Back, "Cancel")];
        const CHARACTERS: &[Hint] = &[hint(Confirm, "Load"), hint(Back, "Files")];
        const OVERWRITE: &[Hint] = &[hint(Confirm, "Answer"), hint(Back, "Keep File")];
        const DRIVE_PATH: &[Hint] = &[
            hint(Confirm, "Type Path"),
            hint(LeftRight, "Drive"),
            hint(Back, "Up"),
        ];
        const DRIVE: &[Hint] = &[
            hint(LeftRight, "Drive"),
            hint(Confirm, "Open"),
            hint(Back, "Up"),
        ];
        const DESTINATION: &[Hint] = &[
            hint(Confirm, "Choose"),
            hint(Back, "Up"),
            hint(LeftRight, "Page"),
            hint(Tab, "Type Path"),
        ];
        const FILES: &[Hint] = &[
            hint(Confirm, "Open"),
            hint(Back, "Up"),
            hint(LeftRight, "Page"),
            hint(Tab, "Type Path"),
        ];
        match (self.editing(), &self.stage) {
            (Some(EditTarget::Path), _) => EDIT_PATH,
            (Some(EditTarget::FileName), _) => EDIT_NAME,
            (None, Stage::Characters { .. }) => CHARACTERS,
            (None, Stage::ConfirmOverwrite { .. }) => OVERWRITE,
            (None, Stage::Files) if self.drive_row() == Some(self.cursor) => {
                if self.path_focused {
                    DRIVE_PATH
                } else {
                    DRIVE
                }
            }
            (None, Stage::Files) if self.is_destination() => DESTINATION,
            (None, Stage::Files) => FILES,
        }
    }
}
