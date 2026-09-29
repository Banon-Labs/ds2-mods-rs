//! The in-game picker: a panel over the pause menu that browses to a save and chooses a character.
//!
//! This replaces the OS file dialog both rows used to open, with the feature set of
//! `../er-mods-rs`'s in-game picker: a drive strip, folders and saves newest first, a typed path
//! with folder completion, each save's characters and levels on its row, a character stage that
//! loads the one chosen, the reason any pick was refused, and for Save Game to File a `[ new ]` row
//! and an overwrite question. Every one of those decisions is `ds2-save-picker-core`'s and is tested
//! on the host; this module draws what that model says and routes presses into it.
//!
//! # Three threads of control, one lock
//!
//! | who | when | what it does here |
//! |---|---|---|
//! | the row's press | the pause menu's confirm | [`open_for_load`] / [`open_for_save`]: build the model, take the hold |
//! | [`on_frame`] | `ds2-overlay`'s clock, every `Present` | read keys and pad, apply them, release the hold after closing |
//! | [`draw`] | `ds2-overlay`'s panel table, the same `Present` | draw, and turn mouse clicks into presses |
//! | [`take_outcome`] | the pause menu's tick | hand a pick to `import::accept` or `export::commit` |
//!
//! The pick is handed on rather than acted on because the flow it starts -- returning to the title,
//! asking the game to save -- has always run from the pause menu's own update, and it keeps doing
//! so. DARK SOULS II presents from its simulation thread, so all four run on one thread and the
//! lock is never contended; it is a `Mutex` because Rust cannot know that.
//!
//! # The pause menu stays still underneath
//!
//! While the panel is up, `ds2_input_harness::hold` blanks every device the game reads and
//! `ds2-overlay` keeps keyboard and mouse window messages from the game window. After the panel
//! closes the hold stays until every key and button is released, so the Enter or B that closed it
//! does not also press the pause menu.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ds2_overlay::style;
use ds2_save_picker_core::{
    PickerActivation, PickerInput, PickerStatusMessage, PickerView, RowKind, SavePickerModel,
};
use hudhook::imgui::{MouseButton, Ui};

use crate::PickerSettings;
use crate::import::Refused;
use crate::picker_input::{Press, Reader};
use crate::{LOG_PREFIX, log_line};

/// What the panel was opened to choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Load Character from File: a container and a character in it.
    Load,
    /// Save Game to File: where to write the running character.
    Save,
}

/// Where the panel is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Taking presses.
    Open,
    /// A pick is waiting for the pause menu's tick. Frames waited so far.
    Committing(u32),
    /// Gone from the screen, holding input until every key is up. Frames waited so far.
    Closing(u32),
}

/// A pick, waiting for the pause menu's tick to act on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Load this container, and this character in it when the panel chose one.
    Load {
        /// The container or archive.
        path: PathBuf,
        /// The game's slot number, or `None` for an archive.
        slot: Option<usize>,
    },
    /// Write the running character here.
    Save {
        /// The destination.
        path: PathBuf,
    },
}

struct Panel {
    model: SavePickerModel,
    mode: Mode,
    phase: Phase,
    reader: Reader,
    /// What was last drawn, rebuilt only when a press changed something.
    view: Option<PickerView>,
    /// The pick waiting for [`take_outcome`].
    outcome: Option<Outcome>,
    /// Rows the panel's height holds, told to the model when it changes.
    capacity: usize,
}

impl Panel {
    fn dirty(&mut self) {
        self.view = None;
    }

    /// Route one press through the model and act on what it did.
    fn apply(&mut self, input: PickerInput) {
        let at_files = !self.model.choosing_character()
            && self.model.confirming_overwrite().is_none()
            && self.model.editing().is_none();
        let activation = self.model.apply(input);
        self.dirty();
        match activation {
            PickerActivation::PickedCharacter { path, slot } => {
                self.commit(Outcome::Load {
                    path,
                    slot: Some(slot),
                });
            }
            PickerActivation::PickedFile(path) => {
                if self.mode == Mode::Load {
                    self.commit(Outcome::Load { path, slot: None });
                }
            }
            PickerActivation::PickedDestination { path, .. } => {
                if self.mode == Mode::Save {
                    self.commit(Outcome::Save { path });
                }
            }
            // A back press that changed nothing in the file stage is a back press at the top: the
            // way out of the panel, where ER's picker had none.
            PickerActivation::Ignored if matches!(input, PickerInput::Back) && at_files => {
                self.close("closed by the player at the top folder");
            }
            PickerActivation::Repopulate | PickerActivation::Ignored => {}
        }
    }

    fn commit(&mut self, outcome: Outcome) {
        log_line(format_args!("{LOG_PREFIX} picker chose {outcome:?}"));
        self.model.set_status_message(PickerStatusMessage::new(
            match self.mode {
                Mode::Load => "LOADING",
                Mode::Save => "SAVING",
            },
            "one moment",
        ));
        self.outcome = Some(outcome);
        self.phase = Phase::Committing(0);
        self.dirty();
    }

    fn close(&mut self, why: &str) {
        log_line(format_args!("{LOG_PREFIX} picker closed -- {why}"));
        self.phase = Phase::Closing(0);
        self.outcome = None;
    }
}

/// The panel, while it is on screen or releasing its hold.
static PANEL: Mutex<Option<Panel>> = Mutex::new(None);

/// `[save_picker]`, handed over by the loader before any row can be pressed.
static SETTINGS: Mutex<Option<PickerSettings>> = Mutex::new(None);

/// Set once the panel and its clock are registered with `ds2-overlay`.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Frames a pick may wait for the pause menu's tick before the panel says the menu has gone.
const COMMIT_DEADLINE_FRAMES: u32 = 120;

/// Frames the hold outlasts the panel at most, waiting for every key to be released.
const RELEASE_DEADLINE_FRAMES: u32 = 90;

/// The file beside the executable that remembers where the last pick was made.
const REMEMBERED_DIR_FILE: &str = "ds2-save-picker-dir.txt";

/// Take `[save_picker]`. Called by the loader once, before the rows can be pressed.
pub fn configure(settings: PickerSettings) {
    if let Ok(mut slot) = SETTINGS.lock() {
        *slot = Some(settings);
    }
}

/// Register the panel's draw function and its clock with `ds2-overlay`. `false` has been logged,
/// and the rows keep opening the OS dialog.
pub fn install() -> bool {
    if !ds2_overlay::panels::add_panel(draw, is_up, Some(wants_input)) {
        log_line(format_args!(
            "{LOG_PREFIX} picker: every ds2-overlay panel slot is taken"
        ));
        return false;
    }
    if !ds2_overlay::frame_hook::add_frame_hook(on_frame) {
        log_line(format_args!(
            "{LOG_PREFIX} picker: every ds2-overlay frame-hook slot is taken"
        ));
        return false;
    }
    INSTALLED.store(true, Ordering::Release);
    log_line(format_args!(
        "{LOG_PREFIX} picker: the in-game panel answers both rows"
    ));
    true
}

fn settings() -> PickerSettings {
    SETTINGS
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_default()
}

/// Whether the in-game panel answers the rows, rather than the OS dialog.
fn in_game() -> bool {
    INSTALLED.load(Ordering::Acquire) && !settings().os_native
}

/// Open the panel to load a character. `false` means the caller opens the OS dialog instead.
pub(crate) fn open_for_load() -> bool {
    open(Mode::Load)
}

/// Open the panel to save the running character. `false` means the caller opens the OS dialog.
pub(crate) fn open_for_save() -> bool {
    open(Mode::Save)
}

fn open(mode: Mode) -> bool {
    // Every press is logged with the overlay's frame count and how long opening took, so "it
    // took two clicks" can be told apart from "the first click opened slowly".
    let pressed_at = std::time::Instant::now();
    let frame = ds2_overlay::frame_hook::ticks();
    if !in_game() {
        log_line(format_args!(
            "{LOG_PREFIX} picker press mode={mode:?} frame={frame} -- no panel, the OS dialog \
             answers"
        ));
        return false;
    }
    let Ok(mut guard) = PANEL.lock() else {
        return false;
    };
    if guard
        .as_ref()
        .is_some_and(|panel| panel.phase != Phase::Closing(0))
    {
        // Already up, or still waiting on a pick: the press that reached here came through the
        // hold somehow, and a second panel would have two picks racing for one tick.
        log_line(format_args!(
            "{LOG_PREFIX} picker press mode={mode:?} frame={frame} -- ignored, a panel is still \
             there phase={:?}",
            guard.as_ref().map(|panel| panel.phase)
        ));
        return true;
    }
    let start = start_directory();
    let mut model = match mode {
        Mode::Load => SavePickerModel::open(&start),
        Mode::Save => SavePickerModel::open_for_destination(
            &start,
            ds2_save_redirect::active_save_file_name(),
        ),
    };
    model.set_posix_drive(Some('Z'));
    model.set_drives(drives());
    model.set_current_container(ds2_save_redirect::live_container());
    // `co2` under Seamless Co-op: without it the player's own live save is not listed.
    model.set_container_extension(
        ds2_save_redirect::active_save_file_name()
            .rsplit_once('.')
            .map(|(_, extension)| extension),
    );
    let mut reader = Reader::new();
    // The Enter or A that pressed the row is still down on this frame.
    reader.swallow_held();
    *guard = Some(Panel {
        model,
        mode,
        phase: Phase::Open,
        reader,
        view: None,
        outcome: None,
        capacity: 0,
    });
    drop(guard);
    ds2_input_harness::hold(true);
    log_line(format_args!(
        "{LOG_PREFIX} picker open mode={mode:?} frame={frame} took={}ms dir={}",
        pressed_at.elapsed().as_millis(),
        start.display()
    ));
    true
}

/// Whether the panel is on screen. `ds2-overlay` renders no imgui frame while no panel is.
fn is_up() -> bool {
    PANEL.try_lock().ok().is_some_and(|guard| {
        guard
            .as_ref()
            .is_some_and(|panel| !matches!(panel.phase, Phase::Closing(_)))
    })
}

/// `ds2-overlay` asks this every frame: while the panel is up, the game window gets no keyboard or
/// mouse message.
fn wants_input() -> bool {
    // Closing included: the click that closed the panel must not reach the pause menu either.
    PANEL.try_lock().ok().is_some_and(|guard| guard.is_some())
}

/// Act on a waiting pick, if there is one. Called from both rows' pause-menu ticks -- whichever of
/// the two rows is registered -- because that is where the row's press used to act. The second
/// call in a frame finds nothing.
pub(crate) fn collect() {
    let Some(outcome) = take_outcome() else {
        return;
    };
    let (picked, result) = match outcome {
        Outcome::Load { path, slot } => {
            let result = crate::import::accept(&path, slot);
            (path, result)
        }
        Outcome::Save { path } => {
            let result = crate::export::commit(&path);
            (path, result)
        }
    };
    match result {
        Ok(()) => finished(&picked),
        Err(refusal) => refused(&refusal),
    }
}

/// The pick waiting to be acted on, if any.
fn take_outcome() -> Option<Outcome> {
    let mut guard = PANEL.lock().ok()?;
    let panel = guard.as_mut()?;
    if !matches!(panel.phase, Phase::Committing(_)) {
        return None;
    }
    panel.outcome.take()
}

/// The tick acted on the pick and it went ahead: close the panel and remember where it was made.
fn finished(picked: &Path) {
    if settings().remember_dir
        && let Some(dir) = ds2_save_picker_core::path::parent(picked)
    {
        remember_directory(&dir);
    }
    if let Ok(mut guard) = PANEL.lock()
        && let Some(panel) = guard.as_mut()
    {
        panel.close("the pick went ahead");
    }
}

/// The tick refused the pick: say why on the panel and let the player choose again.
fn refused(refusal: &Refused) {
    if let Ok(mut guard) = PANEL.lock()
        && let Some(panel) = guard.as_mut()
    {
        log_line(format_args!(
            "{LOG_PREFIX} picker refusal shown: {} -- {}",
            refusal.headline, refusal.detail
        ));
        panel.model.set_status_message(PickerStatusMessage::new(
            refusal.headline,
            refusal.detail.clone(),
        ));
        panel.phase = Phase::Open;
        panel.dirty();
    }
}

/// One frame: keys and pad into the model, the commit deadline, and the hold's release.
fn on_frame() {
    let Ok(mut guard) = PANEL.lock() else {
        return;
    };
    let Some(panel) = guard.as_mut() else {
        return;
    };
    match panel.phase {
        Phase::Closing(frames) => {
            if !panel.reader.anything_down() || frames >= RELEASE_DEADLINE_FRAMES {
                *guard = None;
                drop(guard);
                ds2_input_harness::hold(false);
            } else {
                panel.phase = Phase::Closing(frames + 1);
            }
        }
        Phase::Committing(frames) => {
            // Presses are read and dropped, so none of them fires later.
            let _ = panel.reader.poll();
            if frames >= COMMIT_DEADLINE_FRAMES {
                log_line(format_args!(
                    "{LOG_PREFIX} picker: the pause menu never collected the pick"
                ));
                panel.outcome = None;
                panel.phase = Phase::Open;
                panel.model.set_status_message(PickerStatusMessage::new(
                    "NOTHING HAPPENED",
                    "the pause menu closed before the pick reached it -- choose again",
                ));
                panel.dirty();
            } else {
                panel.phase = Phase::Committing(frames + 1);
            }
        }
        Phase::Open => {
            let editing = panel.model.editing().is_some();
            for press in panel.reader.poll() {
                if panel.phase != Phase::Open {
                    break;
                }
                let input = match press {
                    Press::Close if !editing => {
                        panel.close("closed by the player");
                        break;
                    }
                    Press::Close => PickerInput::Back,
                    Press::Up => PickerInput::Up,
                    Press::Down => PickerInput::Down,
                    Press::Left => PickerInput::Left,
                    Press::Right => PickerInput::Right,
                    Press::Confirm => PickerInput::Confirm,
                    Press::Back => PickerInput::Back,
                    Press::Tab => PickerInput::Tab,
                    Press::PageUp => PickerInput::PageUp,
                    Press::PageDown => PickerInput::PageDown,
                    Press::Backspace => PickerInput::Backspace,
                    Press::Char(typed) => PickerInput::Char(typed),
                    Press::SelectAll => PickerInput::SelectAll,
                    Press::Paste => {
                        if let Some(text) = crate::clipboard::read()
                            && panel.model.paste(&text) != PickerActivation::Ignored
                        {
                            panel.dirty();
                        }
                        continue;
                    }
                    Press::Copy => {
                        if let Some(text) = panel.model.selected_text() {
                            crate::clipboard::write(text);
                        }
                        continue;
                    }
                    Press::Cut => {
                        if let Some(text) = panel.model.cut_selection() {
                            crate::clipboard::write(&text);
                            panel.dirty();
                        }
                        continue;
                    }
                };
                panel.apply(input);
            }
        }
    }
}

/// Where the panel opens: the remembered folder, then `[save_picker] start_dir`, then the folder
/// the OS dialog would have opened in.
fn start_directory() -> PathBuf {
    let settings = settings();
    let usable = |path: PathBuf| path.is_dir().then_some(path);
    let remembered = settings
        .remember_dir
        .then(remembered_directory)
        .flatten()
        .and_then(usable);
    let configured = settings
        .start_dir
        .as_deref()
        .map(|dir| ds2_save_picker_core::autocomplete::resolve_typed_path(dir, Some('Z')))
        .and_then(usable);
    remembered
        .or(configured)
        .or_else(crate::import::start_directory)
        .or_else(ds2_game_base::log::game_directory_path)
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

fn remembered_directory_file() -> Option<PathBuf> {
    ds2_game_base::log::game_directory_path().map(|dir| dir.join(REMEMBERED_DIR_FILE))
}

fn remembered_directory() -> Option<PathBuf> {
    let text = std::fs::read_to_string(remembered_directory_file()?).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

fn remember_directory(dir: &Path) {
    let Some(file) = remembered_directory_file() else {
        return;
    };
    if let Err(error) = std::fs::write(&file, format!("{}\n", dir.display())) {
        log_line(format_args!(
            "{LOG_PREFIX} picker: could not remember {} in {}: {error}",
            dir.display(),
            file.display()
        ));
    }
}

/// Every drive root that answers, `C:\` to `Z:\`. `A:` and `B:` are skipped: on a real machine
/// asking about a floppy drive with no disk is a stall.
fn drives() -> Vec<String> {
    ('C'..='Z')
        .map(|letter| format!("{letter}:\\"))
        .filter(|root| Path::new(root).is_dir())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

// The game's own palette (`ds2_overlay::style`, docs/DS2-UI-DESIGN.md), under this panel's names.
// Folders and the save in use are told apart by their `/` and `*` markers, not by a hue.
const PANEL_BG: [f32; 4] = style::PANEL_BG;
const PANEL_EDGE: [f32; 4] = style::BRONZE;
const DIM_COVER: [f32; 4] = style::DIM_COVER;
const HIGHLIGHT: [f32; 4] = style::INK_2;
const HOVER: [f32; 4] = style::INK_1;
const TITLE: [f32; 4] = style::TEXT;
const TEXT: [f32; 4] = style::TEXT;
const DIM: [f32; 4] = style::BRONZE;
const DISABLED: [f32; 4] = style::ASH;
const FOLDER: [f32; 4] = style::TEXT;
const CURRENT: [f32; 4] = style::BRONZE;
const WARN: [f32; 4] = style::WARN_TEXT;
const FIELD_BG: [f32; 4] = style::SLATE;
const FIELD_EDIT: [f32; 4] = style::SLATE_EDIT;
/// Behind field text Ctrl+A selected.
const SELECTION: [f32; 4] = style::TEXT_SELECTION;
const CELL_BG: [f32; 4] = style::INK_1;
const FOCUS_EDGE: [f32; 4] = style::BRONZE;
/// The bronze rule down the left edge of the row the cursor is on.
const FOCUS_RULE: f32 = 3.0;
const PAD: f32 = 14.0;

/// The panel's draw function, called by `ds2-overlay` once per frame.
fn draw(ui: &Ui) {
    let Ok(mut guard) = PANEL.try_lock() else {
        return;
    };
    let Some(panel) = guard.as_mut() else {
        return;
    };
    if matches!(panel.phase, Phase::Closing(_)) {
        return;
    }
    let display = ui.io().display_size;
    let line = ui.current_font_size();
    let row_height = line + 8.0;
    let width = (display[0] * 0.62)
        .clamp(560.0, 1100.0)
        .min(display[0] - 32.0);
    let height = (display[1] * 0.78).min(display[1] - 32.0);
    let left = (display[0] - width) * 0.5;
    let top = (display[1] - height) * 0.5;

    // Rows the list area holds: the height, less the header, the banner and the footer.
    let reserved = PAD * 2.0 + line * 2.0 + 10.0 + (line * 3.0 + 12.0) + (line + 10.0);
    let capacity = (((height - reserved) / row_height).floor() as usize).clamp(4, 30);
    if capacity != panel.capacity {
        panel.capacity = capacity;
        panel.model.set_row_capacity(capacity);
        panel.dirty();
    }
    if panel.view.is_none() {
        panel.view = Some(panel.model.view());
    }
    let Some(view) = panel.view.clone() else {
        return;
    };

    let list = ui.get_foreground_draw_list();
    list.add_rect([0.0, 0.0], display, DIM_COVER)
        .filled(true)
        .build();
    list.add_rect([left, top], [left + width, top + height], PANEL_BG)
        .filled(true)
        .rounding(style::ROUNDING)
        .build();
    list.add_rect([left, top], [left + width, top + height], PANEL_EDGE)
        .rounding(style::ROUNDING)
        .thickness(style::FRAME_PX)
        .build();

    // In back-buffer pixels; imgui's own position is in window pixels here. See `panels::mouse`.
    let mouse = ds2_overlay::panels::mouse().unwrap_or(ui.io().mouse_pos);
    let inside = |min: [f32; 2], max: [f32; 2]| {
        mouse[0] >= min[0] && mouse[0] < max[0] && mouse[1] >= min[1] && mouse[1] < max[1]
    };
    let clicked = ui.is_mouse_clicked(MouseButton::Left);
    let mut click: Option<PickerInput> = None;

    // Header: title, then where.
    let mut y = top + PAD;
    let inner_left = left + PAD;
    let inner_right = left + width - PAD;
    list.add_text([inner_left, y], TITLE, &view.title);
    let close_label = "[ close ]";
    let close_width = ui.calc_text_size(close_label)[0];
    let close_min = [inner_right - close_width, y];
    let close_max = [inner_right, y + line];
    let close_hover = inside(close_min, close_max);
    list.add_text(
        close_min,
        if close_hover { TITLE } else { DIM },
        close_label,
    );
    y += line + 2.0;
    list.add_text(
        [inner_left, y],
        DIM,
        clip(ui, &view.subtitle, inner_right - inner_left),
    );
    y += line + 10.0;

    // The banner: why a pick was refused, or what is happening.
    let banner_top = y;
    if let Some(status) = &view.status {
        list.add_text([inner_left, y], WARN, status.headline());
        for detail in status.detail_lines() {
            y += line;
            list.add_text(
                [inner_left + 12.0, y],
                TEXT,
                clip(ui, detail, inner_right - inner_left),
            );
        }
    }
    y = banner_top + line * 3.0 + 12.0;

    // The rows.
    for row in &view.rows {
        let row_min = [inner_left - 4.0, y];
        let row_max = [inner_right + 4.0, y + row_height];
        let hovered = inside(row_min, row_max) && panel_is_open(panel.phase);
        if row.highlighted {
            list.add_rect(row_min, row_max, HIGHLIGHT)
                .filled(true)
                .rounding(style::ROUNDING)
                .build();
            list.add_rect(row_min, [row_min[0] + FOCUS_RULE, row_max[1]], FOCUS_EDGE)
                .filled(true)
                .build();
        } else if hovered && row.selectable {
            list.add_rect(row_min, row_max, HOVER)
                .filled(true)
                .rounding(style::ROUNDING)
                .build();
        }
        // Bronze reads 4.37:1 on the focus fill, under body text's 4.5.
        let secondary = if row.highlighted { TEXT } else { DIM };
        let text_y = y + (row_height - line) * 0.5;
        if row.kind == RowKind::DriveStrip {
            let strip = Strip {
                origin: [inner_left, text_y],
                right: inner_right,
                line,
                mouse,
                clicked,
            };
            if let Some(input) = draw_drive_strip(ui, &list, &view, &strip) {
                click = Some(input);
            }
        } else {
            let color = match row.kind {
                _ if !row.selectable => DISABLED,
                _ if row.current => CURRENT,
                RowKind::Dir | RowKind::Parent => FOLDER,
                RowKind::Overwrite => WARN,
                _ => TEXT,
            };
            let color = if row.highlighted && color == CURRENT {
                TEXT
            } else {
                color
            };
            let modified_width = row
                .modified
                .as_deref()
                .map_or(0.0, |text| ui.calc_text_size(text)[0] + 16.0);
            let text_space = (inner_right - inner_left - modified_width) * 0.5;
            let main_space = if row.detail.is_some() {
                text_space
            } else {
                inner_right - inner_left - modified_width
            };
            list.add_text([inner_left, text_y], color, clip(ui, &row.text, main_space));
            if let Some(detail) = &row.detail {
                list.add_text(
                    [inner_left + text_space + 8.0, text_y],
                    secondary,
                    clip(ui, detail, text_space - 8.0),
                );
            }
            if let Some(modified) = &row.modified {
                let modified_x = inner_right - ui.calc_text_size(modified)[0];
                list.add_text([modified_x, text_y], secondary, modified);
            }
            if hovered && clicked && row.selectable {
                click = Some(PickerInput::ClickRow(row.row));
            }
        }
        y += row_height;
    }

    // The new file's name, while it is typed.
    if let Some(field) = &view.name_field {
        let min = [inner_left, y + 4.0];
        let max = [inner_right, y + 4.0 + row_height];
        list.add_rect(min, max, if field.editing { FIELD_EDIT } else { FIELD_BG })
            .filled(true)
            .rounding(style::ROUNDING)
            .build();
        let caret = if field.editing { "_" } else { "" };
        let text_y = min[1] + (row_height - line) * 0.5;
        if field.selected {
            let label = ui.calc_text_size("name: ")[0];
            let typed = ui.calc_text_size(&field.text)[0];
            list.add_rect(
                [min[0] + 5.0 + label, text_y - 1.0],
                [min[0] + 7.0 + label + typed, text_y + line + 1.0],
                SELECTION,
            )
            .filled(true)
            .build();
        }
        list.add_text(
            [min[0] + 6.0, text_y],
            TEXT,
            format!("name: {}{caret}", field.text),
        );
    }

    // The footer.
    let footer_y = top + height - PAD - line;
    list.add_text(
        [inner_left, footer_y],
        DIM,
        clip(ui, view.hint, inner_right - inner_left),
    );

    if !panel_is_open(panel.phase) {
        return;
    }
    if close_hover && clicked {
        panel.close("closed with the close button");
        return;
    }
    if ui.is_mouse_clicked(MouseButton::Right) {
        click = Some(PickerInput::Back);
    }
    let wheel = ui.io().mouse_wheel;
    if click.is_none() && wheel != 0.0 {
        click = Some(if wheel > 0.0 {
            PickerInput::Up
        } else {
            PickerInput::Down
        });
    }
    if let Some(input) = click {
        panel.apply(input);
    }
}

fn panel_is_open(phase: Phase) -> bool {
    phase == Phase::Open
}

/// Where the drive strip goes and what the mouse is doing, for [`draw_drive_strip`].
#[derive(Clone, Copy)]
struct Strip {
    /// Top-left of the first cell's text.
    origin: [f32; 2],
    /// Where the path field ends.
    right: f32,
    /// The font's line height.
    line: f32,
    /// The mouse, in display pixels.
    mouse: [f32; 2],
    /// Whether the left button went down this frame.
    clicked: bool,
}

/// The drive strip and the path field beside it. Returns the press a click on either made.
fn draw_drive_strip(
    ui: &Ui,
    list: &hudhook::imgui::DrawListMut<'_>,
    view: &PickerView,
    strip: &Strip,
) -> Option<PickerInput> {
    let Strip {
        origin,
        right,
        line,
        mouse,
        clicked,
    } = *strip;
    let inside = |min: [f32; 2], max: [f32; 2]| {
        mouse[0] >= min[0] && mouse[0] < max[0] && mouse[1] >= min[1] && mouse[1] < max[1]
    };
    let mut x = origin[0];
    let mut press = None;
    for (index, cell) in view.drives.iter().enumerate() {
        let width = ui.calc_text_size(&cell.label)[0] + 12.0;
        let min = [x, origin[1] - 3.0];
        let max = [x + width, origin[1] + line + 3.0];
        list.add_rect(min, max, CELL_BG)
            .filled(true)
            .rounding(style::ROUNDING)
            .build();
        if cell.focused {
            list.add_rect(min, max, FOCUS_EDGE)
                .rounding(style::ROUNDING)
                .thickness(1.5)
                .build();
        }
        list.add_text(
            [x + 6.0, origin[1]],
            if cell.current { CURRENT } else { TEXT },
            &cell.label,
        );
        if clicked && inside(min, max) {
            press = Some(PickerInput::ClickDriveCell(index));
        }
        x += width + 6.0;
    }
    if let Some(field) = &view.path_field {
        let min = [x + 6.0, origin[1] - 3.0];
        let max = [right, origin[1] + line + 3.0];
        list.add_rect(min, max, if field.editing { FIELD_EDIT } else { FIELD_BG })
            .filled(true)
            .rounding(style::ROUNDING)
            .build();
        if field.focused || field.editing {
            list.add_rect(min, max, FOCUS_EDGE)
                .rounding(style::ROUNDING)
                .thickness(1.5)
                .build();
        }
        let space = max[0] - min[0] - 12.0;
        let shown = clip_left(ui, &field.text, space * 0.8);
        let typed_width = ui.calc_text_size(&shown)[0];
        if field.selected {
            list.add_rect(
                [min[0] + 5.0, origin[1] - 1.0],
                [min[0] + 7.0 + typed_width, origin[1] + line + 1.0],
                SELECTION,
            )
            .filled(true)
            .build();
        }
        list.add_text([min[0] + 6.0, origin[1]], TEXT, &shown);
        if field.editing {
            let ghost = field.ghost.as_deref().unwrap_or("");
            list.add_text([min[0] + 6.0 + typed_width, origin[1]], DIM, ghost);
            let caret_x = min[0] + 6.0 + typed_width;
            list.add_line([caret_x, origin[1]], [caret_x, origin[1] + line], TITLE)
                .build();
        }
        if clicked && inside(min, max) {
            press = Some(PickerInput::ClickPathField);
        }
    }
    press
}

/// `text`, cut at the right with an ellipsis to fit `width` pixels.
fn clip(ui: &Ui, text: &str, width: f32) -> String {
    if ui.calc_text_size(text)[0] <= width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "...";
        if ui.calc_text_size(&candidate)[0] <= width {
            return candidate;
        }
    }
    String::new()
}

/// `text`, cut at the LEFT to fit, because the end of a path is the part that says where it is.
fn clip_left(ui: &Ui, text: &str, width: f32) -> String {
    if ui.calc_text_size(text)[0] <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let candidate = format!("...{}", chars[start..].iter().collect::<String>());
        if ui.calc_text_size(&candidate)[0] <= width {
            return candidate;
        }
    }
    String::new()
}
