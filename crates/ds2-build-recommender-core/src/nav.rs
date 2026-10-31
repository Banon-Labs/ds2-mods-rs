//! The panel without a mouse: which control the D-pad cursor is on, where each direction takes it,
//! and how a numeric field moves under Left and Right.
//!
//! The panel is laid out in rows, and the cursor walks them the way it is drawn: Up and Down change
//! row, Left and Right move along one. [`layout`] is that grid for the current mode and answer, so
//! a mode whose options are different gets a different row and the cursor never lands on a control
//! that is not on screen. The keyboard is only needed to type a weapon name; every other control is
//! reached here and changed with a press.

use crate::model::{Mode, PanelState, SL_MAX, STAT_COUNT, STAT_MAX, STAT_MIN};
use crate::paperdoll::{self, Slot};

/// One thing the cursor can sit on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// The header's close button.
    Close,
    /// One of the nine stats, by index into [`crate::model::STAT_LABELS`].
    Stat(usize),
    /// The soul-level override.
    SlOverride,
    /// "use my character's stats".
    UseCharacter,
    /// The weapon: its search field and the list under it.
    Weapon,
    /// The infusion drop-down.
    Infusion,
    /// The grip, which A cycles: what Optimize for weapon and Generate Build build for.
    Grip,
    /// The goal (objective), which A cycles.
    Objective,
    /// Who the damage goal is scored against: the list of the average and the four armour slots.
    Defender,
    /// One mode tab.
    Mode(Mode),
    /// Weapons for stats: one-handed only.
    OneHand,
    /// Weapons for stats: the class drop-down.
    Class,
    /// Weapons for stats: best per class.
    PerClass,
    /// Weapons for stats: the R1 window.
    Window,
    /// Weapons for stats: rank by attack rating.
    RawAr,
    /// Minimum for weapon: may two-hand.
    TwoHand,
    /// Similar builds: how many neighbours.
    SimilarK,
    /// Similar builds: bleed only.
    Bleed,
    /// Similar builds: poison only.
    Poison,
    /// Optimize for weapon: rank every infusion of the chosen weapon.
    BestInfusion,
    /// The results table, to scroll it.
    Results,
    /// Generate Build.
    Generate,
    /// Generate Build may leave the armour off.
    AllowNaked,
    /// Optimize for weapon and Generate Build drop the soul level's floors.
    IgnoreFloors,
    /// How Optimize for weapon, Generate Build and Best weapons spend the free points.
    Spend,
    /// One of a refusal's fix buttons, by index into its fixes.
    Fix(usize),
    /// The spells Generate Build must cast: the list to choose them from.
    Spells,
    /// Show build / Show results.
    ShowToggle,
    /// Apply to character.
    Apply,
    /// One slot of the generated build's paperdoll, whose item the pane beside it describes.
    Slot(Slot),
}

impl Control {
    /// Whether this control is a number Left and Right change once it is entered.
    pub const fn is_numeric(self) -> bool {
        matches!(
            self,
            Control::Stat(_) | Control::SlOverride | Control::Window | Control::SimilarK
        )
    }
}

/// A D-pad or arrow-key direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// D-pad up, or the up arrow.
    Up,
    /// D-pad down, or the down arrow.
    Down,
    /// D-pad left, or the left arrow.
    Left,
    /// D-pad right, or the right arrow.
    Right,
}

/// What decides the grid besides the mode: whether there is a results table to scroll and whether
/// a generated build exists to show or apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// The mode whose options make the options row.
    pub mode: Mode,
    /// The answer on screen is a table with rows in it.
    pub results: bool,
    /// Generate Build has produced a build.
    pub generated: bool,
    /// How many fix buttons a refusal on screen offers.
    pub fixes: usize,
    /// The generated build is on screen as a paperdoll. Its slots take the place of the weapon's
    /// parameters, the mode's options and the answer, as the design draws it: the build names its
    /// own weapon, infusion and grip.
    pub paperdoll: bool,
    /// Where the buttons that wrap -- the fixes and the footer -- were drawn: on how many lines, and
    /// which button on which.
    pub wrapped: Wrapped,
}

/// Where the mode's options are in [`layout`]'s rows: under the header, the stats, the mode tabs and
/// the weapon's parameters.
const OPTIONS_ROW: usize = 4;

/// The footer's buttons in the order they are drawn, in the groups kept together on a line.
///
/// Generate Build with the two choices only it reads (the armour and the spells); the two that
/// Optimize for weapon and Best weapons read as well (the floors and how the free points are
/// spent); and what is done with the build once there is one (look at it, put it on the
/// character).
pub fn footer(generated: bool) -> [Vec<Control>; 3] {
    let mut made = Vec::with_capacity(2);
    if generated {
        made.push(Control::ShowToggle);
    }
    made.push(Control::Apply);
    [
        vec![Control::Generate, Control::AllowNaked, Control::Spells],
        vec![Control::IgnoreFloors, Control::Spend],
        made,
    ]
}

/// One box of a [`wrap`]ped row: the line it is on, counted from the first, and its left edge and
/// width, measured from the row's left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The line, 0 for the first.
    pub line: usize,
    /// The box's left edge.
    pub x: f32,
    /// The box's width: what it asked for, or `room` if that was more.
    pub width: f32,
}

/// Lay groups of boxes `widths` wide left to right in lines `room` wide, `gap` between two boxes of
/// a group and `group_gap` between two groups on one line. Returns every box's place, in order.
///
/// A group goes on the line in progress only when all of it fits what is left of that line, and on
/// the next line otherwise, so a group is split only when it is wider than a whole line. Inside a
/// group, a box that does not fit what is left of its line starts the next one. A box wider than a
/// line is cut to `room`, and whoever draws it cuts its label to match: no box ever ends past
/// `room`.
pub fn wrap(groups: &[&[f32]], room: f32, gap: f32, group_gap: f32) -> Vec<Placed> {
    let room = room.max(0.0);
    let mut placed = Vec::new();
    let (mut line, mut x, mut empty) = (0, 0.0_f32, true);
    for group in groups {
        let widths: Vec<f32> = group.iter().map(|width| width.clamp(0.0, room)).collect();
        let whole = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
        if !empty && x + group_gap + whole > room {
            (line, x, empty) = (line + 1, 0.0, true);
        }
        for (index, width) in widths.into_iter().enumerate() {
            let lead = match (empty, index) {
                (true, _) => 0.0,
                (false, 0) => group_gap,
                (false, _) => gap,
            };
            let (at, lead) = if !empty && x + lead + width > room {
                (line + 1, 0.0)
            } else {
                (line, lead)
            };
            if at != line {
                (line, x) = (at, 0.0);
            }
            placed.push(Placed {
                line,
                x: x + lead,
                width,
            });
            (x, empty) = (x + lead + width, false);
        }
    }
    placed
}

/// Most wrapped buttons a [`Wrapped`] holds the places of: the footer's and a refusal's fixes.
pub const WRAPPED_MAX: usize = 32;

/// Where the panel last drew each button that it wraps: the footer's and a refusal's fixes.
///
/// Each button's line, counted within its own block, and the centre of its box, in whole pixels
/// from one left edge for every block. [`layout`] breaks each block's row where the drawing broke
/// it, and [`step_in`] takes Up and Down between two of those lines to the button drawn nearest
/// above or below, as between two rows of the paperdoll. The default is every block on one line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wrapped {
    drawn: [Option<(Control, usize, u16)>; WRAPPED_MAX],
}

impl Wrapped {
    /// `controls` as [`wrap`] placed them: the same order, one place each.
    pub fn new(controls: &[Control], placed: &[Placed]) -> Self {
        Self::default().with(controls, placed, 0.0)
    }

    /// These, and another block of `controls` as [`wrap`] placed them, its line drawn from `left`
    /// along the same edge. Past [`WRAPPED_MAX`] buttons are left without a place, which puts each
    /// on the line of the button before it.
    #[must_use]
    pub fn with(mut self, controls: &[Control], placed: &[Placed], left: f32) -> Self {
        let free = self.drawn.iter_mut().filter(|slot| slot.is_none());
        for (slot, (&control, place)) in free.zip(controls.iter().zip(placed)) {
            let centre = (left + place.x + place.width * 0.5)
                .round()
                .clamp(0.0, f32::from(u16::MAX)) as u16;
            *slot = Some((control, place.line, centre));
        }
        self
    }

    /// The line `control` was drawn on and its centre, if it is a wrapped button that was drawn.
    fn find(&self, control: Control) -> Option<(usize, u16)> {
        self.drawn
            .iter()
            .flatten()
            .find(|(drawn, ..)| *drawn == control)
            .map(|&(_, line, centre)| (line, centre))
    }

    /// One block's controls in order, cut into the lines they were drawn on. A button with no
    /// place yet -- the frame it first appears -- goes on the line of the one before it.
    fn rows(&self, controls: Vec<Control>) -> Vec<Vec<Control>> {
        let mut rows: Vec<Vec<Control>> = Vec::new();
        let mut on = None;
        for control in controls {
            let line = self.find(control).map(|(line, _)| line).or(on);
            match rows.last_mut() {
                Some(row) if line == on => row.push(control),
                _ => rows.push(vec![control]),
            }
            on = line;
        }
        rows
    }
}

/// The controls, row by row, as the panel draws them: the header's two buttons, the soul level and
/// the nine stats in one row, the mode tabs, the weapon's parameters, then the mode's options.
///
/// A generated build on screen puts its paperdoll's rows where the parameters and options were. A
/// refusal's fixes and the footer take as many rows each as [`Shape::wrapped`] says they were
/// drawn on.
///
/// Run is not among them. It is a press, X on the pad and R on the keyboard, from wherever the
/// cursor is, so it has no place of its own to walk to.
pub fn layout(shape: Shape) -> Vec<Vec<Control>> {
    let mut rows = vec![
        vec![Control::UseCharacter, Control::Close],
        std::iter::once(Control::SlOverride)
            .chain((0..STAT_COUNT).map(Control::Stat))
            .collect(),
        Mode::ALL.map(Control::Mode).to_vec(),
    ];
    if shape.paperdoll {
        rows.extend(
            paperdoll::ROWS
                .iter()
                .map(|row| row.iter().copied().map(Control::Slot).collect()),
        );
    } else {
        rows.push(vec![
            Control::Weapon,
            Control::Infusion,
            Control::Grip,
            Control::Objective,
            Control::Defender,
        ]);
        let options = match shape.mode {
            Mode::WeaponsForStats => vec![
                Control::OneHand,
                Control::Class,
                Control::PerClass,
                Control::Window,
                Control::RawAr,
            ],
            Mode::OptimizeForWeapon => vec![Control::BestInfusion],
            Mode::MinimumForWeapon => vec![Control::TwoHand],
            Mode::SimilarBuilds => vec![Control::SimilarK, Control::Bleed, Control::Poison],
        };
        rows.push(options);
        if shape.results {
            rows.push(vec![Control::Results]);
        }
        rows.extend(
            shape
                .wrapped
                .rows((0..shape.fixes).map(Control::Fix).collect()),
        );
    }
    rows.extend(shape.wrapped.rows(footer(shape.generated).concat()));
    rows
}

/// Where `control` is in `rows`: its row and its place along it.
pub fn locate(rows: &[Vec<Control>], control: Control) -> Option<(usize, usize)> {
    rows.iter().enumerate().find_map(|(row, controls)| {
        controls
            .iter()
            .position(|&c| c == control)
            .map(|column| (row, column))
    })
}

/// `cursor` if it is still on screen, or the control nearest to where it was.
///
/// That is the right-hand end of the options row for a mode option or the results table that went
/// away, Generate Build for a footer button, and the build's weapon for anything the paperdoll
/// covers.
pub fn resolve(rows: &[Vec<Control>], cursor: Control) -> Control {
    if locate(rows, cursor).is_some() {
        return cursor;
    }
    let primary = Control::Slot(Slot::PRIMARY);
    match cursor {
        Control::ShowToggle | Control::Apply | Control::Fix(_) => Control::Generate,
        _ if locate(rows, primary).is_some() => primary,
        _ => rows
            .get(OPTIONS_ROW)
            .and_then(|options| options.last())
            .copied()
            .unwrap_or(Control::Generate),
    }
}

/// Where one press of `dir` takes the cursor.
///
/// Left and Right wrap from a row's one end to its other; Up and Down stop at the top and bottom
/// rows, and land at
/// the same fraction of the way along the new row, so going down from the ninth stat lands at the
/// right-hand end of the row below rather than its start. Between two rows of the paperdoll they
/// land on the slot drawn nearest above or below instead: its rows are not the same width, and the
/// legs are over the fifth quick item, not over the second arrows.
pub fn step(rows: &[Vec<Control>], cursor: Control, dir: Dir) -> Control {
    let cursor = resolve(rows, cursor);
    let Some((row, column)) = locate(rows, cursor) else {
        return cursor;
    };
    let target_row = match dir {
        Dir::Left => {
            let len = rows[row].len();
            return rows[row][(column + len - 1) % len];
        }
        Dir::Right => return rows[row][(column + 1) % rows[row].len()],
        Dir::Up if row == 0 => return cursor,
        Dir::Up => row - 1,
        Dir::Down if row + 1 == rows.len() => return cursor,
        Dir::Down => row + 1,
    };
    if let Control::Slot(from) = cursor
        && let Some(nearest) = nearest_slot(&rows[target_row], from.centre_x())
    {
        return nearest;
    }
    let from_len = rows[row].len();
    let to_len = rows[target_row].len();
    let target_column = if from_len <= 1 || to_len <= 1 {
        0
    } else {
        // Rounded to nearest, in integers: column * (to-1) / (from-1).
        (column * (to_len - 1) * 2 + (from_len - 1)) / ((from_len - 1) * 2)
    };
    rows[target_row][target_column.min(to_len - 1)]
}

/// [`step`], for fixes and a footer that may be wrapped. What the panel moves its cursor with.
///
/// Up and Down between two lines of wrapped buttons land on the button drawn nearest above or below
/// rather than the one the same fraction of the way along: the lines hold different buttons of
/// different widths.
pub fn step_in(rows: &[Vec<Control>], wrapped: &Wrapped, cursor: Control, dir: Dir) -> Control {
    let cursor = resolve(rows, cursor);
    if let (Dir::Up | Dir::Down, Some((row, _)), Some((_, x))) =
        (dir, locate(rows, cursor), wrapped.find(cursor))
    {
        let target = match dir {
            Dir::Up => row.checked_sub(1),
            _ => Some(row + 1),
        };
        let nearest = target.and_then(|target| rows.get(target)).and_then(|to| {
            to.iter()
                .map(|&control| {
                    wrapped
                        .find(control)
                        .map(|(_, at)| (control, at.abs_diff(x)))
                })
                .collect::<Option<Vec<_>>>()?
                .into_iter()
                .min_by_key(|&(_, off)| off)
                .map(|(control, _)| control)
        });
        if let Some(nearest) = nearest {
            return nearest;
        }
    }
    step(rows, cursor, dir)
}

/// The slot in `row` whose centre is nearest `x`, in the grid's own pixels; the leftmost of two as
/// near. `None` for a row with no slot in it.
fn nearest_slot(row: &[Control], x: f32) -> Option<Control> {
    row.iter()
        .filter_map(|&control| match control {
            Control::Slot(slot) => Some((control, (slot.centre_x() - x).abs())),
            _ => None,
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(control, _)| control)
}

/// How far one press moves a number: Left/Right a small step, Up/Down a big one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// One small step down (Left).
    Down,
    /// One small step up (Right).
    Up,
    /// One big step down (Down).
    DownBig,
    /// One big step up (Up).
    UpBig,
}

impl Nudge {
    /// The nudge a direction means inside a numeric field.
    pub const fn from_dir(dir: Dir) -> Self {
        match dir {
            Dir::Left => Nudge::Down,
            Dir::Right => Nudge::Up,
            Dir::Down => Nudge::DownBig,
            Dir::Up => Nudge::UpBig,
        }
    }

    /// The signed step for a field whose small step is `small` and big step `big`.
    const fn signed(self, small: i32, big: i32) -> i32 {
        match self {
            Nudge::Down => -small,
            Nudge::Up => small,
            Nudge::DownBig => -big,
            Nudge::UpBig => big,
        }
    }
}

/// A stat's small and big steps.
pub const STAT_STEP: (i32, i32) = (1, 10);
/// The soul-level override's small and big steps.
pub const SL_STEP: (i32, i32) = (1, 10);
/// The R1 window's small and big steps, in tenths of a second.
pub const WINDOW_STEP_TENTHS: (i32, i32) = (1, 10);
/// The R1 window's largest value, in seconds. The same bound typing it has.
pub const WINDOW_MAX_S: f32 = 10.0;
/// The neighbour count's small and big steps.
pub const SIMILAR_K_STEP: (i32, i32) = (1, 10);
/// The neighbour count's lower bound. The same bound typing it has.
pub const SIMILAR_K_MIN: u16 = 1;
/// The neighbour count's upper bound. The same bound typing it has.
pub const SIMILAR_K_MAX: u16 = 500;

fn nudge_u16(value: u16, delta: i32, min: u16, max: u16) -> u16 {
    let moved = (i32::from(value) + delta).clamp(i32::from(min), i32::from(max));
    u16::try_from(moved).unwrap_or(min)
}

/// Move the number `control` holds by `nudge`, clamped to what that field accepts. Returns whether
/// anything changed, so a press at a bound is not reported as a change.
///
/// The soul-level override starts from the level the stats make when none is set, and stepping it
/// down to zero clears it, which is what typing `0` into it does.
pub fn nudge(state: &mut PanelState, control: Control, nudge: Nudge) -> bool {
    match control {
        Control::Stat(index) if index < STAT_COUNT => {
            let before = state.stats[index];
            let (small, big) = STAT_STEP;
            state.set_stat(
                index,
                nudge_u16(before, nudge.signed(small, big), STAT_MIN, STAT_MAX),
            );
            state.stats[index] != before
        }
        Control::SlOverride => {
            let before = state.sl_override;
            let from = before.unwrap_or_else(|| state.computed_sl());
            let (small, big) = SL_STEP;
            let moved = nudge_u16(from, nudge.signed(small, big), 0, SL_MAX);
            state.set_sl_override(Some(moved));
            state.sl_override != before
        }
        Control::Window => {
            let before = state.weapons_for.window_s;
            let (small, big) = WINDOW_STEP_TENTHS;
            let tenths = (before * 10.0).round() as i32 + nudge.signed(small, big);
            let max_tenths = (WINDOW_MAX_S * 10.0) as i32;
            state.weapons_for.window_s = tenths.clamp(0, max_tenths) as f32 / 10.0;
            (state.weapons_for.window_s - before).abs() > f32::EPSILON
        }
        Control::SimilarK => {
            let before = state.similar_k;
            let (small, big) = SIMILAR_K_STEP;
            state.similar_k = nudge_u16(
                before,
                nudge.signed(small, big),
                SIMILAR_K_MIN,
                SIMILAR_K_MAX,
            );
            state.similar_k != before
        }
        _ => false,
    }
}

/// Move a list's highlighted row by `delta`, kept inside `len` rows.
pub fn move_in_list(cursor: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    cursor.saturating_add_signed(delta).min(len - 1)
}

/// The first row a list of `len` rows, `visible` at a time, shows so `cursor` is in view, given it
/// showed from `scroll` before.
pub fn scroll_to(cursor: usize, scroll: usize, visible: usize, len: usize) -> usize {
    let visible = visible.max(1);
    let scroll = if cursor < scroll {
        cursor
    } else if cursor >= scroll + visible {
        cursor + 1 - visible
    } else {
        scroll
    };
    scroll.min(len.saturating_sub(visible))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(mode: Mode) -> Shape {
        Shape {
            mode,
            results: false,
            generated: false,
            fixes: 0,
            paperdoll: false,
            wrapped: Wrapped::default(),
        }
    }

    /// A generated build on screen.
    fn doll(mode: Mode) -> Shape {
        Shape {
            generated: true,
            paperdoll: true,
            ..shape(mode)
        }
    }

    /// Walk from `from` pressing each direction in turn.
    fn walk(rows: &[Vec<Control>], from: Control, dirs: &[Dir]) -> Control {
        dirs.iter().fold(from, |at, &dir| step(rows, at, dir))
    }

    #[test]
    fn every_control_is_reachable_from_the_first_stat_in_every_mode() {
        let shapes = Mode::ALL.into_iter().flat_map(|mode| {
            [
                Shape {
                    mode,
                    results: true,
                    generated: true,
                    fixes: 2,
                    paperdoll: false,
                    wrapped: Wrapped::default(),
                },
                doll(mode),
            ]
        });
        for shape in shapes {
            let (mode, rows) = (shape.mode, layout(shape));
            // A breadth-first walk over the four directions.
            let mut seen = vec![Control::Stat(0)];
            let mut frontier = vec![Control::Stat(0)];
            while let Some(at) = frontier.pop() {
                for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
                    let next = step(&rows, at, dir);
                    if !seen.contains(&next) {
                        seen.push(next);
                        frontier.push(next);
                    }
                }
            }
            for control in rows.iter().flatten() {
                assert!(seen.contains(control), "{mode:?}: {control:?} unreachable");
            }
        }
    }

    #[test]
    fn the_grip_sits_between_the_infusion_and_the_goal() {
        let rows = layout(shape(Mode::OptimizeForWeapon));
        assert_eq!(step(&rows, Control::Infusion, Dir::Right), Control::Grip);
        assert_eq!(step(&rows, Control::Grip, Dir::Right), Control::Objective);
        // The defender is the goal's: it says who the damage goal is scored against.
        assert_eq!(
            step(&rows, Control::Objective, Dir::Right),
            Control::Defender
        );
    }

    /// The player asked for it on 2026-09-29: a direction off a row's end comes back at its other
    /// end rather than stopping.
    #[test]
    fn left_and_right_walk_the_level_and_stats_and_wrap_at_the_ends() {
        let rows = layout(shape(Mode::WeaponsForStats));
        assert_eq!(
            step(&rows, Control::SlOverride, Dir::Left),
            Control::Stat(8)
        );
        assert_eq!(
            step(&rows, Control::SlOverride, Dir::Right),
            Control::Stat(0)
        );
        assert_eq!(step(&rows, Control::Stat(0), Dir::Right), Control::Stat(1));
        assert_eq!(
            step(&rows, Control::Stat(8), Dir::Right),
            Control::SlOverride
        );
        assert_eq!(
            step(&rows, Control::Close, Dir::Right),
            Control::UseCharacter
        );
        // A row of one stays where it is.
        let rows = layout(Shape {
            results: true,
            ..shape(Mode::WeaponsForStats)
        });
        assert_eq!(step(&rows, Control::Results, Dir::Left), Control::Results);
    }

    /// Measured on 22cafa3: Down from the weapon went to the mode tabs, which were drawn under the
    /// weapon's parameters although those parameters belong to the tab. The tabs now come first.
    #[test]
    fn up_and_down_keep_the_place_along_the_row() {
        let rows = layout(shape(Mode::WeaponsForStats));
        // The header's two buttons sit over the two ends of the stats row.
        assert_eq!(step(&rows, Control::Stat(8), Dir::Up), Control::Close);
        assert_eq!(
            step(&rows, Control::SlOverride, Dir::Up),
            Control::UseCharacter
        );
        assert_eq!(step(&rows, Control::Close, Dir::Down), Control::Stat(8));
        assert_eq!(step(&rows, Control::Close, Dir::Up), Control::Close);
        assert_eq!(
            step(&rows, Control::SlOverride, Dir::Down),
            Control::Mode(Mode::WeaponsForStats)
        );
        assert_eq!(
            step(&rows, Control::Mode(Mode::WeaponsForStats), Dir::Down),
            Control::Weapon
        );
        assert_eq!(
            step(&rows, Control::Mode(Mode::SimilarBuilds), Dir::Down),
            Control::Defender
        );
    }

    #[test]
    fn the_parameters_lead_to_that_modes_options() {
        let rows = layout(shape(Mode::SimilarBuilds));
        assert_eq!(rows[OPTIONS_ROW][0], Control::SimilarK);
        let options = walk(&rows, Control::Weapon, &[Dir::Down]);
        assert_eq!(options, Control::SimilarK);
        assert_eq!(
            walk(&rows, options, &[Dir::Right, Dir::Right]),
            Control::Poison
        );
        assert_eq!(
            walk(&rows, options, &[Dir::Right, Dir::Right, Dir::Right]),
            Control::SimilarK,
            "the row wraps: Run is a press, not a place"
        );
        // Measured on c7afba5: in Optimize for weapon, Down went straight to Run, and the Best
        // infusion button beside it answered to the mouse only.
        let rows = layout(shape(Mode::OptimizeForWeapon));
        assert_eq!(
            step(&rows, Control::Weapon, Dir::Down),
            Control::BestInfusion
        );
        assert_eq!(
            step(&rows, Control::BestInfusion, Dir::Right),
            Control::BestInfusion
        );
    }

    #[test]
    fn a_control_the_new_layout_lacks_falls_back_to_the_options_or_generate() {
        let rows = layout(shape(Mode::OptimizeForWeapon));
        assert_eq!(resolve(&rows, Control::Window), Control::BestInfusion);
        assert_eq!(resolve(&rows, Control::Results), Control::BestInfusion);
        assert_eq!(resolve(&rows, Control::ShowToggle), Control::Generate);
        // And a step from a vanished control starts from its fallback: the right-hand end of
        // Weapons for stats' options row, so Down lands at the right-hand end of the footer.
        let rows = layout(shape(Mode::WeaponsForStats));
        assert_eq!(resolve(&rows, Control::SimilarK), Control::RawAr);
        assert_eq!(step(&rows, Control::SimilarK, Dir::Down), Control::Apply);
    }

    #[test]
    fn the_footer_is_the_bottom_row_with_results_above_it_when_there_are_any() {
        let rows = layout(Shape {
            mode: Mode::WeaponsForStats,
            results: true,
            generated: true,
            fixes: 0,
            paperdoll: false,
            wrapped: Wrapped::default(),
        });
        assert_eq!(step(&rows, Control::RawAr, Dir::Down), Control::Results);
        assert_eq!(step(&rows, Control::Results, Dir::Down), Control::Generate);
        assert_eq!(
            walk(
                &rows,
                Control::Generate,
                &[
                    Dir::Right,
                    Dir::Right,
                    Dir::Right,
                    Dir::Right,
                    Dir::Right,
                    Dir::Right
                ]
            ),
            Control::Apply
        );
        assert_eq!(
            step(&rows, Control::IgnoreFloors, Dir::Right),
            Control::Spend,
            "the spend choice sits beside the floors"
        );
        assert_eq!(
            walk(&rows, Control::Generate, &[Dir::Right, Dir::Right]),
            Control::Spells,
            "the spell list sits beside the armour check"
        );
        assert_eq!(step(&rows, Control::Apply, Dir::Down), Control::Apply);
    }

    /// A refusal's fix buttons are a row of their own just above the footer, so Down from the
    /// options reaches them and Down again reaches Generate Build.
    #[test]
    fn the_fix_buttons_sit_above_the_footer() {
        let rows = layout(Shape {
            fixes: 3,
            ..shape(Mode::OptimizeForWeapon)
        });
        assert_eq!(
            rows[rows.len() - 2],
            (0..3).map(Control::Fix).collect::<Vec<_>>()
        );
        assert_eq!(
            step(&rows, Control::BestInfusion, Dir::Down),
            Control::Fix(0)
        );
        assert_eq!(step(&rows, Control::Fix(0), Dir::Down), Control::Generate);
        assert_eq!(
            resolve(&layout(shape(Mode::OptimizeForWeapon)), Control::Fix(2)),
            Control::Generate
        );
    }

    #[test]
    fn stats_nudge_by_one_and_ten_inside_their_bounds() {
        let mut state = PanelState::default();
        assert!(nudge(&mut state, Control::Stat(0), Nudge::Up));
        assert_eq!(state.stats[0], 7);
        assert!(nudge(&mut state, Control::Stat(0), Nudge::UpBig));
        assert_eq!(state.stats[0], 17);
        state.stats[0] = STAT_MAX;
        assert!(!nudge(&mut state, Control::Stat(0), Nudge::Up));
        assert_eq!(state.stats[0], STAT_MAX);
        state.stats[0] = 5;
        assert!(nudge(&mut state, Control::Stat(0), Nudge::DownBig));
        assert_eq!(state.stats[0], STAT_MIN);
        assert!(!nudge(&mut state, Control::Stat(0), Nudge::Down));
    }

    #[test]
    fn the_override_starts_from_the_computed_level_and_clears_at_zero() {
        let mut state = PanelState::default();
        let computed = state.computed_sl();
        assert!(nudge(&mut state, Control::SlOverride, Nudge::UpBig));
        assert_eq!(state.sl_override, Some(computed + 10));
        state.sl_override = Some(1);
        assert!(nudge(&mut state, Control::SlOverride, Nudge::Down));
        assert_eq!(state.sl_override, None);
        state.sl_override = Some(SL_MAX);
        assert!(!nudge(&mut state, Control::SlOverride, Nudge::Up));
    }

    #[test]
    fn the_window_moves_in_tenths_and_the_neighbours_stay_in_bounds() {
        let mut state = PanelState::default();
        assert!(nudge(&mut state, Control::Window, Nudge::Up));
        assert!((state.weapons_for.window_s - 0.1).abs() < 1e-6);
        assert!(nudge(&mut state, Control::Window, Nudge::UpBig));
        assert!((state.weapons_for.window_s - 1.1).abs() < 1e-6);
        state.weapons_for.window_s = WINDOW_MAX_S;
        assert!(!nudge(&mut state, Control::Window, Nudge::Up));
        state.similar_k = SIMILAR_K_MIN;
        assert!(!nudge(&mut state, Control::SimilarK, Nudge::Down));
        assert!(nudge(&mut state, Control::SimilarK, Nudge::UpBig));
        assert_eq!(state.similar_k, SIMILAR_K_MIN + 10);
        assert!(!nudge(&mut state, Control::Generate, Nudge::Up));
    }

    #[test]
    fn a_list_cursor_stays_in_the_list_and_in_view() {
        assert_eq!(move_in_list(0, -1, 10), 0);
        assert_eq!(move_in_list(8, 5, 10), 9);
        assert_eq!(move_in_list(3, 1, 0), 0);
        // Twelve visible of forty: moving to 12 scrolls one row.
        assert_eq!(scroll_to(12, 0, 12, 40), 1);
        assert_eq!(scroll_to(0, 5, 12, 40), 0);
        assert_eq!(scroll_to(39, 0, 12, 40), 28);
        assert_eq!(scroll_to(5, 3, 12, 40), 3);
    }

    /// The design: with the build on screen, its summary and slots sit under the mode tabs where
    /// the weapon's parameters and the mode's options were, and the footer stays the bottom row.
    #[test]
    fn the_paperdoll_takes_the_place_of_the_parameters_and_options() {
        let rows = layout(doll(Mode::OptimizeForWeapon));
        assert!(locate(&rows, Control::Weapon).is_none());
        assert!(locate(&rows, Control::BestInfusion).is_none());
        assert_eq!(
            step(&rows, Control::Mode(Mode::WeaponsForStats), Dir::Down),
            Control::Slot(Slot::PRIMARY)
        );
        assert_eq!(
            step(&rows, Control::Slot(Slot::PRIMARY), Dir::Up),
            Control::Mode(Mode::WeaponsForStats)
        );
        assert_eq!(
            step(&rows, Control::Slot(Slot::Item(5)), Dir::Down),
            Control::Generate
        );
        assert_eq!(rows[rows.len() - 1][0], Control::Generate);
        // A row of the paperdoll wraps like any other.
        assert_eq!(
            step(&rows, Control::Slot(Slot::Ring(1)), Dir::Right),
            Control::Slot(Slot::RightHand(0))
        );
    }

    /// Up and Down between two rows of slots land on the slot drawn nearest, not the one the same
    /// fraction of the way along: the rows are different widths.
    #[test]
    fn up_and_down_in_the_paperdoll_land_on_the_slot_drawn_nearest() {
        let rows = layout(doll(Mode::OptimizeForWeapon));
        let go = |from: Slot, dir: Dir| match step(&rows, Control::Slot(from), dir) {
            Control::Slot(to) => to,
            other => panic!("{from:?} {dir:?} left the paperdoll for {other:?}"),
        };
        assert_eq!(go(Slot::RightHand(2), Dir::Down), Slot::LeftHand(2));
        assert_eq!(go(Slot::Ring(1), Dir::Down), Slot::Ring(3));
        assert_eq!(go(Slot::Ring(2), Dir::Down), Slot::Legs);
        assert_eq!(go(Slot::Legs, Dir::Down), Slot::Item(4));
        assert_eq!(go(Slot::Head, Dir::Down), Slot::Item(0));
        assert_eq!(go(Slot::Arrows(0), Dir::Up), Slot::Legs);
        assert_eq!(go(Slot::Arrows(1), Dir::Down), Slot::Bolts(1));
        assert_eq!(go(Slot::Item(3), Dir::Down), Slot::Item(8));
        assert_eq!(go(Slot::Chest, Dir::Up), Slot::LeftHand(1));
    }

    /// A control the paperdoll covers hands the cursor to the build's weapon, and one the build
    /// takes away when it goes hands it back to the options row as before.
    #[test]
    fn a_control_the_paperdoll_covers_falls_back_to_the_builds_weapon() {
        let rows = layout(doll(Mode::WeaponsForStats));
        assert_eq!(
            resolve(&rows, Control::Weapon),
            Control::Slot(Slot::PRIMARY)
        );
        assert_eq!(
            resolve(&rows, Control::Results),
            Control::Slot(Slot::PRIMARY)
        );
        assert_eq!(resolve(&rows, Control::Fix(1)), Control::Generate);
        let rows = layout(shape(Mode::WeaponsForStats));
        assert_eq!(resolve(&rows, Control::Slot(Slot::Legs)), Control::RawAr);
    }

    /// Between two footer buttons, and between two groups on one line: the panel's.
    const GAP: f32 = 12.0;
    const GROUP_GAP: f32 = 32.0;

    /// A footer button's box for `control`, about what the panel draws at 4K: wide enough that the
    /// whole footer does not fit the 1500-pixel panel on one line.
    fn box_width(control: Control) -> f32 {
        match control {
            Control::Generate => 200.0,
            Control::AllowNaked => 260.0,
            Control::Spells => 170.0,
            Control::IgnoreFloors => 380.0,
            Control::Spend => 230.0,
            Control::ShowToggle => 180.0,
            _ => 300.0,
        }
    }

    /// The footer's groups and their boxes laid in `room`, the way the panel lays them.
    fn lay(generated: bool, room: f32) -> (Vec<Control>, Vec<Placed>) {
        let groups = footer(generated);
        let widths: Vec<Vec<f32>> = groups
            .iter()
            .map(|group| group.iter().copied().map(box_width).collect())
            .collect();
        let widths: Vec<&[f32]> = widths.iter().map(Vec::as_slice).collect();
        (groups.concat(), wrap(&widths, room, GAP, GROUP_GAP))
    }

    /// The grid for a generated build's footer as it is drawn in `room`.
    fn wrapped(room: f32) -> Shape {
        let (controls, placed) = lay(true, room);
        Shape {
            generated: true,
            wrapped: Wrapped::new(&controls, &placed),
            ..shape(Mode::OptimizeForWeapon)
        }
    }

    /// The bug: at 3840x2160 the footer's one row ran past the panel's right edge. However narrow
    /// the line and however long a label, no box ends past it, starts before it, or overlaps the
    /// box before it on its line.
    #[test]
    fn no_footer_box_ends_past_the_line_however_narrow() {
        let long = [40.0, 2400.0, 90.0];
        for room in (0..=2000).step_by(7).map(|room| room as f32) {
            for generated in [false, true] {
                let (_, placed) = lay(generated, room);
                let groups: [&[f32]; 3] = [&long, &[room, room + 1.0], &[0.0, 15.0]];
                let more = wrap(&groups, room, GAP, GROUP_GAP);
                for set in [&placed, &more] {
                    for (index, place) in set.iter().enumerate() {
                        assert!(
                            place.x >= 0.0,
                            "room {room}: {place:?} starts before the line"
                        );
                        assert!(
                            place.x + place.width <= room,
                            "room {room}: {place:?} ends past the line"
                        );
                        if let Some(before) = index.checked_sub(1).map(|i| set[i])
                            && before.line == place.line
                        {
                            assert!(before.x + before.width <= place.x, "room {room}: overlap");
                        }
                        if index > 0 {
                            assert!(set[index - 1].line <= place.line, "lines run in order");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_group_moves_to_the_next_line_whole_and_splits_only_when_wider_than_one() {
        // Everything fits: one line, the groups further apart than their buttons.
        let (_, placed) = lay(false, 3000.0);
        assert!(placed.iter().all(|place| place.line == 0));
        assert_eq!(placed[1].x, 200.0 + GAP);
        assert_eq!(placed[3].x, 654.0 + GROUP_GAP);
        // 1000 holds any one group but not two: a line each, each starting at the left.
        let (_, placed) = lay(true, 1000.0);
        let lines: Vec<usize> = placed.iter().map(|place| place.line).collect();
        assert_eq!(lines, [0, 0, 0, 1, 1, 2, 2]);
        assert_eq!((placed[3].x, placed[5].x), (0.0, 0.0));
        // 500 is narrower than the first two groups: they break between their buttons, and the
        // third, which fits a line, still gets one of its own.
        let (_, placed) = lay(true, 500.0);
        let lines: Vec<usize> = placed.iter().map(|place| place.line).collect();
        assert_eq!(lines, [0, 0, 1, 2, 3, 4, 4]);
        // A label wider than the line is cut to it.
        let cut = wrap(&[&[50.0, 900.0]], 400.0, GAP, GROUP_GAP);
        assert_eq!(
            cut[1],
            Placed {
                line: 1,
                x: 0.0,
                width: 400.0
            }
        );
    }

    /// The grid's footer rows are the lines the buttons were drawn on, in the order drawn, so the
    /// cursor walks what is on screen.
    #[test]
    fn the_footer_rows_are_the_lines_it_was_drawn_on() {
        for room in [3000.0, 1500.0, 1000.0, 500.0, 120.0] {
            let (controls, placed) = lay(true, room);
            let rows = layout(wrapped(room));
            let lines = placed.last().map_or(0, |place| place.line + 1);
            let drawn: Vec<Vec<Control>> = (0..lines)
                .map(|line| {
                    controls
                        .iter()
                        .zip(&placed)
                        .filter(|(_, place)| place.line == line)
                        .map(|(&control, _)| control)
                        .collect()
                })
                .collect();
            assert_eq!(rows[rows.len() - lines..], drawn[..], "room {room}");
            // Left and Right stay on the line: off its end comes back at its start.
            for row in &drawn {
                let end = row[row.len() - 1];
                assert_eq!(step(&rows, end, Dir::Right), row[0], "room {room}");
            }
        }
    }

    /// At 1500 the first two groups share a line and Show and Apply have the second: Up and Down
    /// land on the button drawn over or under, not the one the same fraction along.
    #[test]
    fn up_and_down_between_footer_lines_land_on_the_button_drawn_nearest() {
        let shape = wrapped(1500.0);
        let rows = layout(shape);
        let go = |from: Control, dir: Dir| step_in(&rows, &shape.wrapped, from, dir);
        assert_eq!(go(Control::Generate, Dir::Down), Control::ShowToggle);
        assert_eq!(go(Control::AllowNaked, Dir::Down), Control::Apply);
        assert_eq!(go(Control::Spells, Dir::Down), Control::Apply);
        assert_eq!(go(Control::Spend, Dir::Down), Control::Apply);
        assert_eq!(go(Control::Apply, Dir::Up), Control::AllowNaked);
        assert_eq!(go(Control::ShowToggle, Dir::Up), Control::Generate);
        assert_eq!(go(Control::Apply, Dir::Down), Control::Apply);
        // Leaving the footer upward is the ordinary step, into the options row.
        assert_eq!(go(Control::Generate, Dir::Up), Control::BestInfusion);
        assert_eq!(go(Control::BestInfusion, Dir::Down), Control::Generate);
    }

    #[test]
    fn every_footer_button_is_reachable_however_it_wrapped() {
        for room in [1500.0, 1000.0, 500.0, 0.0] {
            let shape = wrapped(room);
            let rows = layout(shape);
            let mut seen = vec![Control::Generate];
            let mut frontier = vec![Control::Generate];
            while let Some(at) = frontier.pop() {
                for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
                    let next = step_in(&rows, &shape.wrapped, at, dir);
                    if !seen.contains(&next) {
                        seen.push(next);
                        frontier.push(next);
                    }
                }
            }
            for control in rows.iter().flatten() {
                assert!(
                    seen.contains(control),
                    "room {room}: {control:?} unreachable"
                );
            }
        }
        assert!(footer(true).concat().len() <= WRAPPED_MAX);
    }

    /// Between two of a refusal's fix buttons, across and down: the panel's.
    const FIX_GAP: f32 = 8.0;

    /// Five fixes' boxes, about the width of `Raise SL to 139` and its kind at 4K.
    const FIX_WIDTHS: [f32; 5] = [260.0, 300.0, 220.0, 340.0, 280.0];

    /// The fixes as the panel lays them in `room`.
    fn lay_fixes(room: f32) -> (Vec<Control>, Vec<Placed>) {
        let controls = (0..FIX_WIDTHS.len()).map(Control::Fix).collect();
        (controls, wrap(&[&FIX_WIDTHS], room, FIX_GAP, 0.0))
    }

    /// A refusal's five fixes on screen, drawn in `room`.
    fn refused(room: f32) -> Shape {
        let (controls, placed) = lay_fixes(room);
        Shape {
            fixes: FIX_WIDTHS.len(),
            wrapped: Wrapped::new(&controls, &placed),
            ..shape(Mode::OptimizeForWeapon)
        }
    }

    /// The fix buttons wrapped on screen while the grid kept them on one row, so Up and Down skipped
    /// a line of them. Now their rows are the lines drawn, just above the footer.
    #[test]
    fn the_fix_rows_are_the_lines_they_were_drawn_on() {
        for room in [3000.0, 900.0, 600.0, 400.0, 100.0] {
            let (controls, placed) = lay_fixes(room);
            for place in &placed {
                assert!(
                    place.x >= 0.0 && place.x + place.width <= room,
                    "room {room}"
                );
            }
            let rows = layout(refused(room));
            let lines = placed.last().map_or(0, |place| place.line + 1);
            let drawn: Vec<Vec<Control>> = (0..lines)
                .map(|line| {
                    controls
                        .iter()
                        .zip(&placed)
                        .filter(|(_, place)| place.line == line)
                        .map(|(&control, _)| control)
                        .collect()
                })
                .collect();
            let footer_row = rows.len() - 1;
            assert_eq!(
                rows[footer_row - lines..footer_row],
                drawn[..],
                "room {room}"
            );
            assert_eq!(rows[footer_row - lines - 1], [Control::BestInfusion]);
        }
    }

    /// At 900 the first three fixes share a line and the last two the next: Up from the fifth lands
    /// on the second, drawn over it, not on the third, which is the same fraction along.
    #[test]
    fn up_and_down_between_fix_lines_land_on_the_button_drawn_nearest() {
        let shape = refused(900.0);
        let rows = layout(shape);
        let go = |from: Control, dir: Dir| step_in(&rows, &shape.wrapped, from, dir);
        assert_eq!(go(Control::Fix(4), Dir::Up), Control::Fix(1));
        assert_eq!(go(Control::Fix(3), Dir::Up), Control::Fix(0));
        assert_eq!(go(Control::Fix(0), Dir::Down), Control::Fix(3));
        assert_eq!(go(Control::Fix(2), Dir::Down), Control::Fix(4));
        assert_eq!(go(Control::Fix(0), Dir::Up), Control::BestInfusion);
        assert_eq!(go(Control::Fix(3), Dir::Down), Control::Generate);
        assert_eq!(go(Control::Fix(4), Dir::Right), Control::Fix(3));
    }

    /// Fixes and footer both wrapped, the fixes indented as the panel draws them, at any width and
    /// with a fix label longer than a line: every button fits its line, and the cursor reaches each
    /// from the first fix.
    #[test]
    fn every_fix_and_footer_button_is_reachable_however_both_wrapped() {
        let widths = [260.0, 2600.0, 220.0, 340.0, 280.0, 90.0];
        let fixes: Vec<Control> = (0..widths.len()).map(Control::Fix).collect();
        for room in (0..=1600).step_by(37).map(|room| room as f32) {
            let fix_room = room - 12.0;
            let fix_places = wrap(&[&widths], fix_room, FIX_GAP, 0.0);
            for place in &fix_places {
                assert!(place.x + place.width <= fix_room.max(0.0), "room {room}");
            }
            let (footer_controls, footer_places) = lay(true, room);
            let shape = Shape {
                fixes: widths.len(),
                generated: true,
                wrapped: Wrapped::new(&footer_controls, &footer_places).with(
                    &fixes,
                    &fix_places,
                    12.0,
                ),
                ..shape(Mode::SimilarBuilds)
            };
            let rows = layout(shape);
            let mut seen = vec![Control::Fix(0)];
            let mut frontier = vec![Control::Fix(0)];
            while let Some(at) = frontier.pop() {
                for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
                    let next = step_in(&rows, &shape.wrapped, at, dir);
                    if !seen.contains(&next) {
                        seen.push(next);
                        frontier.push(next);
                    }
                }
            }
            for control in rows.iter().flatten() {
                assert!(
                    seen.contains(control),
                    "room {room}: {control:?} unreachable"
                );
            }
        }
    }
}
