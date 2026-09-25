//! The picker's rows: a directory listing, then the container's ten characters.
//!
//! # Two stages, because a save file is not a character
//!
//! Today's Load Character from File row opens a common file dialog and swaps the WHOLE container.
//! That is not choosing a character; it is choosing which ten characters the game will show you
//! next time. DARK SOULS II keeps ten of them per container, and the row a player actually wants
//! is *that* one, in *that* file.
//!
//! So the model has two stages and one cursor over both:
//!
//! | stage | rows | what activating one means |
//! |---|---|---|
//! | [`Stage::Files`] | `[..]`, then folders, then saves | walk into it, or choose it |
//! | [`Stage::Characters`] | the container's ten slots, then `[back]` | load that character |
//!
//! A pick that fails its content gate never leaves the file stage: the refusal becomes the
//! picker's status message and the listing is exactly where the player left it.
//!
//! # A character row index IS the game's slot number
//!
//! This is the one invariant worth stating twice. In the character stage, row `n` is slot `n` --
//! always, including for empty slots, which keep their row and refuse to be activated. Packing
//! the occupied slots up into a contiguous prefix would be prettier and would renumber every
//! character below the first gap, and everything downstream of the picker (the staging step, any
//! log line, anything a player counts on screen) means the GAME's slot number when it says slot.
//! The back row sits AFTER all ten for the same reason: a navigation row at the top would shift
//! every slot by one, which is exactly the offset this invariant exists to forbid.
//!
//! The file stage has no such constraint, so there the parent row does sit on top and
//! [`SavePickerModel::entry_row_base`] is the single place that shift is decided. Every entry
//! query derives from it, so a label and its row's action cannot come from different entries.
//!
//! # What is NOT here
//!
//! No drawing, no window, no dialog, no game. The model answers what a row MEANS and what
//! activating it DOES; a surface asks [`crate::text`] for something to put on screen. Nothing in
//! this file opens a handle to anything except a directory it was asked to list and a file the
//! player just chose.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ds2_sl2_core::{SaveSlot, slots::SLOT_COUNT};

use crate::path::{leaf, parent};
use crate::reason::{PickRejection, PickedSource, PickerStatusMessage, accept_slots, accepts_pick};

/// Rows a surface is assumed to be able to draw until it says otherwise.
///
/// A starting value, not a fact about any menu: the surface knows its own geometry and tells the
/// model with [`SavePickerModel::set_row_capacity`].
pub const DEFAULT_ROW_CAPACITY: usize = 10;

/// Row index of the character stage's back row: directly after the last slot, never before the
/// first. See the module docs on why it cannot go on top.
pub const CHARACTER_BACK_ROW: usize = SLOT_COUNT;

/// One thing in the current directory that the picker will show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerEntry {
    /// A subdirectory. Always listed -- a folder is where the next save lives.
    Dir {
        /// The leaf, as the row shows it.
        name: String,
        /// Where descending goes.
        path: PathBuf,
    },
    /// A file whose extension the cheap gate accepted. Whether it is a save it can read is not
    /// decided here; that costs a decrypt per file and is decided when one is picked.
    File {
        /// The leaf, as the row shows it.
        name: String,
        /// What gets staged if this row is picked.
        path: PathBuf,
        /// When it was last written, for the newest-first order. `None` when the listing build
        /// could not read the metadata, which sorts it to the bottom rather than dropping it.
        modified: Option<SystemTime>,
    },
}

impl PickerEntry {
    /// The leaf shown on the row, whichever variant this is.
    pub fn name(&self) -> &str {
        match self {
            PickerEntry::Dir { name, .. } | PickerEntry::File { name, .. } => name,
        }
    }

    /// Where the row points, whichever variant this is.
    pub fn path(&self) -> &Path {
        match self {
            PickerEntry::Dir { path, .. } | PickerEntry::File { path, .. } => path,
        }
    }
}

/// What a row means right now. Produced by [`SavePickerModel::row_meaning`]; a surface renders
/// from it and routes a press through it, so the two cannot disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerRow {
    /// Go up one directory. Exists only while there is somewhere to go.
    ParentDir,
    /// Nothing above, nothing to list. Names the dead end instead of leaving a listing with no
    /// rows at all. Activation does nothing.
    AtRoot,
    /// Open this subdirectory.
    Dir(PathBuf),
    /// Choose this container.
    File(PathBuf),
    /// Character stage: this slot of the picked container. The index IS the game's slot number.
    Character(usize),
    /// Character stage: back to the directory listing.
    Back,
    /// Past the end of the listing. Not drawn, not selectable, activation does nothing.
    Empty,
}

/// What activating a row did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerActivation {
    /// The whole file is the answer: an archive, whose characters nothing can read until the
    /// staging step unwraps it.
    PickedFile(PathBuf),
    /// A character in a container. `slot` is the game's own slot number.
    PickedCharacter {
        /// The container the character was read out of.
        path: PathBuf,
        /// Which slot in it, as the game numbers them.
        slot: usize,
    },
    /// The rows changed -- new directory, new stage, new scroll window. Re-render.
    Repopulate,
    /// Nothing happened. A refused pick leaves its reason in
    /// [`SavePickerModel::status_message`].
    Ignored,
}

/// Which set of rows the picker is showing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Files,
    Characters {
        path: PathBuf,
        /// All ten slots as the container reported them, empties included and in slot order.
        slots: Vec<SaveSlot>,
    },
}

/// The picker's whole state: where it is browsing, what it found, and where the cursor is.
#[derive(Clone, Debug)]
pub struct SavePickerModel {
    current_dir: PathBuf,
    /// Directories first by name, then files newest-first.
    entries: Vec<PickerEntry>,
    /// First entry visible in the file stage's scroll window.
    scroll_offset: usize,
    /// Highlighted row. Kept on a selectable row by every listing change.
    cursor: usize,
    /// Rows the surface says it can draw at once.
    row_capacity: usize,
    /// The last refusal, for the surface to render. Cleared by any navigation, so a stale error
    /// never follows the player into another folder.
    status_message: Option<PickerStatusMessage>,
    stage: Stage,
}

impl SavePickerModel {
    /// Open a picker browsing `dir`.
    pub fn open(dir: &Path) -> Self {
        let mut model = SavePickerModel {
            current_dir: dir.to_path_buf(),
            entries: Vec::new(),
            scroll_offset: 0,
            cursor: 0,
            row_capacity: DEFAULT_ROW_CAPACITY,
            status_message: None,
            stage: Stage::Files,
        };
        model.refresh();
        model.cursor = model.first_selectable_row();
        model
    }

    // -------------------------------------------------------------------------------------
    // Where we are.
    // -------------------------------------------------------------------------------------

    /// The directory whose contents the rows are currently listing.
    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    /// True while the picker is showing a container's characters rather than a directory.
    pub fn choosing_character(&self) -> bool {
        matches!(self.stage, Stage::Characters { .. })
    }

    /// The container the character stage is showing, or `None` in the file stage.
    pub fn picked_container(&self) -> Option<&Path> {
        match &self.stage {
            Stage::Characters { path, .. } => Some(path.as_path()),
            Stage::Files => None,
        }
    }

    /// The picked container's slots, empties included and in slot order. Empty in the file stage.
    pub fn character_slots(&self) -> &[SaveSlot] {
        match &self.stage {
            Stage::Characters { slots, .. } => slots,
            Stage::Files => &[],
        }
    }

    /// One slot of the picked container, addressed by the game's slot number.
    pub fn character_slot(&self, slot: usize) -> Option<&SaveSlot> {
        self.character_slots().get(slot)
    }

    /// Everything the current directory offers, in the order the rows show it.
    pub fn entries(&self) -> &[PickerEntry] {
        &self.entries
    }

    /// How many of those there are -- not the row count, which adds the back row.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    // -------------------------------------------------------------------------------------
    // Row layout.
    // -------------------------------------------------------------------------------------

    /// True when the current directory has somewhere to go up to.
    fn has_parent_row(&self) -> bool {
        matches!(self.stage, Stage::Files) && parent(&self.current_dir).is_some()
    }

    /// Row index of the `[..]` row, when there is one. Always row 0 -- nothing sits above it.
    pub fn parent_row(&self) -> Option<usize> {
        self.has_parent_row().then_some(0)
    }

    /// Row index of the first directory/file entry: the single place the file stage's row shift
    /// is decided.
    pub fn entry_row_base(&self) -> usize {
        usize::from(self.has_parent_row())
    }

    /// Rows this stage can address at all, whether or not they hold anything.
    ///
    /// The file stage is bounded by what the surface can draw. The character stage is NOT: ten
    /// slots plus a back row is a short, fixed list, and a row index that means the game's slot
    /// number cannot also be a window offset. A surface with fewer rows than that must page its
    /// own drawing; the model will not renumber the slots to fit.
    fn addressable_rows(&self) -> usize {
        match self.stage {
            Stage::Files => self.row_capacity,
            Stage::Characters { .. } => CHARACTER_BACK_ROW + 1,
        }
    }

    /// Rows that actually hold something. A surface draws this many and no more.
    pub fn visible_row_count(&self) -> usize {
        match self.stage {
            // Never zero: an empty root still shows the `[ root ]` dead-end marker.
            Stage::Files => (self.entry_row_base() + self.window_entries().len()).max(1),
            Stage::Characters { .. } => CHARACTER_BACK_ROW + 1,
        }
    }

    /// Entries that fit below the fixed rows.
    fn entry_window_capacity(&self) -> usize {
        self.row_capacity
            .saturating_sub(self.entry_row_base())
            .max(1)
    }

    fn max_scroll_offset(&self) -> usize {
        match self.stage {
            Stage::Files => self
                .entries
                .len()
                .saturating_sub(self.entry_window_capacity()),
            Stage::Characters { .. } => 0,
        }
    }

    fn window_entries(&self) -> &[PickerEntry] {
        let start = self.scroll_offset.min(self.entries.len());
        let end = (start + self.entry_window_capacity()).min(self.entries.len());
        self.entries.get(start..end).unwrap_or(&[])
    }

    /// How many rows the surface said it can draw.
    pub fn row_capacity(&self) -> usize {
        self.row_capacity
    }

    /// Tell the model how many rows the surface can draw, and re-clamp what depends on it.
    ///
    /// A no-op when nothing changed, so a surface may call it every frame.
    pub fn set_row_capacity(&mut self, rows: usize) {
        let rows = rows.max(1);
        if rows == self.row_capacity {
            return;
        }
        self.row_capacity = rows;
        self.scroll_offset = self.scroll_offset.min(self.max_scroll_offset());
        if !self.row_selectable(self.cursor) || self.cursor >= self.addressable_rows() {
            self.cursor = self.first_selectable_row();
        }
    }

    /// What `row` means in the stage and window the picker is in right now.
    pub fn row_meaning(&self, row: usize) -> PickerRow {
        match &self.stage {
            Stage::Characters { slots, .. } => {
                if row == CHARACTER_BACK_ROW {
                    return PickerRow::Back;
                }
                if row > CHARACTER_BACK_ROW || slots.get(row).is_none() {
                    return PickerRow::Empty;
                }
                PickerRow::Character(row)
            }
            Stage::Files => {
                if row >= self.row_capacity {
                    return PickerRow::Empty;
                }
                if self.parent_row() == Some(row) {
                    return PickerRow::ParentDir;
                }
                match row
                    .checked_sub(self.entry_row_base())
                    .and_then(|index| self.window_entries().get(index))
                {
                    Some(PickerEntry::Dir { path, .. }) => PickerRow::Dir(path.clone()),
                    Some(PickerEntry::File { path, .. }) => PickerRow::File(path.clone()),
                    // Nothing above and nothing to list: name the dead end rather than show a
                    // listing with no rows in it.
                    None if row == 0 => PickerRow::AtRoot,
                    None => PickerRow::Empty,
                }
            }
        }
    }

    /// True when `row` can hold the cursor and be pressed.
    ///
    /// An empty character slot is NOT selectable and still occupies its row: the cursor steps
    /// over it, the surface still draws it, and its index still means what the game means.
    pub fn row_selectable(&self, row: usize) -> bool {
        match self.row_meaning(row) {
            PickerRow::Empty => false,
            PickerRow::Character(slot) => self
                .character_slot(slot)
                .is_some_and(|slot| slot.state.is_loadable()),
            _ => true,
        }
    }

    /// Where a fresh listing puts the cursor: the first row worth pressing.
    ///
    /// Entries are preferred over navigation rows, so a folder full of saves lands the cursor on
    /// a save rather than on `[..]`. A stage with nothing selectable at all falls back to row 0,
    /// which is the `[ root ]` marker in the file stage and the back row when a container turned
    /// out to hold ten empty slots.
    fn first_selectable_row(&self) -> usize {
        let rows = self.addressable_rows();
        (self.entry_row_base()..rows)
            .find(|&row| self.row_selectable(row))
            .or_else(|| (0..rows).find(|&row| self.row_selectable(row)))
            .unwrap_or(0)
    }

    // -------------------------------------------------------------------------------------
    // Cursor and scrolling.
    // -------------------------------------------------------------------------------------

    /// The row the highlight is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Put the cursor on a specific row. Ignored for a row that cannot hold it, so a click on an
    /// empty slot does not leave the highlight somewhere the player cannot press.
    pub fn set_cursor(&mut self, row: usize) {
        if row < self.addressable_rows() && self.row_selectable(row) {
            self.cursor = row;
        }
    }

    /// How far the entry window has been scrolled.
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// The furthest that offset can go before the last entry is on screen.
    pub fn scroll_max(&self) -> usize {
        self.max_scroll_offset()
    }

    /// Move the file stage's scroll window by one entry. Returns whether it actually moved.
    pub fn scroll_window_one(&mut self, down: bool) -> bool {
        let before = self.scroll_offset;
        self.scroll_offset = if down {
            (self.scroll_offset + 1).min(self.max_scroll_offset())
        } else {
            self.scroll_offset.saturating_sub(1)
        };
        let moved = self.scroll_offset != before;
        if moved {
            self.clear_status_message();
        }
        moved
    }

    /// Move the highlight one selectable row, wrapping.
    ///
    /// A step off either end of the entry window SCROLLS instead of moving: the cursor stays on
    /// the edge row and the listing slides under it, which is what makes a directory with more
    /// saves than rows navigable with nothing but up and down. Only when the window cannot move
    /// any further does the press become ordinary movement, and only then does it wrap.
    pub fn move_cursor(&mut self, down: bool) {
        let base = self.entry_row_base();
        let last_visible = self
            .visible_row_count()
            .saturating_sub(1)
            .min(self.addressable_rows().saturating_sub(1));
        let at_window_edge = if down {
            self.cursor >= last_visible
        } else {
            self.cursor == base
        };
        if at_window_edge && self.scroll_window_one(down) {
            // The window moved under a cursor that stayed put. If what slid under it cannot hold
            // the cursor, fall through to ordinary movement rather than stranding it.
            if self.row_selectable(self.cursor) {
                return;
            }
        }
        let selectable: Vec<usize> = (0..self.addressable_rows())
            .filter(|&row| self.row_selectable(row))
            .collect();
        if selectable.len() < 2 {
            self.cursor = selectable.first().copied().unwrap_or(0);
            return;
        }
        let at = selectable
            .iter()
            .position(|&row| row == self.cursor)
            .unwrap_or(0);
        let next = if down {
            (at + 1) % selectable.len()
        } else {
            (at + selectable.len() - 1) % selectable.len()
        };
        self.cursor = selectable[next];
    }

    // -------------------------------------------------------------------------------------
    // Activation.
    // -------------------------------------------------------------------------------------

    /// Do whatever pressing `row` means.
    pub fn activate(&mut self, row: usize) -> PickerActivation {
        let meaning = self.row_meaning(row);
        if !matches!(meaning, PickerRow::AtRoot | PickerRow::Empty) {
            self.clear_status_message();
        }
        match meaning {
            PickerRow::ParentDir => match parent(&self.current_dir) {
                Some(up) => self.browse(up),
                None => PickerActivation::Ignored,
            },
            PickerRow::Dir(path) => self.browse(path),
            PickerRow::File(path) => self.pick_file(path),
            PickerRow::Character(slot) => self.pick_character(slot),
            PickerRow::Back => {
                self.stage = Stage::Files;
                self.refresh();
                self.cursor = self.first_selectable_row();
                PickerActivation::Repopulate
            }
            PickerRow::AtRoot | PickerRow::Empty => PickerActivation::Ignored,
        }
    }

    /// Press the highlighted row.
    pub fn activate_cursor(&mut self) -> PickerActivation {
        self.activate(self.cursor)
    }

    fn browse(&mut self, dir: PathBuf) -> PickerActivation {
        self.current_dir = dir;
        self.stage = Stage::Files;
        self.refresh();
        self.cursor = self.first_selectable_row();
        PickerActivation::Repopulate
    }

    /// Show the characters of a container whose slots have already been read.
    ///
    /// The stage transition and its refusal live here, apart from the reading, for two reasons: a
    /// caller that already holds the bytes should not have to hand back a path and have them read
    /// again, and a container is expensive to fake -- `ds2-sl2-core` keeps its key and its cipher
    /// private, correctly -- so this is where the transition can be exercised against slots a
    /// test chose.
    ///
    /// Re-runs [`crate::reason::accept_slots`] even when [`accepts_pick`] just did. It is one
    /// pass over ten records, and one place deciding "these slots are worth showing" is worth
    /// more than the pass it costs.
    ///
    /// # Errors
    ///
    /// The [`PickRejection`] [`crate::reason::accept_slots`] produced, when the container holds
    /// nothing worth showing. The picker stays in the file stage.
    pub fn show_characters(
        &mut self,
        path: &Path,
        slots: Vec<SaveSlot>,
    ) -> Result<(), PickRejection> {
        let slots = accept_slots(slots)?;
        self.stage = Stage::Characters {
            path: path.to_path_buf(),
            slots,
        };
        self.scroll_offset = 0;
        self.cursor = self.first_selectable_row();
        Ok(())
    }

    /// A file was chosen. An archive ends the pick; a container opens the character stage; a
    /// refusal leaves the listing alone and says why.
    fn pick_file(&mut self, path: PathBuf) -> PickerActivation {
        match accepts_pick(&path) {
            Ok(PickedSource::Container(slots)) => match self.show_characters(&path, slots) {
                Ok(()) => PickerActivation::Repopulate,
                Err(rejection) => self.refuse(rejection),
            },
            Ok(PickedSource::Archive) => PickerActivation::PickedFile(path),
            Err(rejection) => self.refuse(rejection),
        }
    }

    fn pick_character(&mut self, slot: usize) -> PickerActivation {
        let Stage::Characters { path, slots } = &self.stage else {
            return PickerActivation::Ignored;
        };
        match slots.get(slot) {
            Some(found) if found.state.is_loadable() => PickerActivation::PickedCharacter {
                path: path.clone(),
                slot,
            },
            // An empty slot keeps its row and refuses the press. Saying so is kinder than a row
            // that silently does nothing when pressed.
            _ => self.refuse(PickRejection::NoLoadableCharacter),
        }
    }

    fn refuse(&mut self, rejection: PickRejection) -> PickerActivation {
        self.status_message = Some(rejection.status_message());
        PickerActivation::Ignored
    }

    // -------------------------------------------------------------------------------------
    // Status and listing.
    // -------------------------------------------------------------------------------------

    /// Whatever the last refused pick left behind, if it has not been cleared.
    pub fn status_message(&self) -> Option<&PickerStatusMessage> {
        self.status_message.as_ref()
    }

    /// Replace it.
    pub fn set_status_message(&mut self, message: PickerStatusMessage) {
        self.status_message = Some(message);
    }

    /// Drop it, so the next render shows nothing.
    pub fn clear_status_message(&mut self) {
        self.status_message = None;
    }

    /// Re-read the current directory.
    ///
    /// An unreadable directory yields an EMPTY listing rather than an error: the picker stays
    /// navigable -- the player can still go up -- where a hard failure would strand them in a
    /// folder with no way out.
    ///
    /// Only the cheap gate runs here. A file is listed because its extension says it might be a
    /// save; whether it IS one costs a decrypt, and paying that for every file in a folder to
    /// hide a broken one would also HIDE it, leaving a player who downloaded a bad save staring
    /// at a folder that does not contain the file they can plainly see in it. The expensive gate
    /// runs on the pick, where its refusal can be read.
    pub fn refresh(&mut self) {
        self.entries.clear();
        self.scroll_offset = 0;
        let Ok(read) = std::fs::read_dir(&self.current_dir) else {
            return;
        };
        let mut dirs: Vec<PickerEntry> = Vec::new();
        let mut files: Vec<PickerEntry> = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            // A name that does not round-trip through UTF-8 is a name nothing downstream could
            // carry, so it is not offered rather than offered and then refused.
            let Some(name) = leaf(&path).map(str::to_owned) else {
                continue;
            };
            // Dot-prefixed entries are the host's business, not the player's.
            if name.starts_with('.') {
                continue;
            }
            // STAT the target rather than trusting the dirent's type: under Wine a symlinked or
            // subvolume directory comes back as something other than a directory, and dropping
            // those would hide most of `Z:\`.
            if path.is_dir() {
                dirs.push(PickerEntry::Dir { name, path });
                continue;
            }
            if ds2_save_file_core::accepts(&path).is_err() {
                continue;
            }
            files.push(PickerEntry::File {
                name,
                modified: entry.metadata().ok().and_then(|meta| meta.modified().ok()),
                path,
            });
        }
        self.entries = order_entries(dirs, files);
    }
}

/// Directories first by name, then saves newest-first.
///
/// Newest-first because the save a player is looking for is overwhelmingly the one they just
/// touched -- the one they downloaded, or the one the game wrote last. Ties and unreadable
/// timestamps fall back to the name, so the order is total and a listing cannot shuffle between
/// two refreshes of the same unchanged folder.
///
/// Pure, and separate from [`SavePickerModel::refresh`], so the order can be asserted against
/// timestamps a test chose rather than timestamps a filesystem happened to produce.
pub fn order_entries(mut dirs: Vec<PickerEntry>, mut files: Vec<PickerEntry>) -> Vec<PickerEntry> {
    dirs.sort_by(|left, right| {
        left.name()
            .to_ascii_lowercase()
            .cmp(&right.name().to_ascii_lowercase())
    });
    files.sort_by(|left, right| {
        let modified = |entry: &PickerEntry| match entry {
            PickerEntry::File { modified, .. } => *modified,
            PickerEntry::Dir { .. } => None,
        };
        modified(right).cmp(&modified(left)).then_with(|| {
            left.name()
                .to_ascii_lowercase()
                .cmp(&right.name().to_ascii_lowercase())
        })
    });
    dirs.append(&mut files);
    dirs
}

#[cfg(test)]
mod tests;
