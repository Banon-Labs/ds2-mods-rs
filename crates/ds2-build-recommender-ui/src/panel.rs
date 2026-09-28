//! The panel: its state between frames, the keyboard half, and the drawing and mouse half.
//!
//! One `Mutex` because Rust cannot know what the picker's module docs establish: DARK SOULS II
//! presents from its simulation thread, so [`open`], `on_frame` and `draw` all run on one thread and
//! the lock is never contended.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ds2_build_import_core::Infusion;
use ds2_build_recommender_core::backend::{
    self, Answer, Calibration, DAMAGE_TYPES, GeneratedBuild, RecommenderBackend, ResultRow,
    StubBackend,
};
use ds2_build_recommender_core::model::{Mode, Objective, PanelState, STAT_COUNT, STAT_LABELS};
use ds2_build_recommender_core::weapons;
use hudhook::imgui::{DrawListMut, MouseButton, Ui};

use crate::input::{Press, Reader};
use crate::{LOG_PREFIX, LogFn};

static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Write one line to the loader's log, if [`install`] was given a logger.
fn log_line(args: std::fmt::Arguments<'_>) {
    let logger = LOGGER.load(Ordering::Acquire);
    if logger != 0 {
        // SAFETY: the only writer is `install`, which stores a `LogFn` cast from a real function.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(logger) };
        logger(args);
    }
}

/// Who answers. The stub until the ranking in `scripts/ds2-builds-recommend.py` is ported.
static BACKEND: StubBackend = StubBackend;

fn backend() -> &'static dyn RecommenderBackend {
    &BACKEND
}

/// A field that takes typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    /// One of the nine stats, by index into [`STAT_LABELS`].
    Stat(usize),
    /// The soul-level override.
    SlOverride,
    /// The weapon search.
    Search,
    /// Weapons for stats: the R1 window in seconds.
    Window,
    /// Similar builds: how many neighbours.
    SimilarK,
}

impl Field {
    /// Where Tab goes from here: through the stats, then the override, then nowhere.
    const fn next(self) -> Option<Field> {
        match self {
            Field::Stat(index) if index + 1 < STAT_COUNT => Some(Field::Stat(index + 1)),
            Field::Stat(_) => Some(Field::SlOverride),
            _ => None,
        }
    }
}

/// A drop-down list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    Weapon,
    Infusion,
    Class,
}

/// What the lower half shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    /// The current mode's answer.
    Answer,
    /// The generated build.
    Build,
}

/// Where the panel is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Taking presses.
    Open,
    /// Gone from the screen, holding input until every key is up. Frames waited so far.
    Closing(u32),
}

/// Something a click asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    /// A click on a surface that does nothing, so it does not count as a click outside.
    Nothing,
    Close,
    Focus(Field),
    OpenList(List),
    ChooseWeapon(&'static str),
    ChooseInfusion(Infusion),
    ChooseClass(Option<&'static str>),
    SetObjective(Objective),
    SetMode(Mode),
    ToggleOneHand,
    TogglePerClass,
    ToggleRawAr,
    ToggleTwoHand,
    ToggleBleed,
    TogglePoison,
    Run,
    UseCharacter,
    Generate,
    Show(Shown),
    AskApply,
    ConfirmApply,
    CancelApply,
}

struct Panel {
    state: PanelState,
    phase: Phase,
    reader: Reader,
    focus: Option<Field>,
    /// What has been typed into the focused field since it was focused.
    edit: String,
    /// Whether anything was typed, so focusing and leaving a field changes nothing.
    edited: bool,
    list: Option<List>,
    list_scroll: usize,
    answer: Option<Answer>,
    results_scroll: usize,
    generated: Option<GeneratedBuild>,
    /// Why the last Generate Build was refused.
    refused: Vec<String>,
    shown: Shown,
    confirming: bool,
    status: Option<String>,
    calibration: Calibration,
}

impl Panel {
    fn new(reader: Reader) -> Self {
        Self {
            state: PanelState::default(),
            phase: Phase::Open,
            reader,
            focus: None,
            edit: String::new(),
            edited: false,
            list: None,
            list_scroll: 0,
            answer: None,
            results_scroll: 0,
            generated: None,
            refused: Vec::new(),
            shown: Shown::Answer,
            confirming: false,
            status: None,
            calibration: backend().calibration(),
        }
    }

    /// The inputs changed: an answer for the old ones is not shown for the new.
    fn changed(&mut self) {
        self.answer = None;
        self.results_scroll = 0;
        self.refused.clear();
        self.status = None;
    }

    fn close(&mut self, why: &str) {
        log_line(format_args!("{LOG_PREFIX} panel closed -- {why}"));
        self.phase = Phase::Closing(0);
    }

    fn focus(&mut self, field: Field) {
        if self.focus == Some(field) {
            return;
        }
        self.commit_focus();
        self.focus = Some(field);
        self.edit.clear();
        self.edited = false;
        if field == Field::Search {
            self.list = Some(List::Weapon);
            self.list_scroll = 0;
        }
    }

    /// Leave the focused field, keeping what was typed.
    fn commit_focus(&mut self) {
        let Some(field) = self.focus.take() else {
            return;
        };
        if field == Field::Search {
            if self.list == Some(List::Weapon) {
                self.list = None;
            }
        } else if self.edited {
            self.apply_edit(field);
        }
        self.edit.clear();
        self.edited = false;
    }

    /// Put the typed text into the field's value. Called on every keystroke, so the soul level
    /// follows the stats as they are typed.
    fn apply_edit(&mut self, field: Field) {
        let number = self.edit.parse::<u16>().ok();
        match field {
            Field::Stat(index) => {
                if let Some(value) = number {
                    self.state.set_stat(index, value);
                    self.changed();
                }
            }
            Field::SlOverride => {
                self.state.set_sl_override(number);
                self.changed();
            }
            Field::Window => {
                if let Ok(seconds) = self.edit.parse::<f32>() {
                    self.state.weapons_for.window_s = seconds.clamp(0.0, 10.0);
                    self.changed();
                }
            }
            Field::SimilarK => {
                if let Some(k) = number {
                    self.state.similar_k = k.clamp(1, 500);
                    self.changed();
                }
            }
            Field::Search => self.list_scroll = 0,
        }
    }

    fn type_char(&mut self, typed: char) {
        let Some(field) = self.focus else {
            return;
        };
        let accepted = match field {
            Field::Search => self.edit.len() < 40,
            Field::Window => (typed.is_ascii_digit() || typed == '.') && self.edit.len() < 4,
            _ => typed.is_ascii_digit() && self.edit.len() < 3,
        };
        if accepted {
            self.edit.push(typed);
            self.edited = true;
            self.apply_edit(field);
        }
    }

    fn backspace(&mut self) {
        let Some(field) = self.focus else {
            return;
        };
        self.edit.pop();
        self.edited = true;
        self.apply_edit(field);
    }

    /// Enter in a field: the search takes its first match, anything else is kept.
    fn confirm_field(&mut self) {
        if self.focus == Some(Field::Search)
            && let Some(first) = weapons::search(&self.edit).first()
        {
            let key = first.key;
            self.commit_focus();
            self.choose_weapon(key);
            return;
        }
        self.commit_focus();
    }

    fn choose_weapon(&mut self, key: &'static str) {
        self.state.choose_weapon(key);
        self.list = None;
        self.changed();
        log_line(format_args!("{LOG_PREFIX} weapon {key}"));
    }

    fn run(&mut self) {
        let answer = backend::ask(backend(), &self.state);
        log_line(format_args!(
            "{LOG_PREFIX} run mode={:?} sl={} -> {}",
            self.state.mode,
            self.state.sl(),
            match &answer {
                Answer::Rows(rows) => format!("{} rows", rows.len()),
                Answer::Build(_) => "a build".to_owned(),
                Answer::FloorViolations(lines) => format!("under floors: {}", lines.join(", ")),
                Answer::Nothing(why) => (*why).to_owned(),
            }
        ));
        self.answer = Some(answer);
        self.results_scroll = 0;
        self.shown = Shown::Answer;
    }

    fn generate(&mut self) {
        match backend::generate(backend(), &self.state) {
            Ok(build) => {
                log_line(format_args!(
                    "{LOG_PREFIX} generated {} SL {} primary={:?} stub={}",
                    build.class, build.sl, build.primary, build.stub
                ));
                self.generated = Some(build);
                self.refused.clear();
                self.shown = Shown::Build;
            }
            Err(lines) => {
                log_line(format_args!(
                    "{LOG_PREFIX} generate refused: {}",
                    lines.join(", ")
                ));
                self.generated = None;
                self.refused = lines;
                self.shown = Shown::Answer;
            }
        }
    }

    fn apply(&mut self) {
        self.confirming = false;
        let Some(generated) = &self.generated else {
            return;
        };
        let (build, extras) = backend::to_import(generated);
        log_line(format_args!(
            "{LOG_PREFIX} apply confirmed: {} SL {} with {} extra grants -- handed to \
             ds2-build-import",
            generated.class,
            generated.sl,
            extras.len()
        ));
        ds2_build_import::queue_generated(build, extras);
        self.status = Some(
            "Queued: the pause menu applies it on its next frame -- the log says what changed"
                .to_owned(),
        );
    }

    fn act(&mut self, action: Action) {
        // A click anywhere but a field leaves the field, and anywhere but a list closes the list.
        if !matches!(action, Action::Focus(_) | Action::Nothing) {
            self.commit_focus();
        }
        if !matches!(
            action,
            Action::OpenList(_)
                | Action::ChooseWeapon(_)
                | Action::ChooseInfusion(_)
                | Action::ChooseClass(_)
                | Action::Nothing
        ) {
            self.list = None;
        }
        match action {
            Action::Nothing => {}
            Action::Close => self.close("closed with the close button"),
            Action::Focus(field) => self.focus(field),
            Action::OpenList(list) => {
                self.list = if self.list == Some(list) {
                    None
                } else {
                    Some(list)
                };
                self.list_scroll = 0;
            }
            Action::ChooseWeapon(key) => self.choose_weapon(key),
            Action::ChooseInfusion(infusion) => {
                if self.state.choose_infusion(infusion) {
                    self.changed();
                }
                self.list = None;
            }
            Action::ChooseClass(class) => {
                self.state.weapons_for.class = class.map(str::to_owned);
                self.list = None;
                self.changed();
            }
            Action::SetObjective(objective) => {
                self.state.objective = objective;
                self.changed();
            }
            Action::SetMode(mode) => {
                self.state.mode = mode;
                self.changed();
                self.shown = Shown::Answer;
            }
            Action::ToggleOneHand => {
                self.state.weapons_for.one_hand ^= true;
                self.changed();
            }
            Action::TogglePerClass => {
                self.state.weapons_for.per_class ^= true;
                self.changed();
            }
            Action::ToggleRawAr => {
                self.state.weapons_for.raw_ar ^= true;
                self.changed();
            }
            Action::ToggleTwoHand => {
                self.state.two_hand ^= true;
                self.changed();
            }
            Action::ToggleBleed => {
                self.state.status.bleed ^= true;
                self.changed();
            }
            Action::TogglePoison => {
                self.state.status.poison ^= true;
                self.changed();
            }
            Action::Run => self.run(),
            Action::UseCharacter => match backend().current_character_stats() {
                Some(stats) => {
                    self.state.stats = stats;
                    self.changed();
                    self.status = Some(if backend().is_stub() {
                        "Stats filled in -- the stub's fixed character, not yours yet".to_owned()
                    } else {
                        "Stats read from your character".to_owned()
                    });
                }
                None => self.status = Some("Could not read the character's stats".to_owned()),
            },
            Action::Generate => self.generate(),
            Action::Show(shown) => self.shown = shown,
            Action::AskApply => self.confirming = self.generated.is_some(),
            Action::ConfirmApply => self.apply(),
            Action::CancelApply => self.confirming = false,
        }
    }

    /// One press from the keyboard or pad.
    fn press(&mut self, press: Press) {
        if self.confirming {
            // Apply is a click. A key can only back out of it.
            if matches!(press, Press::Close | Press::Back) {
                self.confirming = false;
            }
            return;
        }
        if self.focus.is_some() {
            match press {
                Press::Char(typed) => self.type_char(typed),
                Press::Backspace => self.backspace(),
                Press::Confirm => self.confirm_field(),
                Press::Tab => {
                    let next = self.focus.and_then(Field::next);
                    self.commit_focus();
                    if let Some(next) = next {
                        self.focus(next);
                    }
                }
                Press::Close | Press::Back => self.commit_focus(),
                Press::Up | Press::PageUp => self.list_scroll = self.list_scroll.saturating_sub(1),
                Press::Down | Press::PageDown => self.list_scroll += 1,
            }
            return;
        }
        match press {
            Press::Close | Press::Back if self.list.is_some() => self.list = None,
            Press::Close | Press::Back => self.close("closed by the player"),
            Press::Tab => self.focus(Field::Stat(0)),
            Press::Up => self.results_scroll = self.results_scroll.saturating_sub(1),
            Press::Down => self.results_scroll += 1,
            Press::PageUp => self.results_scroll = self.results_scroll.saturating_sub(10),
            Press::PageDown => self.results_scroll += 10,
            Press::Confirm | Press::Backspace | Press::Char(_) => {}
        }
    }
}

/// The panel, while it is on screen or releasing its hold.
static PANEL: Mutex<Option<Panel>> = Mutex::new(None);

/// Set once the panel and its clock are registered with `ds2-overlay`.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Frames the hold outlasts the panel at most, waiting for every key to be released.
const RELEASE_DEADLINE_FRAMES: u32 = 90;

/// Register the panel's draw function and its clock with `ds2-overlay`, and point this crate's log
/// at the loader's. `false` has been logged, and the row then opens nothing.
///
/// `ds2-overlay` must already be installed for the panel to draw; this only fills its tables.
pub fn install(logger: LogFn) -> bool {
    LOGGER.store(logger as usize, Ordering::Release);
    if !ds2_overlay::panels::add_panel(draw, is_up, Some(wants_input)) {
        log_line(format_args!(
            "{LOG_PREFIX} every ds2-overlay panel slot is taken -- the row opens nothing"
        ));
        return false;
    }
    if !ds2_overlay::frame_hook::add_frame_hook(on_frame) {
        log_line(format_args!(
            "{LOG_PREFIX} every ds2-overlay frame-hook slot is taken -- the row opens nothing"
        ));
        return false;
    }
    INSTALLED.store(true, Ordering::Release);
    log_line(format_args!(
        "{LOG_PREFIX} panel installed, backend={}",
        if backend().is_stub() { "stub" } else { "real" }
    ));
    true
}

/// Show the panel and take the player's input. The Build Recommender row's action.
///
/// Runs on the game thread, from the pause menu's confirm. Opening a panel that is already up does
/// nothing; one still releasing its hold is replaced.
pub fn open() {
    let frame = ds2_overlay::frame_hook::ticks();
    if !INSTALLED.load(Ordering::Acquire) {
        log_line(format_args!(
            "{LOG_PREFIX} press frame={frame} -- no panel, it was never installed"
        ));
        return;
    }
    let Ok(mut guard) = PANEL.lock() else {
        return;
    };
    if guard
        .as_ref()
        .is_some_and(|panel| panel.phase == Phase::Open)
    {
        log_line(format_args!(
            "{LOG_PREFIX} press frame={frame} -- ignored, the panel is already up"
        ));
        return;
    }
    let mut reader = Reader::new();
    // The Enter or A that pressed the row is still down on this frame.
    reader.swallow_held();
    *guard = Some(Panel::new(reader));
    drop(guard);
    ds2_input_harness::hold(true);
    log_line(format_args!("{LOG_PREFIX} panel open frame={frame}"));
}

/// Whether the panel is on screen. `ds2-overlay` renders no imgui frame while no panel is.
fn is_up() -> bool {
    PANEL.try_lock().ok().is_some_and(|guard| {
        guard
            .as_ref()
            .is_some_and(|panel| panel.phase == Phase::Open)
    })
}

/// While the panel exists, closing included, the game window gets no keyboard or mouse message.
fn wants_input() -> bool {
    PANEL.try_lock().ok().is_some_and(|guard| guard.is_some())
}

/// One frame: keys and pad into the panel, and the hold's release after it closes.
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
        Phase::Open => {
            for press in panel.reader.poll() {
                if panel.phase != Phase::Open {
                    break;
                }
                panel.press(press);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

const PANEL_BG: [f32; 4] = [0.04, 0.04, 0.05, 0.95];
const PANEL_EDGE: [f32; 4] = [0.55, 0.48, 0.34, 0.9];
const DIM_COVER: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const TITLE: [f32; 4] = [0.95, 0.88, 0.66, 1.0];
const TEXT: [f32; 4] = [0.93, 0.92, 0.88, 1.0];
const DIM: [f32; 4] = [0.62, 0.61, 0.58, 1.0];
const DISABLED: [f32; 4] = [0.42, 0.42, 0.40, 1.0];
const WARN: [f32; 4] = [1.0, 0.72, 0.30, 1.0];
const GOOD: [f32; 4] = [0.55, 0.95, 0.60, 1.0];
const FIELD_BG: [f32; 4] = [0.10, 0.12, 0.16, 1.0];
const FIELD_EDIT: [f32; 4] = [0.14, 0.18, 0.26, 1.0];
const CELL_BG: [f32; 4] = [0.12, 0.12, 0.13, 1.0];
const CELL_ON: [f32; 4] = [0.30, 0.26, 0.16, 1.0];
const CELL_HOVER: [f32; 4] = [0.20, 0.20, 0.22, 1.0];
const LIST_BG: [f32; 4] = [0.07, 0.08, 0.10, 0.98];
const ROW_HOVER: [f32; 4] = [1.0, 1.0, 1.0, 0.08];
const FOCUS_EDGE: [f32; 4] = [0.95, 0.88, 0.66, 1.0];
const PAD: f32 = 14.0;
const GAP: f32 = 8.0;

/// A frame's drawing surface and the clickable rectangles laid on it, in draw order.
struct Canvas<'ui> {
    ui: &'ui Ui,
    list: DrawListMut<'ui>,
    mouse: [f32; 2],
    line: f32,
    row: f32,
    targets: Vec<([f32; 2], [f32; 2], Action)>,
}

impl Canvas<'_> {
    fn width(&self, text: &str) -> f32 {
        self.ui.calc_text_size(text)[0]
    }

    fn inside(&self, min: [f32; 2], max: [f32; 2]) -> bool {
        let [x, y] = self.mouse;
        x >= min[0] && x < max[0] && y >= min[1] && y < max[1]
    }

    fn text(&self, at: [f32; 2], color: [f32; 4], text: &str) {
        self.list.add_text(at, color, text);
    }

    fn rect(&self, min: [f32; 2], max: [f32; 2], color: [f32; 4]) {
        self.list
            .add_rect(min, max, color)
            .filled(true)
            .rounding(3.0)
            .build();
    }

    fn edge(&self, min: [f32; 2], max: [f32; 2], color: [f32; 4]) {
        self.list
            .add_rect(min, max, color)
            .rounding(3.0)
            .thickness(1.5)
            .build();
    }

    /// A clickable cell. Returns its right edge.
    fn button(&mut self, x: f32, y: f32, label: &str, on: bool, action: Option<Action>) -> f32 {
        let min = [x, y];
        let max = [x + self.width(label) + 16.0, y + self.row];
        let hovered = action.is_some() && self.inside(min, max);
        let fill = if on {
            CELL_ON
        } else if hovered {
            CELL_HOVER
        } else {
            CELL_BG
        };
        self.rect(min, max, fill);
        if on {
            self.edge(min, max, FOCUS_EDGE);
        }
        let color = if action.is_some() { TEXT } else { DISABLED };
        self.text([x + 8.0, y + (self.row - self.line) * 0.5], color, label);
        if let Some(action) = action {
            self.targets.push((min, max, action));
        }
        max[0]
    }

    /// `[x] label`. Returns its right edge.
    fn check(&mut self, x: f32, y: f32, label: &str, checked: bool, action: Action) -> f32 {
        let text = format!("[{}] {label}", if checked { "x" } else { " " });
        self.button(x, y, &text, false, Some(action))
    }

    /// A text field of `width`, showing `typed` while focused and `value` otherwise. Returns its
    /// right edge.
    fn field(&mut self, x: f32, y: f32, width: f32, spec: FieldSpec<'_>) -> f32 {
        let min = [x, y];
        let max = [x + width, y + self.row];
        self.rect(min, max, if spec.focused { FIELD_EDIT } else { FIELD_BG });
        if spec.focused || self.inside(min, max) {
            self.edge(min, max, FOCUS_EDGE);
        }
        let text_y = y + (self.row - self.line) * 0.5;
        let space = width - 12.0;
        if spec.focused {
            let shown = clip_left(self.ui, spec.typed, space - 8.0);
            self.text([x + 6.0, text_y], TEXT, &shown);
            let caret = x + 6.0 + self.width(&shown);
            self.list
                .add_line([caret, text_y], [caret, text_y + self.line], TITLE)
                .build();
            if spec.typed.is_empty() {
                self.text(
                    [caret + 4.0, text_y],
                    DISABLED,
                    &clip(self.ui, spec.value, space - 8.0),
                );
            }
        } else {
            let color = if spec.placeholder { DISABLED } else { TEXT };
            self.text([x + 6.0, text_y], color, &clip(self.ui, spec.value, space));
        }
        self.targets.push((min, max, Action::Focus(spec.field)));
        max[0]
    }
}

/// What a [`Canvas::field`] shows.
struct FieldSpec<'a> {
    field: Field,
    focused: bool,
    typed: &'a str,
    /// The value when not typing, or the hint behind an empty field when typing.
    value: &'a str,
    /// Whether `value` is a hint rather than a value.
    placeholder: bool,
}

impl Panel {
    fn spec<'a>(&'a self, field: Field, value: &'a str, placeholder: bool) -> FieldSpec<'a> {
        FieldSpec {
            field,
            focused: self.focus == Some(field),
            typed: &self.edit,
            value,
            placeholder,
        }
    }
}

/// The panel's draw function, called by `ds2-overlay` once per frame.
fn draw(ui: &Ui) {
    let Ok(mut guard) = PANEL.try_lock() else {
        return;
    };
    let Some(panel) = guard.as_mut() else {
        return;
    };
    if panel.phase != Phase::Open {
        return;
    }
    let display = ui.io().display_size;
    let line = ui.current_font_size();
    let row = line + 8.0;
    let width = (display[0] * 0.82)
        .clamp(760.0, 1500.0)
        .min(display[0] - 32.0);
    let height = (display[1] * 0.88).min(display[1] - 32.0);
    let left = (display[0] - width) * 0.5;
    let top = (display[1] - height) * 0.5;
    let right = left + width - PAD;
    let inner = left + PAD;

    let mut canvas = Canvas {
        ui,
        list: ui.get_foreground_draw_list(),
        // In back-buffer pixels; imgui's own position is in window pixels here. See `panels::mouse`.
        mouse: ds2_overlay::panels::mouse().unwrap_or(ui.io().mouse_pos),
        line,
        row,
        targets: Vec::new(),
    };
    canvas.rect([0.0, 0.0], display, DIM_COVER);
    canvas
        .list
        .add_rect([left, top], [left + width, top + height], PANEL_BG)
        .filled(true)
        .rounding(6.0)
        .build();
    canvas.edge([left, top], [left + width, top + height], PANEL_EDGE);
    // The panel's own surface swallows clicks, so a click on empty panel space is not "outside".
    canvas
        .targets
        .push(([left, top], [left + width, top + height], Action::Nothing));

    // Header.
    let mut y = top + PAD;
    canvas.text([inner, y], TITLE, "Build Recommender");
    let close = "[ close ]";
    let close_x = right - canvas.width(close) - 16.0;
    canvas.button(close_x, y - 4.0, close, false, Some(Action::Close));
    y += line + 4.0;
    if backend().is_stub() {
        canvas.text(
            [inner, y],
            WARN,
            "Stub backend: every number here is a placeholder until the ranking is ported.",
        );
        y += line + 6.0;
    }

    // The nine stats.
    let number_width = canvas.width("999") + 16.0;
    let mut x = inner;
    for (index, label) in STAT_LABELS.iter().enumerate() {
        canvas.text([x, y + 4.0], DIM, label);
        x += canvas.width(label) + 4.0;
        let value = panel.state.stats[index].to_string();
        let spec = panel.spec(Field::Stat(index), &value, false);
        x = canvas.field(x, y, number_width, spec) + GAP * 1.5;
    }
    y += row + GAP;

    // The soul level, its override, and the character's stats.
    let computed = panel.state.computed_sl();
    let mut x = inner;
    let sl_text = format!("SL {computed}");
    canvas.text([x, y + 4.0], TITLE, &sl_text);
    x += canvas.width(&sl_text) + GAP * 2.0;
    canvas.text([x, y + 4.0], DIM, "override");
    x += canvas.width("override") + 4.0;
    let override_text = panel
        .state
        .sl_override
        .map_or_else(|| "none".to_owned(), |sl| sl.to_string());
    let spec = panel.spec(
        Field::SlOverride,
        &override_text,
        panel.state.sl_override.is_none(),
    );
    x = canvas.field(x, y, canvas.width("none") + 24.0, spec) + GAP;
    let runs_at = format!("runs at SL {}", panel.state.sl());
    canvas.text([x, y + 4.0], DIM, &runs_at);
    x += canvas.width(&runs_at) + GAP * 2.0;
    canvas.button(
        x,
        y,
        "use my character's stats",
        false,
        Some(Action::UseCharacter),
    );
    y += row + GAP;

    // The weapon, its infusion, and the objective.
    let mut x = inner;
    canvas.text([x, y + 4.0], DIM, "Weapon");
    x += canvas.width("Weapon") + 4.0;
    let weapon_name = panel
        .state
        .weapon
        .and_then(weapons::by_key)
        .map_or("click and type to search", |row| row.name);
    let weapon_field_x = x;
    let weapon_field_w = (width * 0.30).max(260.0);
    let spec = panel.spec(Field::Search, weapon_name, panel.state.weapon.is_none());
    x = canvas.field(x, y, weapon_field_w, spec) + GAP;
    let weapon_list_y = y + row + 2.0;
    canvas.text([x, y + 4.0], DIM, "Infusion");
    x += canvas.width("Infusion") + 4.0;
    let infusion_x = x;
    let infusion_label = format!("{} v", weapons::display_name(panel.state.infusion));
    x = canvas.button(
        x,
        y,
        &infusion_label,
        panel.list == Some(List::Infusion),
        Some(Action::OpenList(List::Infusion)),
    ) + GAP * 2.0;
    canvas.text([x, y + 4.0], DIM, "Objective");
    x += canvas.width("Objective") + 4.0;
    for objective in Objective::ALL {
        x = canvas.button(
            x,
            y,
            objective.label(),
            panel.state.objective == objective,
            Some(Action::SetObjective(objective)),
        ) + 4.0;
    }
    y += row + GAP * 1.5;

    // The mode tabs.
    let mut x = inner;
    for mode in Mode::ALL {
        x = canvas.button(
            x,
            y,
            mode.label(),
            panel.state.mode == mode,
            Some(Action::SetMode(mode)),
        ) + 4.0;
    }
    y += row + GAP;

    // The mode's options, and Run.
    let class_x = draw_options(panel, &mut canvas, inner, y);
    let run_label = "Run";
    let run_x = right - canvas.width(run_label) - 16.0;
    canvas.button(
        run_x,
        y,
        run_label,
        false,
        panel.state.ready().then_some(Action::Run),
    );
    let class_list_y = y + row + 2.0;
    y += row + GAP;

    // Calibration.
    let calibration = panel.calibration;
    let calibration_text = if calibration.n == 0 {
        "calibration: none yet -- the stub has no corpus to check its infusion picks against"
            .to_owned()
    } else {
        format!(
            "calibration: the best infusion matched real builds {:.0}% of the time, top two {:.0}% \
             (n={})",
            calibration.top1 * 100.0,
            calibration.top2 * 100.0,
            calibration.n
        )
    };
    canvas.text([inner, y], DIM, &calibration_text);
    y += line + GAP;

    // The footer is laid out from the bottom so the content area gets whatever is left.
    let hint_y = top + height - PAD - line;
    let footer_y = hint_y - GAP - row;
    let content_top = y;
    let content_bottom = footer_y - GAP;
    canvas
        .list
        .add_line(
            [inner, content_top - 4.0],
            [right, content_top - 4.0],
            PANEL_EDGE,
        )
        .build();

    let results_rect = ([inner, content_top], [right, content_bottom]);
    if panel.shown == Shown::Build
        && let Some(build) = panel.generated.clone()
    {
        draw_build(&mut canvas, &build, results_rect);
    } else {
        draw_answer(panel, &mut canvas, results_rect);
    }

    // Footer.
    let mut x = inner;
    x = canvas.button(x, footer_y, "Generate Build", false, Some(Action::Generate)) + GAP;
    if panel.generated.is_some() {
        let (label, shown) = if panel.shown == Shown::Build {
            ("Show results", Shown::Answer)
        } else {
            ("Show build", Shown::Build)
        };
        x = canvas.button(x, footer_y, label, false, Some(Action::Show(shown))) + GAP;
    }
    x = canvas.button(
        x,
        footer_y,
        "Apply to character...",
        false,
        panel.generated.is_some().then_some(Action::AskApply),
    ) + GAP * 2.0;
    if let Some(status) = &panel.status {
        canvas.text([x, footer_y + 4.0], GOOD, &clip(ui, status, right - x));
    }
    canvas.text(
        [inner, hint_y],
        DIM,
        &clip(
            ui,
            "Click a field and type; Tab moves through the stats, Enter keeps, Esc leaves the \
             field, then the panel. Up/Down scroll the results.",
            right - inner,
        ),
    );

    // The open list, over everything else.
    let mut list_rect = None;
    match panel.list {
        Some(List::Weapon) => {
            let filter = if panel.focus == Some(Field::Search) {
                panel.edit.clone()
            } else {
                String::new()
            };
            let rows: Vec<(String, Action)> = weapons::search(&filter)
                .into_iter()
                .map(|row| {
                    let label = if row.class.is_empty() {
                        row.name.to_owned()
                    } else {
                        format!("{}  ({})", row.name, row.class)
                    };
                    (label, Action::ChooseWeapon(row.key))
                })
                .collect();
            list_rect = Some(draw_list(
                &mut canvas,
                [weapon_field_x, weapon_list_y],
                weapon_field_w,
                &rows,
                &mut panel.list_scroll,
                "no weapon matches",
            ));
        }
        Some(List::Infusion) => {
            let offered = panel
                .state
                .weapon
                .map(weapons::infusions_for)
                .unwrap_or_else(|| {
                    "NMFLDPBREU"
                        .chars()
                        .filter_map(weapons::infusion_for_code)
                        .collect()
                });
            let rows: Vec<(String, Action)> = offered
                .into_iter()
                .map(|infusion| {
                    (
                        weapons::display_name(infusion).to_owned(),
                        Action::ChooseInfusion(infusion),
                    )
                })
                .collect();
            let list_width = canvas.width("Enchanted") + 60.0;
            list_rect = Some(draw_list(
                &mut canvas,
                [infusion_x, weapon_list_y],
                list_width,
                &rows,
                &mut panel.list_scroll,
                "",
            ));
        }
        Some(List::Class) => {
            let rows: Vec<(String, Action)> =
                std::iter::once(("All classes".to_owned(), Action::ChooseClass(None)))
                    .chain(
                        weapons::weapon_classes()
                            .into_iter()
                            .map(|class| (class.to_owned(), Action::ChooseClass(Some(class)))),
                    )
                    .collect();
            let list_width = canvas.width("Curved Greatsword") + 60.0;
            list_rect = Some(draw_list(
                &mut canvas,
                [class_x, class_list_y],
                list_width,
                &rows,
                &mut panel.list_scroll,
                "",
            ));
        }
        None => {}
    }

    // The confirm, over everything: only its own two buttons can be clicked.
    if panel.confirming
        && let Some(build) = &panel.generated
    {
        canvas.targets.clear();
        draw_confirm(&mut canvas, build, display);
    }

    // Mouse.
    let wheel = ui.io().mouse_wheel;
    if wheel != 0.0 {
        let steps: isize = if wheel > 0.0 { -3 } else { 3 };
        let scroll = |value: &mut usize| {
            *value = value.saturating_add_signed(steps);
        };
        if let Some((min, max)) = list_rect
            && canvas.inside(min, max)
        {
            scroll(&mut panel.list_scroll);
        } else if canvas.inside(results_rect.0, results_rect.1) {
            scroll(&mut panel.results_scroll);
        }
    }
    let targets = std::mem::take(&mut canvas.targets);
    if ui.is_mouse_clicked(MouseButton::Left) {
        // Topmost first: the last rectangle drawn under the mouse is the one the player sees.
        let hit = targets
            .iter()
            .rev()
            .find(|(min, max, _)| canvas.inside(*min, *max))
            .map(|(_, _, action)| *action);
        match hit {
            Some(action) => panel.act(action),
            None if panel.confirming => {}
            None => {
                panel.commit_focus();
                panel.list = None;
            }
        }
    }
    if ui.is_mouse_clicked(MouseButton::Right) {
        if panel.confirming {
            panel.confirming = false;
        } else {
            panel.commit_focus();
            panel.list = None;
        }
    }
}

/// The current mode's options, on one line from `x`. Returns where the class list hangs from.
fn draw_options(panel: &Panel, canvas: &mut Canvas<'_>, x: f32, y: f32) -> f32 {
    let mut x = x;
    let mut class_x = x;
    match panel.state.mode {
        Mode::WeaponsForStats => {
            let opts = &panel.state.weapons_for;
            x = canvas.check(
                x,
                y,
                "one-handed only",
                opts.one_hand,
                Action::ToggleOneHand,
            ) + GAP;
            class_x = x;
            let class = format!("{} v", opts.class.as_deref().unwrap_or("All classes"));
            x = canvas.button(
                x,
                y,
                &class,
                panel.list == Some(List::Class),
                Some(Action::OpenList(List::Class)),
            ) + GAP;
            x = canvas.check(
                x,
                y,
                "best per class",
                opts.per_class,
                Action::TogglePerClass,
            ) + GAP;
            canvas.text([x, y + 4.0], DIM, "R1 window s");
            x += canvas.width("R1 window s") + 4.0;
            let window = format!("{:.1}", opts.window_s);
            let spec = panel.spec(Field::Window, &window, false);
            x = canvas.field(x, y, canvas.width("10.0") + 20.0, spec) + GAP;
            canvas.check(x, y, "raw AR", opts.raw_ar, Action::ToggleRawAr);
        }
        Mode::OptimizeForWeapon => {
            let text = if panel.state.weapon.is_some() {
                format!(
                    "the weapon, infusion and objective above, at SL {}",
                    panel.state.sl()
                )
            } else {
                "choose a weapon above".to_owned()
            };
            canvas.text([x, y + 4.0], DIM, &text);
        }
        Mode::MinimumForWeapon => {
            x = canvas.check(
                x,
                y,
                "may two-hand for strength",
                panel.state.two_hand,
                Action::ToggleTwoHand,
            ) + GAP;
            if panel.state.weapon.is_none() {
                canvas.text([x, y + 4.0], DIM, "choose a weapon above");
            }
        }
        Mode::SimilarBuilds => {
            canvas.text([x, y + 4.0], DIM, "neighbours");
            x += canvas.width("neighbours") + 4.0;
            let k = panel.state.similar_k.to_string();
            let spec = panel.spec(Field::SimilarK, &k, false);
            x = canvas.field(x, y, canvas.width("999") + 20.0, spec) + GAP;
            x = canvas.check(
                x,
                y,
                "bleed only",
                panel.state.status.bleed,
                Action::ToggleBleed,
            ) + GAP;
            canvas.check(
                x,
                y,
                "poison only",
                panel.state.status.poison,
                Action::TogglePoison,
            );
        }
    }
    class_x
}

/// The answer to the last Run, or why there is none.
fn draw_answer(panel: &mut Panel, canvas: &mut Canvas<'_>, (min, max): ([f32; 2], [f32; 2])) {
    let line = canvas.line;
    let mut y = min[1];
    if !panel.refused.is_empty() {
        canvas.text([min[0], y], WARN, "Generate Build refused:");
        for reason in &panel.refused {
            y += line + 2.0;
            canvas.text([min[0] + 12.0, y], TEXT, reason);
        }
        return;
    }
    match &panel.answer {
        None => {
            let text = if panel.state.ready() {
                "Press Run to ask."
            } else {
                "Choose a weapon, then press Run."
            };
            canvas.text([min[0], y], DIM, text);
        }
        Some(Answer::Nothing(why)) => canvas.text([min[0], y], DIM, why),
        Some(Answer::FloorViolations(lines)) => {
            canvas.text(
                [min[0], y],
                WARN,
                "These stats are under this soul level's floors, so nothing is recommended for them:",
            );
            for violation in lines {
                y += line + 2.0;
                canvas.text([min[0] + 12.0, y], TEXT, violation);
            }
        }
        Some(Answer::Build(build)) => {
            canvas.text(
                [min[0], y],
                TITLE,
                &format!(
                    "{}, SL {}{}",
                    build.class,
                    build.sl,
                    if build.two_handed { ", two-handed" } else { "" }
                ),
            );
            y += line + 4.0;
            canvas.text([min[0], y], TEXT, &stats_line(&build.stats));
            y += line + 4.0;
            canvas.text(
                [min[0], y],
                DIM,
                &format!("objective value {:.0}", build.value),
            );
        }
        Some(Answer::Rows(rows)) => {
            let rows = rows.clone();
            let score = match panel.state.mode {
                Mode::SimilarBuilds => "builds",
                _ if panel.state.weapons_for.raw_ar => "AR",
                _ => "damage",
            };
            draw_table(canvas, &rows, score, &mut panel.results_scroll, (min, max));
        }
    }
}

/// A ranking as a table, scrolled to `scroll`.
fn draw_table(
    canvas: &mut Canvas<'_>,
    rows: &[ResultRow],
    score: &str,
    scroll: &mut usize,
    (min, max): ([f32; 2], [f32; 2]),
) {
    let line = canvas.line;
    let step = line + 4.0;
    let width = max[0] - min[0];
    // Column starts, as fractions of the table's width.
    let columns: [(&str, f32); 14] = [
        ("#", 0.0),
        ("weapon", 0.03),
        ("infusion", 0.25),
        (score, 0.34),
        (DAMAGE_TYPES[0], 0.41),
        (DAMAGE_TYPES[1], 0.47),
        (DAMAGE_TYPES[2], 0.53),
        (DAMAGE_TYPES[3], 0.59),
        (DAMAGE_TYPES[4], 0.65),
        ("grip", 0.71),
        ("HA", 0.79),
        ("ctr", 0.84),
        ("class", 0.89),
        ("", 1.0),
    ];
    for (title, at) in &columns[..13] {
        canvas.text([min[0] + width * at, min[1]], DIM, title);
    }
    let visible = (((max[1] - min[1]) / step).floor() as usize).saturating_sub(1);
    *scroll = (*scroll).min(rows.len().saturating_sub(visible));
    let mut y = min[1] + step;
    for (index, row) in rows.iter().enumerate().skip(*scroll).take(visible) {
        let row_min = [min[0] - 4.0, y - 2.0];
        let row_max = [max[0] + 4.0, y + step - 2.0];
        if canvas.inside(row_min, row_max) {
            canvas.rect(row_min, row_max, ROW_HOVER);
        }
        let cell = |column: usize| min[0] + width * columns[column].1;
        let room = |column: usize| width * (columns[column + 1].1 - columns[column].1) - 6.0;
        let optional =
            |value: Option<f32>| value.map_or_else(|| "-".to_owned(), |v| format!("x{v:.2}"));
        let cells = [
            (index + 1).to_string(),
            row.weapon.clone(),
            weapons::display_name(row.infusion).to_owned(),
            format!("{:.0}", row.damage),
            format!("{:.0}", row.ar_by_type[0]),
            format!("{:.0}", row.ar_by_type[1]),
            format!("{:.0}", row.ar_by_type[2]),
            format!("{:.0}", row.ar_by_type[3]),
            format!("{:.0}", row.ar_by_type[4]),
            row.grip.clone(),
            optional(row.hyperarmor),
            optional(row.counter),
            row.class.clone(),
        ];
        for (column, text) in cells.iter().enumerate() {
            let color = if column == 9 && row.two_hand_only {
                WARN
            } else {
                TEXT
            };
            canvas.text(
                [cell(column), y],
                color,
                &clip(canvas.ui, text, room(column)),
            );
        }
        y += step;
    }
    if rows.len() > visible {
        let shown_to = (*scroll + visible).min(rows.len());
        canvas.text(
            [min[0], max[1] - line],
            DIM,
            &format!("{}-{shown_to} of {}", *scroll + 1, rows.len()),
        );
    }
}

/// The generated build.
fn draw_build(canvas: &mut Canvas<'_>, build: &GeneratedBuild, (min, max): ([f32; 2], [f32; 2])) {
    let line = canvas.line;
    let step = line + 4.0;
    let mut y = min[1];
    let (primary_key, infusion) = &build.primary;
    let primary = weapons::by_key(primary_key).map_or(primary_key.as_str(), |row| row.name);
    canvas.text(
        [min[0], y],
        TITLE,
        &format!(
            "{}, SL {} -- {} ({}), {}{}",
            build.class,
            build.sl,
            primary,
            weapons::display_name(*infusion),
            if build.two_handed {
                "two-handed"
            } else {
                "one-handed"
            },
            if build.stub { "   [stub build]" } else { "" }
        ),
    );
    y += step;
    canvas.text([min[0], y], TEXT, &stats_line(&build.stats));
    y += step;
    let armor = if build.armor.is_empty() {
        "none".to_owned()
    } else {
        build.armor.join(", ")
    };
    canvas.text([min[0], y], TEXT, &format!("Armor: {armor}"));
    y += step;
    // The four ring slots as Apply equips them; the spare copies are Apply's business, not this line's.
    let equipped = (0..backend::SUGGESTED_RINGS)
        .map(|slot| {
            build
                .suggested_rings
                .get(slot)
                .map_or("empty", String::as_str)
        })
        .collect::<Vec<_>>()
        .join(", ");
    canvas.text(
        [min[0], y],
        TEXT,
        &clip(canvas.ui, &format!("Rings: {equipped}"), max[0] - min[0]),
    );
    y += step;
    if !build.common_rings.is_empty() {
        canvas.text(
            [min[0], y],
            DIM,
            &clip(
                canvas.ui,
                &format!(
                    "Common to most builds, one each: {}",
                    build.common_rings.join(", ")
                ),
                max[0] - min[0],
            ),
        );
        y += step;
    }
    y += 4.0;
    let half = (max[0] - min[0]) * 0.5;
    let weapon_text = |row: &ResultRow| {
        format!(
            "{} ({}) {:.0}",
            row.weapon,
            weapons::display_name(row.infusion),
            row.damage
        )
    };
    canvas.text(
        [min[0], y],
        DIM,
        &format!("One-handed ({})", build.weapons_1h.len()),
    );
    canvas.text(
        [min[0] + half, y],
        DIM,
        &format!("Two-hand only ({})", build.weapons_2h_only.len()),
    );
    y += step;
    let rows_fit = ((max[1] - y) / step).floor().max(0.0) as usize;
    // The one-handed list takes two columns of the left half when it would not fit in one.
    let left_columns = if build.weapons_1h.len() > rows_fit {
        2
    } else {
        1
    };
    let column_width = half / left_columns as f32;
    for (index, row) in build.weapons_1h.iter().enumerate() {
        let (column, line_index) = (index / rows_fit.max(1), index % rows_fit.max(1));
        if column >= left_columns {
            break;
        }
        canvas.text(
            [
                min[0] + column_width * column as f32,
                y + step * line_index as f32,
            ],
            TEXT,
            &clip(canvas.ui, &weapon_text(row), column_width - 8.0),
        );
    }
    for (index, row) in build.weapons_2h_only.iter().take(rows_fit).enumerate() {
        canvas.text(
            [min[0] + half, y + step * index as f32],
            TEXT,
            &clip(canvas.ui, &weapon_text(row), half - 8.0),
        );
    }
}

/// The question Apply asks before anything happens to the character.
fn draw_confirm(canvas: &mut Canvas<'_>, build: &GeneratedBuild, display: [f32; 2]) {
    let line = canvas.line;
    let (import, extras) = backend::to_import(build);
    let grants = import
        .weapons
        .chunks(2)
        .map(|pair| &pair[0])
        .chain(&import.armor)
        .chain(&import.rings)
        .filter(|name| !ds2_build_import_core::is_empty_slot(name))
        .count()
        + extras.len();
    let lines = [
        (
            TITLE,
            format!(
                "Apply {} SL {} to the character you are playing?",
                build.class, build.sl
            ),
        ),
        (
            WARN,
            format!(
                "Soul memory is raised to what soul level {} needs. That cannot be undone.",
                build.sl
            ),
        ),
        (
            TEXT,
            "The nine stats are rewritten; a build below this character keeps its stats."
                .to_owned(),
        ),
        (
            TEXT,
            format!(
                "Up to {grants} items are granted and the build's gear is equipped; what you \
                 already hold is not granted again."
            ),
        ),
    ];
    let width = lines
        .iter()
        .map(|(_, text)| canvas.width(text))
        .fold(0.0_f32, f32::max)
        + PAD * 2.0;
    let height = PAD * 2.0 + (line + 6.0) * lines.len() as f32 + canvas.row + GAP;
    let min = [(display[0] - width) * 0.5, (display[1] - height) * 0.5];
    let max = [min[0] + width, min[1] + height];
    canvas
        .list
        .add_rect(min, max, LIST_BG)
        .filled(true)
        .rounding(6.0)
        .build();
    canvas.edge(min, max, WARN);
    canvas.targets.push((min, max, Action::Nothing));
    let mut y = min[1] + PAD;
    for (color, text) in &lines {
        canvas.text([min[0] + PAD, y], *color, text);
        y += line + 6.0;
    }
    y += GAP;
    let x = canvas.button(
        min[0] + PAD,
        y,
        "Apply, and raise soul memory",
        false,
        Some(Action::ConfirmApply),
    ) + GAP;
    canvas.button(x, y, "Cancel", false, Some(Action::CancelApply));
}

/// A drop-down list hanging from `at`, scrolled to `scroll`. Returns its rectangle.
fn draw_list(
    canvas: &mut Canvas<'_>,
    at: [f32; 2],
    width: f32,
    rows: &[(String, Action)],
    scroll: &mut usize,
    empty: &str,
) -> ([f32; 2], [f32; 2]) {
    const VISIBLE: usize = 12;
    let step = canvas.line + 6.0;
    *scroll = (*scroll).min(rows.len().saturating_sub(VISIBLE));
    let shown = rows.len().clamp(1, VISIBLE);
    let min = at;
    let max = [at[0] + width, at[1] + step * shown as f32 + 8.0];
    canvas
        .list
        .add_rect(min, max, LIST_BG)
        .filled(true)
        .rounding(3.0)
        .build();
    canvas.edge(min, max, PANEL_EDGE);
    canvas.targets.push((min, max, Action::Nothing));
    if rows.is_empty() {
        canvas.text([at[0] + 8.0, at[1] + 4.0], DIM, empty);
        return (min, max);
    }
    let mut y = at[1] + 4.0;
    for (label, action) in rows.iter().skip(*scroll).take(VISIBLE) {
        let row_min = [at[0] + 2.0, y - 1.0];
        let row_max = [at[0] + width - 2.0, y + step - 1.0];
        if canvas.inside(row_min, row_max) {
            canvas.rect(row_min, row_max, ROW_HOVER);
        }
        canvas.text(
            [at[0] + 8.0, y + 2.0],
            TEXT,
            &clip(canvas.ui, label, width - 16.0),
        );
        canvas.targets.push((row_min, row_max, *action));
        y += step;
    }
    if rows.len() > VISIBLE {
        let more = format!("{}/{}", (*scroll + VISIBLE).min(rows.len()), rows.len());
        let more_x = max[0] - canvas.width(&more) - 8.0;
        canvas.text([more_x, max[1] - step + 2.0], DIM, &more);
    }
    (min, max)
}

/// `VIG 10  END 16  ...`.
fn stats_line(stats: &[u16; STAT_COUNT]) -> String {
    STAT_LABELS
        .iter()
        .zip(stats)
        .map(|(label, value)| format!("{label} {value}"))
        .collect::<Vec<_>>()
        .join("   ")
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

/// `text`, cut at the left to fit, so the end being typed stays in view.
fn clip_left(ui: &Ui, text: &str, width: f32) -> String {
    if ui.calc_text_size(text)[0] <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let candidate: String = chars[start..].iter().collect();
        if ui.calc_text_size(&candidate)[0] <= width {
            return candidate;
        }
    }
    String::new()
}
