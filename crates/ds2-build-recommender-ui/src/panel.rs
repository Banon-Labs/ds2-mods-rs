//! The panel: its state between frames, the keyboard half, and the drawing and mouse half.
//!
//! One `Mutex` because Rust cannot know what the picker's module docs establish: DARK SOULS II
//! presents from its simulation thread, so [`open`], `on_frame` and `draw` all run on one thread and
//! the lock is never contended.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use ds2_build_import_core::Infusion;
use ds2_build_recommender_core::backend::{
    self, Answer, Calibration, DAMAGE_TYPES, Fix, GeneratedBuild, RecommenderBackend, Refusal,
    ResultRow, StubBackend, WeaponCard, Wield,
};
use ds2_build_recommender_core::corpus::{self, CorpusBackend};
use ds2_build_recommender_core::flex::{
    FLYNN_NOTE, Flexibility, flex_line, flex_load, flex_load_line, flex_rank, flex_wields,
};
use ds2_build_recommender_core::model::{
    ARMOR_SLOTS, Defender, Grip, Mode, Objective, PanelState, STAT_COUNT, STAT_LABELS,
};
use ds2_build_recommender_core::paperdoll::{self, Held, Paperdoll, Piece, Slot};

/// STR's place in the nine stats, for the log lines that say what a grip did to it.
const STR_INDEX: usize = 4;
use ds2_build_recommender_core::nav::{self, Control, Dir, Nudge, Shape};
use ds2_build_recommender_core::weapons::{self, WeaponRow};
use ds2_overlay::fefont::{Button, button};
use ds2_overlay::panels::{EmptySlot, IconDraw};
use ds2_overlay::style;
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

/// Rows the weapon picker shows at once until its first draw has measured the screen: the design's
/// three, which fit a 1920x1080 screen.
const PICKER_ROWS: usize = 3;

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
    /// Who the damage goal is scored against: the average defender, then the four armour slots,
    /// each of which opens [`List::DefenderPiece`].
    Defender,
    /// The armour one slot of a chosen defender can wear, by index into [`ARMOR_SLOTS`].
    DefenderPiece(usize),
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
    /// The goal's next option: Damage, Bleed, Poison, round again.
    CycleObjective,
    /// The other grip.
    CycleGrip,
    /// Score damage against the soul level's average defender again.
    AverageDefender,
    /// Put a piece on the defender: the slot, by index into [`ARMOR_SLOTS`], and the piece, by
    /// index into the backend's `armor_pieces` for it.
    ChooseDefenderPiece(usize, usize),
    SetMode(Mode),
    /// Put the results table's highlight on this row.
    PickRow(usize),
    /// Describe this slot of the paperdoll, and put the cursor on it.
    PickSlot(Slot),
    ToggleOneHand,
    TogglePerClass,
    ToggleRawAr,
    ToggleTwoHand,
    /// Generate Build: whether the build may leave the armour off.
    ToggleAllowNaked,
    /// Optimize for weapon and Generate Build: whether the soul level's floors are dropped.
    ToggleIgnoreFloors,
    /// Make one of the refusal's checked fixes, by index into [`Panel::fixes`], and ask again.
    Fix(usize),
    /// Generate Build: add or take out one spell, by index into the backend's spell table.
    ToggleSpell(usize),
    /// Generate Build: no spells.
    ClearSpells,
    ToggleBleed,
    TogglePoison,
    /// Close the open list, keeping what was typed into its filter.
    CloseList,
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
    /// Rows the weapon picker showed on its last draw, which LB and RB page by.
    picker_rows: usize,
    /// How many buttons the footer had when the log last said whether they fit.
    footer_logged: usize,
    /// Whether the log has said how the weapon picker opened this time.
    picker_logged: bool,
    answer: Option<Answer>,
    results_scroll: usize,
    /// The highlighted row of the results table, whose breakdown is drawn under it.
    results_cursor: usize,
    generated: Option<GeneratedBuild>,
    /// [`Self::generated`] slot by slot, as Apply leaves the character: what the paperdoll draws.
    doll: Option<Paperdoll>,
    /// The slot the pane beside the paperdoll describes: the last one the cursor was on, or the
    /// build's weapon.
    slot: Slot,
    /// Whether the log has said what the paperdoll drew for the build on screen.
    doll_logged: bool,
    /// The weapon flexibility of the build in [`Self::answer`], asked once when it arrived: the
    /// neighbour search reads the whole corpus, which is not a per-frame cost.
    answer_flex: Option<Flexibility>,
    /// What [`Self::answer`]'s damage was scored against, as `backend::defender_line` says it:
    /// asked once when the answer arrived, and only when its score is damage, the one score the
    /// defender changes.
    answer_defender: Option<String>,
    /// The same for [`Self::generated`], in its armour and rings.
    generated_flex: Option<Flexibility>,
    /// Why the last Generate Build was refused.
    refused: Vec<String>,
    /// The fixes the last refusal offered, Optimize for weapon's or Generate Build's: each was
    /// checked by the backend to produce a build. Empty when nothing is refused.
    fixes: Vec<Fix>,
    /// What a fix asks again after it is made: [`Action::Run`] or [`Action::Generate`].
    fix_then: Action,
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
            picker_rows: PICKER_ROWS,
            footer_logged: 0,
            picker_logged: false,
            answer: None,
            results_scroll: 0,
            results_cursor: 0,
            generated: None,
            doll: None,
            slot: Slot::PRIMARY,
            doll_logged: false,
            answer_flex: None,
            answer_defender: None,
            generated_flex: None,
            refused: Vec::new(),
            fixes: Vec::new(),
            fix_then: Action::Generate,
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
            fixes: if self.shown == Shown::Answer {
                self.fixes.len()
            } else {
                0
            },
            paperdoll: self.doll_shown(),
        }
    }

    /// Whether the generated build is on screen, as its paperdoll.
    fn doll_shown(&self) -> bool {
        self.shown == Shown::Build && self.doll.is_some()
    }

    /// What `slot` of the generated build holds, for the log: `Sanctum Crossbow (Lightning) [item
    /// 1650000]`, or what Apply does with it when it holds nothing.
    fn held_line(&self, slot: Slot) -> String {
        match self.doll.as_ref().map(|doll| doll.held(slot)) {
            Some(Held::Item(piece)) => format!(
                "{} [{}]",
                piece_name(piece),
                piece
                    .item
                    .map_or_else(|| "no item id".to_owned(), |id| format!("item {id}"))
            ),
            Some(Held::Bare) => "bare in the build; Apply leaves what is worn there".to_owned(),
            Some(Held::Kept) => "not in the build; Apply leaves it as it is".to_owned(),
            None => "no build generated".to_owned(),
        }
    }

    /// The weapons the picker lists: every one whose name matches what its filter holds.
    fn weapon_rows(&self) -> Vec<&'static WeaponRow> {
        let filter = if self.focus == Some(Field::Search) {
            self.edit.as_str()
        } else {
            ""
        };
        weapons::search(filter)
    }

    /// Rows `list` shows at once, which LB and RB page by.
    fn list_visible(&self, list: List) -> usize {
        match list {
            List::Weapon => self.picker_rows,
            List::Infusion
            | List::Class
            | List::Spell
            | List::Defender
            | List::DefenderPiece(_) => LIST_VISIBLE,
        }
    }

    /// The name of the piece the defender wears in `slot`, `None` for the average defender.
    fn defender_piece_name(&self, slot: usize) -> Option<String> {
        let key = self.state.defender.pieces()?.get(slot)?;
        Some(
            backend()
                .armor_pieces(slot)
                .into_iter()
                .find(|(piece, _)| piece == key)
                .map_or_else(|| key.clone(), |(_, name)| name),
        )
    }

    /// The Defender control's value: `Average`, or the chosen set by its chest (or first worn
    /// piece) and how many other pieces it wears, `Havel's Armor +3`; `Naked` for none.
    fn defender_label(&self) -> String {
        if self.state.defender.pieces().is_none() {
            return "Average".to_owned();
        }
        let worn: Vec<String> = [1, 0, 2, 3]
            .into_iter()
            .filter_map(|slot| self.defender_piece_name(slot))
            .filter(|name| name != "Naked")
            .collect();
        match worn.split_first() {
            None => "Naked".to_owned(),
            Some((first, [])) => first.clone(),
            Some((first, rest)) => format!("{first} +{}", rest.len()),
        }
    }

    /// The rows `list` offers, with what choosing each does. The same rows the list draws.
    fn list_rows(&self, list: List) -> Vec<(String, Action)> {
        match list {
            List::Weapon => self
                .weapon_rows()
                .into_iter()
                .map(|row| {
                    let label = if row.class.is_empty() {
                        row.name.to_owned()
                    } else {
                        format!("{}  ({})", row.name, row.class)
                    };
                    (label, Action::ChooseWeapon(row.key))
                })
                .collect(),
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
            List::Defender => {
                let mark = |on: bool| if on { "[x]" } else { "[ ]" };
                let average = self.state.defender == Defender::Average;
                std::iter::once((
                    format!("{} Average defender at this soul level", mark(average)),
                    Action::AverageDefender,
                ))
                .chain(ARMOR_SLOTS.iter().enumerate().map(|(slot, label)| {
                    let worn = self
                        .defender_piece_name(slot)
                        .unwrap_or_else(|| "(choose to wear a set)".to_owned());
                    (
                        format!("{label}: {worn}"),
                        Action::OpenList(List::DefenderPiece(slot)),
                    )
                }))
                .collect()
            }
            List::DefenderPiece(slot) => backend()
                .armor_pieces(slot)
                .into_iter()
                .enumerate()
                .map(|(index, (_, name))| (name, Action::ChooseDefenderPiece(slot, index)))
                .collect(),
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
            Control::Objective => format!("objective={:?}", state.objective),
            Control::Defender => format!("defender={:?}", state.defender),
            Control::Grip => format!("grip={:?}", state.grip),
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
            Control::Results => format!("results row={}", self.results_cursor + 1),
            Control::Generate => "generate build".to_owned(),
            Control::AllowNaked => format!("allow-no-armor={}", state.allow_naked),
            Control::IgnoreFloors => format!("ignore-floors={}", state.ignore_floors),
            Control::Fix(index) => format!(
                "fix {index}: {}",
                self.fixes
                    .get(index)
                    .map_or("none", |fix| fix.label.as_str())
            ),
            Control::Spells => format!("spells={:?}", state.spells),
            Control::ShowToggle => format!("showing {:?}", self.shown),
            Control::Apply => format!("apply (build ready={})", self.generated.is_some()),
            Control::Slot(slot) => format!("slot {}: {}", slot.label(), self.held_line(slot)),
        }
    }

    /// Move the cursor one step, and say where it landed.
    fn move_cursor(&mut self, dir: Dir) {
        let rows = nav::layout(self.shape());
        let next = nav::step(&rows, self.cursor, dir);
        if next != self.cursor {
            self.cursor = next;
            if let Control::Slot(slot) = next {
                self.slot = slot;
            }
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
            Control::Objective => Some(Action::CycleObjective),
            Control::Defender => Some(Action::OpenList(List::Defender)),
            Control::Grip => Some(Action::CycleGrip),
            Control::Mode(mode) => Some(Action::SetMode(mode)),
            Control::OneHand => Some(Action::ToggleOneHand),
            Control::PerClass => Some(Action::TogglePerClass),
            Control::RawAr => Some(Action::ToggleRawAr),
            Control::TwoHand => Some(Action::ToggleTwoHand),
            Control::Bleed => Some(Action::ToggleBleed),
            Control::Poison => Some(Action::TogglePoison),
            Control::BestInfusion => self.state.weapon.is_some().then_some(Action::BestInfusion),
            Control::Results => None,
            Control::Generate => Some(Action::Generate),
            Control::AllowNaked => Some(Action::ToggleAllowNaked),
            Control::IgnoreFloors => Some(Action::ToggleIgnoreFloors),
            Control::Fix(index) => (index < self.fixes.len()).then_some(Action::Fix(index)),
            Control::Spells => Some(Action::OpenList(List::Spell)),
            Control::ShowToggle => Some(Action::Show(if self.shown == Shown::Build {
                Shown::Answer
            } else {
                Shown::Build
            })),
            Control::Apply => self.generated.is_some().then_some(Action::AskApply),
            Control::Slot(_) => None,
        };
        match (control, action) {
            (Control::Results, _) => {
                self.scrolling_results = true;
                log_line(format_args!(
                    "{LOG_PREFIX} press A on Results -- Up/Down scroll, LB/RB page, B leaves"
                ));
            }
            // The pane beside the slots already describes the one the cursor is on.
            (Control::Slot(_), _) => log_line(format_args!(
                "{LOG_PREFIX} press A on {control:?} -- a slot shows what the build puts there, \
                 nothing to choose [{}]",
                self.describe(control)
            )),
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
        self.list_scroll = nav::scroll_to(
            self.list_cursor,
            self.list_scroll,
            self.list_visible(list),
            rows.len(),
        );
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
            List::Weapon | List::Spell | List::Defender => None,
            List::DefenderPiece(slot) => self.state.defender.pieces().and_then(|pieces| {
                backend()
                    .armor_pieces(slot)
                    .iter()
                    .position(|(key, _)| Some(key) == pieces.get(slot))
            }),
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
        self.list_scroll = nav::scroll_to(self.list_cursor, 0, self.list_visible(list), rows.len());
    }

    /// The inputs changed: an answer for the old ones is not shown for the new.
    fn changed(&mut self) {
        self.answer = None;
        self.answer_flex = None;
        self.answer_defender = None;
        self.results_scroll = 0;
        self.results_cursor = 0;
        self.scrolling_results = false;
        self.refused.clear();
        self.fixes.clear();
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
            self.picker_logged = false;
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
        // The last Run was refused for floors and nothing has changed since (a change clears the
        // answer): this Run raises the stats to them first, as the refusal offered.
        if matches!(self.answer, Some(Answer::FloorViolations(_))) {
            let before = self.state.stats;
            if backend::raise_to_floors(backend(), &mut self.state) {
                log_line(format_args!(
                    "{LOG_PREFIX} raised to the floors at SL {}: {before:?} -> {:?}",
                    self.state.sl(),
                    self.state.stats
                ));
                self.changed();
            }
        }
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
                Answer::Refused(refusal) => format!("refused: {}", refusal_summary(refusal)),
            }
        ));
        self.fixes = match &answer {
            Answer::Refused(refusal) => refusal.fixes.clone(),
            _ => Vec::new(),
        };
        self.fix_then = Action::Run;
        // An optimized or minimum build wears nothing the answer names, so its load is all spare.
        self.answer_flex = match &answer {
            Answer::Build(build) => ask_flexibility(&build.stats, build.sl, &[], &[]),
            _ => None,
        };
        self.answer = Some(answer);
        self.note_defender();
        self.results_scroll = 0;
        self.results_cursor = 0;
        self.shown = Shown::Answer;
    }

    /// Ask what the answer just shown was scored against, when its score is damage, and log it:
    /// the defender is a choice the player cannot see in the numbers themselves.
    fn note_defender(&mut self) {
        let damage = self.state.mode != Mode::MinimumForWeapon
            && self.state.score_heading() == "Damage"
            && matches!(self.answer, Some(Answer::Rows(_) | Answer::Build(_)));
        let sl = self.state.sl();
        self.answer_defender = damage
            .then(|| backend().defense(sl, &self.state.defender, self.state.reply))
            .flatten()
            .map(|defense| backend::defender_line(&defense, sl));
        if let Some(line) = &self.answer_defender {
            log_line(format_args!("{LOG_PREFIX} {line}"));
        }
    }

    /// The chosen weapon's infusions, best first, with the winner's margin over the runner-up in
    /// the status line.
    fn best_infusion(&mut self) {
        let answer = backend::best_infusion(backend(), &self.state);
        self.fixes.clear();
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
        self.note_defender();
        self.results_scroll = 0;
        self.results_cursor = 0;
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
                let doll = Paperdoll::of(&build);
                log_line(format_args!("{LOG_PREFIX} paperdoll: {}", doll_line(&doll)));
                self.doll = Some(doll);
                self.doll_logged = false;
                self.generated = Some(build);
                self.refused.clear();
                if self.fix_then == Action::Generate {
                    self.fixes.clear();
                }
                self.shown = Shown::Build;
                // The design's opening state: the cursor on the build's weapon, and the pane on it.
                self.slot = Slot::PRIMARY;
                self.cursor = Control::Slot(Slot::PRIMARY);
            }
            Err(refusal) => {
                log_line(format_args!(
                    "{LOG_PREFIX} generate refused: {}",
                    refusal_summary(&refusal)
                ));
                self.generated = None;
                self.generated_flex = None;
                self.doll = None;
                self.refused = refusal.lines;
                self.fixes = refusal.fixes;
                self.fix_then = Action::Generate;
                self.shown = Shown::Answer;
            }
        }
    }

    /// Make the refusal's fix `index` to the panel's inputs and ask the same question again, so
    /// one press turns a refusal into a build.
    fn fix(&mut self, index: usize) {
        let Some(fix) = self.fixes.get(index).cloned() else {
            return;
        };
        let applied = fix.change.apply(&mut self.state);
        log_line(format_args!(
            "{LOG_PREFIX} fix \"{}\" applied={applied} -> sl={} spells={:?} ignore_floors={}",
            fix.label,
            self.state.sl(),
            self.state.spells,
            self.state.ignore_floors
        ));
        let then = self.fix_then;
        self.changed();
        self.act(then);
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
            Action::CycleGrip => {
                self.state.grip = cycled(Grip::ALL, self.state.grip);
                self.changed();
            }
            Action::CycleObjective => {
                self.state.objective = cycled(Objective::ALL, self.state.objective);
                self.changed();
            }
            Action::AverageDefender => {
                if self.state.defender != Defender::Average {
                    self.state.defender = Defender::Average;
                    self.changed();
                }
                log_line(format_args!("{LOG_PREFIX} defender average"));
            }
            Action::ChooseDefenderPiece(slot, index) => {
                if let Some((key, name)) = backend().armor_pieces(slot).into_iter().nth(index) {
                    self.state.set_defender_piece(slot, &key);
                    self.changed();
                    log_line(format_args!(
                        "{LOG_PREFIX} defender {} {key} ({name}): {:?}",
                        ARMOR_SLOTS.get(slot).copied().unwrap_or("?"),
                        self.state.defender
                    ));
                }
                // Back to the slots, on the next one: a set is chosen a slot at a time.
                self.list = Some(List::Defender);
                self.list_cursor = (slot + 2).min(ARMOR_SLOTS.len());
                self.list_scroll = 0;
            }
            Action::PickRow(index) => {
                self.results_cursor = index;
                self.cursor = Control::Results;
            }
            Action::PickSlot(slot) => {
                self.slot = slot;
                self.cursor = Control::Slot(slot);
                log_line(format_args!(
                    "{LOG_PREFIX} slot clicked [{}]",
                    self.describe(self.cursor)
                ));
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
            Action::ToggleIgnoreFloors => {
                self.state.ignore_floors ^= true;
                log_line(format_args!(
                    "{LOG_PREFIX} ignore floors -> {}",
                    self.state.ignore_floors
                ));
                self.changed();
            }
            Action::Fix(index) => self.fix(index),
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
            // Leaving the field and closing the list is what every action but a choice does, above.
            Action::CloseList => log_line(format_args!("{LOG_PREFIX} list closed by a click")),
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
                // No field takes typing here, so R is free to be the keyboard's X.
                Press::Run | Press::Char('r' | 'R') => self.run_pressed(),
                Press::Backspace | Press::Char(_) => {}
            }
        }
    }

    /// X or R: Run, whichever control the cursor is on. There is no Run button to walk to.
    fn run_pressed(&mut self) {
        if self.state.ready() {
            log_line(format_args!("{LOG_PREFIX} press Run (X or R) -> run"));
            self.act(Action::Run);
        } else {
            log_line(format_args!(
                "{LOG_PREFIX} press Run (X or R) -- disabled, no weapon chosen"
            ));
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
            Press::PageUp | Press::PageDown | Press::Backspace | Press::Run | Press::Char(_) => {}
        }
    }

    /// An open list: Up/Down move the highlight, LB/RB a page, A chooses, B closes. Typing still
    /// filters the weapon list while its search field is focused.
    fn press_list(&mut self, list: List, press: Press) {
        let page = self.list_visible(list) as isize;
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
            Press::Left | Press::Right | Press::Tab | Press::Run => {}
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
            Press::Run | Press::Char('r' | 'R') => self.run_pressed(),
            Press::Confirm | Press::Close | Press::Back => {
                self.scrolling_results = false;
                log_line(format_args!("{LOG_PREFIX} left the results table"));
            }
            _ => {}
        }
    }

    /// Move the results table's highlight by `delta` rows, never past either end. The draw
    /// scrolls the table to keep it in view.
    fn scroll_results(&mut self, delta: isize) {
        let rows = match &self.answer {
            Some(Answer::Rows(rows)) => rows.len(),
            _ => 0,
        };
        self.results_cursor = nav::move_in_list(self.results_cursor, delta, rows);
        log_line(format_args!(
            "{LOG_PREFIX} results {delta:+} -> row {} of {rows}",
            self.results_cursor + 1
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

/// Opaque, as the design is: at `style::PANEL_BG`'s 0.96 the pause menu's text read through the
/// table on 22cafa3.
const PANEL_BG: [f32; 4] = style::INK_0;
const PANEL_EDGE: [f32; 4] = style::BRONZE;
/// The rules between the panel's sections: `BRONZE_DIM`, faint.
const RULE: [f32; 4] = style::with_alpha(style::BRONZE_DIM, 0.35);
const DIM_COVER: [f32; 4] = style::DIM_COVER;
const TITLE: [f32; 4] = style::TEXT;
const TEXT: [f32; 4] = style::TEXT;
const DIM: [f32; 4] = style::TEXT_DIM;
/// A value the player can change, and an unselected tab: the design's muted gold. Drawn in
/// [`TEXT`] on the focus fill, where bronze falls under 4.5:1.
const VALUE: [f32; 4] = style::BRONZE;
const DISABLED: [f32; 4] = style::TEXT_DISABLED;
const WARN: [f32; 4] = style::WARN_TEXT;
const GOOD: [f32; 4] = style::BRONZE;
const CELL_BG: [f32; 4] = style::INK_1;
const CELL_ON: [f32; 4] = style::INK_2;
/// Between [`style::INK_1`] and [`style::INK_2`], so a hovered cell differs from a chosen one. Derived.
const CELL_HOVER: [f32; 4] = style::rgb(0x29_27_24);
const LIST_BG: [f32; 4] = style::PANEL_BG;
const ROW_HOVER: [f32; 4] = style::INK_1;
const FOCUS_EDGE: [f32; 4] = style::BRONZE;
/// The D-pad cursor's ring, and the highlighted row of an open list.
const CURSOR_EDGE: [f32; 4] = style::BRONZE;
const LIST_CURSOR: [f32; 4] = style::INK_2;
const PAD: f32 = 14.0;
const GAP: f32 = 8.0;
/// Inside a [`Canvas::pair`], either side of its text.
const PAIR_PAD: f32 = 8.0;
/// Between two parameters' boxes, which puts [`PARAMS_GAP`] between their text.
const PAIR_BOX_GAP: f32 = PARAMS_GAP - PAIR_PAD * 2.0;

// The panel's spacing, from its design (the Build Recommender Layout canvas, 2026-09-30), in
// back-buffer pixels. Each is the space around a section's TEXT; where a control draws a box around
// its text the box's own padding is taken out of it.

/// Left and right of the panel's text.
const INSET: f32 = 28.0;
/// Above and below the title.
const HEADER_TOP: f32 = 18.0;
const HEADER_BOTTOM: f32 = 14.0;
/// Between "Use my stats" and Close.
const HEADER_GAP: f32 = 32.0;
/// Left and right of the stats strip, which runs wider than the text.
const STATS_INSET: f32 = 12.0;
/// Above a stat's label, between the label and the value, and below the value.
const STAT_TOP: f32 = 14.0;
const STAT_GAP: f32 = 6.0;
const STAT_BOTTOM: f32 = 12.0;
/// Above the mode tabs, between two tabs' text, and between a tab's text and the line under the
/// chosen one.
const TABS_TOP: f32 = 34.0;
const TAB_GAP: f32 = 46.0;
const TAB_LINE_GAP: f32 = 10.0;
/// The weapon's parameters and the mode's options: above them, between their two lines, below
/// them, where their text starts, and between two parameters' text.
const PARAMS_TOP: f32 = 24.0;
const PARAMS_ROW_GAP: f32 = 14.0;
const PARAMS_BOTTOM: f32 = 20.0;
const PARAMS_INSET: f32 = 36.0;
const PARAMS_GAP: f32 = 36.0;
/// Above and below the answer.
const CONTENT_TOP: f32 = 16.0;
const CONTENT_BOTTOM: f32 = 18.0;
/// Above the footer's buttons, and between them and the key bar's rule.
const FOOTER_TOP: f32 = 22.0;
const FOOTER_GAP: f32 = 24.0;
/// Above and below the key bar.
const KEYS_TOP: f32 = 22.0;
const KEYS_BOTTOM: f32 = 28.0;
/// Inside a button: left and right of its label, and above and below it.
const BUTTON_PAD_X: f32 = 14.0;
const BUTTON_PAD_Y: f32 = 7.0;
/// Between two footer buttons.
const BUTTON_GAP: f32 = 12.0;

// The weapon picker, from the same design.

/// As wide as the picker gets; a narrower screen gets a narrower one.
const PICKER_W: f32 = 1216.0;
/// Above the picker's title, below it, and between its filter and the rule under it.
const PICKER_TITLE_TOP: f32 = 30.0;
const PICKER_TITLE_BOTTOM: f32 = 22.0;
const PICKER_FILTER_BOTTOM: f32 = 22.0;
/// The filter's box: its height, and the space between its label and it.
const FILTER_H: f32 = 44.0;
const FILTER_GAP: f32 = 16.0;
/// Left of the rows, which start further out than the text so the highlight's bar has room.
const PICKER_ROWS_INSET: f32 = 24.0;
/// The part of a weapon's icon a row shows, in the texture's pixels, and its size: the band of the
/// texture the art is drawn in, one texture pixel to one screen pixel. Everything else in a row is
/// laid out against it (the design's "Weapon icon at its real size" board).
const ICON_CROP: [f32; 4] = ds2_overlay::item_icon::WEAPON_ART;
const ICON_W: f32 = ICON_CROP[2] - ICON_CROP[0];
const ICON_H: f32 = ICON_CROP[3] - ICON_CROP[1];
/// Inside a row: its highlight's bar, the space after the bar, above and below the icon, and
/// between the icon, the text and the attack.
const PICKER_BAR: f32 = 3.0;
const PICKER_ROW_PAD_X: f32 = 21.0;
const PICKER_ROW_PAD_Y: f32 = 16.0;
const PICKER_ROW_GAP: f32 = 28.0;
/// One row: the icon and the space above and below it.
const PICKER_ROW_H: f32 = PICKER_ROW_PAD_Y * 2.0 + ICON_H;
/// A row's text, whose top is the icon's top and whose last line ends at the icon's bottom: the
/// label column's width and the gap after it, between two lines of the grid, and between two
/// requirements.
const PICKER_LABEL_W: f32 = 120.0;
const PICKER_LABEL_GAP: f32 = 16.0;
const PICKER_GRID_GAP: f32 = 9.0;
const PICKER_REQ_GAP: f32 = 24.0;
/// The attack column's width.
const PICKER_ATTACK_W: f32 = 230.0;
/// The scroll bar's width, and its gap from the rows.
const PICKER_SCROLL_W: f32 = 6.0;
const PICKER_SCROLL_GAP: f32 = 12.0;
/// Above and below the picker's key bar.
const PICKER_KEYS_TOP: f32 = 22.0;
const PICKER_KEYS_BOTTOM: f32 = 28.0;
/// The picker's distance from the screen's top and bottom at least.
const PICKER_MARGIN: f32 = 44.0;
/// The edge of the icon's frame.
const ICON_EDGE: [f32; 4] = style::rgb(0x3a_34_2d);
/// The "2H only" tag's text, light on `RUST`.
const TAG_TEXT: [f32; 4] = style::rgb(0xea_df_d2);
/// A damage type's name in the picker, in [`DAMAGE_TYPES`] order.
const ATTACK_NAMES: [&str; 5] = ["Physical", "Magic", "Fire", "Lightning", "Dark"];

// The generated build as a paperdoll, from the design's "Generated build as a paperdoll" board.

/// Above and below the build's summary line, which takes the weapon's parameters' place.
const SUMMARY_TOP: f32 = 24.0;
const SUMMARY_BOTTOM: f32 = 20.0;
/// Above and below the paperdoll.
const DOLL_TOP: f32 = 28.0;
const DOLL_BOTTOM: f32 = 30.0;
/// Between the slots and the pane describing one.
const DOLL_GAP: f32 = 48.0;
/// The narrowest the pane gets; on a narrower panel the slots shrink to leave it this.
const DETAIL_MIN_W: f32 = 440.0;
/// Between a group's heading and its slots, between two bands of slots, and between the arrows
/// and the bolts. Scaled with the slots.
const LABEL_GAP: f32 = 10.0;
const BAND_GAP: f32 = 26.0;
const AMMO_GAP: f32 = 16.0;
/// The paperdoll's height in the design's pixels, less its five lines of headings: three bands of
/// weapon, ring and armour slots, then the belt's two rows beside the arrows over the bolts.
const DOLL_SCALED_H: f32 =
    3.0 * (LABEL_GAP + paperdoll::BIG + BAND_GAP) + 2.0 * (LABEL_GAP + paperdoll::SMALL) + AMMO_GAP;
/// The smallest the slots are drawn, as a share of the design's size.
const DOLL_MIN_SCALE: f32 = 0.45;
/// How much of a slot's side the largest empty-slot silhouette spans: the design's glyph box,
/// 71 of 112.
const EMPTY_FILL: f32 = 71.0 / 112.0;
/// Around an item's icon inside its slot, in the design's pixels.
const SLOT_ICON_INSET: f32 = 6.0;
/// An empty slot's fill and edge, the design's. A filled slot is [`CELL_BG`] edged in
/// [`ICON_EDGE`], and the one the cursor is on [`CELL_ON`] edged in bronze.
const SLOT_EMPTY: [f32; 4] = style::rgb(0x14_13_12);
const SLOT_EMPTY_EDGE: [f32; 4] = style::rgb(0x26_22_20);
/// Between the pane's sections, and between two of its lines.
const DETAIL_SECTION_GAP: f32 = 22.0;
const DETAIL_LINE_GAP: f32 = 6.0;

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
    /// Left and right of a button's label: the design's [`BUTTON_PAD_X`], or [`PAIR_PAD`] on a
    /// frame whose footer would not fit the panel at the design's.
    button_pad: f32,
}

impl Canvas<'_> {
    /// Whether `control` is the one under the cursor.
    fn on(&self, control: Option<Control>) -> bool {
        control.is_some() && control == Some(self.cursor)
    }

    /// The design's cursor: the focus fill, with a bronze line along its foot.
    fn focus(&self, min: [f32; 2], max: [f32; 2]) {
        self.rect(min, max, CELL_ON);
        self.rect([min[0], max[1] - 2.0], max, CURSOR_EDGE);
    }

    /// Draw the cursor over `min..max` if `control` is the one under it. Drawn before the text.
    fn mark(&self, min: [f32; 2], max: [f32; 2], control: Option<Control>) {
        if self.on(control) {
            self.focus(min, max);
        }
    }

    /// A rule across the panel at `y`, between two of its sections.
    fn rule(&self, left: f32, right: f32, y: f32) {
        self.rect([left, y], [right, y + 1.0], RULE);
    }

    /// `label value` with no box, the design's parameter: the label in body text, the value in
    /// bronze, and the cursor's fill behind both. A click on it is `action`; without one the value
    /// is drawn disabled. Returns its right edge.
    fn pair(
        &mut self,
        x: f32,
        y: f32,
        label: &str,
        value: &str,
        action: Option<Action>,
        control: Option<Control>,
    ) -> f32 {
        let label_w = if label.is_empty() {
            0.0
        } else {
            self.width(label) + self.width(" ")
        };
        let min = [x, y];
        let max = [
            x + PAIR_PAD * 2.0 + label_w + self.width(value),
            y + self.row,
        ];
        let focused = self.on(control);
        if focused {
            self.focus(min, max);
        } else if action.is_some() && self.inside(min, max) {
            self.rect(min, max, CELL_HOVER);
        }
        let text_y = y + (self.row - self.line) * 0.5;
        self.text([x + PAIR_PAD, text_y], TEXT, label);
        let color = if action.is_none() {
            DISABLED
        } else if focused {
            TEXT
        } else {
            VALUE
        };
        self.text([x + PAIR_PAD + label_w, text_y], color, value);
        if let Some(action) = action {
            self.targets.push((min, max, action));
        }
        max[0]
    }

    /// A [`Self::pair`] whose value is typed into: `label`, then the field's text at least
    /// `value_w` wide. Returns its right edge and where the value starts, for a list to hang from.
    fn field_pair(
        &mut self,
        x: f32,
        y: f32,
        label: &str,
        spec: &FieldSpec<'_>,
        value_w: f32,
    ) -> (f32, f32) {
        let label_w = self.width(label) + self.width(" ");
        // As wide as what is typed: a search longer than the name it replaces is not cut.
        let room = value_w
            .max(self.width(spec.value))
            .max(self.width(spec.typed) + 1.0);
        let shown = self.field_shown(spec, room);
        let min = [x, y];
        let value_x = x + PAIR_PAD + label_w;
        let max = [
            value_x + value_w.max(self.width(&shown) + 6.0) + PAIR_PAD,
            y + self.row,
        ];
        let control = Some(spec.field.control());
        if spec.focused || self.on(control) {
            self.focus(min, max);
        } else if self.inside(min, max) {
            self.rect(min, max, CELL_HOVER);
        }
        let text_y = y + (self.row - self.line) * 0.5;
        self.text([x + PAIR_PAD, text_y], TEXT, label);
        let color = if spec.focused || self.on(control) {
            TEXT
        } else {
            VALUE
        };
        self.field_text([value_x, text_y], spec, &shown, color);
        self.targets.push((min, max, Action::Focus(spec.field)));
        (max[0], value_x)
    }

    /// What a field shows in `width`: what is typed while it is focused, its value otherwise.
    fn field_shown(&self, spec: &FieldSpec<'_>, width: f32) -> String {
        if spec.focused && !spec.typed.is_empty() {
            clip_left(self.ui, spec.typed, width)
        } else {
            clip(self.ui, spec.value, width)
        }
    }

    /// Draw `shown` at `at`, with the caret after what is typed, or before the value while it is
    /// focused with nothing typed -- the value itself, stepped with the D-pad, or a hint.
    fn field_text(&self, at: [f32; 2], spec: &FieldSpec<'_>, shown: &str, color: [f32; 4]) {
        let caret = |x: f32| {
            self.list
                .add_line([x, at[1]], [x, at[1] + self.line], TITLE)
                .build();
        };
        if !spec.focused {
            self.text(at, color, shown);
        } else if spec.typed.is_empty() {
            caret(at[0]);
            let color = if spec.placeholder { DISABLED } else { color };
            self.text([at[0] + 4.0, at[1]], color, shown);
        } else {
            self.text(at, color, shown);
            caret(at[0] + self.width(shown) + 1.0);
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
            .rounding(style::ROUNDING)
            .build();
    }

    fn edge(&self, min: [f32; 2], max: [f32; 2], color: [f32; 4]) {
        self.list
            .add_rect(min, max, color)
            .rounding(style::ROUNDING)
            .thickness(1.5)
            .build();
    }

    /// How tall [`Self::button`] draws.
    fn button_h(&self) -> f32 {
        self.line + BUTTON_PAD_Y * 2.0
    }

    /// The design's `·` between two parts of a line at `x`, which the game's font has no glyph
    /// for: a dot in [`DIM`] with a space either side, on a line of text at `y`. Returns its width.
    fn dot(&self, x: f32, y: f32) -> f32 {
        let space = self.width(" ");
        let radius = (self.line * 0.07).max(1.5);
        self.list
            .add_circle([x + space + radius, y + self.line * 0.55], radius, DIM)
            .filled(true)
            .build();
        space * 2.0 + radius * 2.0
    }

    /// `text` in the title face at `at`, cut at the right to fit `width`.
    fn big_text(&self, at: [f32; 2], color: [f32; 4], text: &str, width: f32) {
        let _big = ds2_overlay::panels::big_font(self.ui).map(|id| self.ui.push_font(id));
        self.list.add_text(at, color, clip(self.ui, text, width));
    }

    /// The game's own red X, `size` pixels square at `at`, or two `BLOOD` strokes when its atlas
    /// did not load. Returns the width it took.
    fn cross(&self, at: [f32; 2], size: f32) -> f32 {
        let drawn = ds2_overlay::panels::refusal_mark(&self.list, at, size);
        if drawn > 0.0 {
            return drawn;
        }
        let far = [at[0] + size, at[1] + size];
        self.list
            .add_line(at, far, style::BLOOD)
            .thickness(2.5)
            .build();
        self.list
            .add_line([far[0], at[1]], [at[0], far[1]], style::BLOOD)
            .thickness(2.5)
            .build();
        size + size * 0.3
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
        let max = [
            x + self.width(label) + self.button_pad * 2.0,
            y + self.button_h(),
        ];
        let hovered = action.is_some() && self.inside(min, max);
        let fill = if on {
            CELL_ON
        } else if hovered {
            CELL_HOVER
        } else {
            CELL_BG
        };
        self.rect(min, max, fill);
        self.mark(min, max, control);
        if on {
            self.edge(min, max, FOCUS_EDGE);
        }
        let color = if action.is_some() { TEXT } else { DISABLED };
        self.text([x + self.button_pad, y + BUTTON_PAD_Y], color, label);
        if let Some(action) = action {
            self.targets.push((min, max, action));
        }
        max[0]
    }
}

/// `[x] label`, a check button's label.
fn checked(label: &str, on: bool) -> String {
    format!("[{}] {label}", if on { "x" } else { " " })
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
    let left = (display[0] - width) * 0.5;
    let right = left + width - INSET;
    let inner = left + INSET;
    let title_h = ds2_overlay::panels::title_height(ui);
    let button_h = line + BUTTON_PAD_Y * 2.0;
    // A control drawn as a `row`-tall box around a line of text puts its text this far down it.
    let lift = (row - line) * 0.5;
    // A stat's cell: its label over its value.
    let stat_h = STAT_TOP + line + STAT_GAP + line + STAT_BOTTOM;

    // Every section's top, from the panel's top, in the order they are drawn: the design's space
    // around each section's text, and a rule between sections. The panel is as tall as its
    // sections and what the lower half has to show -- no taller -- and is centred on that.
    let title_at = HEADER_TOP;
    let stats_rule_at = title_at + title_h + HEADER_BOTTOM;
    let stats_at = stats_rule_at + 1.0;
    let tabs_rule_at = stats_at + stat_h;
    let tabs_at = tabs_rule_at + 1.0 + TABS_TOP;
    let params_at = tabs_at + line + TAB_LINE_GAP + 2.0 + PARAMS_TOP;
    let options_at = params_at + line + PARAMS_ROW_GAP;
    // A generated build on screen puts its summary where the parameters and options were, as the
    // design draws it: the build names its own weapon, infusion and grip.
    let doll = panel.doll_shown();
    let summary_at = tabs_at + line + TAB_LINE_GAP + 2.0 + SUMMARY_TOP;
    let (content_rule_at, content_pad_top, content_pad_bottom) = if doll {
        (summary_at + line + SUMMARY_BOTTOM, DOLL_TOP, DOLL_BOTTOM)
    } else {
        (
            options_at + line + PARAMS_BOTTOM,
            CONTENT_TOP,
            CONTENT_BOTTOM,
        )
    };
    let content_at = content_rule_at + 1.0 + content_pad_top;
    // The answer's space, its rule, the buttons, the key bar's rule and the key bar.
    let below_content = content_pad_bottom
        + 1.0
        + FOOTER_TOP
        + button_h
        + FOOTER_GAP
        + 1.0
        + KEYS_TOP
        + line
        + KEYS_BOTTOM;
    let wanted = content_height(panel, ui, right - inner, line);
    let room = display[1] - 32.0 - content_at - below_content;
    let content_h = wanted.min(room).max(line);
    let height = content_at + content_h + below_content;
    let top = ((display[1] - height) * 0.5).max(0.0);

    let title_y = top + title_at;
    let stats_y = top + stats_at;
    let tabs_y = top + tabs_at;
    // The parameters and options are boxes around their text.
    let params_y = top + params_at - lift;
    let options_y = top + options_at - lift;
    let content_top = top + content_at;
    let content_bottom = content_top + content_h;
    let footer_rule_y = content_bottom + content_pad_bottom;
    let footer_y = footer_rule_y + 1.0 + FOOTER_TOP;
    let keys_rule_y = footer_y + button_h + FOOTER_GAP;
    let hint_y = keys_rule_y + 1.0 + KEYS_TOP;

    // A control that left the grid (a mode's option, a results table that was cleared) hands the
    // cursor to its fallback before anything is drawn under it.
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
        button_pad: BUTTON_PAD_X,
    };
    canvas.rect([0.0, 0.0], display, DIM_COVER);
    canvas
        .list
        .add_rect([left, top], [left + width, top + height], PANEL_BG)
        .filled(true)
        .rounding(style::ROUNDING)
        .build();
    // The game's own window frame; a plain bronze line when its atlas did not load.
    if !ds2_overlay::panels::frame(
        &canvas.list,
        [left, top],
        [left + width, top + height],
        display[1],
    ) {
        canvas.edge([left, top], [left + width, top + height], PANEL_EDGE);
    }
    // The panel's own surface swallows clicks, so a click on empty panel space is not "outside".
    canvas
        .targets
        .push(([left, top], [left + width, top + height], Action::Nothing));
    let (rule_left, rule_right) = (left + 1.0, left + width - 1.0);
    for y in [
        top + stats_rule_at,
        top + tabs_rule_at,
        top + content_rule_at,
        footer_rule_y,
        keys_rule_y,
    ] {
        canvas.rule(rule_left, rule_right, y);
    }

    // Header: the title, and at the right the placeholder tag, the character's stats and Close.
    ds2_overlay::panels::title(ui, &canvas.list, [inner, title_y], TITLE, "Recommender");
    let pair_y = title_y + (title_h - row) * 0.5;
    let pair_w = |canvas: &Canvas<'_>, value: &str| canvas.width(value) + PAIR_PAD * 2.0;
    // Close's text ends at the inset; its box runs past it.
    let close_x = right + PAIR_PAD - pair_w(&canvas, "Close");
    canvas.pair(
        close_x,
        pair_y,
        "",
        "Close",
        Some(Action::Close),
        Some(Control::Close),
    );
    let use_label = "Use my stats";
    let use_x = close_x - (HEADER_GAP - PAIR_PAD * 2.0) - pair_w(&canvas, use_label);
    canvas.pair(
        use_x,
        pair_y,
        "",
        use_label,
        Some(Action::UseCharacter),
        Some(Control::UseCharacter),
    );
    // No `ds2-build-recommender.dat` beside the game: every number is the stub's placeholder.
    if backend().is_stub() {
        let tag = "Placeholder data";
        let tag_w = canvas.width(tag) + 16.0;
        let min = [use_x - GAP * 2.0 - tag_w, pair_y];
        canvas.rect(min, [min[0] + tag_w, pair_y + row], style::RUST);
        canvas.text([min[0] + 8.0, pair_y + (row - line) * 0.5], TEXT, tag);
    }

    // The soul level and the nine stats: ten columns, a label over a value, edge to edge.
    let strip_left = left + STATS_INSET;
    let cell_w = (width - STATS_INSET * 2.0) / (STAT_COUNT + 1) as f32;
    let computed = panel.state.computed_sl();
    let overridden = panel.state.sl_override.is_some() && panel.state.sl() != computed;
    let mut cells = vec![(
        if overridden {
            format!("SL ({computed})")
        } else {
            "SL".to_owned()
        },
        panel.state.sl().to_string(),
        Field::SlOverride,
    )];
    for (index, label) in STAT_LABELS.iter().enumerate() {
        cells.push((
            (*label).to_owned(),
            panel.state.stats[index].to_string(),
            Field::Stat(index),
        ));
    }
    for (column, (label, value, field)) in cells.iter().enumerate() {
        let min = [strip_left + cell_w * column as f32, stats_y];
        let max = [min[0] + cell_w, stats_y + stat_h];
        let spec = panel.spec(*field, value, false);
        let lit = spec.focused || canvas.on(Some(field.control()));
        if lit {
            canvas.focus(min, max);
        } else if canvas.inside(min, max) {
            canvas.rect(min, max, CELL_HOVER);
        }
        let centre = (min[0] + max[0]) * 0.5;
        canvas.text(
            [centre - canvas.width(label) * 0.5, min[1] + STAT_TOP],
            if lit { TEXT } else { DIM },
            label,
        );
        let shown = canvas.field_shown(&spec, cell_w - 16.0);
        // An override the stats do not make is a value the player set: bronze, like the others.
        let color = if overridden && *field == Field::SlOverride && !lit {
            VALUE
        } else {
            TEXT
        };
        canvas.field_text(
            [
                centre - canvas.width(&shown) * 0.5,
                min[1] + STAT_TOP + line + STAT_GAP,
            ],
            &spec,
            &shown,
            color,
        );
        canvas.targets.push((min, max, Action::Focus(*field)));
    }

    // The mode tabs: the chosen one bright over a bronze line, the others in bronze.
    let mut x = inner + 4.0;
    for mode in Mode::ALL {
        let label = mode.label();
        let label_w = canvas.width(label);
        let min = [x - PAIR_PAD, tabs_y - lift];
        let max = [x + label_w + PAIR_PAD, tabs_y - lift + row];
        let control = Some(Control::Mode(mode));
        let chosen = panel.state.mode == mode;
        if canvas.on(control) {
            canvas.focus(min, max);
        } else if canvas.inside(min, max) {
            canvas.rect(min, max, CELL_HOVER);
        }
        let color = if chosen || canvas.on(control) {
            TEXT
        } else {
            VALUE
        };
        canvas.text([x, tabs_y], color, label);
        if chosen {
            let under = tabs_y + line + TAB_LINE_GAP;
            canvas.rect(
                [x - 4.0, under],
                [x + label_w + 4.0, under + 2.0],
                CURSOR_EDGE,
            );
        }
        canvas.targets.push((min, max, Action::SetMode(mode)));
        x += label_w + TAB_GAP;
    }

    // The weapon's parameters and the mode's options, or the generated build's summary in their
    // place. Where the infusion and class lists hang from.
    let params_x = left + PARAMS_INSET - PAIR_PAD;
    let (infusion_at, class_at, defender_at) = if doll {
        draw_summary(panel, &canvas, [inner, top + summary_at], right);
        let under = [inner, top + content_rule_at + 2.0];
        (under, under, under)
    } else {
        let (infusion_x, defender_x) = draw_params(panel, &mut canvas, params_x, params_y);
        // Run is X or R from anywhere, and has no button (the design, 2026-09-30).
        let class_x = draw_options(panel, &mut canvas, params_x, options_y);
        (
            [infusion_x, params_y + row + 2.0],
            [class_x, options_y + row + 2.0],
            [defender_x, params_y + row + 2.0],
        )
    };

    let results_rect = ([inner, content_top], [right, content_bottom]);
    if doll {
        draw_paperdoll(panel, &mut canvas, results_rect);
    } else {
        draw_answer(panel, &mut canvas, results_rect);
    }

    // Footer: its buttons in a row, as the design pads them, or tighter when they would not fit.
    let mut footer = vec![
        (
            "Generate Build".to_owned(),
            false,
            Some(Action::Generate),
            Control::Generate,
        ),
        (
            checked("Allow no armor", panel.state.allow_naked),
            false,
            Some(Action::ToggleAllowNaked),
            Control::AllowNaked,
        ),
        (
            format!("Spells ({}) v", panel.state.spells.len()),
            panel.list == Some(List::Spell),
            Some(Action::OpenList(List::Spell)),
            Control::Spells,
        ),
        (
            checked("Ignore typical minimums", panel.state.ignore_floors),
            false,
            Some(Action::ToggleIgnoreFloors),
            Control::IgnoreFloors,
        ),
    ];
    if panel.generated.is_some() {
        let (label, shown) = if panel.shown == Shown::Build {
            ("Show results", Shown::Answer)
        } else {
            ("Show build", Shown::Build)
        };
        footer.push((
            label.to_owned(),
            false,
            Some(Action::Show(shown)),
            Control::ShowToggle,
        ));
    }
    footer.push((
        "Apply to character...".to_owned(),
        false,
        panel.generated.is_some().then_some(Action::AskApply),
        Control::Apply,
    ));
    let labels_w: f32 = footer.iter().map(|(label, ..)| canvas.width(label)).sum();
    let gaps_w = BUTTON_GAP * (footer.len() - 1) as f32;
    let needed = labels_w + gaps_w + BUTTON_PAD_X * 2.0 * footer.len() as f32;
    if needed > right - inner {
        canvas.button_pad = PAIR_PAD;
    }
    if std::mem::replace(&mut panel.footer_logged, footer.len()) != footer.len() {
        log_line(format_args!(
            "{LOG_PREFIX} footer: {} buttons need {needed:.0}px of {:.0}px -- padded {}",
            footer.len(),
            right - inner,
            canvas.button_pad
        ));
    }
    let mut x = inner;
    let mut spells_x = inner;
    for (label, on, action, control) in &footer {
        if *control == Control::Spells {
            spells_x = x;
        }
        x = canvas.button(x, footer_y, label, *on, *action, Some(*control)) + BUTTON_GAP;
    }
    x += BUTTON_GAP;
    if let Some(status) = &panel.status {
        canvas.text(
            [x, footer_y + BUTTON_PAD_Y],
            GOOD,
            &clip(ui, status, right - x),
        );
    }
    // Key help, one button and one word each, for the device the player is on: the panel's own
    // binds (`input.rs`), which are not the game's key config.
    const PAD_HINT: [(&str, &str); 6] = [
        (button(Button::DPad), "Move"),
        (button(Button::A), "Select"),
        (button(Button::DPadLeftRight), "Step 1"),
        (button(Button::X), "Run"),
        (button(Button::Bumpers), "Page"),
        (button(Button::B), "Back"),
    ];
    const KEY_HINT: [(&str, &str); 6] = [
        ("Arrows", "Move"),
        ("Enter", "Select"),
        ("Left/Right", "Step 1"),
        ("R", "Run"),
        ("PgUp PgDn", "Page"),
        ("Esc", "Back"),
    ];
    let hints: &[(&str, &str)] = if panel.reader.pad_last() {
        &PAD_HINT
    } else {
        &KEY_HINT
    };
    ds2_overlay::panels::hint_bar(ui, &canvas.list, [inner, hint_y], right, hints);

    // The open list, over everything else: the weapon picker in a window of its own, the others
    // hanging from what opened them.
    let mut list_rect = None;
    let mut picker_rect = None;
    let hanging = match panel.list {
        Some(List::Weapon) => {
            picker_rect = Some(draw_picker(panel, &mut canvas, display));
            None
        }
        Some(List::Infusion) => Some((infusion_at, canvas.width("Enchanted") + 60.0, "")),
        Some(List::Class) => Some((class_at, canvas.width("Curved Greatsword") + 60.0, "")),
        // Above the footer, which sits at the bottom: as tall as `draw_list` will draw it.
        Some(List::Spell) => {
            let shown = panel.list_rows(List::Spell).len().clamp(1, LIST_VISIBLE) as f32;
            let list_height = (line + 6.0) * shown + 8.0;
            Some((
                [spells_x, footer_y - 2.0 - list_height],
                canvas.width("[x] Promised Walk of Peace  2 slots  INT 30 FTH 40") + 40.0,
                "no spell data: the stub backend offers none",
            ))
        }
        // As wide as its longest row, and pulled left where the goal's end of the line leaves too
        // little room before the panel's edge.
        Some(list @ (List::Defender | List::DefenderPiece(_))) => {
            let width = panel
                .list_rows(list)
                .iter()
                .map(|(label, _)| canvas.width(label))
                .fold(canvas.width("Average defender"), f32::max)
                + 40.0;
            Some((
                [defender_at[0].min(right - width), defender_at[1]],
                width,
                "no armour data: the stub backend offers none",
            ))
        }
        None => None,
    };
    if let (Some(list), Some((at, list_width, empty))) = (panel.list, hanging) {
        let rows = panel.list_rows(list);
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
        if let Some((min, max)) = picker_rect
            && canvas.inside(min, max)
        {
            // A row a notch, the picker's rows being tall; the next draw keeps it in range.
            panel.list_scroll = panel.list_scroll.saturating_add_signed(steps.signum());
        } else if let Some((min, max)) = list_rect
            && canvas.inside(min, max)
        {
            scroll(&mut panel.list_scroll);
        } else if !doll
            && canvas.inside(results_rect.0, results_rect.1)
            && let Some(Answer::Rows(rows)) = &panel.answer
        {
            panel.results_cursor = nav::move_in_list(panel.results_cursor, steps, rows.len());
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

/// `On` or `Off`, a switch's value.
const fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}

/// The next of `all` after `now`, round to the first.
fn cycled<T: Copy + PartialEq, const N: usize>(all: [T; N], now: T) -> T {
    let at = all.iter().position(|value| *value == now).unwrap_or(0);
    all[(at + 1) % N]
}

/// The weapon's parameters, which every tab reads, on one line from `x`: each a label and a value,
/// no boxes. The weapon opens the picker, which has the filter; here is only what is chosen.
/// The defender sits beside the goal, whose damage it is the other half of. Returns where the
/// infusion list and the defender lists hang from.
fn draw_params(panel: &Panel, canvas: &mut Canvas<'_>, x: f32, y: f32) -> (f32, f32) {
    let weapon_name = panel
        .state
        .weapon
        .and_then(weapons::by_key)
        .map_or("[choose]", |row| row.name);
    let x = canvas.pair(
        x,
        y,
        "Weapon",
        weapon_name,
        Some(Action::Focus(Field::Search)),
        Some(Control::Weapon),
    ) + PAIR_BOX_GAP;
    let infusion_x = x + PAIR_PAD + canvas.width("Infusion") + canvas.width(" ");
    let x = canvas.pair(
        x,
        y,
        "Infusion",
        weapons::display_name(panel.state.infusion),
        Some(Action::OpenList(List::Infusion)),
        Some(Control::Infusion),
    ) + PAIR_BOX_GAP;
    let x = canvas.pair(
        x,
        y,
        "Grip",
        panel.state.grip.label(),
        Some(Action::CycleGrip),
        Some(Control::Grip),
    ) + PAIR_BOX_GAP;
    let x = canvas.pair(
        x,
        y,
        "Goal",
        panel.state.objective.label(),
        Some(Action::CycleObjective),
        Some(Control::Objective),
    ) + PAIR_BOX_GAP;
    let defender_x = x + PAIR_PAD + canvas.width("Defender") + canvas.width(" ");
    canvas.pair(
        x,
        y,
        "Defender",
        &panel.defender_label(),
        Some(Action::OpenList(List::Defender)),
        Some(Control::Defender),
    );
    (infusion_x, defender_x)
}

/// The current mode's options, on one line from `x`, each a label and a value as the weapon's
/// parameters are. Returns where the class list hangs from.
fn draw_options(panel: &Panel, canvas: &mut Canvas<'_>, x: f32, y: f32) -> f32 {
    let mut x = x;
    let mut class_x = x;
    // A value starts after its label and the space the pair puts between them.
    let value_x = |canvas: &Canvas<'_>, x: f32, label: &str| {
        x + PAIR_PAD + canvas.width(label) + canvas.width(" ")
    };
    match panel.state.mode {
        Mode::WeaponsForStats => {
            let opts = &panel.state.weapons_for;
            x = canvas.pair(
                x,
                y,
                "One-handed only",
                on_off(opts.one_hand),
                Some(Action::ToggleOneHand),
                Some(Control::OneHand),
            ) + PAIR_BOX_GAP;
            class_x = value_x(canvas, x, "Class");
            x = canvas.pair(
                x,
                y,
                "Class",
                opts.class.as_deref().unwrap_or("All"),
                Some(Action::OpenList(List::Class)),
                Some(Control::Class),
            ) + PAIR_BOX_GAP;
            x = canvas.pair(
                x,
                y,
                "Best per class",
                on_off(opts.per_class),
                Some(Action::TogglePerClass),
                Some(Control::PerClass),
            ) + PAIR_BOX_GAP;
            let window = format!("{:.1}s", opts.window_s);
            let spec = panel.spec(Field::Window, &window, false);
            x = canvas.field_pair(x, y, "R1 window", &spec, 0.0).0 + PAIR_BOX_GAP;
            canvas.pair(
                x,
                y,
                "Rank by",
                if opts.raw_ar { "AR" } else { "Damage" },
                Some(Action::ToggleRawAr),
                Some(Control::RawAr),
            );
        }
        Mode::OptimizeForWeapon => {
            // Every infusion of the chosen weapon at the stats above, by the goal.
            x = canvas.pair(
                x,
                y,
                "",
                "Best infusion",
                panel.state.weapon.is_some().then_some(Action::BestInfusion),
                Some(Control::BestInfusion),
            ) + PAIR_BOX_GAP;
            // The parameters above already say which weapon, infusion, goal and grip; only the
            // grip's effect on strength is not on screen.
            let text = if panel.state.weapon.is_some() {
                if panel.state.grip.two_handed() {
                    "2H: STR requirement halved"
                } else {
                    "1H: full STR requirement"
                }
            } else {
                "Choose a weapon above"
            };
            canvas.text([x, y + (canvas.row - canvas.line) * 0.5], DIM, text);
        }
        Mode::MinimumForWeapon => {
            x = canvas.pair(
                x,
                y,
                "Two-hand for STR",
                on_off(panel.state.two_hand),
                Some(Action::ToggleTwoHand),
                Some(Control::TwoHand),
            ) + PAIR_BOX_GAP;
            if panel.state.weapon.is_none() {
                canvas.text(
                    [x, y + (canvas.row - canvas.line) * 0.5],
                    DIM,
                    "Choose a weapon above",
                );
            }
        }
        Mode::SimilarBuilds => {
            let k = panel.state.similar_k.to_string();
            let spec = panel.spec(Field::SimilarK, &k, false);
            x = canvas.field_pair(x, y, "Neighbours", &spec, 0.0).0 + PAIR_BOX_GAP;
            x = canvas.pair(
                x,
                y,
                "Bleed only",
                on_off(panel.state.status.bleed),
                Some(Action::ToggleBleed),
                Some(Control::Bleed),
            ) + PAIR_BOX_GAP;
            canvas.pair(
                x,
                y,
                "Poison only",
                on_off(panel.state.status.poison),
                Some(Action::TogglePoison),
                Some(Control::Poison),
            );
        }
    }
    class_x
}

/// A refusal for the log: its lines, then the fixes it offers.
fn refusal_summary(refusal: &Refusal) -> String {
    let fixes: Vec<&str> = refusal.fixes.iter().map(|fix| fix.label.as_str()).collect();
    format!("{} | fixes: {}", refusal.lines.join("; "), fixes.join(", "))
}

/// A refusal's lines under `title`, then one button per checked fix. Returns the y under them.
fn draw_refusal(
    canvas: &mut Canvas<'_>,
    title: &str,
    lines: &[String],
    fixes: &[Fix],
    (min, max): ([f32; 2], [f32; 2]),
) -> f32 {
    let line = canvas.line;
    let mut y = min[1];
    canvas.text([min[0], y], WARN, title);
    for reason in lines {
        for part in wrap(canvas.ui, reason, max[0] - min[0] - 24.0) {
            y += line + 2.0;
            canvas.text([min[0] + 12.0, y], TEXT, &part);
        }
    }
    if fixes.is_empty() {
        return y + line;
    }
    y += line + 8.0;
    canvas.text(
        [min[0], y],
        TEXT,
        "Make it build (each was checked to give a build):",
    );
    y += line + 4.0;
    let mut x = min[0] + 12.0;
    for (index, fix) in fixes.iter().enumerate() {
        let width = canvas.width(&fix.label) + canvas.button_pad * 2.0;
        if x + width > max[0] && x > min[0] + 12.0 {
            x = min[0] + 12.0;
            y += canvas.button_h() + 4.0;
        }
        x = canvas.button(
            x,
            y,
            &fix.label,
            false,
            Some(Action::Fix(index)),
            Some(Control::Fix(index)),
        ) + 8.0;
    }
    y + canvas.button_h()
}

/// The answer to the last Run, or why there is none.
fn draw_answer(panel: &mut Panel, canvas: &mut Canvas<'_>, (min, max): ([f32; 2], [f32; 2])) {
    let line = canvas.line;
    let mut min = min;
    // What the damage below was scored against, over it, so the column has a meaning.
    if panel.refused.is_empty()
        && let Some(defender) = &panel.answer_defender
    {
        canvas.text(
            [min[0], min[1]],
            DIM,
            &clip(canvas.ui, defender, max[0] - min[0]),
        );
        min[1] += line + 6.0;
    }
    let mut y = min[1];
    if !panel.refused.is_empty() {
        let (lines, fixes) = (panel.refused.clone(), panel.fixes.clone());
        draw_refusal(
            canvas,
            "Generate Build refused:",
            &lines,
            &fixes,
            (min, max),
        );
        return;
    }
    match &panel.answer {
        Some(Answer::Refused(refusal)) => {
            let refusal = refusal.clone();
            draw_refusal(
                canvas,
                "Optimize found no build:",
                &refusal.lines,
                &refusal.fixes,
                (min, max),
            );
        }
        // Nothing asked yet: the key bar says how to ask, so only the calibration is left.
        None => canvas.text([min[0], y], DIM, &calibration_line(panel.calibration)),
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
            y += line + 8.0;
            canvas.text(
                [min[0], y],
                TEXT,
                "Run again to raise them to the floors and ask.",
            );
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
            let score = panel.state.score_heading();
            let focused = panel.scrolling_results || panel.cursor == Control::Results;
            panel.results_cursor = panel.results_cursor.min(rows.len().saturating_sub(1));
            let table = Table {
                rows: &rows,
                score,
                cursor: panel.results_cursor,
                focused,
            };
            draw_table(canvas, &table, &mut panel.results_scroll, (min, max));
        }
    }
}

/// `Infusion picks match real builds: 42%, ...`: how often the ranking's infusion pick agreed with
/// the builds it was checked against.
fn calibration_line(calibration: Calibration) -> String {
    if calibration.n == 0 {
        "Infusion picks: not yet checked against real builds".to_owned()
    } else {
        format!(
            "Infusion picks match real builds: {:.0}%, top two {:.0}% (n={})",
            calibration.top1 * 100.0,
            calibration.top2 * 100.0,
            calibration.n
        )
    }
}

/// A ranking for [`draw_table`].
struct Table<'a> {
    rows: &'a [ResultRow],
    /// The score column's heading.
    score: &'a str,
    /// The highlighted row.
    cursor: usize,
    /// Whether the D-pad is on the table, which brightens the highlight.
    focused: bool,
}

/// The table's heading line, over its first row.
fn table_header_h(line: f32) -> f32 {
    line + 10.0
}

/// One row of the table: taller than a line, as the design spaces them.
fn table_row_h(line: f32) -> f32 {
    line + 14.0
}

/// The line under the table: the highlighted row's damage by type, and where the page is.
fn table_footer_h(line: f32) -> f32 {
    line + 10.0
}

/// A ranking as a table of five columns, scrolled so its highlighted row is in view. The numbers
/// behind the score -- damage by type, hyper armour, counter -- are the line under it, for the
/// highlighted row only.
fn draw_table(
    canvas: &mut Canvas<'_>,
    table: &Table<'_>,
    scroll: &mut usize,
    (min, max): ([f32; 2], [f32; 2]),
) {
    let line = canvas.line;
    let (header_h, row_h) = (table_header_h(line), table_row_h(line));
    let width = max[0] - min[0];
    // The text's inset from the row's left edge, past the highlight's bar.
    let inset = 16.0;
    let text_width = width - inset;
    // Column starts, as fractions of the text's width.
    let columns: [(&str, f32); 6] = [
        ("Weapon", 0.0),
        ("Infusion", 0.28),
        (table.score, 0.46),
        ("Grip", 0.60),
        ("Class", 0.76),
        ("", 1.0),
    ];
    let cell = |column: usize| min[0] + inset + text_width * columns[column].1;
    let room = |column: usize| text_width * (columns[column + 1].1 - columns[column].1) - 12.0;
    for (column, (title, _)) in columns[..5].iter().enumerate() {
        canvas.text([cell(column), min[1]], VALUE, title);
    }
    let rows = table.rows;
    let body = max[1] - min[1] - header_h - table_footer_h(line);
    let visible = ((body / row_h).floor() as usize).max(1);
    *scroll = nav::scroll_to(table.cursor, *scroll, visible, rows.len());
    let mut y = min[1] + header_h;
    for (index, row) in rows.iter().enumerate().skip(*scroll).take(visible) {
        let row_min = [min[0], y];
        let row_max = [max[0], y + row_h];
        if index == table.cursor {
            let (fill, bar) = if table.focused {
                (CELL_ON, CURSOR_EDGE)
            } else {
                (CELL_BG, style::BRONZE_DIM)
            };
            canvas.rect(row_min, row_max, fill);
            canvas.rect(row_min, [row_min[0] + 3.0, row_max[1]], bar);
        } else if canvas.inside(row_min, row_max) {
            canvas.rect(row_min, row_max, ROW_HOVER);
        }
        let cells = [
            row.weapon.clone(),
            weapons::display_name(row.infusion).to_owned(),
            format!("{:.0}", row.damage),
            row.grip.clone(),
            row.class.clone(),
        ];
        let text_y = y + (row_h - line) * 0.5;
        for (column, text) in cells.iter().enumerate() {
            canvas.text(
                [cell(column), text_y],
                TEXT,
                &clip(canvas.ui, text, room(column)),
            );
        }
        canvas
            .targets
            .push((row_min, row_max, Action::PickRow(index)));
        y += row_h;
    }
    let footer_y = max[1] - line;
    let mut page_x = max[0];
    if rows.len() > visible {
        let shown_to = (*scroll + visible).min(rows.len());
        let page = format!("{}-{shown_to} of {}", *scroll + 1, rows.len());
        page_x = max[0] - canvas.width(&page);
        canvas.text([page_x, footer_y], DIM, &page);
    }
    if let Some(row) = rows.get(table.cursor) {
        let optional =
            |value: Option<f32>| value.map_or_else(|| "-".to_owned(), |v| format!("x{v:.2}"));
        let by_type = DAMAGE_TYPES
            .iter()
            .zip(row.ar_by_type)
            .map(|(name, ar)| format!("{name} {ar:.0}"))
            .collect::<Vec<_>>()
            .join("  ");
        let detail = format!(
            "{}:  {by_type}   hyper armour {}   counter {}",
            row.weapon,
            optional(row.hyperarmor),
            optional(row.counter)
        );
        canvas.text(
            [min[0] + inset, footer_y],
            DIM,
            &clip(canvas.ui, &detail, page_x - min[0] - inset - 16.0),
        );
    }
}

/// How tall the lower half has to be for what it shows now, so the panel is no taller than its
/// content: the table's rows, the build's lines, the refusal's reasons. `width` is the half's.
fn content_height(panel: &Panel, ui: &Ui, width: f32, line: f32) -> f32 {
    let lines = |n: usize| n as f32 * (line + 4.0);
    let button_h = line + BUTTON_PAD_Y * 2.0;
    let refusal = |reasons: &[String], fixes: usize| {
        let wrapped: usize = reasons
            .iter()
            .map(|reason| wrap(ui, reason, width - 24.0).len())
            .sum();
        let fix_rows = if fixes == 0 {
            0.0
        } else {
            // The heading, then the buttons, which wrap onto a second row when many.
            8.0 + line + 4.0 + 2.0 * (button_h + 4.0)
        };
        line + wrapped as f32 * (line + 2.0) + fix_rows + line
    };
    // The paperdoll at the design's size: its slots decide its height, and the pane beside them
    // shows as many of the build's other weapons as fit.
    if panel.doll_shown() {
        return doll_height(line, 1.0);
    }
    if !panel.refused.is_empty() {
        return refusal(&panel.refused, panel.fixes.len());
    }
    // draw_answer's defender line over the answer.
    let defender = if panel.answer_defender.is_some() {
        line + 6.0
    } else {
        0.0
    };
    defender
        + match &panel.answer {
            Some(Answer::Rows(rows)) => {
                table_header_h(line)
                    + rows.len().max(1) as f32 * table_row_h(line)
                    + table_footer_h(line)
            }
            Some(Answer::Refused(refused)) => refusal(&refused.lines, refused.fixes.len()),
            Some(Answer::Build(_)) => lines(6),
            // The heading, the violations, and the line offering the second Run.
            Some(Answer::FloorViolations(violations)) => {
                (violations.len() + 1) as f32 * (line + 2.0) + line + 8.0 + line
            }
            Some(Answer::Nothing(_)) | None => lines(1),
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

/// How tall the paperdoll is at scale `k`: five lines of headings, and the slots and the gaps
/// between them at `k` times the design's size.
fn doll_height(line: f32, k: f32) -> f32 {
    5.0 * line + DOLL_SCALED_H * k
}

/// The scale the paperdoll's slots fit `height` and `width` at: the design's own size where there
/// is room, smaller on a short or narrow screen, and never under [`DOLL_MIN_SCALE`].
fn doll_scale(line: f32, height: f32, width: f32) -> f32 {
    let tall = (height - 5.0 * line) / DOLL_SCALED_H;
    let wide = (width - DOLL_GAP - DETAIL_MIN_W) / paperdoll::GRID_W;
    tall.min(wide).clamp(DOLL_MIN_SCALE, 1.0)
}

/// How far below the paperdoll's top `slot` starts at scale `k`: under its band's heading line,
/// the belt's second row under its first, and the bolts under the arrows and a heading of their
/// own.
fn slot_top(slot: Slot, line: f32, k: f32) -> f32 {
    let band = line + (LABEL_GAP + paperdoll::BIG + BAND_GAP) * k;
    let first = line + LABEL_GAP * k;
    match slot {
        Slot::RightHand(_) | Slot::Ring(0 | 1) => first,
        Slot::LeftHand(_) | Slot::Ring(_) => band + first,
        Slot::Head | Slot::Chest | Slot::Hands | Slot::Legs => 2.0 * band + first,
        Slot::Item(i) if i < 5 => 3.0 * band + first,
        Slot::Item(_) => 3.0 * band + first + (paperdoll::SMALL + paperdoll::SMALL_GAP) * k,
        Slot::Arrows(_) => 3.0 * band + first,
        Slot::Bolts(_) => 3.0 * band + first + (paperdoll::SMALL + AMMO_GAP) * k + first,
    }
}

/// The silhouette the Equipment page draws in `slot` while it is empty.
const fn empty_art(slot: Slot) -> EmptySlot {
    match slot {
        Slot::RightHand(_) => EmptySlot::RightWeapon,
        Slot::LeftHand(_) => EmptySlot::LeftWeapon,
        Slot::Ring(_) => EmptySlot::Ring,
        Slot::Head => EmptySlot::Head,
        Slot::Chest => EmptySlot::Chest,
        Slot::Hands => EmptySlot::Hands,
        Slot::Legs => EmptySlot::Legs,
        Slot::Item(_) => EmptySlot::Item,
        Slot::Arrows(_) => EmptySlot::Arrows,
        Slot::Bolts(_) => EmptySlot::Bolts,
    }
}

/// `Sanctum Crossbow (Lightning)`; a ring or a piece of armour by its name alone.
fn piece_name(piece: &Piece) -> String {
    match piece.infusion {
        Some(infusion) => format!("{} ({})", piece.name, weapons::display_name(infusion)),
        None => piece.name.clone(),
    }
}

/// The paperdoll for the log, written once per build: every slot it fills, with what and under
/// which item id, the armour it leaves bare, and how many slots Apply leaves alone.
fn doll_line(doll: &Paperdoll) -> String {
    let mut filled = Vec::new();
    let mut bare = Vec::new();
    let mut kept = 0usize;
    for (slot, held) in doll.slots() {
        match held {
            Held::Item(piece) => filled.push(format!(
                "{} = {} [{}]",
                slot.label(),
                piece_name(piece),
                piece
                    .item
                    .map_or_else(|| "no item id".to_owned(), |id| format!("item {id}"))
            )),
            Held::Bare => bare.push(slot.label()),
            Held::Kept => kept += 1,
        }
    }
    format!(
        "{} | bare: {} | {kept} slots Apply leaves as they are",
        filled.join(", "),
        if bare.is_empty() {
            "none".to_owned()
        } else {
            bare.join(", ")
        }
    )
}

/// The build in one line where the weapon's parameters were: its class and level, its weapon,
/// infusion and grip, and at the right where it ranks among the builds nearest it.
fn draw_summary(panel: &Panel, canvas: &Canvas<'_>, at: [f32; 2], right: f32) {
    let Some(build) = &panel.generated else {
        return;
    };
    let rank = panel
        .generated_flex
        .as_ref()
        .map(flex_rank)
        .unwrap_or_default();
    let rank_x = right - canvas.width(&rank);
    canvas.text([rank_x, at[1]], DIM, &rank);
    let (key, infusion) = &build.primary;
    let weapon = weapons::by_key(key).map_or(key.as_str(), |row| row.name);
    let head = format!("{}, {}", build.class, sl_label(&build.stats, build.sl));
    let tail = format!(
        "{weapon} ({}), {}{}",
        weapons::display_name(*infusion),
        if build.two_handed {
            "two-handed"
        } else {
            "one-handed"
        },
        if build.stub { "   [stub build]" } else { "" }
    );
    let room = rank_x - GAP * 2.0;
    canvas.text(at, TITLE, &clip(canvas.ui, &head, room - at[0]));
    let x = at[0] + canvas.width(&head);
    if x < room {
        let x = x + canvas.dot(x, at[1]);
        canvas.text(
            [x, at[1]],
            TITLE,
            &clip(canvas.ui, &tail, (room - x).max(0.0)),
        );
    }
}

/// `text` wrapped and centred in a slot, dim: what a slot shows for an item the game has no icon
/// for.
fn draw_in_slot(canvas: &Canvas<'_>, text: &str, lo: [f32; 2], hi: [f32; 2]) {
    let room = hi[0] - lo[0] - 8.0;
    let parts = wrap(canvas.ui, text, room);
    let step = canvas.line + 2.0;
    let shown = parts
        .len()
        .min(((hi[1] - lo[1]) / step).floor().max(1.0) as usize);
    let mut y = (lo[1] + hi[1] - step * shown as f32) * 0.5;
    for part in parts.iter().take(shown) {
        let part = clip(canvas.ui, part, room);
        canvas.text([(lo[0] + hi[0] - canvas.width(&part)) * 0.5, y], DIM, &part);
        y += step;
    }
}

/// The generated build as the Equipment page lays a character out, with the pane describing a slot
/// beside it. A filled slot shows its item's icon; an empty one the silhouette the page itself
/// draws there. Every slot is a click target that moves the cursor onto it.
fn draw_paperdoll(panel: &mut Panel, canvas: &mut Canvas<'_>, (min, max): ([f32; 2], [f32; 2])) {
    let Some(doll) = panel.doll.clone() else {
        return;
    };
    let line = canvas.line;
    let k = doll_scale(line, max[1] - min[1], max[0] - min[0]);
    let (mut drawn, mut missing, mut loading, mut art) = (Vec::new(), Vec::new(), 0usize, true);
    for (slot, held) in doll.slots() {
        let slot = *slot;
        let x = min[0] + slot.x() * k;
        let y = min[1] + slot_top(slot, line, k);
        if let Some(heading) = slot.group() {
            canvas.text([x, y - LABEL_GAP * k - line], DIM, heading);
        }
        let side = slot.side() * k;
        let (lo, hi) = ([x, y], [x + side, y + side]);
        let cursor = canvas.on(Some(Control::Slot(slot)));
        let filled = matches!(held, Held::Item(_));
        let fill = if cursor {
            CELL_ON
        } else if canvas.inside(lo, hi) {
            CELL_HOVER
        } else if filled {
            CELL_BG
        } else {
            SLOT_EMPTY
        };
        canvas.rect(lo, hi, fill);
        match held {
            Held::Item(piece) => {
                let inset = SLOT_ICON_INSET * k;
                let status = piece.item.map_or(IconDraw::Missing, |id| {
                    ds2_overlay::panels::item_icon(
                        &canvas.list,
                        id,
                        [lo[0] + inset, lo[1] + inset],
                        [hi[0] - inset, hi[1] - inset],
                        Some(ds2_overlay::item_icon::SLOT_ART),
                    )
                });
                match status {
                    IconDraw::Drawn => drawn.push(slot.label()),
                    IconDraw::Loading => loading += 1,
                    IconDraw::Missing => {
                        missing.push(slot.label());
                        draw_in_slot(canvas, &piece.name, lo, hi);
                    }
                }
            }
            Held::Bare | Held::Kept => {
                art &= ds2_overlay::panels::empty_slot(
                    &canvas.list,
                    empty_art(slot),
                    lo,
                    hi,
                    EMPTY_FILL,
                );
            }
        }
        // The cursor is the design's 2px bronze edge. The slot the pane describes keeps a dimmer
        // one while the cursor is elsewhere, so the pane is never about an unmarked slot.
        let (edge, thickness) = if cursor {
            (CURSOR_EDGE, 2.0)
        } else if slot == panel.slot {
            (style::BRONZE_DIM, 1.0)
        } else if filled {
            (ICON_EDGE, 1.0)
        } else {
            (SLOT_EMPTY_EDGE, 1.0)
        };
        let half = thickness * 0.5;
        canvas
            .list
            .add_rect(
                [lo[0] + half, lo[1] + half],
                [hi[0] - half, hi[1] - half],
                edge,
            )
            .thickness(thickness)
            .build();
        canvas.targets.push((lo, hi, Action::PickSlot(slot)));
    }
    let detail_x = min[0] + paperdoll::GRID_W * k + DOLL_GAP;
    draw_doll_detail(panel, canvas, &doll, [detail_x, min[1]], max);
    if !panel.doll_logged && loading == 0 {
        panel.doll_logged = true;
        let display = canvas.ui.io().display_size;
        log_line(format_args!(
            "{LOG_PREFIX} paperdoll drawn at {k:.2} of the design's size on a {:.0}x{:.0} target, \
             cursor on {:?}: icons drawn for [{}], none for [{}]; empty slots {}",
            display[0],
            display[1],
            panel.cursor,
            drawn.join(", "),
            missing.join(", "),
            if art {
                "show the Equipment page's silhouettes from In-game_01"
            } else {
                "are plain boxes: In-game_01 did not load"
            }
        ));
    }
}

/// One line of the pane: its parts, each in its own colour, with the design's dot between two.
type Note = Vec<([f32; 4], String)>;

/// What the pane says under a slot's item: how a weapon is held, why a ring is worn and what else
/// is granted beside it, why armour is missing, and what Apply does with a slot the build leaves
/// alone.
fn slot_notes(build: &GeneratedBuild, slot: Slot, held: &Held) -> Vec<Note> {
    match (slot, held) {
        (Slot::RightHand(_) | Slot::LeftHand(_), Held::Item(_)) => vec![vec![
            (
                TEXT,
                if build.two_handed {
                    "Wielded two-handed"
                } else {
                    "Wielded one-handed"
                }
                .to_owned(),
            ),
            (
                DIM,
                if build.two_handed {
                    "2H: STR requirement halved"
                } else {
                    "1H: full STR requirement"
                }
                .to_owned(),
            ),
        ]],
        (Slot::Ring(_), Held::Item(ring)) => {
            let prefix = format!("{}: ", ring.name);
            let why = build
                .ring_trades
                .iter()
                .find_map(|trade| trade.strip_prefix(prefix.as_str()))
                .map_or_else(
                    || {
                        (
                            DIM,
                            "Worn most by the builds nearest these stats".to_owned(),
                        )
                    },
                    |trade| (TEXT, format!("In place of stat points: {trade}")),
                );
            let mut notes = vec![vec![why]];
            if !build.common_rings.is_empty() {
                notes.push(vec![(
                    DIM,
                    format!(
                        "Common to most builds, one each: {}",
                        build.common_rings.join(", ")
                    ),
                )]);
            }
            notes
        }
        (Slot::Head | Slot::Chest | Slot::Hands | Slot::Legs, Held::Item(_)) => build
            .trade
            .iter()
            .map(|trade| {
                vec![
                    (
                        TEXT,
                        format!(
                            "Poise {:.0}: holds through counter hits up to {:.0}",
                            trade.poise, trade.poise_target
                        ),
                    ),
                    (
                        DIM,
                        format!(
                            "R1 lands in {:.0}% of trades, {:+.1} damage per trade",
                            trade.trade_rate * 100.0,
                            trade.exchange
                        ),
                    ),
                ]
            })
            .chain(
                build
                    .armor_note
                    .iter()
                    .map(|note| vec![(WARN, note.clone())]),
            )
            .collect(),
        (_, Held::Item(_)) => build
            .armor_note
            .iter()
            .map(|note| vec![(WARN, note.clone())])
            .collect(),
        (_, Held::Bare) => vec![
            vec![(
                DIM,
                build
                    .armor_note
                    .clone()
                    .unwrap_or_else(|| "The build wears nothing here".to_owned()),
            )],
            vec![(WARN, "Apply leaves what you wear here".to_owned())],
        ],
        (_, Held::Kept) => vec![vec![(DIM, "Apply leaves what you have here".to_owned())]],
    }
}

/// The pane beside the paperdoll: the slot the cursor is on -- its name, its item and what that
/// means -- over the build's stats, the weapons it wields and the load it leaves, its spells, and
/// its other weapons with their attack, as many as fit.
fn draw_doll_detail(
    panel: &Panel,
    canvas: &mut Canvas<'_>,
    doll: &Paperdoll,
    at: [f32; 2],
    max: [f32; 2],
) {
    let Some(build) = &panel.generated else {
        return;
    };
    let line = canvas.line;
    let step = line + DETAIL_LINE_GAP;
    let [x, mut y] = at;
    let width = max[0] - x;
    let title_h = ds2_overlay::panels::title_height(canvas.ui);

    // The slot, its item, and two lines on it -- always two, so nothing under them moves as the
    // cursor does.
    let (slot, held) = (panel.slot, doll.held(panel.slot));
    canvas.text([x, y], DIM, &slot.label());
    y += line + 4.0;
    match held {
        Held::Item(piece) => {
            canvas.big_text([x, y], TITLE, &piece.name, width);
            if let Some(infusion) = piece.infusion {
                let name_w = ds2_overlay::panels::title_width(canvas.ui, &piece.name);
                let tag = format!(" ({})", weapons::display_name(infusion));
                canvas.big_text([x + name_w, y], VALUE, &tag, (width - name_w).max(0.0));
            }
        }
        Held::Bare => canvas.big_text([x, y], DIM, "Bare", width),
        Held::Kept => canvas.big_text([x, y], DIM, "Not in this build", width),
    }
    y += title_h + 4.0;
    let notes = slot_notes(build, slot, held);
    for index in 0..2 {
        if let Some(note) = notes.get(index) {
            let mut text_x = x;
            for (part, (color, text)) in note.iter().enumerate() {
                if part > 0 {
                    text_x += canvas.dot(text_x, y);
                }
                let shown = clip(canvas.ui, text, (max[0] - text_x).max(0.0));
                canvas.text([text_x, y], *color, &shown);
                text_x += canvas.width(&shown);
            }
        }
        y += line + 4.0;
    }
    y += SUMMARY_BOTTOM;
    canvas.rule(x, max[0], y);
    y += 1.0 + DETAIL_SECTION_GAP;

    // The build's stats, a label over a value, across the pane.
    canvas.text([x, y], DIM, "Stats");
    y += line + LABEL_GAP;
    let column = width / STAT_COUNT as f32;
    for (index, label) in STAT_LABELS.iter().enumerate() {
        let centre = x + column * (index as f32 + 0.5);
        let value = build.stats[index].to_string();
        canvas.text([centre - canvas.width(label) * 0.5, y], DIM, label);
        canvas.text(
            [centre - canvas.width(&value) * 0.5, y + line + 2.0],
            TITLE,
            &value,
        );
    }
    y += 2.0 * line + 2.0 + DETAIL_SECTION_GAP;

    // How many weapons the stats wield and the load the armour and rings leave for one.
    let mut lines: Vec<([f32; 4], String)> = Vec::new();
    if let Some(flex) = &panel.generated_flex {
        lines.push((TEXT, format!("Weapons: {}", flex_wields(flex))));
        lines.push((DIM, format!("Load: {}", flex_load(flex))));
        lines.push((DIM, FLYNN_NOTE.to_owned()));
    }
    // The spells Apply attunes, and what to cast them with.
    if !build.spells.is_empty() {
        lines.push((
            TEXT,
            format!(
                "Spells: {} ({} of {} attunement slots)",
                build.spell_names.join(", "),
                build.slots_used,
                build.slots
            ),
        ));
        let catalysts = if build.catalysts.is_empty() {
            "none these stats can wield casts them".to_owned()
        } else {
            build
                .catalysts
                .iter()
                .map(|pick| format!("{} {} {:.0}", pick.school, pick.name, pick.power))
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push((TEXT, format!("Catalyst: {catalysts}")));
    }
    for (color, text) in &lines {
        if y + line > max[1] {
            return;
        }
        canvas.text([x, y], *color, &clip(canvas.ui, text, width));
        y += step;
    }
    if !lines.is_empty() {
        y += DETAIL_SECTION_GAP - DETAIL_LINE_GAP;
    }

    // Its other weapons, one-handed first as the design lists them, then the two-hand-only ones,
    // each with its attack at the right, as many as fit.
    for (heading, rows) in [
        ("Other one-handed picks for this build", &build.weapons_1h),
        ("Two-hand only picks", &build.weapons_2h_only),
    ] {
        if rows.is_empty() || y + step + line > max[1] {
            continue;
        }
        canvas.text([x, y], DIM, heading);
        y += step;
        for row in rows {
            if y + line > max[1] {
                break;
            }
            let damage = format!("{:.0}", row.damage);
            let damage_x = max[0] - canvas.width(&damage);
            canvas.text([damage_x, y], TITLE, &damage);
            let name = clip(canvas.ui, &row.weapon, damage_x - x - GAP * 2.0);
            canvas.text([x, y], TEXT, &name);
            let tag = format!(" ({})", weapons::display_name(row.infusion));
            let tag_x = x + canvas.width(&name);
            if tag_x + canvas.width(&tag) < damage_x - GAP {
                canvas.text([tag_x, y], DIM, &tag);
            }
            y += step;
        }
        y += DETAIL_SECTION_GAP - DETAIL_LINE_GAP;
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
        (TITLE, format!("Apply {} SL {}?", build.class, build.sl)),
        (
            WARN,
            format!(
                "Soul memory rises to SL {}. This cannot be undone.",
                build.sl
            ),
        ),
        (
            TEXT,
            format!(
                "Stats and class become {}'s, unless the build is lower.",
                build.class
            ),
        ),
        (
            TEXT,
            format!("Grants up to {grants} items you lack, and equips the gear."),
        ),
    ];
    let width = lines
        .iter()
        .map(|(_, text)| canvas.width(text))
        .fold(0.0_f32, f32::max)
        + PAD * 2.0;
    let height = PAD * 2.0 + (line + 6.0) * lines.len() as f32 + canvas.button_h() + GAP;
    let min = [(display[0] - width) * 0.5, (display[1] - height) * 0.5];
    let max = [min[0] + width, min[1] + height];
    canvas
        .list
        .add_rect(min, max, LIST_BG)
        .filled(true)
        .rounding(style::ROUNDING)
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

/// The weapon picker: a window over the panel listing every weapon its filter matches, each with
/// the game's own icon and what the panel's stats make of it. Only its own controls take a click;
/// one outside it closes it. Returns its rectangle.
fn draw_picker(
    panel: &mut Panel,
    canvas: &mut Canvas<'_>,
    display: [f32; 2],
) -> ([f32; 2], [f32; 2]) {
    let ui = canvas.ui;
    let line = canvas.line;
    let title_h = ds2_overlay::panels::title_height(ui);
    // The title, the filter and the rule under it; the rule over the key bar and the bar.
    let head_h =
        PICKER_TITLE_TOP + title_h + PICKER_TITLE_BOTTOM + FILTER_H + PICKER_FILTER_BOTTOM + 1.0;
    let foot_h = 1.0 + PICKER_KEYS_TOP + line + PICKER_KEYS_BOTTOM;
    let fit = (display[1] - PICKER_MARGIN * 2.0 - head_h - foot_h) / PICKER_ROW_H;
    let visible = (fit.floor().max(1.0) as usize).min(LIST_VISIBLE);
    panel.picker_rows = visible;
    let width = PICKER_W.min(display[0] - 64.0);
    let height = head_h + PICKER_ROW_H * visible as f32 + foot_h;
    let min = [
        ((display[0] - width) * 0.5).round(),
        ((display[1] - height) * 0.5).max(0.0).round(),
    ];
    let max = [min[0] + width, min[1] + height];
    let (inner, right) = (min[0] + INSET, max[0] - INSET);

    canvas.targets.clear();
    canvas.rect([0.0, 0.0], display, DIM_COVER);
    canvas
        .targets
        .push(([0.0, 0.0], display, Action::CloseList));
    canvas.rect(min, max, PANEL_BG);
    if !ds2_overlay::panels::frame(&canvas.list, min, max, display[1]) {
        canvas.edge(min, max, PANEL_EDGE);
    }
    canvas.targets.push((min, max, Action::Nothing));

    let title_y = min[1] + PICKER_TITLE_TOP;
    ds2_overlay::panels::title(ui, &canvas.list, [inner, title_y], TITLE, "Weapon");
    let close_w = canvas.width("Close") + PAIR_PAD * 2.0;
    canvas.pair(
        right + PAIR_PAD - close_w,
        title_y + (title_h - canvas.row) * 0.5,
        "",
        "Close",
        Some(Action::CloseList),
        None,
    );

    // The filter: what is typed, which the list narrows to as it is typed.
    let filter_y = title_y + title_h + PICKER_TITLE_BOTTOM;
    let text_y = filter_y + (FILTER_H - line) * 0.5;
    canvas.text([inner, text_y], DIM, "Filter");
    let box_min = [inner + canvas.width("Filter") + FILTER_GAP, filter_y];
    let box_max = [right, filter_y + FILTER_H];
    canvas.rect(box_min, box_max, style::SLATE);
    canvas.rect([box_min[0], box_max[1] - 2.0], box_max, CURSOR_EDGE);
    let spec = panel.spec(Field::Search, "type part of a name", true);
    let shown = canvas.field_shown(&spec, box_max[0] - box_min[0] - BUTTON_PAD_X * 2.0);
    canvas.field_text([box_min[0] + BUTTON_PAD_X, text_y], &spec, &shown, TEXT);
    let rule_y = box_max[1] + PICKER_FILTER_BOTTOM;
    canvas.rule(min[0] + 1.0, max[0] - 1.0, rule_y);

    let rows = panel.weapon_rows();
    let total = rows.len();
    panel.list_scroll = panel.list_scroll.min(total.saturating_sub(visible));
    if !std::mem::replace(&mut panel.picker_logged, true) {
        log_line(format_args!(
            "{LOG_PREFIX} weapon picker open: {visible} rows of {PICKER_ROW_H}px on a {:.0}x{:.0} \
             target, {total} weapons match {:?}, cards from the {} backend",
            display[0],
            display[1],
            panel.edit,
            if backend().is_stub() {
                "stub"
            } else {
                "corpus"
            }
        ));
    }
    let rows_top = rule_y + 1.0;
    let rows_h = PICKER_ROW_H * visible as f32;
    let rows_left = min[0] + PICKER_ROWS_INSET;
    let rows_right = right - PICKER_SCROLL_W - PICKER_SCROLL_GAP;
    if rows.is_empty() {
        canvas.text(
            [inner, rows_top + PICKER_ROW_PAD_Y],
            DIM,
            "No weapon matches",
        );
    }
    let (stats, infusion) = (panel.state.stats, panel.state.infusion);
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(panel.list_scroll)
        .take(visible)
    {
        let y = rows_top + PICKER_ROW_H * (index - panel.list_scroll) as f32;
        let card = backend().weapon_card(row.key, infusion, &stats);
        draw_picker_row(
            canvas,
            row,
            card.as_ref(),
            [rows_left, y],
            rows_right,
            index == panel.list_cursor,
        );
        canvas.targets.push((
            [rows_left, y],
            [rows_right, y + PICKER_ROW_H],
            Action::ChooseWeapon(row.key),
        ));
    }

    // Where the rows shown sit in the whole list.
    let track = [rows_right + PICKER_SCROLL_GAP, rows_top];
    canvas.rect(
        track,
        [track[0] + PICKER_SCROLL_W, rows_top + rows_h],
        CELL_BG,
    );
    if total > visible {
        let thumb_h = (rows_h * visible as f32 / total as f32).max(24.0);
        let thumb_y =
            rows_top + (rows_h - thumb_h) * panel.list_scroll as f32 / (total - visible) as f32;
        canvas.rect(
            [track[0], thumb_y],
            [track[0] + PICKER_SCROLL_W, thumb_y + thumb_h],
            style::BRONZE_DIM,
        );
    }

    // The list's own keys, and which row of how many is highlighted.
    let keys_rule_y = rows_top + rows_h;
    canvas.rule(min[0] + 1.0, max[0] - 1.0, keys_rule_y);
    let hint_y = keys_rule_y + 1.0 + PICKER_KEYS_TOP;
    const PAD_HINT: [(&str, &str); 4] = [
        (button(Button::DPad), "Move"),
        (button(Button::A), "Select"),
        (button(Button::Bumpers), "Page"),
        (button(Button::B), "Back"),
    ];
    const KEY_HINT: [(&str, &str); 4] = [
        ("Up/Down", "Move"),
        ("Enter", "Select"),
        ("PgUp PgDn", "Page"),
        ("Esc", "Back"),
    ];
    let hints: &[(&str, &str)] = if panel.reader.pad_last() {
        &PAD_HINT
    } else {
        &KEY_HINT
    };
    let count = if total == 0 {
        String::new()
    } else {
        format!("{}/{total}", panel.list_cursor.min(total - 1) + 1)
    };
    let count_x = right - canvas.width(&count);
    ds2_overlay::panels::hint_bar(ui, &canvas.list, [inner, hint_y], count_x - GAP, hints);
    canvas.text([count_x, hint_y], DIM, &count);
    (min, max)
}

/// One row of the weapon picker, from `at` to `right`, laid out against its icon: the art at the
/// texture's own size, the name level with the art's top, the stats grid ending level with its
/// bottom, and each attack line on a line of that grid. Without a `card` -- the stub backend has
/// none -- only the icon, the name and the class.
fn draw_picker_row(
    canvas: &Canvas<'_>,
    row: &WeaponRow,
    card: Option<&WeaponCard>,
    at: [f32; 2],
    right: f32,
    highlighted: bool,
) {
    let line = canvas.line;
    let bottom = at[1] + PICKER_ROW_H;
    if highlighted {
        canvas.rect(at, [right, bottom], CELL_ON);
        canvas.rect(at, [at[0] + PICKER_BAR, bottom], CURSOR_EDGE);
    } else if canvas.inside(at, [right, bottom]) {
        canvas.rect(at, [right, bottom], ROW_HOVER);
    }
    canvas.rule(at[0], right, bottom - 1.0);

    // The icon: the band of its texture the art is in, in a frame that fits the band exactly. The
    // class's name stands in where the game has no icon.
    let icon_min = [
        at[0] + PICKER_BAR + PICKER_ROW_PAD_X,
        at[1] + PICKER_ROW_PAD_Y,
    ];
    let icon_max = [icon_min[0] + ICON_W, icon_min[1] + ICON_H];
    canvas.rect(icon_min, icon_max, CELL_BG);
    let drawn = weapons::item_id(row.key).map_or(IconDraw::Missing, |id| {
        ds2_overlay::panels::item_icon(
            &canvas.list,
            id,
            icon_min,
            icon_max,
            Some(ds2_overlay::item_icon::WEAPON_ART_FRACTION),
        )
    });
    if drawn == IconDraw::Missing {
        let class = if row.class.is_empty() {
            "No icon"
        } else {
            row.class
        };
        let room = ICON_W - 20.0;
        let lines = wrap(canvas.ui, class, room);
        let step = line + 2.0;
        let mut y = (icon_min[1] + icon_max[1] - step * lines.len() as f32) * 0.5;
        for text in &lines {
            let text = clip(canvas.ui, text, room);
            let x = (icon_min[0] + icon_max[0] - canvas.width(&text)) * 0.5;
            canvas.text([x, y], DIM, &text);
            y += step;
        }
    }
    canvas
        .list
        .add_rect(icon_min, icon_max, ICON_EDGE)
        .thickness(1.0)
        .build();

    // The top of each of the grid's three lines, the last ending at the icon's bottom.
    let grid_y = |index: usize| {
        icon_max[1] - line - (2usize.saturating_sub(index)) as f32 * (line + PICKER_GRID_GAP)
    };

    // The attack by type, right-aligned, unless the stats cannot hold the weapon at all: its last
    // line on the grid's last, each one above on the line above, closer only when there are more
    // types than the icon is tall for.
    let attack_right = right - PICKER_ROW_GAP;
    let attack: Vec<String> = card
        .filter(|card| card.wield != Wield::Neither)
        .map(|card| {
            ATTACK_NAMES
                .iter()
                .zip(card.attack)
                .filter_map(|(name, value)| value.map(|value| format!("{name} {value:.0}")))
                .collect()
        })
        .unwrap_or_default();
    if !attack.is_empty() {
        let step = (line + PICKER_GRID_GAP).min((ICON_H - line) / attack.len() as f32);
        let heading_y = grid_y(2) - step * attack.len() as f32;
        canvas.text(
            [attack_right - canvas.width("Attack"), heading_y],
            DIM,
            "Attack",
        );
        for (index, text) in attack.iter().enumerate() {
            let y = heading_y + step * (index + 1) as f32;
            canvas.text([attack_right - canvas.width(text), y], TEXT, text);
        }
    }

    // The name level with the icon's top and the class under it, then what the weapon asks for,
    // what it weighs and how it can be held.
    let text_x = icon_max[0] + PICKER_ROW_GAP;
    let text_w = attack_right - PICKER_ATTACK_W - PICKER_ROW_GAP - text_x;
    let title_h = ds2_overlay::panels::title_height(canvas.ui);
    canvas.big_text([text_x, icon_min[1]], TITLE, row.name, text_w);
    canvas.text(
        [text_x, icon_min[1] + title_h + 2.0],
        DIM,
        &clip(canvas.ui, row.class, text_w),
    );
    let Some(card) = card else {
        return;
    };
    let value_x = text_x + PICKER_LABEL_W + PICKER_LABEL_GAP;
    let mark = (line * 0.6).round();
    let mark_y = |y: f32| y + (line - mark) * 0.5;

    let mut y = grid_y(0);
    canvas.text([text_x, y], DIM, "Requires");
    let mut x = value_x;
    for req in &card.requirements {
        let text = format!(
            "{} {}",
            STAT_LABELS.get(req.stat).copied().unwrap_or("?"),
            req.value
        );
        canvas.text([x, y], TEXT, &text);
        x += canvas.width(&text);
        if !req.met {
            x += 7.0 + canvas.cross([x + 7.0, mark_y(y)], mark);
        }
        x += PICKER_REQ_GAP;
    }
    if card.requirements.is_empty() {
        canvas.text([value_x, y], DIM, "None");
    }

    y = grid_y(1);
    canvas.text([text_x, y], DIM, "Weight");
    canvas.text([value_x, y], TEXT, &format!("{:.1}", card.weight));

    y = grid_y(2);
    canvas.text([text_x, y], DIM, "Grip");
    match card.wield {
        Wield::Both => canvas.text([value_x, y], TEXT, "1H / 2H"),
        // Not colour alone: the tag says it in words.
        Wield::TwoHandedOnly => {
            let text = "2H only";
            let tag_w = canvas.width(text) + 20.0;
            canvas.rect([value_x, y], [value_x + tag_w, y + line], style::RUST);
            canvas.text([value_x + 10.0, y], TAG_TEXT, text);
        }
        Wield::Neither => {
            let mark_w = canvas.cross([value_x, mark_y(y)], mark);
            canvas.text([value_x + mark_w, y], TEXT, "Can't wield");
        }
    }
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
        .rounding(style::ROUNDING)
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

/// `text` broken at spaces into lines that each fit `width`; a word wider than `width` gets a line
/// of its own.
fn wrap(ui: &Ui, text: &str, width: f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if current.is_empty() || ui.calc_text_size(&candidate)[0] <= width {
            current = candidate;
        } else {
            out.push(std::mem::replace(&mut current, word.to_owned()));
        }
    }
    out.push(current);
    out
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
