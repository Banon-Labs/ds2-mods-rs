//! The panel without a mouse: which control the D-pad cursor is on, where each direction takes it,
//! and how a numeric field moves under Left and Right.
//!
//! The panel is laid out in rows, and the cursor walks them the way it is drawn: Up and Down change
//! row, Left and Right move along one. [`layout`] is that grid for the current mode and answer, so
//! a mode whose options are different gets a different row and the cursor never lands on a control
//! that is not on screen. The keyboard is only needed to type a weapon name; every other control is
//! reached here and changed with a press.

use crate::model::{Grip, Mode, Objective, PanelState, SL_MAX, STAT_COUNT, STAT_MAX, STAT_MIN};

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
    /// One grip button: what Optimize for weapon and Generate Build build for.
    Grip(Grip),
    /// One objective button.
    Objective(Objective),
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
    /// Run.
    Run,
    /// The results table, to scroll it.
    Results,
    /// Generate Build.
    Generate,
    /// Generate Build may leave the armour off.
    AllowNaked,
    /// The spells Generate Build must cast: the list to choose them from.
    Spells,
    /// Show build / Show results.
    ShowToggle,
    /// Apply to character.
    Apply,
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
}

/// The controls, row by row, as the panel draws them.
pub fn layout(shape: Shape) -> Vec<Vec<Control>> {
    let mut rows = vec![
        vec![Control::Close],
        (0..STAT_COUNT).map(Control::Stat).collect(),
        vec![Control::SlOverride, Control::UseCharacter],
        [Control::Weapon, Control::Infusion]
            .into_iter()
            .chain(Grip::ALL.map(Control::Grip))
            .chain(Objective::ALL.map(Control::Objective))
            .collect(),
        Mode::ALL.map(Control::Mode).to_vec(),
    ];
    let mut options = match shape.mode {
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
    options.push(Control::Run);
    rows.push(options);
    if shape.results {
        rows.push(vec![Control::Results]);
    }
    let mut footer = vec![Control::Generate, Control::AllowNaked, Control::Spells];
    if shape.generated {
        footer.push(Control::ShowToggle);
    }
    footer.push(Control::Apply);
    rows.push(footer);
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

/// `cursor` if it is still on screen, or the control nearest to where it was: the options row's
/// Run for a mode option or the results table that went away, Generate Build for a footer button.
pub fn resolve(rows: &[Vec<Control>], cursor: Control) -> Control {
    if locate(rows, cursor).is_some() {
        return cursor;
    }
    match cursor {
        Control::ShowToggle | Control::Apply => Control::Generate,
        _ => Control::Run,
    }
}

/// Where one press of `dir` takes the cursor.
///
/// Left and Right stop at a row's ends; Up and Down stop at the top and bottom rows, and land at
/// the same fraction of the way along the new row, so going down from the ninth stat lands at the
/// right-hand end of the row below rather than its start.
pub fn step(rows: &[Vec<Control>], cursor: Control, dir: Dir) -> Control {
    let cursor = resolve(rows, cursor);
    let Some((row, column)) = locate(rows, cursor) else {
        return cursor;
    };
    let target_row = match dir {
        Dir::Left => return rows[row][column.saturating_sub(1)],
        Dir::Right => return rows[row][(column + 1).min(rows[row].len() - 1)],
        Dir::Up if row == 0 => return cursor,
        Dir::Up => row - 1,
        Dir::Down if row + 1 == rows.len() => return cursor,
        Dir::Down => row + 1,
    };
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
        }
    }

    /// Walk from `from` pressing each direction in turn.
    fn walk(rows: &[Vec<Control>], from: Control, dirs: &[Dir]) -> Control {
        dirs.iter().fold(from, |at, &dir| step(rows, at, dir))
    }

    #[test]
    fn every_control_is_reachable_from_the_first_stat_in_every_mode() {
        for mode in Mode::ALL {
            let rows = layout(Shape {
                mode,
                results: true,
                generated: true,
            });
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
    fn the_grip_sits_between_the_infusion_and_the_objective() {
        let rows = layout(shape(Mode::OptimizeForWeapon));
        assert_eq!(
            walk(&rows, Control::Infusion, &[Dir::Right, Dir::Right]),
            Control::Grip(Grip::TwoHanded)
        );
        assert_eq!(
            step(&rows, Control::Grip(Grip::TwoHanded), Dir::Right),
            Control::Objective(Objective::Damage)
        );
    }

    #[test]
    fn left_and_right_walk_the_stats_and_stop_at_the_ends() {
        let rows = layout(shape(Mode::WeaponsForStats));
        assert_eq!(step(&rows, Control::Stat(0), Dir::Left), Control::Stat(0));
        assert_eq!(step(&rows, Control::Stat(0), Dir::Right), Control::Stat(1));
        assert_eq!(step(&rows, Control::Stat(8), Dir::Right), Control::Stat(8));
    }

    #[test]
    fn up_and_down_keep_the_place_along_the_row() {
        let rows = layout(shape(Mode::WeaponsForStats));
        // Ninth of nine stats -> the right-hand end of [override, use character].
        assert_eq!(
            step(&rows, Control::Stat(8), Dir::Down),
            Control::UseCharacter
        );
        assert_eq!(
            step(&rows, Control::Stat(0), Dir::Down),
            Control::SlOverride
        );
        // Close sits alone above the stats and lands on the first.
        assert_eq!(step(&rows, Control::Close, Dir::Down), Control::Stat(0));
        assert_eq!(step(&rows, Control::Stat(4), Dir::Up), Control::Close);
        assert_eq!(step(&rows, Control::Close, Dir::Up), Control::Close);
    }

    #[test]
    fn the_mode_tabs_lead_to_that_modes_options_and_run() {
        let rows = layout(shape(Mode::SimilarBuilds));
        let options = walk(&rows, Control::Mode(Mode::WeaponsForStats), &[Dir::Down]);
        assert_eq!(options, Control::SimilarK);
        assert_eq!(
            walk(&rows, options, &[Dir::Right, Dir::Right, Dir::Right]),
            Control::Run
        );
        // Measured on c7afba5: in Optimize for weapon, Down from the mode tab went straight to
        // Run, and the Best infusion button beside it answered to the mouse only.
        let rows = layout(shape(Mode::OptimizeForWeapon));
        assert_eq!(
            step(&rows, Control::Mode(Mode::OptimizeForWeapon), Dir::Down),
            Control::BestInfusion
        );
        assert_eq!(step(&rows, Control::BestInfusion, Dir::Right), Control::Run);
    }

    #[test]
    fn a_control_the_new_layout_lacks_falls_back_to_run_or_generate() {
        let rows = layout(shape(Mode::OptimizeForWeapon));
        assert_eq!(resolve(&rows, Control::Window), Control::Run);
        assert_eq!(resolve(&rows, Control::Results), Control::Run);
        assert_eq!(resolve(&rows, Control::ShowToggle), Control::Generate);
        // And a step from a vanished control starts from its fallback: Run, the right-hand end of
        // Optimize for weapon's options row, so Down lands at the right-hand end of the footer.
        assert_eq!(step(&rows, Control::Window, Dir::Down), Control::Apply);
    }

    #[test]
    fn the_footer_is_the_bottom_row_with_results_above_it_when_there_are_any() {
        let rows = layout(Shape {
            mode: Mode::WeaponsForStats,
            results: true,
            generated: true,
        });
        assert_eq!(step(&rows, Control::Run, Dir::Down), Control::Results);
        assert_eq!(step(&rows, Control::Results, Dir::Down), Control::Generate);
        assert_eq!(
            walk(
                &rows,
                Control::Generate,
                &[Dir::Right, Dir::Right, Dir::Right, Dir::Right]
            ),
            Control::Apply
        );
        assert_eq!(
            walk(&rows, Control::Generate, &[Dir::Right, Dir::Right]),
            Control::Spells,
            "the spell list sits beside the armour check"
        );
        assert_eq!(step(&rows, Control::Apply, Dir::Down), Control::Apply);
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
        assert!(!nudge(&mut state, Control::Run, Nudge::Up));
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
}
