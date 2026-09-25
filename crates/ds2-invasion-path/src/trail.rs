//! Where the Prism Stones go, and which ones you have walked past.
//!
//! # What this module is and is not
//!
//! Pure bookkeeping over world points, with no engine call in it and no `cfg(windows)` on it, so
//! the part that decides *where a stone lands* is proved by `cargo test` rather than by walking
//! around a map counting them. `crate::sfx` owns the engine handles; `crate::gametick` drives
//! both. The handle type is a parameter here for exactly that reason -- the tests use a `u32`.
//!
//! # The four rules
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
//! 4. **A route that moved takes its stones with it.** Rule 3 keeps a re-plan from stacking
//!    stones, and on its own it also keeps the stones of a route that no longer exists -- so a
//!    target who doubles back grows the player a second branch down a corridor nobody is walking,
//!    and both branches read as the way to go. [`Trail::follows`] asks whether the stones are
//!    still on the fresh route; when they are not, the whole trail goes out and is laid again
//!    from the player's feet.
//!
//! # How often a route is re-asked, and why it is not a timer
//!
//! [`Cadence`] and [`Pace`]. A fixed half-second interval spends a navmesh search on a player who
//! has not moved and leaves a sprinting one nine metres behind their own trail. The gate is drift
//! -- how far the two ends have walked from where the route was planned -- inside a floor that
//! stops a warp asking every frame and a ceiling that catches a fog gate opening while both ends
//! stand still.
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
// DEBT: ds2-mods-rs-2rs -- module-wide dead_code, reason not yet recorded.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::geometry::{arc_length_of_nearest, distance_to_path, length, resample, sub};
use crate::navpath::RoutePoint;

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
    /// Ground the engine refused to put a stone on, and how many times it has refused.
    ///
    /// # The fountain this exists to stop
    ///
    /// Measured live on 2026-09-24: **600 spawns in 40 seconds, every one of them at
    /// `[18.93, 6.80, -7.22]`**, which on screen is a cone of particles where a single marker
    /// should be, and a trail that stops dead there instead of reaching the character.
    ///
    /// The loop is short. [`Trail::plan`] offers the first candidate no stone is standing on;
    /// `crate::sfx::spawn` returns `None` when the engine hands back a control block with no
    /// effect node; the caller only calls [`Trail::remember`] for the ones that took, so nothing
    /// changes; and the next tick offers the same candidate again. Fifteen times a second, for as
    /// long as you stand there.
    ///
    /// `plan` is also budgeted -- `markers_per_pass` divided between the open lanes, which is one
    /// candidate per lane per tick in an ordinary two-target session -- so the stuck site consumes
    /// the entire budget and no stone beyond it is ever attempted. That is why the trail ends
    /// there rather than merely doubling up.
    ///
    /// The old comment on `plan` argued the opposite case, and it was right about its own risk and
    /// wrong about the cost: "a spawn the engine threw away does not leave a phantom in the
    /// bookkeeping that stops a real one landing there later". A phantom that blocks one marker is
    /// a marker missing. Retrying forever is a particle fountain and a truncated path.
    ///
    /// So a refusal is recorded and the site is skipped, and the count is kept rather than a bare
    /// flag: [`REFUSALS_BEFORE_SKIPPING`] attempts are allowed first, because a spawn can fail for
    /// a reason that passes.
    refused: Vec<Refusal>,
}

/// A patch of ground the engine has declined to put an effect on.
#[derive(Debug)]
struct Refusal {
    at: [f32; 3],
    attempts: u32,
}

/// How many times one site may be refused before the trail stops offering it.
///
/// Not one: the FX manager can be momentarily out of nodes, and a site written off on a single
/// bad frame would leave a permanent hole in a trail that was about to work. Not many either --
/// every attempt is an engine spawn on the simulation's own frame, and the whole point is to stop
/// spending them on ground that keeps saying no.
const REFUSALS_BEFORE_SKIPPING: u32 = 3;

// Derived `Default` would demand `H: Default`, which a handle owning engine memory must not have.
impl<H> Default for Trail<H> {
    fn default() -> Self {
        Self {
            placed: Vec::new(),
            refused: Vec::new(),
        }
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

impl Spacing {
    /// How far a stone may be from a fresh route and still count as standing on it.
    ///
    /// The same half-spacing [`Trail::plan`] uses to decide a candidate's ground is already
    /// taken. One number, so "this stone is on the route" and "do not lay another one here"
    /// cannot drift apart into two rules that disagree.
    pub(crate) fn on_route_tolerance(&self) -> f32 {
        self.meters * DUPLICATE_FRACTION
    }
}

/// When a route is worth asking the engine for again.
///
/// # Why this is not a timer
///
/// It was one: half a second, taken from the engine's own AI, which reloads
/// `ChrAiNavimeshCtrl + 0x2c0` with `0.5f` and refreshes its route bind when that expires. That
/// is a sound number for an NPC whose whole job is to keep walking, and it is the wrong shape for
/// this, in both directions at once:
///
/// - **Standing still, it burns a navmesh search twice a second** to redraw a line that has not
///   moved a centimetre. The search is real engine work, on the frame the game simulates from.
/// - **Sprinting, half a second is nine metres.** The route the stones are laid along is up to
///   nine metres out of date, which is most of a corridor -- and it is exactly when the player is
///   moving that they are looking at the trail.
///
/// So the clock is not what decides. Movement is: the route goes stale when either end has walked
/// away from where it was planned, and `move_meters` is how far that has to be. The two times are
/// an envelope around that rule rather than the rule itself. `min_seconds` is the rate limit that
/// stops a warp or a long fall issuing a search every frame; `max_seconds` is the backstop for
/// everything that can invalidate a route while both ends stand still -- a fog gate opening, a
/// door pulled, a lift arriving.
///
/// # Where velocity went
///
/// Into `move_meters`, and deliberately not into a second term beside it. Drift is velocity's
/// integral, and the integral is what actually invalidates a route: a player at 6 m/s who has
/// been running for a tenth of a second still has a good route, and one who crossed the same
/// ground slowly has an equally stale one. Gating on the derivative as well would re-plan for the
/// accelerating and spare the arrived, which is backwards.
///
/// What speed decides is how often the drift gate opens, and it does that by itself: at a sprint
/// the threshold is crossed in a third of a second, and standing still it is never crossed at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cadence {
    /// Metres of drift, summed over both ends, before the route is stale.
    pub(crate) move_meters: f32,
    /// Shortest gap between two searches, however fast either end is moving.
    pub(crate) min_seconds: f32,
    /// Longest a route may stand unasked while nothing moves.
    pub(crate) max_seconds: f32,
}

/// One route's clock, and the pair of positions it was last planned between.
///
/// Separate from [`Trail`] because the two answer different questions and fail differently: the
/// trail is about stones on the ground, this is about when to spend a navmesh search.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pace {
    /// Where the player and the target were when the last search went out, or `None` before the
    /// first one -- which is why the first plan is always immediate.
    planned: Option<([f32; 3], [f32; 3])>,
    /// Seconds since that search went out.
    elapsed: f32,
    /// Metres per second the player is covering, smoothed. Reported, not acted on; see
    /// [`Cadence`]'s note on where velocity went.
    speed: f32,
    /// Where the player was on the previous tick, for the speed estimate.
    was: Option<[f32; 3]>,
}

/// How much of the previous speed estimate survives one tick.
///
/// A single frame's displacement over a single frame's delta is a noisy number -- a stutter or a
/// hitching `delta` swings it by a factor of several -- and the value is read by a human out of a
/// log line. Smoothing makes "you are running" and "you are standing" tellable apart at a glance.
/// It decides nothing, so the constant is not load-bearing.
const SPEED_SMOOTHING: f32 = 0.9;

impl Pace {
    /// Advance the clock and the speed estimate. Once per tick, before [`Pace::due`].
    pub(crate) fn observe(&mut self, from: [f32; 3], delta: f32) {
        let delta = if delta.is_finite() {
            delta.max(0.0)
        } else {
            0.0
        };
        self.elapsed += delta;
        if let Some(was) = self.was
            && delta > f32::EPSILON
        {
            let sample = length(sub(from, was)) / delta;
            if sample.is_finite() {
                self.speed = self.speed * SPEED_SMOOTHING + sample * (1.0 - SPEED_SMOOTHING);
            }
        }
        self.was = Some(from);
    }

    /// Is a fresh search due, with the player at `from` and the target at `to`?
    pub(crate) fn due(&self, from: [f32; 3], to: [f32; 3], cadence: Cadence) -> bool {
        let Some((planned_from, planned_to)) = self.planned else {
            // Never planned. There is nothing for this to be stale relative to, and the player is
            // looking at an empty screen until it answers.
            return true;
        };
        if self.elapsed < cadence.min_seconds {
            return false;
        }
        if self.elapsed >= cadence.max_seconds {
            return true;
        }
        let drift = length(sub(from, planned_from)) + length(sub(to, planned_to));
        drift.is_finite() && drift >= cadence.move_meters
    }

    /// Record that a search has just gone out for this pair.
    pub(crate) fn requested(&mut self, from: [f32; 3], to: [f32; 3]) {
        self.planned = Some((from, to));
        self.elapsed = 0.0;
    }

    /// Metres per second the player is covering, smoothed.
    pub(crate) fn speed(&self) -> f32 {
        self.speed
    }
}

/// Sample the whole route evenly, end to end, as one continuous line.
///
/// # A trail has two ends, and splitting it is not a fix
///
/// This did split. Most of a fresh route is portal midpoints with nothing computed between them
/// -- `0x140bb4ac0` expands exactly one segment per plan -- so the version before this one ended
/// each expanded run and laid a single stone on each portal, on the reasoning that a stone should
/// never stand on ground nothing computed.
///
/// Tried live on 2026-09-24 and it was worse, for a reason the reasoning missed: with one segment
/// expanded, "only known ground" is about eleven metres of it. The trail stopped being a trail and
/// became four stones near your feet and three lone dots tens of metres apart. A path a player
/// follows has exactly two ends -- them and you -- and anything that breaks it into pieces has
/// destroyed the thing, however defensible each piece is.
///
/// So the sampling is continuous again, and the real problem is pushed where it belongs: the
/// route needs to be fully expanded BEFORE it is drawn, by the engine, rather than approximated
/// after the fact by this. [`RoutePoint::ground`] is kept because it is what says which spans are
/// still straight-line guesses, and that is the measurement the expansion work will be judged by.
fn sample_known_ground(path: &[RoutePoint], spacing: Spacing) -> Vec<[f32; 3]> {
    let line: Vec<[f32; 3]> = path.iter().map(|point| point.at).collect();
    resample(&line, spacing.meters, spacing.max_markers.saturating_add(1))
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
        // A route whose points coincide has no extent, so every stone in the world projects to
        // its start and "behind" means nothing. Retiring on that would empty the trail on any
        // frame the planner handed back a degenerate answer.
        if path
            .windows(2)
            .map(|pair| length(sub(pair[1], pair[0])))
            .sum::<f32>()
            <= f32::EPSILON
        {
            return Vec::new();
        }
        let mut retired = Vec::new();
        let mut keep = Vec::with_capacity(self.placed.len());
        for stone in self.placed.drain(..) {
            // BEHIND IS A POSITION ALONG THE ROUTE, NOT AN ANGLE FROM ITS FIRST STEP.
            //
            // The angle test that stood here retired everything past a bend: a route that turns
            // back on itself puts good ground at a negative dot from the opening direction. Live,
            // that pinned the trail at fourteen stones and re-laid the same one fifteen times a
            // second -- see `crate::geometry::arc_length_of_nearest` for the log line and the
            // fountain it produced.
            //
            // The route starts at the player, so a stone still on the way is somewhere along it
            // and a stone genuinely walked past projects back to its very beginning. Retiring on
            // "projects to the start AND is further than `keep_behind` from it" is the same
            // intent with none of the geometry.
            let arc = arc_length_of_nearest(stone.at, path);
            let behind = arc <= f32::EPSILON;
            if behind && length(sub(stone.at, player)) > keep_behind {
                retired.push(stone.handle);
            } else {
                keep.push(stone);
            }
        }
        self.placed = keep;
        retired
    }

    /// Is every stone still standing on `route`?
    ///
    /// # Why the trail is torn down whole rather than mended
    ///
    /// [`Trail::plan`] is idempotent -- a candidate within half a spacing of a stone already down
    /// is skipped -- and that is what stops a re-plan stacking forty effects on one flagstone. It
    /// also means a route that has genuinely MOVED leaves its old stones exactly where they were:
    /// the trail grows a second branch down a corridor nobody is walking any more, and on screen
    /// both branches look equally like the way to go. That is worse than no trail, because a
    /// player follows it.
    ///
    /// So the question asked of a fresh route is not "which stones can be kept" but "is this
    /// still the same path". A stone further than `tolerance` from the polyline says no, and the
    /// caller puts the WHOLE trail out and lays it again from the player's feet. Mending it stone
    /// by stone would leave the trail permanently part-old, which is the failure this avoids
    /// rather than a tidier version of it.
    ///
    /// `tolerance` is half a spacing -- the same distance [`Trail::plan`] calls "already taken"
    /// -- so a stone is on the route exactly when `plan` would have re-used its ground. A route
    /// that merely re-snapped to slightly different navigation nodes therefore does not tear
    /// anything down.
    pub(crate) fn follows(&self, route: &[[f32; 3]], tolerance: f32) -> bool {
        if self.placed.is_empty() {
            return true;
        }
        if route.len() < 2 || !tolerance.is_finite() || tolerance < 0.0 {
            // Nothing to be on, or no rule for what "on" means. Not the same as being off the
            // route: the caller has nothing to lay against either, and tearing the trail down on
            // a momentarily empty answer would make it strobe.
            return true;
        }
        self.placed
            .iter()
            .all(|stone| distance_to_path(stone.at, route) <= tolerance)
    }

    /// Where to put stones this pass, from your feet outwards.
    ///
    /// # Stones only go where the navmesh has been walked
    ///
    /// A route is not one kind of point -- see [`crate::navpath::RoutePoint`]. Near you it is an
    /// expanded polyline that follows the ground; past that it is a handful of portal midpoints
    /// tens of metres apart, because `0x140bb4ac0` expands exactly one segment per plan and the
    /// engine fills the rest in as an agent walks into them.
    ///
    /// Both kinds are real positions on the navmesh, so both get a stone. What does not get a
    /// stone is the SPAN between two portals: nothing computed that line, and spacing markers
    /// evenly along it puts them through hillsides and out over cliffs. A live route to a
    /// character 63.9 m away ended with hops of 33.9 m and 28.8 m; at 2.7 m spacing that is
    /// twenty-three stones in mid-air, which is exactly what was on screen.
    ///
    /// So the path is split at every `ground: false` point, each run is sampled on its own, and a
    /// portal contributes just itself. The trail still reaches the target -- the same route ended
    /// 1.0 m from them -- it simply stops inventing the ground in between.
    ///
    /// Returns an empty vector when the trail is already complete, which is the steady state and
    /// not a failure. Nothing is recorded: the caller spawns each point and calls
    /// [`Trail::remember`] for the ones that actually took, so a spawn the engine threw away
    /// does not leave a phantom in the bookkeeping that stops a real one landing there later.
    pub(crate) fn plan(&self, path: &[RoutePoint], spacing: Spacing) -> Vec<[f32; 3]> {
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
        let samples = sample_known_ground(path, spacing);
        let threshold = spacing.meters * DUPLICATE_FRACTION;
        let mut out: Vec<[f32; 3]> = Vec::new();
        // `sample_known_ground` keeps the very first point, which is the player's own feet. A
        // stone there is inside the character model and invisible, so the trail begins one
        // spacing out.
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
            // Ground the engine has refused often enough is treated exactly like ground that
            // already holds a stone: skipped, so the pass moves on to the next candidate instead
            // of spending every tick's whole budget on the same square metre.
            let refused = self.refused.iter().any(|refusal| {
                refusal.attempts >= REFUSALS_BEFORE_SKIPPING
                    && length(sub(refusal.at, candidate)) <= threshold
            });
            if !taken && !refused {
                out.push(candidate);
            }
        }
        out
    }

    /// Record a stone that the engine really placed.
    pub(crate) fn remember(&mut self, at: [f32; 3], handle: H) {
        self.placed.push(Stone { at, handle });
    }

    /// Record that the engine would not put a stone here.
    ///
    /// Called for every attempt that came back without a live effect. After
    /// [`REFUSALS_BEFORE_SKIPPING`] of them the site stops being offered -- see the `refused`
    /// field for the 600-spawn fountain that made this necessary.
    ///
    /// Returns `true` on the attempt that writes the site off, so the caller can say so once
    /// rather than on every refusal.
    pub(crate) fn refuse(&mut self, at: [f32; 3], tolerance: f32) -> bool {
        if let Some(refusal) = self
            .refused
            .iter_mut()
            .find(|refusal| length(sub(refusal.at, at)) <= tolerance)
        {
            refusal.attempts = refusal.attempts.saturating_add(1);
            return refusal.attempts == REFUSALS_BEFORE_SKIPPING;
        }
        self.refused.push(Refusal { at, attempts: 1 });
        REFUSALS_BEFORE_SKIPPING <= 1
    }

    /// How many patches of ground have been written off.
    pub(crate) fn refused(&self) -> usize {
        self.refused
            .iter()
            .filter(|refusal| refusal.attempts >= REFUSALS_BEFORE_SKIPPING)
            .count()
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
        // The refusals go with the stones. A site the engine would not take an effect on is a
        // fact about a moment -- the FX pool was full, the map was mid-swap -- and carrying that
        // verdict into the next trail would leave a permanent hole in a route that has since
        // become perfectly spawnable.
        self.refused.clear();
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

    /// A path the engine expanded end to end.
    ///
    /// What every test written before the portal distinction assumed, and still the case worth
    /// testing the spacing rules against. The first point's flag is ignored either way --
    /// `sample_known_ground` keeps it regardless, as the player's own feet.
    fn walked(points: &[[f32; 3]]) -> Vec<RoutePoint> {
        points
            .iter()
            .map(|at| RoutePoint {
                at: *at,
                ground: true,
            })
            .collect()
    }

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
            for (index, at) in trail.plan(&walked(path), spacing).into_iter().enumerate() {
                trail.remember(at, (pass * 100 + index) as u32);
            }
        }
    }

    #[test]
    fn a_pass_places_no_more_than_its_budget() {
        let trail = Numbered::default();
        assert_eq!(trail.plan(&walked(&line(100.0, 10.0)), spacing()).len(), 3);
    }

    #[test]
    fn the_trail_unrolls_from_your_feet_outwards() {
        let mut trail = Numbered::default();
        let path = line(100.0, 10.0);
        let first = trail.plan(&walked(&path), spacing());
        for (index, at) in first.iter().enumerate() {
            trail.remember(*at, index as u32);
        }
        let second = trail.plan(&walked(&path), spacing());
        assert!(
            second[0][0] > first[2][0],
            "pass two went backwards: {first:?} then {second:?}"
        );
    }

    #[test]
    fn no_stone_lands_on_the_player() {
        let trail = Numbered::default();
        let placed = trail.plan(&walked(&line(100.0, 10.0)), spacing());
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
            trail.plan(&walked(&path), spacing()).is_empty(),
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
        assert!(trail.plan(&walked(&drifted), spacing()).is_empty());
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
        assert!(
            trail
                .plan(&walked(&[[0.0, 0.0, 0.0]]), spacing())
                .is_empty()
        );
        assert!(trail.plan(&walked(&[]), spacing()).is_empty());
    }

    #[test]
    fn a_zero_budget_plans_nothing() {
        let trail = Numbered::default();
        let none = Spacing {
            per_pass: 0,
            ..spacing()
        };
        assert!(trail.plan(&walked(&line(100.0, 10.0)), none).is_empty());
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
        let planned = trail.plan(&walked(&path), spacing());
        assert_eq!(planned.len(), 3);
        // The engine dropped all three; nothing is recorded.
        assert_eq!(trail.placed(), 0);
        assert_eq!(trail.plan(&walked(&path), spacing()), planned);
    }
}

#[cfg(test)]
mod cadence_tests {
    use super::*;

    type Numbered = Trail<u32>;

    /// A path the engine expanded end to end.
    ///
    /// What every test written before the portal distinction assumed, and still the case worth
    /// testing the spacing rules against. The first point's flag is ignored either way --
    /// `sample_known_ground` keeps it regardless, as the player's own feet.
    fn walked(points: &[[f32; 3]]) -> Vec<RoutePoint> {
        points
            .iter()
            .map(|at| RoutePoint {
                at: *at,
                ground: true,
            })
            .collect()
    }

    fn spacing() -> Spacing {
        Spacing {
            meters: 2.0,
            keep_behind_meters: 12.0,
            max_markers: 10,
            per_pass: 3,
        }
    }

    fn cadence() -> Cadence {
        Cadence {
            move_meters: 2.0,
            min_seconds: 0.25,
            max_seconds: 3.0,
        }
    }

    fn straight(length_meters: f32) -> Vec<[f32; 3]> {
        (0..=(length_meters as i32 / 10))
            .map(|step| [(step * 10) as f32, 0.0, 0.0])
            .collect()
    }

    /// Run the clock forward without moving the player.
    fn idle(pace: &mut Pace, at: [f32; 3], seconds: f32) {
        let mut spent = 0.0;
        while spent < seconds {
            pace.observe(at, 1.0 / 60.0);
            spent += 1.0 / 60.0;
        }
    }

    #[test]
    fn the_first_route_is_asked_for_immediately() {
        let pace = Pace::default();
        assert!(pace.due([0.0, 0.0, 0.0], [50.0, 0.0, 0.0], cadence()));
    }

    #[test]
    fn standing_still_does_not_burn_a_search() {
        let mut pace = Pace::default();
        pace.requested([0.0, 0.0, 0.0], [50.0, 0.0, 0.0]);
        // Two seconds of nobody moving: well past the old half-second timer, still inside the
        // backstop, and nothing has drifted.
        idle(&mut pace, [0.0, 0.0, 0.0], 2.0);
        assert!(!pace.due([0.0, 0.0, 0.0], [50.0, 0.0, 0.0], cadence()));
    }

    #[test]
    fn walking_past_the_threshold_asks_again() {
        let mut pace = Pace::default();
        pace.requested([0.0, 0.0, 0.0], [50.0, 0.0, 0.0]);
        idle(&mut pace, [0.0, 0.0, 0.0], 0.5);
        assert!(pace.due([2.5, 0.0, 0.0], [50.0, 0.0, 0.0], cadence()));
    }

    #[test]
    fn the_target_moving_counts_as_much_as_the_player_moving() {
        let mut pace = Pace::default();
        pace.requested([0.0, 0.0, 0.0], [50.0, 0.0, 0.0]);
        idle(&mut pace, [0.0, 0.0, 0.0], 0.5);
        assert!(
            pace.due([0.0, 0.0, 0.0], [53.0, 0.0, 0.0], cadence()),
            "an invader running away left the route where they used to be"
        );
    }

    #[test]
    fn a_sprint_cannot_ask_faster_than_the_floor() {
        let mut pace = Pace::default();
        pace.requested([0.0, 0.0, 0.0], [50.0, 0.0, 0.0]);
        // One frame later, a hundred metres away -- a warp, or a fall down a shaft.
        pace.observe([100.0, 0.0, 0.0], 1.0 / 60.0);
        assert!(
            !pace.due([100.0, 0.0, 0.0], [50.0, 0.0, 0.0], cadence()),
            "a warp issued a navmesh search on the same frame it arrived"
        );
    }

    #[test]
    fn a_route_nobody_has_moved_along_is_still_refreshed_eventually() {
        let mut pace = Pace::default();
        pace.requested([0.0, 0.0, 0.0], [50.0, 0.0, 0.0]);
        idle(&mut pace, [0.0, 0.0, 0.0], 3.5);
        assert!(
            pace.due([0.0, 0.0, 0.0], [50.0, 0.0, 0.0], cadence()),
            "a fog gate could open and the route would never notice"
        );
    }

    #[test]
    fn asking_again_restarts_the_clock() {
        let mut pace = Pace::default();
        pace.requested([0.0, 0.0, 0.0], [50.0, 0.0, 0.0]);
        idle(&mut pace, [0.0, 0.0, 0.0], 3.5);
        pace.requested([0.0, 0.0, 0.0], [50.0, 0.0, 0.0]);
        assert!(!pace.due([0.0, 0.0, 0.0], [50.0, 0.0, 0.0], cadence()));
    }

    #[test]
    fn a_running_player_reads_as_running() {
        let mut pace = Pace::default();
        let mut x = 0.0f32;
        // Six metres a second for a second, which is about a DARK SOULS II sprint.
        for _ in 0..60 {
            pace.observe([x, 0.0, 0.0], 1.0 / 60.0);
            x += 6.0 / 60.0;
        }
        assert!(pace.speed() > 4.0, "a sprint measured {} m/s", pace.speed());
        idle(&mut pace, [x, 0.0, 0.0], 1.0);
        assert!(
            pace.speed() < 1.0,
            "standing still measured {} m/s",
            pace.speed()
        );
    }

    #[test]
    fn a_stalled_frame_does_not_divide_by_zero() {
        let mut pace = Pace::default();
        pace.observe([0.0, 0.0, 0.0], 0.0);
        pace.observe([9.0, 0.0, 0.0], 0.0);
        assert!(pace.speed().is_finite(), "speed became {}", pace.speed());
        pace.observe([9.0, 0.0, 0.0], f32::NAN);
        assert!(pace.speed().is_finite());
    }

    #[test]
    fn a_trail_laid_along_a_route_is_still_on_it() {
        let mut trail = Numbered::default();
        let path = straight(100.0);
        for (index, at) in trail
            .plan(&walked(&path), spacing())
            .into_iter()
            .enumerate()
        {
            trail.remember(at, index as u32);
        }
        assert!(trail.placed() > 0);
        assert!(trail.follows(&path, spacing().on_route_tolerance()));
    }

    #[test]
    fn a_route_that_turns_down_another_corridor_tears_the_trail_down() {
        let mut trail = Numbered::default();
        let path = straight(100.0);
        for (index, at) in trail
            .plan(&walked(&path), spacing())
            .into_iter()
            .enumerate()
        {
            trail.remember(at, index as u32);
        }
        // The target moved; the planner now sends you the other way from the same spot.
        let elsewhere = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 100.0]];
        assert!(
            !trail.follows(&elsewhere, spacing().on_route_tolerance()),
            "stones down a corridor nobody is walking were kept"
        );
    }

    #[test]
    fn a_route_that_merely_re_snapped_is_not_a_teardown() {
        let mut trail = Numbered::default();
        let path = straight(100.0);
        for (index, at) in trail
            .plan(&walked(&path), spacing())
            .into_iter()
            .enumerate()
        {
            trail.remember(at, index as u32);
        }
        // Half a metre of drift, the amount a re-snap to a neighbouring navigation node moves a
        // route. Inside half a spacing, so nothing comes down.
        let nudged: Vec<[f32; 3]> = path.iter().map(|p| [p[0], 0.0, 0.5]).collect();
        assert!(trail.follows(&nudged, spacing().on_route_tolerance()));
    }

    #[test]
    fn an_empty_trail_follows_anything() {
        let trail = Numbered::default();
        assert!(trail.follows(&straight(100.0), 1.0));
        assert!(trail.follows(&[], 1.0));
    }

    #[test]
    fn a_momentarily_empty_route_does_not_strobe_the_trail() {
        let mut trail = Numbered::default();
        trail.remember([5.0, 0.0, 0.0], 1);
        assert!(
            trail.follows(&[], 1.0),
            "one answerless pass would have put the whole trail out"
        );
        assert!(trail.follows(&[[0.0, 0.0, 0.0]], 1.0));
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    type Numbered = Trail<u32>;

    fn spacing() -> Spacing {
        Spacing {
            meters: 2.0,
            keep_behind_meters: 12.0,
            max_markers: 10,
            // One per pass, which is what `markers_per_pass` divides down to with two lanes open
            // -- and the budget the stuck site was eating whole.
            per_pass: 1,
        }
    }

    fn straight() -> Vec<RoutePoint> {
        (0u8..=20)
            .map(|step| RoutePoint {
                at: [f32::from(step) * 5.0, 0.0, 0.0],
                ground: true,
            })
            .collect()
    }

    /// The fountain, in one test: the engine refuses a site, and the trail must not go on
    /// offering it until the end of time.
    #[test]
    fn a_site_the_engine_keeps_refusing_is_given_up() {
        let mut trail = Numbered::default();
        let path = straight();
        let first = trail.plan(&path, spacing());
        assert_eq!(first.len(), 1, "one candidate per pass: {first:?}");
        let stuck = first[0];

        // Every pass refuses it, the way a spawn that returns no handle does.
        for pass in 0..REFUSALS_BEFORE_SKIPPING {
            let planned = trail.plan(&path, spacing());
            assert_eq!(
                planned[0], stuck,
                "pass {pass} moved on before the site was written off"
            );
            trail.refuse(planned[0], spacing().on_route_tolerance());
        }

        let after = trail.plan(&path, spacing());
        assert_ne!(
            after[0], stuck,
            "the trail is still hammering the refused site -- this is the 600-spawn fountain"
        );
    }

    #[test]
    fn the_trail_carries_on_past_a_refused_site() {
        let mut trail = Numbered::default();
        let path = straight();
        // Refuse whatever comes first, until it is written off.
        for _ in 0..REFUSALS_BEFORE_SKIPPING {
            let planned = trail.plan(&path, spacing());
            trail.refuse(planned[0], spacing().on_route_tolerance());
        }
        // Now lay the rest, refusing nothing.
        for _ in 0..20 {
            for at in trail.plan(&path, spacing()) {
                trail.remember(at, 0);
            }
        }
        assert!(
            trail.placed() >= 5,
            "only {} stone(s) got down past the refused site",
            trail.placed()
        );
    }

    #[test]
    fn one_bad_frame_does_not_write_a_site_off() {
        let mut trail = Numbered::default();
        let path = straight();
        let planned = trail.plan(&path, spacing());
        trail.refuse(planned[0], spacing().on_route_tolerance());
        assert_eq!(
            trail.plan(&path, spacing())[0],
            planned[0],
            "a single refusal gave the ground up; the FX pool is momentarily full all the time"
        );
        assert_eq!(trail.refused(), 0);
    }

    #[test]
    fn tearing_the_trail_down_forgives_every_refusal() {
        let mut trail = Numbered::default();
        let path = straight();
        let stuck = trail.plan(&path, spacing())[0];
        for _ in 0..REFUSALS_BEFORE_SKIPPING {
            trail.refuse(stuck, spacing().on_route_tolerance());
        }
        assert_eq!(trail.refused(), 1);
        let _ = trail.take_all();
        assert_eq!(
            trail.refused(),
            0,
            "a verdict from one moment outlived its trail"
        );
        assert_eq!(
            trail.plan(&path, spacing())[0],
            stuck,
            "the fresh trail must be allowed to try that ground again"
        );
    }
}

#[cfg(test)]
mod bend_tests {
    use super::*;

    type Numbered = Trail<u32>;

    fn spacing() -> Spacing {
        Spacing {
            meters: 2.7,
            keep_behind_meters: 12.0,
            max_markers: 72,
            // What `markers_per_pass` divides down to with two lanes open, and the budget the
            // stuck site was consuming whole.
            per_pass: 1,
        }
    }

    /// A route that leaves the player heading one way and then turns back past them -- the shape
    /// of the ground around Majula's cliff, and the shape that broke the old rule.
    ///
    /// The opening step points along +x. Everything after the bend sits at negative x, so the
    /// old `dot(stone - player, forward) < 0` test called it all "behind" and retired it.
    fn switchback() -> Vec<RoutePoint> {
        let mut out: Vec<RoutePoint> = Vec::new();
        for step in 0u8..6 {
            out.push(RoutePoint {
                at: [f32::from(step) * 3.0, 0.0, 0.0],
                ground: step != 0,
            });
        }
        for step in 0u8..14 {
            out.push(RoutePoint {
                at: [15.0 - f32::from(step) * 3.0, 0.0, 6.0],
                ground: true,
            });
        }
        out
    }

    fn line(points: &[RoutePoint]) -> Vec<[f32; 3]> {
        points.iter().map(|point| point.at).collect()
    }

    /// The live failure in one test: lay a stone, then retire nothing, then lay the next.
    ///
    /// Live this logged `laid 1/1 stone(s) this pass -- 14 down of at most 72` forever, because
    /// each pass laid one stone past the bend and the next retired it.
    #[test]
    fn a_stone_past_a_bend_is_not_retired_the_moment_it_is_laid() {
        let mut trail = Numbered::default();
        let path = switchback();
        let mut handle = 0u32;
        let mut high = 0usize;
        for pass in 0..40 {
            for at in trail.plan(&path, spacing()) {
                trail.remember(at, handle);
                handle += 1;
            }
            let retired = trail.retire_behind(&line(&path), spacing().keep_behind_meters);
            assert!(
                retired.is_empty(),
                "pass {pass} retired {} stone(s) off a route nobody has walked yet",
                retired.len()
            );
            high = high.max(trail.placed());
        }
        assert!(
            high >= 10,
            "the trail stalled at {high} stone(s) -- this is the fourteen-stone ceiling"
        );
    }

    #[test]
    fn a_stone_you_really_walked_past_is_still_retired() {
        let mut trail = Numbered::default();
        // Behind the start of the route and well past `keep_behind`.
        trail.remember([-40.0, 0.0, -30.0], 1);
        let path = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [20.0, 0.0, 0.0]];
        let retired = trail.retire_behind(&path, 12.0);
        assert_eq!(
            retired.len(),
            1,
            "the rule stopped retiring anything at all"
        );
        assert_eq!(trail.placed(), 0);
    }

    #[test]
    fn a_stone_just_off_the_start_is_left_alone() {
        let mut trail = Numbered::default();
        // Behind, but inside `keep_behind`: the trail must not appear to end at your feet.
        trail.remember([-5.0, 0.0, 0.0], 1);
        let path = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
        assert!(trail.retire_behind(&path, 12.0).is_empty());
        assert_eq!(trail.placed(), 1);
    }
}
