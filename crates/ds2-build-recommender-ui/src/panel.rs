//! The panel: its state between frames, the keyboard half, and the drawing and mouse half.
//!
//! One `Mutex` because Rust cannot know what the picker's module docs establish: DARK SOULS II
//! presents from its simulation thread, so [`open`], `on_frame` and `draw` all run on one thread and
//! the lock is never contended.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use ds2_build_import_core::Infusion;
use ds2_build_recommender_core::backend::{
    self, Answer, Calibration, DAMAGE_TYPES, GeneratedBuild, RecommenderBackend, ResultRow,
    StubBackend,
};
use ds2_build_recommender_core::corpus::{self, CorpusBackend};
use ds2_build_recommender_core::flex::{Flexibility, flex_line, flex_load_line};
use ds2_build_recommender_core::model::{
    Grip, Mode, Objective, PanelState, STAT_COUNT, STAT_LABELS,
};

/// STR's place in the nine stats, for the log lines that say what a grip did to it.
const STR_INDEX: usize = 4;
use ds2_build_recommender_core::nav::{self, Control, Dir, Nudge, Shape};
use ds2_build_recommender_core::weapons;
use hudhook::imgui::{DrawListMut, MouseButton, Ui};

use crate::input::{Press, Reader};
use crate::session::{Opened, Phase, Session};
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

/// Who answers: the ported ranking over the data file beside the game, once [`install`] has read
/// it. Until then, or when there is no file to read, [`STUB`].
static CORPUS: OnceLock<CorpusBackend> = OnceLock::new();

/// The fixed answers the panel falls back to without a data file. The panel says they are.
static STUB: StubBackend = StubBackend;

fn backend() -> &'static dyn RecommenderBackend {
    match CORPUS.get() {
        Some(corpus) => corpus,
        None => &STUB,
    }
}

/// Ask the backend how flexible a build is, and say in the log what it answered: the panel's line
/// word for word, or why there is none.
fn ask_flexibility(
    stats: &[u16; STAT_COUNT],
    sl: u16,
    armor: &[String],
    rings: &[String],
) -> Option<Flexibility> {
    let flex = backend().flexibility(stats, sl, armor, rings);
    match &flex {
        Some(flex) => log_line(format_args!(
            "{LOG_PREFIX} flexibility SL {sl}: {} | {}",
            flex_line(flex),
            flex_load_line(flex)
        )),
        None => log_line(format_args!(
            "{LOG_PREFIX} flexibility SL {sl}: none -- the stub, or a data file without neighbour \
             counts"
        )),
    }
    flex
}

/// Read `<Game>/ds2-build-recommender.dat`, beside the running executable, into [`CORPUS`], and
/// say in the log which backend the panel answers with and why.
fn load_backend() {
    let path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(corpus::DATA_FILE_NAME)));
    let Some(path) = path else {
        log_line(format_args!(
            "{LOG_PREFIX} no game directory to read {} from -- backend=stub",
            corpus::DATA_FILE_NAME
        ));
        return;
    };
    match CorpusBackend::load(&path) {
        Ok(loaded) => {
            let builds = loaded.corpus_len();
            if CORPUS.set(loaded).is_ok() {
                log_line(format_args!(
                    "{LOG_PREFIX} backend=corpus builds={builds} data={}",
                    path.display()
                ));
            }
        }
        Err(error) => log_line(format_args!(
            "{LOG_PREFIX} {} unreadable ({error}) -- backend=stub, every number a placeholder",
            path.display()
        )),
    }
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

    /// The cursor's name for this field.
    const fn control(self) -> Control {
        match self {
            Field::Stat(index) => Control::Stat(index),
            Field::SlOverride => Control::SlOverride,
            Field::Search => Control::Weapon,
            Field::Window => Control::Window,
            Field::SimilarK => Control::SimilarK,
        }
    }
}

/// Rows a drop-down list shows at once.
const LIST_VISIBLE: usize = 12;

/// Rows LB and RB move the results table.
const RESULTS_PAGE: usize = 10;

/// A drop-down list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    Weapon,
    Infusion,
    Class,
    /// The spells Generate Build must cast. Choosing a row toggles it and leaves the list open.
    Spell,
}

/// What the lower half shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    /// The current mode's answer.
    Answer,
    /// The generated build.
    Build,
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
    SetGrip(Grip),
    SetMode(Mode),
    ToggleOneHand,
    TogglePerClass,
    ToggleRawAr,
    ToggleTwoHand,
    /// Generate Build: whether the build may leave the armour off.
    ToggleAllowNaked,
    /// Generate Build: add or take out one spell, by index into the backend's spell table.
    ToggleSpell(usize),
    /// Generate Build: no spells.
    ClearSpells,
    ToggleBleed,
    TogglePoison,
    Run,
    /// Rank every infusion of the chosen weapon at the panel's stats.
    BestInfusion,
    UseCharacter,
    Generate,
    Show(Shown),
    AskApply,
    ConfirmApply,
    CancelApply,
}

struct Panel {
    state: PanelState,
    /// A press or click asked to close; the session takes it off the screen after this frame.
    wants_close: bool,
    reader: Reader,
    focus: Option<Field>,
    /// What has been typed into the focused field since it was focused.
    edit: String,
    /// The weapon search as last typed, put back into the field when it is focused again.
    search: String,
    /// Whether anything was typed, so focusing and leaving a field changes nothing.
    edited: bool,
    list: Option<List>,
    list_scroll: usize,
    /// The highlighted row of the open list, which A chooses.
    list_cursor: usize,
    answer: Option<Answer>,
    results_scroll: usize,
    generated: Option<GeneratedBuild>,
    /// The weapon flexibility of the build in [`Self::answer`], asked once when it arrived: the
    /// neighbour search reads the whole corpus, which is not a per-frame cost.
    answer_flex: Option<Flexibility>,
    /// The same for [`Self::generated`], in its armour and rings.
    generated_flex: Option<Flexibility>,
    /// Why the last Generate Build was refused.
    refused: Vec<String>,
    shown: Shown,
    confirming: bool,
    /// In the Apply confirm, whether the cursor is on Apply rather than Cancel. Starts on Cancel.
    confirm_on_apply: bool,
    status: Option<String>,
    calibration: Calibration,
    /// The control the D-pad and arrow keys are on.
    cursor: Control,
    /// A was pressed on the results table: Up and Down scroll it until B or A.
    scrolling_results: bool,
}

impl Panel {
    fn new(reader: Reader) -> Self {
        Self {
            state: PanelState::default(),
            wants_close: false,
            reader,
            focus: None,
            edit: String::new(),
            search: String::new(),
            edited: false,
            list: None,
            list_scroll: 0,
            list_cursor: 0,
            answer: None,
            results_scroll: 0,
            generated: None,
            answer_flex: None,
            generated_flex: None,
            refused: Vec::new(),
            shown: Shown::Answer,
            confirming: false,
            confirm_on_apply: false,
            status: None,
            calibration: backend().calibration(),
            cursor: Control::Stat(0),
            scrolling_results: false,
        }
    }

    /// What decides the cursor's grid this frame.
    fn shape(&self) -> Shape {
        Shape {
            mode: self.state.mode,
            results: self.shown == Shown::Answer
                && self.refused.is_empty()
                && matches!(&self.answer, Some(Answer::Rows(rows)) if !rows.is_empty()),
            generated: self.generated.is_some(),
        }
    }

    /// The rows `list` offers, with what choosing each does. The same rows the list draws.
    fn list_rows(&self, list: List) -> Vec<(String, Action)> {
        match list {
            List::Weapon => {
                let filter = if self.focus == Some(Field::Search) {
                    self.edit.as_str()
                } else {
                    ""
                };
                weapons::search(filter)
                    .into_iter()
                    .map(|row| {
                        let label = if row.class.is_empty() {
                            row.name.to_owned()
                        } else {
                            format!("{}  ({})", row.name, row.class)
                        };
                        (label, Action::ChooseWeapon(row.key))
                    })
                    .collect()
            }
            List::Infusion => self
                .state
                .weapon
                .map(weapons::infusions_for)
                .unwrap_or_else(|| {
                    "NMFLDPBREU"
                        .chars()
                        .filter_map(weapons::infusion_for_code)
                        .collect()
                })
                .into_iter()
                .map(|infusion| {
                    (
                        weapons::display_name(infusion).to_owned(),
                        Action::ChooseInfusion(infusion),
                    )
                })
                .collect(),
            List::Class => std::iter::once(("All classes".to_owned(), Action::ChooseClass(None)))
                .chain(
                    weapons::weapon_classes()
                        .into_iter()
                        .map(|class| (class.to_owned(), Action::ChooseClass(Some(class)))),
                )
                .collect(),
            List::Spell => {
                let spells = backend().spells();
                if spells.is_empty() {
                    return Vec::new();
                }
                let chosen = &self.state.spells;
                std::iter::once((
                    format!("No spells ({} chosen)", chosen.len()),
                    Action::ClearSpells,
                ))
                .chain(spells.iter().enumerate().map(|(index, spell)| {
                    let mark = if chosen.contains(&spell.key) {
                        "[x]"
                    } else {
                        "[ ]"
                    };
                    (
                        format!(
                            "{mark} {}  {} slot{}  INT {} FTH {}",
                            spell.name,
                            spell.slots,
                            if spell.slots == 1 { "" } else { "s" },
                            spell.intelligence,
                            spell.faith
                        ),
                        Action::ToggleSpell(index),
                    )
                }))
                .collect()
            }
        }
    }

    /// What `control` holds right now, for the log: `VIG 12`, `mode=SimilarBuilds`, `bleed=true`.
    fn describe(&self, control: Control) -> String {
        let state = &self.state;
        let opts = &state.weapons_for;
        match control {
            Control::Close => "close".to_owned(),
            Control::Stat(index) => format!(
                "{} {}",
                STAT_LABELS.get(index).copied().unwrap_or("?"),
                state.stats.get(index).copied().unwrap_or(0)
            ),
            Control::SlOverride => format!(
                "sl-override={} (runs at SL {})",
                state
                    .sl_override
                    .map_or_else(|| "none".to_owned(), |sl| sl.to_string()),
                state.sl()
            ),
            Control::UseCharacter => "use my character's stats".to_owned(),
            Control::Weapon => format!("weapon={}", state.weapon.unwrap_or("none")),
            Control::Infusion => format!("infusion={}", weapons::display_name(state.infusion)),
            Control::Objective(objective) => {
                format!("objective {objective:?} (selected {:?})", state.objective)
            }
            Control::Grip(grip) => format!("grip {grip:?} (selected {:?})", state.grip),
            Control::Mode(mode) => format!("mode tab {mode:?} (selected {:?})", state.mode),
            Control::OneHand => format!("one-hand={}", opts.one_hand),
            Control::Class => format!("class={}", opts.class.as_deref().unwrap_or("all")),
            Control::PerClass => format!("per-class={}", opts.per_class),
            Control::Window => format!("window={:.1}s", opts.window_s),
            Control::RawAr => format!("raw-ar={}", opts.raw_ar),
            Control::TwoHand => format!("two-hand={}", state.two_hand),
            Control::SimilarK => format!("neighbours={}", state.similar_k),
            Control::Bleed => format!("bleed-only={}", state.status.bleed),
            Control::Poison => format!("poison-only={}", state.status.poison),
            Control::BestInfusion => format!("best infusion (weapon={})", state.weapon.is_some()),
            Control::Run => format!("run (ready={})", state.ready()),
            Control::Results => format!("results scroll={}", self.results_scroll),
            Control::Generate => "generate build".to_owned(),
            Control::AllowNaked => format!("allow-no-armor={}", state.allow_naked),
            Control::Spells => format!("spells={:?}", state.spells),
            Control::ShowToggle => format!("showing {:?}", self.shown),
            Control::Apply => format!("apply (build ready={})", self.generated.is_some()),
        }
    }

    /// Move the cursor one step, and say where it landed.
    fn move_cursor(&mut self, dir: Dir) {
        let rows = nav::layout(self.shape());
        let next = nav::step(&rows, self.cursor, dir);
        if next != self.cursor {
            self.cursor = next;
            log_line(format_args!(
                "{LOG_PREFIX} cursor {dir:?} -> {next:?} [{}]",
                self.describe(next)
            ));
        }
    }

    /// A (or Enter) on the control under the cursor.
    fn activate(&mut self) {
        let rows = nav::layout(self.shape());
        self.cursor = nav::resolve(&rows, self.cursor);
        let control = self.cursor;
        let action = match control {
            Control::Close => Some(Action::Close),
            Control::Stat(index) => Some(Action::Focus(Field::Stat(index))),
            Control::SlOverride => Some(Action::Focus(Field::SlOverride)),
            Control::Window => Some(Action::Focus(Field::Window)),
            Control::SimilarK => Some(Action::Focus(Field::SimilarK)),
            Control::Weapon => Some(Action::Focus(Field::Search)),
            Control::UseCharacter => Some(Action::UseCharacter),
            Control::Infusion => Some(Action::OpenList(List::Infusion)),
            Control::Class => Some(Action::OpenList(List::Class)),
            Control::Objective(objective) => Some(Action::SetObjective(objective)),
            Control::Grip(grip) => Some(Action::SetGrip(grip)),
            Control::Mode(mode) => Some(Action::SetMode(mode)),
            Control::OneHand => Some(Action::ToggleOneHand),
            Control::PerClass => Some(Action::TogglePerClass),
            Control::RawAr => Some(Action::ToggleRawAr),
            Control::TwoHand => Some(Action::ToggleTwoHand),
            Control::Bleed => Some(Action::ToggleBleed),
            Control::Poison => Some(Action::TogglePoison),
            Control::BestInfusion => self.state.weapon.is_some().then_some(Action::BestInfusion),
            Control::Run => self.state.ready().then_some(Action::Run),
            Control::Results => None,
            Control::Generate => Some(Action::Generate),
            Control::AllowNaked => Some(Action::ToggleAllowNaked),
            Control::Spells => Some(Action::OpenList(List::Spell)),
            Control::ShowToggle => Some(Action::Show(if self.shown == Shown::Build {
                Shown::Answer
            } else {
                Shown::Build
            })),
            Control::Apply => self.generated.is_some().then_some(Action::AskApply),
        };
        match (control, action) {
            (Control::Results, _) => {
                self.scrolling_results = true;
                log_line(format_args!(
                    "{LOG_PREFIX} press A on Results -- Up/Down scroll, LB/RB page, B leaves"
                ));
            }
            (_, None) => log_line(format_args!(
                "{LOG_PREFIX} press A on {control:?} -- disabled [{}]",
                self.describe(control)
            )),
            (_, Some(action)) => {
                self.act(action);
                if control.is_numeric() {
                    log_line(format_args!(
                        "{LOG_PREFIX} press A on {control:?} -- adjusting [{}]: Left/Right step \
                         one, Up/Down step ten, A or B keeps",
                        self.describe(control)
                    ));
                } else {
                    log_line(format_args!(
                        "{LOG_PREFIX} press A on {control:?} -> [{}]{}",
                        self.describe(control),
                        if self.list.is_some() {
                            " -- list open"
                        } else if self.confirming {
                            " -- confirm open"
                        } else {
                            ""
                        }
                    ));
                }
            }
        }
    }

    /// Up/Down/LB/RB in the open list: move the highlighted row, keeping it in view.
    fn move_in_list(&mut self, list: List, delta: isize) {
        let rows = self.list_rows(list);
        self.list_cursor = nav::move_in_list(self.list_cursor, delta, rows.len());
        self.list_scroll =
            nav::scroll_to(self.list_cursor, self.list_scroll, LIST_VISIBLE, rows.len());
        log_line(format_args!(
            "{LOG_PREFIX} list {list:?} row {}/{} [{}]",
            self.list_cursor + 1,
            rows.len(),
            rows.get(self.list_cursor)
                .map_or("empty", |(label, _)| label.as_str())
        ));
    }

    /// A in the open list: choose the highlighted row. An empty list just closes.
    fn choose_in_list(&mut self, list: List) {
        let rows = self.list_rows(list);
        match rows.get(self.list_cursor) {
            Some((label, action)) => {
                log_line(format_args!(
                    "{LOG_PREFIX} list {list:?} chose row {} [{label}]",
                    self.list_cursor + 1
                ));
                let action = *action;
                self.act(action);
            }
            None => {
                self.commit_focus();
                self.list = None;
            }
        }
    }

    /// Where the highlight starts when `list` opens: on what is chosen now, or the top.
    fn open_list_cursor(&mut self, list: List) {
        let rows = self.list_rows(list);
        let current = match list {
            List::Weapon | List::Spell => None,
            List::Infusion => rows
                .iter()
                .position(|(_, action)| *action == Action::ChooseInfusion(self.state.infusion)),
            List::Class => {
                let class = self.state.weapons_for.class.clone();
                rows.iter().position(|(label, action)| match &class {
                    None => *action == Action::ChooseClass(None),
                    Some(class) => label == class,
                })
            }
        };
        self.list_cursor = current.unwrap_or(0);
        self.list_scroll = nav::scroll_to(self.list_cursor, 0, LIST_VISIBLE, rows.len());
    }

    /// The inputs changed: an answer for the old ones is not shown for the new.
    fn changed(&mut self) {
        self.answer = None;
        self.answer_flex = None;
        self.results_scroll = 0;
        self.scrolling_results = false;
        self.refused.clear();
        self.status = None;
    }

    fn close(&mut self, why: &str) {
        log_line(format_args!("{LOG_PREFIX} panel closed -- {why}"));
        self.wants_close = true;
    }

    /// Show a kept panel again. Everything the player set stays -- stats, override, mode,
    /// objective, weapon, infusion, class, window, neighbours, search text, the answer and its
    /// scroll, the generated build, the cursor. Only the layers a close backed out of are shut: a
    /// field half-typed is kept as typed, an open list and the Apply confirm are closed.
    fn reopen(&mut self, reader: Reader) {
        self.commit_focus();
        self.reader = reader;
        self.wants_close = false;
        self.list = None;
        self.confirming = false;
        self.scrolling_results = false;
    }

    /// One line saying what came back, so a reopen that lost something shows in the log.
    fn log_restored(&self) {
        let state = &self.state;
        log_line(format_args!(
            "{LOG_PREFIX} panel restored: mode={:?} objective={:?} stats={:?} sl-override={:?} \
             weapon={} infusion={} class={} window={:.1}s neighbours={} search={:?} answer={} \
             results-scroll={} generated={} cursor={:?}",
            state.mode,
            state.objective,
            state.stats,
            state.sl_override,
            state.weapon.unwrap_or("none"),
            weapons::display_name(state.infusion),
            state.weapons_for.class.as_deref().unwrap_or("all"),
            state.weapons_for.window_s,
            state.similar_k,
            self.search,
            match &self.answer {
                Some(Answer::Rows(rows)) => format!("{} rows", rows.len()),
                Some(_) => "yes".to_owned(),
                None => "none".to_owned(),
            },
            self.results_scroll,
            self.generated.is_some(),
            self.cursor,
        ));
    }

    fn focus(&mut self, field: Field) {
        if self.focus == Some(field) {
            return;
        }
        self.commit_focus();
        self.focus = Some(field);
        self.cursor = field.control();
        self.edit.clear();
        self.edited = false;
        if field == Field::Search {
            // The search as it was left, so the list comes back filtered the same way.
            self.edit = self.search.clone();
            self.list = Some(List::Weapon);
            self.list_scroll = 0;
            self.list_cursor = 0;
        }
    }

    /// Leave the focused field, keeping what was typed.
    fn commit_focus(&mut self) {
        let Some(field) = self.focus.take() else {
            return;
        };
        if field == Field::Search {
            self.search = self.edit.clone();
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
            Field::Search => {
                self.list_scroll = 0;
                self.list_cursor = 0;
            }
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
                Answer::Build(build) => format!(
                    "a build: {} {} STR {} (grip asked {:?})",
                    build.class,
                    if build.two_handed {
                        "two-handed"
                    } else {
                        "one-handed"
                    },
                    build.stats[STR_INDEX],
                    self.state.grip
                ),
                Answer::FloorViolations(lines) => format!("under floors: {}", lines.join(", ")),
                Answer::Nothing(why) => (*why).to_owned(),
            }
        ));
        // An optimized or minimum build wears nothing the answer names, so its load is all spare.
        self.answer_flex = match &answer {
            Answer::Build(build) => ask_flexibility(&build.stats, build.sl, &[], &[]),
            _ => None,
        };
        self.answer = Some(answer);
        self.results_scroll = 0;
        self.shown = Shown::Answer;
    }

    /// The chosen weapon's infusions, best first, with the winner's margin over the runner-up in
    /// the status line.
    fn best_infusion(&mut self) {
        let answer = backend::best_infusion(backend(), &self.state);
        let summary = match &answer {
            Answer::Rows(rows) => {
                let name = |row: &ResultRow| weapons::display_name(row.infusion);
                match (rows.first(), rows.get(1), backend::infusion_margin(rows)) {
                    (Some(best), Some(second), Some(margin)) => format!(
                        "Best infusion: {}, {:+.1}% over {}",
                        name(best),
                        margin * 100.0,
                        name(second)
                    ),
                    (Some(best), ..) => {
                        format!("Best infusion: {} (nothing to compare)", name(best))
                    }
                    _ => "Best infusion: nothing ranked".to_owned(),
                }
            }
            Answer::Nothing(why) => format!("Best infusion: {why}"),
            _ => "Best infusion: nothing ranked".to_owned(),
        };
        log_line(format_args!(
            "{LOG_PREFIX} best infusion weapon={:?} sl={} objective={:?} -> {summary}",
            self.state.weapon,
            self.state.sl(),
            self.state.objective
        ));
        self.answer = Some(answer);
        self.results_scroll = 0;
        self.shown = Shown::Answer;
        self.status = Some(summary);
    }

    fn generate(&mut self) {
        // Any class may win: applying a build for another class changes the character's class to
        // it (ds2-build-import writes the class before the stats), so the best build is offered
        // whatever the character started as.
        match backend::generate(backend(), &self.state, None) {
            Ok(build) => {
                log_line(format_args!(
                    "{LOG_PREFIX} generated {} SL {} primary={:?} {} STR {} (grip asked {:?}) stub={}",
                    build.class,
                    build.sl,
                    build.primary,
                    if build.two_handed {
                        "two-handed"
                    } else {
                        "one-handed"
                    },
                    build.stats[STR_INDEX],
                    self.state.grip,
                    build.stub
                ));
                if !build.spells.is_empty() {
                    log_line(format_args!(
                        "{LOG_PREFIX} generated spells={:?} slots {}/{} INT {} FTH {} \
                         catalysts={:?}",
                        build.spells,
                        build.slots_used,
                        build.slots,
                        build.stats[7],
                        build.stats[8],
                        build.catalysts
                    ));
                }
                self.generated_flex =
                    ask_flexibility(&build.stats, build.sl, &build.armor, &build.suggested_rings);
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
                self.generated_flex = None;
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
        let current = ds2_build_import::character_class();
        let (build, extras) = backend::to_import(generated);
        log_line(format_args!(
            "{LOG_PREFIX} apply confirmed: {} SL {} with {} extra grants onto a {} -- handed to \
             ds2-build-import",
            generated.class,
            generated.sl,
            extras.len(),
            current.map_or_else(|| "character of unread class".to_owned(), |c| c.to_string())
        ));
        ds2_build_import::queue_generated(build, extras);
        self.status = Some(match current {
            Some(class) if !generated.class.eq_ignore_ascii_case(class.key()) => format!(
                "Queued: your {} becomes a {} on the pause menu's next frame -- the log says what \
                 changed",
                class.key(),
                generated.class
            ),
            _ => "Queued: the pause menu applies it on its next frame -- the log says what changed"
                .to_owned(),
        });
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
                | Action::ToggleSpell(_)
                | Action::ClearSpells
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
                if self.list.is_some() {
                    self.open_list_cursor(list);
                }
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
            Action::SetGrip(grip) => {
                self.state.grip = grip;
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
            Action::ToggleAllowNaked => self.state.allow_naked ^= true,
            Action::ToggleSpell(index) => {
                if let Some(spell) = backend().spells().get(index) {
                    let chosen = self.state.toggle_spell(&spell.key);
                    log_line(format_args!(
                        "{LOG_PREFIX} spell {} {} -> spells={:?}",
                        spell.key,
                        if chosen { "chosen" } else { "not chosen" },
                        self.state.spells
                    ));
                }
            }
            Action::ClearSpells => {
                self.state.spells.clear();
                log_line(format_args!("{LOG_PREFIX} spells cleared"));
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
            Action::BestInfusion => self.best_infusion(),
            // Read from the game, not the backend: the corpus backend has no game to read, and
            // asking it made this button do nothing (ds2-mods-rs-fynp).
            Action::UseCharacter => match ds2_build_import::character_stats() {
                Some(game) => {
                    let stats = backend::planner_order(&game);
                    log_line(format_args!(
                        "{LOG_PREFIX} use my character's stats: {stats:?} (class {})",
                        ds2_build_import::character_class()
                            .map_or_else(|| "unread".to_owned(), |c| c.to_string())
                    ));
                    self.state.stats = stats;
                    self.changed();
                    self.status = Some("Stats read from your character".to_owned());
                }
                None => {
                    log_line(format_args!(
                        "{LOG_PREFIX} use my character's stats: no character to read"
                    ));
                    self.status = Some("Could not read the character's stats".to_owned());
                }
            },
            Action::Generate => self.generate(),
            Action::Show(shown) => self.shown = shown,
            Action::AskApply => {
                self.confirming = self.generated.is_some();
                self.confirm_on_apply = false;
            }
            Action::ConfirmApply => self.apply(),
            Action::CancelApply => self.confirming = false,
        }
    }

    /// One press from the keyboard or pad.
    ///
    /// Four layers, innermost first: the Apply confirm, an open list, a field being typed into or
    /// adjusted, and the results table being scrolled. With none of them up the D-pad moves the
    /// cursor, A presses what it is on, and B or Start closes the panel.
    fn press(&mut self, press: Press) {
        if self.confirming {
            self.press_confirm(press);
        } else if let Some(list) = self.list
            && matches!(self.focus, None | Some(Field::Search))
        {
            self.press_list(list, press);
        } else if let Some(field) = self.focus {
            self.press_field(field, press);
        } else if self.scrolling_results {
            self.press_results(press);
        } else {
            match press {
                Press::Up => self.move_cursor(Dir::Up),
                Press::Down => self.move_cursor(Dir::Down),
                Press::Left => self.move_cursor(Dir::Left),
                Press::Right => self.move_cursor(Dir::Right),
                Press::Confirm => self.activate(),
                Press::Close | Press::Back => self.close("closed by the player"),
                Press::Tab => self.focus(Field::Stat(0)),
                Press::PageUp => self.scroll_results(-(RESULTS_PAGE as isize)),
                Press::PageDown => self.scroll_results(RESULTS_PAGE as isize),
                Press::Backspace | Press::Char(_) => {}
            }
        }
    }

    /// The Apply confirm: any direction swaps Apply and Cancel, A presses the one highlighted, B
    /// cancels. The cursor starts on Cancel, so a stray A does not raise soul memory.
    fn press_confirm(&mut self, press: Press) {
        match press {
            Press::Up | Press::Down | Press::Left | Press::Right | Press::Tab => {
                self.confirm_on_apply ^= true;
                log_line(format_args!(
                    "{LOG_PREFIX} confirm cursor -> {}",
                    if self.confirm_on_apply {
                        "Apply"
                    } else {
                        "Cancel"
                    }
                ));
            }
            Press::Confirm if self.confirm_on_apply => {
                log_line(format_args!("{LOG_PREFIX} press A on confirm Apply"));
                self.act(Action::ConfirmApply);
            }
            Press::Confirm | Press::Close | Press::Back => {
                log_line(format_args!(
                    "{LOG_PREFIX} apply cancelled from the pad or keys"
                ));
                self.act(Action::CancelApply);
            }
            Press::PageUp | Press::PageDown | Press::Backspace | Press::Char(_) => {}
        }
    }

    /// An open list: Up/Down move the highlight, LB/RB a page, A chooses, B closes. Typing still
    /// filters the weapon list while its search field is focused.
    fn press_list(&mut self, list: List, press: Press) {
        let page = LIST_VISIBLE as isize;
        match press {
            Press::Up => self.move_in_list(list, -1),
            Press::Down => self.move_in_list(list, 1),
            Press::PageUp => self.move_in_list(list, -page),
            Press::PageDown => self.move_in_list(list, page),
            Press::Confirm => self.choose_in_list(list),
            Press::Close | Press::Back => {
                log_line(format_args!("{LOG_PREFIX} list {list:?} closed"));
                self.commit_focus();
                self.list = None;
            }
            Press::Char(typed) => self.type_char(typed),
            Press::Backspace => self.backspace(),
            Press::Left | Press::Right | Press::Tab => {}
        }
    }

    /// A field: typing replaces its number, the D-pad steps it, A or B keeps it.
    fn press_field(&mut self, field: Field, press: Press) {
        let dir = match press {
            Press::Up => Some(Dir::Up),
            Press::Down => Some(Dir::Down),
            Press::Left => Some(Dir::Left),
            Press::Right => Some(Dir::Right),
            _ => None,
        };
        if let Some(dir) = dir {
            let control = field.control();
            let before = self.describe(control);
            if nav::nudge(&mut self.state, control, Nudge::from_dir(dir)) {
                self.edit.clear();
                self.edited = false;
                self.changed();
                log_line(format_args!(
                    "{LOG_PREFIX} adjust {control:?} {dir:?}: [{before}] -> [{}]",
                    self.describe(control)
                ));
            }
            return;
        }
        match press {
            Press::Char(typed) => self.type_char(typed),
            Press::Backspace => self.backspace(),
            Press::Tab => {
                let next = self.focus.and_then(Field::next);
                self.commit_focus();
                if let Some(next) = next {
                    self.focus(next);
                }
            }
            Press::Confirm | Press::Close | Press::Back => {
                self.commit_focus();
                log_line(format_args!(
                    "{LOG_PREFIX} kept {:?} [{}]",
                    field.control(),
                    self.describe(field.control())
                ));
            }
            _ => {}
        }
    }

    /// The results table after A on it: Up/Down a row, LB/RB a page, A or B leaves.
    fn press_results(&mut self, press: Press) {
        match press {
            Press::Up => self.scroll_results(-1),
            Press::Down => self.scroll_results(1),
            Press::PageUp => self.scroll_results(-(RESULTS_PAGE as isize)),
            Press::PageDown => self.scroll_results(RESULTS_PAGE as isize),
            Press::Confirm | Press::Close | Press::Back => {
                self.scrolling_results = false;
                log_line(format_args!("{LOG_PREFIX} left the results table"));
            }
            _ => {}
        }
    }

    /// Move the results table by `delta` rows, never past its last row. The draw then clamps it
    /// again to what fits on screen.
    fn scroll_results(&mut self, delta: isize) {
        let rows = match &self.answer {
            Some(Answer::Rows(rows)) => rows.len(),
            _ => 0,
        };
        self.results_scroll = self
            .results_scroll
            .saturating_add_signed(delta)
            .min(rows.saturating_sub(1));
        log_line(format_args!(
            "{LOG_PREFIX} results scroll {delta:+} -> first row {} of {rows}",
            self.results_scroll + 1
        ));
    }
}

/// The panel, built on the first open and kept for the rest of the game session: a close hides it
/// and the next open shows it again as it was left. See `crate::session`.
static PANEL: Mutex<Session<Panel>> = Mutex::new(Session::new());

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
    load_backend();
    INSTALLED.store(true, Ordering::Release);
    log_line(format_args!(
        "{LOG_PREFIX} panel installed, backend={}",
        if backend().is_stub() {
            "stub"
        } else {
            "corpus"
        }
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
    if guard.phase() == Phase::Open {
        log_line(format_args!(
            "{LOG_PREFIX} press frame={frame} -- ignored, the panel is already up"
        ));
        return;
    }
    let mut reader = Reader::new();
    // The Enter or A that pressed the row is still down on this frame.
    reader.swallow_held();
    let mut fresh_reader = Some(reader);
    let opened = guard.open(|| Panel::new(fresh_reader.take().unwrap_or_else(Reader::new)));
    if opened == Opened::Restored
        && let Some(panel) = guard.value_mut()
        && let Some(reader) = fresh_reader.take()
    {
        panel.reopen(reader);
        panel.log_restored();
    }
    drop(guard);
    ds2_input_harness::hold(true);
    log_line(format_args!(
        "{LOG_PREFIX} panel open frame={frame} ({})",
        if opened == Opened::Restored {
            "restored as it was left"
        } else {
            "fresh, first open this session"
        }
    ));
}

/// Whether the panel is on screen. `ds2-overlay` renders no imgui frame while no panel is.
fn is_up() -> bool {
    PANEL
        .try_lock()
        .ok()
        .is_some_and(|guard| guard.phase() == Phase::Open)
}

/// While the panel is up or releasing its hold, the game window gets no keyboard or mouse message.
fn wants_input() -> bool {
    PANEL
        .try_lock()
        .ok()
        .is_some_and(|guard| matches!(guard.phase(), Phase::Open | Phase::Closing(_)))
}

/// Hand a close a press or click asked for to the session. The panel itself stays.
fn settle_close(session: &mut Session<Panel>) {
    if let Some(panel) = session.value_mut()
        && std::mem::take(&mut panel.wants_close)
    {
        session.close();
    }
}

/// One frame: keys and pad into the panel, and the hold's release after it closes.
fn on_frame() {
    let Ok(mut guard) = PANEL.lock() else {
        return;
    };
    match guard.phase() {
        Phase::Closing(_) => {
            let released = guard
                .value_mut()
                .is_none_or(|panel| !panel.reader.anything_down());
            if guard.tick_closing(released, RELEASE_DEADLINE_FRAMES) {
                drop(guard);
                ds2_input_harness::hold(false);
            }
        }
        Phase::Open => {
            if let Some(panel) = guard.value_mut() {
                for press in panel.reader.poll() {
                    if panel.wants_close {
                        break;
                    }
                    panel.press(press);
                }
            }
            settle_close(&mut guard);
        }
        Phase::Unopened | Phase::Closed => {}
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
/// The D-pad cursor's ring, and the highlighted row of an open list.
const CURSOR_EDGE: [f32; 4] = [0.45, 0.80, 1.0, 1.0];
const LIST_CURSOR: [f32; 4] = [0.45, 0.80, 1.0, 0.22];
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
    /// The control the D-pad cursor is on, ringed wherever it is drawn.
    cursor: Control,
}

impl Canvas<'_> {
    /// Ring `min..max` if `control` is the one under the cursor.
    fn mark(&self, min: [f32; 2], max: [f32; 2], control: Option<Control>) {
        if control == Some(self.cursor) {
            self.list
                .add_rect(
                    [min[0] - 3.0, min[1] - 3.0],
                    [max[0] + 3.0, max[1] + 3.0],
                    CURSOR_EDGE,
                )
                .rounding(4.0)
                .thickness(2.5)
                .build();
        }
    }

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

    /// A clickable cell, which the cursor rings when it is `control`. Returns its right edge.
    fn button(
        &mut self,
        x: f32,
        y: f32,
        label: &str,
        on: bool,
        action: Option<Action>,
        control: Option<Control>,
    ) -> f32 {
        let min = [x, y];
        let max = [x + self.width(label) + 16.0, y + self.row];
        self.mark(min, max, control);
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
    fn check(
        &mut self,
        x: f32,
        y: f32,
        label: &str,
        checked: bool,
        action: Action,
        control: Control,
    ) -> f32 {
        let text = format!("[{}] {label}", if checked { "x" } else { " " });
        self.button(x, y, &text, false, Some(action), Some(control))
    }

    /// A text field of `width`, showing `typed` while focused and `value` otherwise. Returns its
    /// right edge.
    fn field(&mut self, x: f32, y: f32, width: f32, spec: FieldSpec<'_>) -> f32 {
        let min = [x, y];
        let max = [x + width, y + self.row];
        self.mark(min, max, Some(spec.field.control()));
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
                // The value itself while it is being stepped with the D-pad; a hint when it is a
                // placeholder.
                self.text(
                    [caret + 4.0, text_y],
                    if spec.placeholder { DISABLED } else { TEXT },
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
    if guard.phase() != Phase::Open {
        return;
    }
    if let Some(panel) = guard.value_mut() {
        draw_panel(panel, ui);
    }
    // The close button's click.
    settle_close(&mut guard);
}

/// Draw the panel, and turn this frame's mouse clicks into actions.
fn draw_panel(panel: &mut Panel, ui: &Ui) {
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

    // A control that left the grid (a mode's option, a results table that was cleared) hands the
    // cursor to its fallback before anything is ringed.
    panel.cursor = nav::resolve(&nav::layout(panel.shape()), panel.cursor);
    let mut canvas = Canvas {
        ui,
        list: ui.get_foreground_draw_list(),
        // In back-buffer pixels; imgui's own position is in window pixels here. See `panels::mouse`.
        mouse: ds2_overlay::panels::mouse().unwrap_or(ui.io().mouse_pos),
        line,
        row,
        targets: Vec::new(),
        cursor: panel.cursor,
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
    canvas.button(
        close_x,
        y - 4.0,
        close,
        false,
        Some(Action::Close),
        Some(Control::Close),
    );
    y += line + 4.0;
    if backend().is_stub() {
        canvas.text(
            [inner, y],
            WARN,
            "Stub backend: no ds2-build-recommender.dat beside the game, so every number here is a \
             placeholder.",
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
        Some(Control::UseCharacter),
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
        .map_or("A or click, then pick or type", |row| row.name);
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
        Some(Control::Infusion),
    ) + GAP * 2.0;
    canvas.text([x, y + 4.0], DIM, "Grip");
    x += canvas.width("Grip") + 4.0;
    for grip in Grip::ALL {
        x = canvas.button(
            x,
            y,
            grip.label(),
            panel.state.grip == grip,
            Some(Action::SetGrip(grip)),
            Some(Control::Grip(grip)),
        ) + 4.0;
    }
    x += GAP;
    canvas.text([x, y + 4.0], DIM, "Objective");
    x += canvas.width("Objective") + 4.0;
    for objective in Objective::ALL {
        x = canvas.button(
            x,
            y,
            objective.label(),
            panel.state.objective == objective,
            Some(Action::SetObjective(objective)),
            Some(Control::Objective(objective)),
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
            Some(Control::Mode(mode)),
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
        Some(Control::Run),
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
    canvas.mark(results_rect.0, results_rect.1, Some(Control::Results));
    if panel.scrolling_results {
        canvas.edge(results_rect.0, results_rect.1, CURSOR_EDGE);
    }
    if panel.shown == Shown::Build
        && let Some(build) = panel.generated.clone()
    {
        let flex = panel.generated_flex.clone();
        draw_build(&mut canvas, &build, flex.as_ref(), results_rect);
    } else {
        draw_answer(panel, &mut canvas, results_rect);
    }

    // Footer.
    let mut x = inner;
    x = canvas.button(
        x,
        footer_y,
        "Generate Build",
        false,
        Some(Action::Generate),
        Some(Control::Generate),
    ) + GAP;
    x = canvas.check(
        x,
        footer_y,
        "Allow no armor",
        panel.state.allow_naked,
        Action::ToggleAllowNaked,
        Control::AllowNaked,
    ) + GAP;
    let spells_x = x;
    let spells_label = format!("Spells ({}) v", panel.state.spells.len());
    x = canvas.button(
        x,
        footer_y,
        &spells_label,
        panel.list == Some(List::Spell),
        Some(Action::OpenList(List::Spell)),
        Some(Control::Spells),
    ) + GAP;
    if panel.generated.is_some() {
        let (label, shown) = if panel.shown == Shown::Build {
            ("Show results", Shown::Answer)
        } else {
            ("Show build", Shown::Build)
        };
        x = canvas.button(
            x,
            footer_y,
            label,
            false,
            Some(Action::Show(shown)),
            Some(Control::ShowToggle),
        ) + GAP;
    }
    x = canvas.button(
        x,
        footer_y,
        "Apply to character...",
        false,
        panel.generated.is_some().then_some(Action::AskApply),
        Some(Control::Apply),
    ) + GAP * 2.0;
    if let Some(status) = &panel.status {
        canvas.text([x, footer_y + 4.0], GOOD, &clip(ui, status, right - x));
    }
    canvas.text(
        [inner, hint_y],
        DIM,
        &clip(
            ui,
            "D-pad/arrows move, A/Enter presses; on a number Left/Right step 1, Up/Down 10; in a \
             list Up/Down, LB/RB page; B/Esc backs out, then closes. Keys can also type a weapon \
             name.",
            right - inner,
        ),
    );

    // The open list, over everything else.
    let mut list_rect = None;
    if let Some(list) = panel.list {
        let rows = panel.list_rows(list);
        let (at, list_width, empty) = match list {
            List::Weapon => (
                [weapon_field_x, weapon_list_y],
                weapon_field_w,
                "no weapon matches",
            ),
            List::Infusion => (
                [infusion_x, weapon_list_y],
                canvas.width("Enchanted") + 60.0,
                "",
            ),
            List::Class => (
                [class_x, class_list_y],
                canvas.width("Curved Greatsword") + 60.0,
                "",
            ),
            // Above the footer, which sits at the bottom: as tall as `draw_list` will draw it.
            List::Spell => {
                let shown = rows.len().clamp(1, LIST_VISIBLE) as f32;
                let list_height = (line + 6.0) * shown + 8.0;
                (
                    [spells_x, footer_y - 2.0 - list_height],
                    canvas.width("[x] Promised Walk of Peace  2 slots  INT 30 FTH 40") + 40.0,
                    "no spell data: the stub backend offers none",
                )
            }
        };
        let highlighted = panel.list_cursor;
        list_rect = Some(draw_list(
            &mut canvas,
            at,
            list_width,
            &rows,
            &mut panel.list_scroll,
            highlighted,
            empty,
        ));
    }

    // The confirm, over everything: only its own two buttons can be clicked.
    if panel.confirming
        && let Some(build) = &panel.generated
    {
        canvas.targets.clear();
        draw_confirm(&mut canvas, build, display, panel.confirm_on_apply);
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
                Control::OneHand,
            ) + GAP;
            class_x = x;
            let class = format!("{} v", opts.class.as_deref().unwrap_or("All classes"));
            x = canvas.button(
                x,
                y,
                &class,
                panel.list == Some(List::Class),
                Some(Action::OpenList(List::Class)),
                Some(Control::Class),
            ) + GAP;
            x = canvas.check(
                x,
                y,
                "best per class",
                opts.per_class,
                Action::TogglePerClass,
                Control::PerClass,
            ) + GAP;
            canvas.text([x, y + 4.0], DIM, "R1 window s");
            x += canvas.width("R1 window s") + 4.0;
            let window = format!("{:.1}", opts.window_s);
            let spec = panel.spec(Field::Window, &window, false);
            x = canvas.field(x, y, canvas.width("10.0") + 20.0, spec) + GAP;
            canvas.check(
                x,
                y,
                "raw AR",
                opts.raw_ar,
                Action::ToggleRawAr,
                Control::RawAr,
            );
        }
        Mode::OptimizeForWeapon => {
            // Every infusion of the chosen weapon at the stats above, by the objective.
            x = canvas.button(
                x,
                y,
                "Best infusion",
                false,
                panel.state.weapon.is_some().then_some(Action::BestInfusion),
                Some(Control::BestInfusion),
            ) + GAP;
            let text = if panel.state.weapon.is_some() {
                format!(
                    "the weapon, infusion, objective and grip above ({}), at SL {}",
                    if panel.state.grip.two_handed() {
                        "two-handed: STR requirement halved"
                    } else {
                        "one-handed: full STR requirement"
                    },
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
                Control::TwoHand,
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
                Control::Bleed,
            ) + GAP;
            canvas.check(
                x,
                y,
                "poison only",
                panel.state.status.poison,
                Action::TogglePoison,
                Control::Poison,
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
                    "{}, {}{}",
                    build.class,
                    sl_label(&build.stats, build.sl),
                    if build.two_handed {
                        ", two-handed"
                    } else {
                        ", one-handed"
                    }
                ),
            );
            y += line + 4.0;
            canvas.text([min[0], y], TEXT, &stats_line(&build.stats));
            y += line + 4.0;
            if let Some(flex) = &panel.answer_flex {
                y = draw_flexibility(canvas, flex, [min[0], y], max[0] - min[0]);
            }
            if !build.gear.is_empty() {
                canvas.text(
                    [min[0], y],
                    TEXT,
                    &clip(
                        canvas.ui,
                        &format!(
                            "Wearing (its stat bonuses count): {}",
                            build.gear.join(", ")
                        ),
                        max[0] - min[0],
                    ),
                );
                y += line + 4.0;
            }
            if panel.state.mode == Mode::OptimizeForWeapon {
                canvas.text(
                    [min[0], y],
                    DIM,
                    &format!("objective value {:.0}", build.value),
                );
            }
        }
        Some(Answer::Rows(rows)) => {
            let rows = rows.clone();
            let score = match panel.state.mode {
                Mode::SimilarBuilds => "builds",
                // Build-up per hit times hits per attack (or within the window).
                _ if panel.state.objective == Objective::Bleed => "bleed x hits",
                _ if panel.state.objective == Objective::Poison => "poison x hits",
                // Best infusion ranks by damage whatever the Weapons-for-stats tab's raw AR says.
                Mode::WeaponsForStats if panel.state.weapons_for.raw_ar => "AR",
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
/// The soul level the shown stats make, which is what a header says; the level asked for only
/// beside it, when the two differ: `SL 33 (asked for 100)`.
fn sl_label(stats: &[u16; STAT_COUNT], asked: u16) -> String {
    let made = ds2_build_recommender_core::model::soul_level(stats);
    if made == asked {
        format!("SL {made}")
    } else {
        format!("SL {made} (asked for {asked})")
    }
}

/// The two flexibility lines at `at`: how many weapons the build wields and its percentile among
/// the builds nearest it, then the load it leaves for weapons. Returns the `y` below them.
fn draw_flexibility(canvas: &mut Canvas<'_>, flex: &Flexibility, at: [f32; 2], width: f32) -> f32 {
    let step = canvas.line + 4.0;
    canvas.text(
        at,
        TEXT,
        &clip(canvas.ui, &format!("Weapons: {}", flex_line(flex)), width),
    );
    canvas.text(
        [at[0], at[1] + step],
        DIM,
        &clip(canvas.ui, &format!("Load: {}", flex_load_line(flex)), width),
    );
    at[1] + 2.0 * step
}

fn draw_build(
    canvas: &mut Canvas<'_>,
    build: &GeneratedBuild,
    flex: Option<&Flexibility>,
    (min, max): ([f32; 2], [f32; 2]),
) {
    let line = canvas.line;
    let step = line + 4.0;
    let mut y = min[1];
    let (primary_key, infusion) = &build.primary;
    let primary = weapons::by_key(primary_key).map_or(primary_key.as_str(), |row| row.name);
    canvas.text(
        [min[0], y],
        TITLE,
        &format!(
            "{}, {} -- {} ({}), {}{}",
            build.class,
            sl_label(&build.stats, build.sl),
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
    if let Some(flex) = flex {
        y = draw_flexibility(canvas, flex, [min[0], y], max[0] - min[0]);
    }
    // The four armour slots as Apply equips them, head to legs.
    let armor = if build.armor.is_empty() {
        "none (Allow no armor)".to_owned()
    } else {
        ["Head", "Chest", "Hands", "Legs"]
            .iter()
            .zip(&build.armor)
            .map(|(slot, piece)| format!("{slot}: {piece}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    canvas.text(
        [min[0], y],
        TEXT,
        &clip(canvas.ui, &format!("Armor: {armor}"), max[0] - min[0]),
    );
    y += step;
    if let Some(note) = &build.armor_note {
        canvas.text(
            [min[0], y],
            WARN,
            &clip(canvas.ui, &format!("Armor: {note}"), max[0] - min[0]),
        );
        y += step;
    }
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
    // The spells Apply attunes, the slots they take of what ATT gives, and what to cast them with.
    if !build.spells.is_empty() {
        canvas.text(
            [min[0], y],
            TEXT,
            &clip(
                canvas.ui,
                &format!(
                    "Spells: {} ({} of {} attunement slots)",
                    build.spell_names.join(", "),
                    build.slots_used,
                    build.slots
                ),
                max[0] - min[0],
            ),
        );
        y += step;
        let catalysts = if build.catalysts.is_empty() {
            "none these stats can wield casts them".to_owned()
        } else {
            build
                .catalysts
                .iter()
                .map(|pick| {
                    let over = pick.passed_over.as_ref().map_or(String::new(), |over| {
                        format!(" ({over} casts harder; stats too low)")
                    });
                    format!("{} {} {:.0}{over}", pick.school, pick.name, pick.power)
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        canvas.text(
            [min[0], y],
            TEXT,
            &clip(
                canvas.ui,
                &format!("Catalyst: {catalysts}"),
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
fn draw_confirm(
    canvas: &mut Canvas<'_>,
    build: &GeneratedBuild,
    display: [f32; 2],
    on_apply: bool,
) {
    let line = canvas.line;
    let (import, extras) = backend::to_import(build);
    let grants = import
        .weapons
        .chunks(2)
        .map(|pair| &pair[0])
        .chain(&import.armor)
        .chain(&import.rings)
        .chain(&import.spells)
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
            format!(
                "The nine stats are rewritten and the starting class becomes {}; a build below \
                 this character keeps both.",
                build.class
            ),
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
        on_apply,
        Some(Action::ConfirmApply),
        None,
    ) + GAP;
    canvas.button(x, y, "Cancel", !on_apply, Some(Action::CancelApply), None);
}

/// A drop-down list hanging from `at`, scrolled to `scroll`, with row `highlighted` marked as the
/// one A chooses. Returns its rectangle.
fn draw_list(
    canvas: &mut Canvas<'_>,
    at: [f32; 2],
    width: f32,
    rows: &[(String, Action)],
    scroll: &mut usize,
    highlighted: usize,
    empty: &str,
) -> ([f32; 2], [f32; 2]) {
    const VISIBLE: usize = LIST_VISIBLE;
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
    for (index, (label, action)) in rows.iter().enumerate().skip(*scroll).take(VISIBLE) {
        let row_min = [at[0] + 2.0, y - 1.0];
        let row_max = [at[0] + width - 2.0, y + step - 1.0];
        if index == highlighted {
            canvas.rect(row_min, row_max, LIST_CURSOR);
            canvas.edge(row_min, row_max, CURSOR_EDGE);
        } else if canvas.inside(row_min, row_max) {
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
