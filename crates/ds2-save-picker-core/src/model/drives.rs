//! The drive strip: which drives there are, which one the picker is on, and the path field that
//! closes the ring.
//!
//! # The drives are handed in
//!
//! The Windows panel probes `A:` to `Z:` and passes the roots that answered to
//! [`SavePickerModel::set_drives`]. Nothing here asks the host, because the host this is tested on
//! has no drives, and a probe that only runs in the game is a probe nothing tests. A test hands in
//! whatever roots it likes -- including real scratch folders, since a "drive" here is just a root
//! the current folder may lie under.
//!
//! # One ring, left and right
//!
//! On the strip row, left and right step along `[C:] [S:] [Z:] [ path ]` and wrap at both ends.
//! Stepping onto a drive switches to it at once and lands in the folder last browsed there, so
//! hopping between a save folder on one drive and one on another costs two presses instead of two
//! walks. `er-save-picker-core` wrapped only to the left for a while, and a strip that dead-ends
//! one press to the right looks exactly like a strip that is broken.
//!
//! # A window of at most seven cells
//!
//! Past [`DRIVE_STRIP_MAX_CELLS`] drives the strip shows a window with `[<]` and `[>]` at the ends.
//! The window is recomputed from the current drive on every view rather than scrolled as state,
//! so it cannot drift away from the drive it is supposed to be showing.

use std::path::PathBuf;

use super::{PickerActivation, SavePickerModel};
use crate::path::{fold, is_under};

/// The most cells the strip shows at once, the `[<]` and `[>]` markers included.
pub const DRIVE_STRIP_MAX_CELLS: usize = 7;

/// What one strip cell stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveCellKind {
    /// A drive, by its index in the list handed to [`SavePickerModel::set_drives`].
    Drive(usize),
    /// `[<]`: more drives to the left. Pressing it steps to the nearest one.
    MoreLeft,
    /// `[>]`: more drives to the right. Pressing it steps to the nearest one.
    MoreRight,
}

impl SavePickerModel {
    /// Hand the model the drive roots the surface found, in the order the strip shows them.
    ///
    /// An empty list removes the strip row. The cursor is re-placed because the rows under it
    /// moved by one.
    pub fn set_drives(&mut self, drives: Vec<String>) {
        if drives == self.drives {
            return;
        }
        self.drives = drives;
        self.path_focused = false;
        self.remember_dir();
        self.reclamp();
        self.cursor = self.first_selectable_row();
    }

    /// The drive roots the strip shows.
    pub fn drives(&self) -> &[String] {
        &self.drives
    }

    /// Index of the drive the current folder lies on, when it lies on one of them.
    ///
    /// The longest matching root wins, so a scratch root nested inside another still answers
    /// with the nearer one.
    pub fn current_drive(&self) -> Option<usize> {
        self.drive_of(&self.current_dir)
    }

    fn drive_of(&self, dir: &std::path::Path) -> Option<usize> {
        self.drives
            .iter()
            .enumerate()
            .filter(|(_, root)| is_under(dir, root))
            .max_by_key(|(_, root)| fold(root).len())
            .map(|(index, _)| index)
    }

    /// True while the strip's focus sits on the path field rather than on a drive.
    pub fn path_field_focused(&self) -> bool {
        self.path_focused
    }

    /// The folder last browsed on drive `index`, if any was.
    pub fn remembered_dir(&self, index: usize) -> Option<&std::path::Path> {
        let root = self.drives.get(index)?;
        self.last_dir_per_drive
            .get(&fold(root))
            .map(PathBuf::as_path)
    }

    /// Record the current folder as the last one browsed on its drive.
    pub(super) fn remember_dir(&mut self) {
        if let Some(index) = self.current_drive() {
            let key = fold(&self.drives[index]);
            self.last_dir_per_drive
                .insert(key, self.current_dir.clone());
        }
    }

    /// Step one place along the ring of drives and the path field. See the module docs.
    pub fn step_drive_strip(&mut self, right: bool) -> PickerActivation {
        let count = self.drives.len();
        if count == 0 || !self.in_files() {
            return PickerActivation::Ignored;
        }
        let ring = count + 1;
        let at = if self.path_focused {
            count
        } else {
            self.current_drive().unwrap_or(count)
        };
        let next = if right {
            (at + 1) % ring
        } else {
            (at + ring - 1) % ring
        };
        self.clear_status_message();
        if next == count {
            self.path_focused = true;
        } else {
            self.switch_to_drive(next);
        }
        if let Some(row) = self.drive_row() {
            self.cursor = row;
        }
        PickerActivation::Repopulate
    }

    /// Move to drive `index`, resuming the folder last browsed there.
    ///
    /// A remembered folder that has since gone falls back to the drive root rather than browsing
    /// a path that no longer resolves. Choosing the drive already current changes nothing but the
    /// focus.
    pub fn switch_to_drive(&mut self, index: usize) -> PickerActivation {
        let Some(root) = self.drives.get(index).cloned() else {
            return PickerActivation::Ignored;
        };
        self.path_focused = false;
        self.clear_status_message();
        if self.current_drive() == Some(index) {
            return PickerActivation::Repopulate;
        }
        let resume = self
            .remembered_dir(index)
            .filter(|dir| dir.is_dir())
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(&root));
        let activation = self.browse(resume);
        if let Some(row) = self.drive_row() {
            self.cursor = row;
        }
        activation
    }

    /// Enter on the strip row: the path field starts editing; a drive opens at its root.
    ///
    /// The root rather than the remembered folder, because the remembered folder is where
    /// stepping onto the drive already took the player -- a second press that did the same thing
    /// would be a press that does nothing.
    pub(super) fn activate_drive_strip(&mut self) -> PickerActivation {
        match (self.path_focused, self.current_drive()) {
            (false, Some(index)) => {
                let root = PathBuf::from(&self.drives[index]);
                let activation = self.browse(root);
                if let Some(row) = self.drive_row() {
                    self.cursor = row;
                }
                activation
            }
            _ => self.begin_path_edit(),
        }
    }

    /// The strip's visible cells, in drawing order.
    pub fn drive_strip_cells(&self) -> Vec<DriveCellKind> {
        let count = self.drives.len();
        if count <= DRIVE_STRIP_MAX_CELLS {
            return (0..count).map(DriveCellKind::Drive).collect();
        }
        // Keep the current drive in view; with the path field focused, the end of the strip is
        // what sits next to it.
        let anchor = if self.path_focused {
            count - 1
        } else {
            self.current_drive().unwrap_or(0)
        };
        let single_marker = DRIVE_STRIP_MAX_CELLS - 1;
        let (start, len, left, right) = if anchor < single_marker {
            (0, single_marker, false, true)
        } else if anchor >= count - single_marker {
            (count - single_marker, single_marker, true, false)
        } else {
            let len = DRIVE_STRIP_MAX_CELLS - 2;
            let start = anchor.saturating_sub(len / 2).clamp(1, count - 1 - len);
            (start, len, true, true)
        };
        let mut cells = Vec::with_capacity(DRIVE_STRIP_MAX_CELLS);
        if left {
            cells.push(DriveCellKind::MoreLeft);
        }
        cells.extend((start..start + len).map(DriveCellKind::Drive));
        if right {
            cells.push(DriveCellKind::MoreRight);
        }
        cells
    }

    /// A click on strip cell `cell`, counted in [`SavePickerModel::drive_strip_cells`] order.
    pub fn click_drive_cell(&mut self, cell: usize) -> PickerActivation {
        if !self.has_drive_row() {
            return PickerActivation::Ignored;
        }
        let cells = self.drive_strip_cells();
        let target = match cells.get(cell) {
            Some(DriveCellKind::Drive(index)) => Some(*index),
            // The markers step to the first drive past the window's edge.
            Some(DriveCellKind::MoreLeft) => cells.iter().find_map(|kind| match kind {
                DriveCellKind::Drive(index) => index.checked_sub(1),
                _ => None,
            }),
            Some(DriveCellKind::MoreRight) => cells.iter().rev().find_map(|kind| match kind {
                DriveCellKind::Drive(index) => Some(index + 1),
                _ => None,
            }),
            None => None,
        };
        match target {
            Some(index) => self.switch_to_drive(index),
            None => PickerActivation::Ignored,
        }
    }

    /// The label a drive cell shows: the root without its separator, `C:` for `C:\`.
    pub fn drive_label(&self, index: usize) -> Option<String> {
        let root = self.drives.get(index)?;
        let trimmed = root.trim_end_matches(['\\', '/']);
        Some(if trimmed.is_empty() {
            root.clone()
        } else {
            trimmed.to_owned()
        })
    }
}
