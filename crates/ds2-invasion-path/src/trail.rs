//! Where the Prism Stones go, and which ones you have walked past.
//!
//! # What this module is and is not
//!
//! Pure bookkeeping over world points, with no engine call in it and no `cfg(windows)` on it, so
//! the part that decides *where a stone lands* is proved by `cargo test` rather than by walking
//! around a map counting them. `crate::sfx` owns the engine handles; `crate::gametick` drives
//! both. The handle type is a parameter here for exactly that reason -- the tests use a `u32`.
//!
//! # The three rules
//!
//! 1. **Spaced along the path, never at its corners.** `crate::geometry::resample` is the whole
//!    of that: the engine emits route vertices where the mesh turns, so a doorway would collect
//!    six stones inside two metres and a courtyard would get none.
//! 2. **A few per pass.** `markers_per_pass` stones are placed per tick, so a trail unrolls from
//!    your feet at a visible speed instead of appearing whole. It also bounds how many engine
//!    spawns land in one frame, which matters because that frame is the one the game simulates
//!    from.
//! 3. **Never twice in the same place.** A route is re-planned as you move, so nearly the same
//!    sample points come back every few seconds. A candidate within half a spacing of a stone
//!    that is already down is skipped, which makes re-planning idempotent rather than a way to
//!    stack forty effects on one flagstone.
//!
//! # "Behind you" is a direction, not a distance
//!
//! `marker_keep_behind_meters` retires the stones you have walked past. A stone is behind when
//! the route's own forward direction says so -- `dot(stone - you, forward) < 0` -- and only then
//! does the distance threshold apply.
//!
//! Testing distance alone is the bug this is written to avoid, and it is not a subtle one: the
//! stones AHEAD of you are the far ones, so a distance-only rule retires the trail you are
//! walking towards the moment it passes twelve metres, and the next pass lays it again. The
//! result is a trail that never gets further than twelve metres from you and spawns effects
//! forever. There is a test for it below.
//!
//! # These stones can actually be put out
//!
//! bd `ds2-mods-rs-3al` blocker (c) recorded that no stop call was known, which would have made
//! every stone permanent for the session and made "retire" mean "forget". It is known now --
//! [`ds2_rva::KATANA_SFX_STOP`] -- so retiring a stone really extinguishes it, and this module
//! hands the caller the handles to do it with rather than dropping them.

// Windows-only in practice; ungated so every rule above stays host-testable.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::geometry::{dot, length, normalize, resample, sub};

/// A candidate is skipped when a stone is already within this fraction of one spacing.
///
/// Half, so two stones can never end up closer together than half the spacing however often the
/// route is re-planned, and so a route that shifts by a metre does not re-lay itself.
const DUPLICATE_FRACTION: f32 = 0.5;

/// One stone believed to be on the ground.
#[derive(Debug)]
struct Stone<H> {
    at: [f32; 3],
    handle: H,
}

/// The stones this session has put down, and the rules for adding to them.
///
/// `H` is whatever the caller needs to extinguish one later -- `crate::sfx::Handle` in the game,
/// a `u32` in the tests.
#[derive(Debug)]
pub(crate) struct Trail<H> {
    placed: Vec<Stone<H>>,
}

// Derived `Default` would demand `H: Default`, which a handle owning engine memory must not have.
impl<H> Default for Trail<H> {
    fn default() -> Self {
        Self { placed: Vec::new() }
    }
}

/// Everything the trail needs from the config, in metres and counts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spacing {
    pub(crate) meters: f32,
    pub(crate) keep_behind_meters: f32,
    pub(crate) max_markers: usize,
    pub(crate) per_pass: usize,
}

impl<H> Trail<H> {
    /// Hand back the handles of every stone the player has walked past.
    ///
    /// `path` is the route in walking order, start first, so `path[0]` is where the player is and
    /// the first distinct point after it gives the forward direction. A route too short to have
    /// a direction retires nothing, which is correct: without a forward vector there is no
    /// "behind".
    ///
    /// The returned handles are no longer owned by this trail. Extinguish them.
    #[must_use]
    pub(crate) fn retire_behind(&mut self, path: &[[f32; 3]], keep_behind: f32) -> Vec<H> {
        if path.len() < 2 || !keep_behind.is_finite() || keep_behind <= 0.0 {
            return Vec::new();
        }
        let player = path[0];
        let Some(forward) = path
            .iter()
            .skip(1)
            .find_map(|point| normalize(sub(*point, player)))
        else {
            return Vec::new();
        };
        let mut retired = Vec::new();
        let mut keep = Vec::with_capacity(self.placed.len());
        for stone in self.placed.drain(..) {
            let offset = sub(stone.at, player);
            if dot(offset, forward) < 0.0 && length(offset) > keep_behind {
                retired.push(stone.handle);
            } else {
                keep.push(stone);
            }
        }
        self.placed = keep;
        retired
    }

    /// Where to put stones this pass, from your feet outwards.
    ///
    /// Returns an empty vector when the trail is already complete, which is the steady state and
    /// not a failure. Nothing is recorded: the caller spawns each point and calls
    /// [`Trail::remember`] for the ones that actually took, so a spawn the engine threw away
    /// does not leave a phantom in the bookkeeping that stops a real one landing there later.
    pub(crate) fn plan(&self, path: &[[f32; 3]], spacing: Spacing) -> Vec<[f32; 3]> {
        if path.len() < 2 || spacing.per_pass == 0 || spacing.max_markers == 0 {
            return Vec::new();
        }
        if !spacing.meters.is_finite() || spacing.meters <= 0.0 {
            return Vec::new();
        }
        let room = spacing.max_markers.saturating_sub(self.placed.len());
        if room == 0 {
            return Vec::new();
        }
        // `resample` includes the start point, which is the player's own feet. A stone there is
        // inside the character model and invisible, so the trail begins one spacing out -- hence
        // one more sample asked for and the first one dropped.
        let samples = resample(path, spacing.meters, spacing.max_markers.saturating_add(1));
        let threshold = spacing.meters * DUPLICATE_FRACTION;
        let mut out: Vec<[f32; 3]> = Vec::new();
        for candidate in samples.into_iter().skip(1) {
            if out.len() >= spacing.per_pass || out.len() >= room {
                break;
            }
            let taken = self
                .placed
                .iter()
                .map(|stone| stone.at)
                .chain(out.iter().copied())
                .any(|at| length(sub(at, candidate)) <= threshold);
            if !taken {
                out.push(candidate);
            }
        }
        out
    }

    /// Record a stone that the engine really placed.
    pub(crate) fn remember(&mut self, at: [f32; 3], handle: H) {
        self.placed.push(Stone { at, handle });
    }

    /// How many stones this trail believes are on the ground.
    pub(crate) fn placed(&self) -> usize {
        self.placed.len()
    }

    /// Every stone's handle, for asking the engine whether they are still alive.
    ///
    /// Borrowed rather than taken: the self-check samples the same stones three times, so a
    /// reader that consumed them could only ever ask once.
    pub(crate) fn handles(&self) -> impl Iterator<Item = &H> {
        self.placed.iter().map(|stone| &stone.handle)
    }

    /// Give up every stone, for a map change or an overlay switched off.
    ///
    /// The handles come back so the caller can extinguish them; dropping the returned vector
    /// without doing so is how a session ends up glittering.
    #[must_use]
    pub(crate) fn take_all(&mut self) -> Vec<H> {
        self.placed
            .drain(..)
            .map(|stone| stone.handle)
            .collect::<Vec<H>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trail whose handles are just numbers, so the rules can be tested without an engine.
    type Numbered = Trail<u32>;

    fn line(length_meters: f32, step: f32) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        let mut x = 0.0f32;
        while x <= length_meters {
            out.push([x, 0.0, 0.0]);
            x += step;
        }
        out
    }

    fn spacing() -> Spacing {
        Spacing {
            meters: 2.0,
            keep_behind_meters: 12.0,
            max_markers: 10,
            per_pass: 3,
        }
    }

    /// Lay as much trail as `passes` allows, recording every planned stone as placed.
    fn lay(trail: &mut Numbered, path: &[[f32; 3]], spacing: Spacing, passes: usize) {
        for pass in 0..passes {
            for (index, at) in trail.plan(path, spacing).into_iter().enumerate() {
                trail.remember(at, (pass * 100 + index) as u32);
            }
        }
    }

    #[test]
    fn a_pass_places_no_more_than_its_budget() {
        let trail = Numbered::default();
        assert_eq!(trail.plan(&line(100.0, 10.0), spacing()).len(), 3);
    }

    #[test]
    fn the_trail_unrolls_from_your_feet_outwards() {
        let mut trail = Numbered::default();
        let path = line(100.0, 10.0);
        let first = trail.plan(&path, spacing());
        for (index, at) in first.iter().enumerate() {
            trail.remember(*at, index as u32);
        }
        let second = trail.plan(&path, spacing());
        assert!(
            second[0][0] > first[2][0],
            "pass two went backwards: {first:?} then {second:?}"
        );
    }

    #[test]
    fn no_stone_lands_on_the_player() {
        let trail = Numbered::default();
        let placed = trail.plan(&line(100.0, 10.0), spacing());
        assert!(
            placed.iter().all(|stone| stone[0] > 1.0),
            "a stone was placed at the player's feet: {placed:?}"
        );
    }

    #[test]
    fn re_planning_the_same_route_places_nothing_twice() {
        let mut trail = Numbered::default();
        let path = line(100.0, 10.0);
        lay(&mut trail, &path, spacing(), 10);
        let before = trail.placed();
        assert!(
            trail.plan(&path, spacing()).is_empty(),
            "re-planning stacked more stones on the same ground"
        );
        assert_eq!(trail.placed(), before);
    }

    #[test]
    fn a_route_that_shifts_slightly_does_not_re_lay_itself() {
        let mut trail = Numbered::default();
        lay(&mut trail, &line(100.0, 10.0), spacing(), 10);
        // Half a metre of drift, well inside half a spacing.
        let drifted: Vec<[f32; 3]> = line(100.0, 10.0)
            .into_iter()
            .map(|point| [point[0], 0.0, 0.5])
            .collect();
        assert!(trail.plan(&drifted, spacing()).is_empty());
    }

    #[test]
    fn the_budget_is_a_ceiling_on_the_whole_trail() {
        let mut trail = Numbered::default();
        lay(&mut trail, &line(500.0, 10.0), spacing(), 100);
        assert_eq!(trail.placed(), spacing().max_markers);
    }

    #[test]
    fn a_stone_you_have_walked_past_comes_back_to_be_put_out() {
        let mut trail = Numbered::default();
        lay(&mut trail, &line(100.0, 10.0), spacing(), 10);
        let before = trail.placed();
        // Now stand 40 m further along the same line, still facing the same way.
        let moved: Vec<[f32; 3]> = (0..=6u8)
            .map(|step| [40.0 + f32::from(step) * 10.0, 0.0, 0.0])
            .collect();
        let retired = trail.retire_behind(&moved, spacing().keep_behind_meters);
        assert!(
            !retired.is_empty(),
            "nothing behind the player was retired; {before} were placed"
        );
        assert_eq!(trail.placed(), before - retired.len());
    }

    /// The bug a distance-only test produces, and the reason `retire_behind` takes the path
    /// rather than a position: the stones you are walking TOWARDS are the far ones.
    #[test]
    fn a_stone_ahead_of_you_is_never_retired_however_far_away() {
        let mut trail = Numbered::default();
        let path = line(200.0, 10.0);
        let wide = Spacing {
            meters: 20.0,
            ..spacing()
        };
        lay(&mut trail, &path, wide, 10);
        assert!(trail.placed() > 1);
        assert!(
            trail
                .retire_behind(&path, wide.keep_behind_meters)
                .is_empty(),
            "stones ahead of the player were retired as if walked past"
        );
    }

    #[test]
    fn a_stone_just_behind_you_is_left_alone() {
        let mut trail = Numbered::default();
        trail.remember([-5.0, 0.0, 0.0], 1);
        let path = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
        assert!(trail.retire_behind(&path, 12.0).is_empty());
        assert_eq!(trail.placed(), 1);
    }

    #[test]
    fn a_route_too_short_to_have_a_direction_retires_nothing() {
        let mut trail = Numbered::default();
        trail.remember([-500.0, 0.0, 0.0], 1);
        assert!(trail.retire_behind(&[[0.0, 0.0, 0.0]], 12.0).is_empty());
        // A route whose points coincide has no forward vector either.
        let still = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        assert!(trail.retire_behind(&still, 12.0).is_empty());
    }

    #[test]
    fn a_route_of_one_point_plans_nothing() {
        let trail = Numbered::default();
        assert!(trail.plan(&[[0.0, 0.0, 0.0]], spacing()).is_empty());
        assert!(trail.plan(&[], spacing()).is_empty());
    }

    #[test]
    fn a_zero_budget_plans_nothing() {
        let trail = Numbered::default();
        let none = Spacing {
            per_pass: 0,
            ..spacing()
        };
        assert!(trail.plan(&line(100.0, 10.0), none).is_empty());
    }

    #[test]
    fn taking_a_trail_hands_every_handle_back() {
        let mut trail = Numbered::default();
        lay(&mut trail, &line(100.0, 10.0), spacing(), 10);
        let placed = trail.placed();
        assert!(placed > 0);
        assert_eq!(trail.take_all().len(), placed);
        assert_eq!(trail.placed(), 0);
    }

    /// A spawn the engine throws away must not reserve the ground it failed on.
    #[test]
    fn a_stone_that_did_not_take_is_not_remembered() {
        let trail = Numbered::default();
        let path = line(100.0, 10.0);
        let planned = trail.plan(&path, spacing());
        assert_eq!(planned.len(), 3);
        // The engine dropped all three; nothing is recorded.
        assert_eq!(trail.placed(), 0);
        assert_eq!(trail.plan(&path, spacing()), planned);
    }
}
