//! The seam where this crate is allowed to talk to the game, and everything that happens there.
//!
//! # Why `Present` is not this seam, corrected
//!
//! The repo has said for a while that `Present` runs on "the game's render thread". **That is
//! wrong, and so is the conclusion people drew from it in both directions.** DARK SOULS II
//! presents from the MAIN thread -- the one that owns the window and runs the simulation.
//! `0x140af09b0` calls the game-state tick and the draw step twenty bytes apart in one function
//! body, and several `GameManagerImp` state handlers (`0x1401bdda0` and seven siblings) call the
//! Present wrapper themselves. The only threads the engine creates through
//! `DLKR::DLThread::DLThread` are `Core.Logging.AsyncStrategy`, `Core.Res.TaskManager`,
//! `Core.Res.PostProcessor`, `MOMainThread`, `MtTaskWorkerThread` and `DrawCommandThread`, and
//! `DrawCommandThread::Run` (`0x140b698e0`) is a blocking command-replay pump with no swap chain
//! in it at all.
//!
//! So a `Present` detour is NOT racing the simulation. It is still the wrong place, for a reason
//! the thread question was standing in for: `0x140af02f0` presents with the `KatanaMainApp` sync
//! object AND the `DrawSystem` lock held, after `EndDraw` has cleared the frame's in-progress
//! flag. Calling engine code from in there means taking the FX manager's unlocked lists inside
//! two locks that `DrawCommandThread` and the task workers also take, in a frame the engine
//! considers over. `NvNavigationSystem::Update` holds nothing, and it is mid-simulation, which is
//! where all 46 of the engine's own SFX spawn sites are.
//!
//! # Why the callee and not one of its callers
//!
//! `NvNavigationSystem::Update` is reached from four game-state handlers. Hooking it instead of
//! them is one detour rather than four, it arrives with the `NvNavigationSystem` already in `rcx`
//! and already proven non-null (every caller tests it first), and it cannot fire at the title
//! screen or during a load -- which is exactly the window in which neither the navmesh nor the
//! SFX system exists.
//!
//! # The shape of one tick
//!
//! ```text
//! original(nav_system, delta)     <-- the engine steps our planners, among everything else
//! then, ours:
//!   1. has the map changed?        forget every planner and trail, extinguishing nothing
//!   2. who is wanted?              open a lane per target, close the lanes nobody asked for
//!   3. per lane, a search in flight?  poll it -- FAILED before READY, always
//!   4. per lane, has either end moved? snap both ends, request
//!   5. per lane, a route to follow?   lay this lane's share of `markers_per_pass` along it
//! ```
//!
//! Step 2 is after the original on purpose and not as a matter of taste: creating a planner
//! pushes onto the head of the intrusive list the original is in the middle of walking.
//!
//! # One lane per target
//!
//! A lane is a target, an `NvRoutePlanner` of its own, a clock, and a trail. They are independent
//! all the way down: two invaders in one session get two searches in flight at once, two routes,
//! and two trails of stones in their own two colours, and neither waits on the other.
//!
//! This file used to serve exactly one, and the argument for that was real -- a planner is engine
//! state, and one per agent is what the engine's own AI allocates. It is also what the engine's
//! own AI does *per agent*, which is the part the argument dropped: an area with a dozen hollows
//! in it is already stepping a dozen of these. Four more is a rounding error against that, and
//! the alternative was a feature that could only ever point at one of the people hunting you.
//!
//! What is capped is how many lanes exist ([`MAX_LANES`], and `max_routes` below it in the file
//! the player edits), and the total number of stones and spawns across all of them -- so adding a
//! target divides the trail budget rather than multiplying the engine's work.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::log::log;
use crate::navquery::{self, Poll};
use crate::sfx;
use crate::trail::{Cadence, Pace, Spacing, Trail};

/// `NvNavigationSystem::Update(NvNavigationSystem*, f32 delta)`.
///
/// The delta really is a float in `xmm1` -- see
/// [`ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE`]. Declaring it as an integer would compile and would
/// hand the engine whatever was left in that register.
type NavUpdate = unsafe extern "system" fn(usize, f32);

/// Trampoline back to the real `NvNavigationSystem::Update`.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// The most lanes this file will ever run, whatever the config file asks for.
///
/// A backstop under `max_routes`, not the setting itself: the draw side publishes at most
/// `max_routes` targets and the parser already caps that, so reaching this would mean two
/// independent caps had both been got wrong. Each lane is an engine object on the navigation
/// system's update list, and that is not a thing to leave unbounded behind a number read from a
/// file a player edits with a text editor while the game runs.
const MAX_LANES: usize = 8;

/// Seconds a lane stays quiet about tearing its trail down, having just said so.
///
/// A teardown is worth seeing -- it is the visible thing this file does when a target doubles
/// back -- and at the replan rate it is also worth seeing at most every few seconds. Five, so a
/// chase across a map leaves a readable handful of lines rather than a column nobody scrolls
/// through.
const TEARDOWN_QUIET_SECONDS: f32 = 5.0;

/// Seconds a self-check may sit waiting for a route before it gives up and says so.
///
/// **A diagnostic that can hang is worse than one that gives up**, and there are several ways for
/// this one to wait forever, all of them ordinary: `marker_effect_id` left at `0` so there is
/// nothing to place, no `KatanaSfxSystem` yet, a planner that never answers, an NPC that is
/// somewhere no route reaches. Each has its own log line where it can be detected, and this is
/// the backstop for the ones nobody has thought of. Thirty seconds is far longer than the whole
/// check should take and short enough that a user watching the screen has not given up first.
const SELF_CHECK_DEADLINE_SECONDS: f32 = 30.0;

/// Ticks a search may stay pending before it is abandoned.
///
/// `NvNavigationSystem::Update` steps every listed object up to 0x80 times per tick, so a search
/// that has not answered in this many ticks is not going to -- it means the planner stopped being
/// stepped, which is a different failure from "there is no way to walk there" and must not look
/// like a route that is merely slow.
const PENDING_TICK_BUDGET: u32 = 120;

/// One target the draw side wants a route to. Written every frame, read on the tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wanted {
    /// Where the local player is.
    pub(crate) from: [f32; 3],
    /// Where the player being routed to is.
    pub(crate) to: [f32; 3],
    /// That player's `PlayerCtrl` address, so an answer can be matched to the question.
    pub(crate) target: u64,
    /// The stone to lay along this target's route. `0` means no trail for this one.
    ///
    /// Per target rather than per session, and that is what makes several trails readable at
    /// once: the draw side picks it from the same palette slot the target's line is drawn in, so
    /// the blue stones and the blue arrow belong to the same person. Three trails in one colour
    /// would be one trail with three branches to anybody looking at it.
    pub(crate) effect_id: u32,
}

/// The marker settings shared by every lane, forwarded from the config the draw side owns.
///
/// The per-target part -- which stone -- rides on [`Wanted`] instead. These are the numbers that
/// are about the trail as a thing rather than about whose trail it is.
///
/// The budgets inside are for the whole overlay, not for one lane. See [`lay_markers`], which
/// divides them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Markers {
    pub(crate) spacing: Spacing,
}

/// The answer the draw side reads back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Answer {
    /// Which player this is a route to.
    pub(crate) target: u64,
    /// The route in walking order, start first, or `None` when the planner said there is no way
    /// to walk there. `None` is the arrow's cue and is a complete answer, not a missing one.
    pub(crate) route: Option<Vec<[f32; 3]>>,
}

/// Is the self-check running? Set from the config every frame.
///
/// When it is, two things change and nothing else does: spawns go through
/// `crate::sfx::spawn_reporting` instead of `spawn`, and what happened at each step -- the snap,
/// the guard, the route, every spawn attempt -- is described in the log. The route, the planner,
/// the cadence, the spacing and the budgets are the same code the real trail uses, which is the
/// point. A self-check that ran its own private path would prove only that its own private path
/// works.
///
/// It changes nothing about what the trail does. It used to stop it, to sample the same stones
/// three times; see [`Check`] for why that is gone.
static SELF_CHECK: Mutex<bool> = Mutex::new(false);

/// Set once the self-check has said everything it is going to say.
///
/// **The draw side no longer reads this, and that is the fix for an overlay that switched itself
/// off.** It used to: `done` stopped the draw side nominating an NPC, which dropped `WANTED` to
/// `None`, which took the target away -- so thirty seconds after the check gave up on a route
/// that was never going to arrive, a solo session drew nothing at all and had no way to say
/// whether the mod was even loaded. A diagnostic must not be able to disable the feature it
/// observes.
///
/// What it still does, which is all it was ever entitled to do: stop THIS file laying and
/// re-laying stones for a check that has finished, and stop the check printing its lines twice.
static SELF_CHECK_DONE: Mutex<bool> = Mutex::new(false);

/// Turn the self-check on or off. Called from the draw side every frame.
///
/// Going from off to on re-arms it, so editing the config file while the game runs re-runs the
/// check rather than requiring a restart -- the file is re-read every second, and a diagnostic
/// you can only run once per launch is a diagnostic nobody runs twice.
pub(crate) fn set_self_check(on: bool) {
    let Ok(mut slot) = SELF_CHECK.try_lock() else {
        return;
    };
    if on
        && !*slot
        && let Ok(mut done) = SELF_CHECK_DONE.try_lock()
    {
        *done = false;
    }
    *slot = on;
}

/// Everyone the draw side wants a route to. Empty is "stand down"; it is not "wait".
static WANTED: Mutex<Vec<Wanted>> = Mutex::new(Vec::new());
/// The marker settings, or `None` before the overlay has run once.
static MARKERS: Mutex<Option<Markers>> = Mutex::new(None);
/// One answer per lane, keyed by target.
static ANSWERS: Mutex<Vec<Answer>> = Mutex::new(Vec::new());

/// When a route is worth asking for again.
///
/// Its own static rather than a field on [`Markers`], because `MARKERS` is `None` whenever the
/// trail is switched off and the routes still have to be planned at some cadence -- an overlay
/// with `marker_effect_id = 0` still draws lines and still needs them fresh. Folding the two
/// together would have made "no stones" silently mean "no re-planning".
///
/// The defaults here are the ones in `crate::config`, repeated because a `Mutex` in a `static`
/// has to be built in a const context and cannot call across to them. They are overwritten by
/// [`set_cadence`] on the first frame the overlay draws.
static CADENCE: Mutex<Cadence> = Mutex::new(Cadence {
    move_meters: 2.0,
    min_seconds: 0.25,
    max_seconds: 3.0,
});

/// Ask for routes. Called from the draw side; returns immediately and does no engine work.
///
/// The whole list, every frame, rather than additions and removals: the roster is re-read and
/// re-sorted every frame anyway, so a list is the thing the draw side already has, and a lane
/// whose target is not in it is a lane whose target has gone. Handing over deltas would mean two
/// places believing things about who is present, and one of them would be wrong first.
pub(crate) fn ask(wanted: Vec<Wanted>) {
    if let Ok(mut slot) = WANTED.try_lock() {
        *slot = wanted;
    }
}

/// Tell the tick what the config says about markers.
pub(crate) fn set_markers(markers: Option<Markers>) {
    if let Ok(mut slot) = MARKERS.try_lock() {
        *slot = markers;
    }
}

/// Tell the tick how often a route is worth re-asking for.
pub(crate) fn set_cadence(cadence: Cadence) {
    if let Ok(mut slot) = CADENCE.try_lock() {
        *slot = cadence;
    }
}

/// The newest answer about `target`, if there is one.
///
/// Matched by target rather than returned blind. A route drawn to the player it was not planned
/// for is a confident line to the wrong person, which is worse than the arrow it replaced, and
/// the roster is re-sorted every frame. With a lane per target that is no longer the
/// near-certainty it was when one planner served everybody -- but the match is what makes it
/// true rather than likely.
pub(crate) fn answer_for(target: u64) -> Option<Option<Vec<[f32; 3]>>> {
    let slot = ANSWERS.try_lock().ok()?;
    slot.iter()
        .find(|answer| answer.target == target)
        .map(|answer| answer.route.clone())
}

/// One target's route: its own planner, its own clock, its own stones.
///
/// Nothing here is shared with another lane, which is what lets two of them be in flight at the
/// same time. What they do share -- the spawn budget and the total stone count -- is divided in
/// [`lay_markers`] rather than held here, because a division depends on how many lanes are open
/// and a lane does not know that about itself.
struct Lane {
    /// The `PlayerCtrl` address this lane routes to.
    target: u64,
    /// This lane's `NvRoutePlanner`, linked onto the navigation system's update list.
    planner: usize,
    /// Ticks the in-flight search has been waiting, or `None` when nothing is in flight.
    pending: Option<u32>,
    /// When this lane's route is worth asking for again, and how fast the player is going.
    pace: Pace,
    /// The newest route, or `None` when the planner said there is no way to walk there. The same
    /// value the draw side reads back through [`answer_for`], kept here as well so the trail does
    /// not have to go through a lock to find the ground it is laying along.
    route: Option<Vec<crate::navpath::RoutePoint>>,
    /// Set when `route` has just been replaced, and cleared by the trail once it has checked
    /// whether its stones are still on it. The teardown test is not free -- every stone against
    /// every segment -- and running it on a route nothing has changed about would be paying for
    /// an answer that cannot have moved.
    fresh: bool,
    /// The stone this lane's trail is laid in, from the target's palette slot.
    effect_id: u32,
    /// Seconds before this lane may say out loud that it tore its trail down again. A teardown is
    /// worth seeing once; at the replan rate it would otherwise be four lines a second.
    quiet_for: f32,
    /// The stones on the ground.
    trail: Trail<sfx::Handle>,
}

impl Lane {
    fn new(target: u64, planner: usize, effect_id: u32) -> Self {
        Self {
            target,
            planner,
            pending: None,
            pace: Pace::default(),
            route: None,
            fresh: false,
            effect_id,
            quiet_for: 0.0,
            trail: Trail::default(),
        }
    }
}

/// Everything the tick owns. Touched from the tick and nowhere else.
struct Tick {
    /// The `NvNavigationSystem` every lane's planner belongs to.
    nav_system: usize,
    /// The map area the trails below were laid in.
    area: u32,
    /// One lane per target being routed to, at most [`MAX_LANES`] of them.
    lanes: Vec<Lane>,
    /// Which lane gets first call on this tick's spawn budget.
    ///
    /// Rotated every tick. Without it the budget is spent front-to-back and the last lane is
    /// served only by what the ones before it did not want -- so the nearest player's trail would
    /// finish while the furthest never started, which looks exactly like the furthest one being
    /// broken.
    turn: usize,
    /// One-shot log lines. A tick runs sixty times a second; none of these may repeat.
    said_planner: bool,
    said_quality: bool,
    said_route: bool,
    said_no_route: bool,
    /// Whether the audit's scope paragraph has been printed. The per-route verdict repeats; the
    /// paragraph saying what the audit cannot see does not, because a paragraph that repeats is
    /// a paragraph nobody reads.
    said_audit_scope: bool,
    said_full: bool,
    /// Whether the self-check has printed its snap line. Once per run, not twice a second.
    said_snap: bool,
    /// How far through a self-check run this is.
    check: Check,
    /// Stock Prism Stone id -> the sparkle-free copy registered for it, or `None` when building
    /// or registering that copy failed and the trail falls back to the stock stone. Filled once
    /// per colour, on first use; see [`stripped_stone`].
    stripped: std::collections::HashMap<u32, Option<u32>>,
}

/// The id to spawn for `stock`: its sparkle-free copy when there is one, else `stock` itself.
///
/// The first time a Prism Stone colour is asked for, its `.ffx` is read out of the player's own
/// `Game/sfx/sfx9999.ffxbnd.dcx`, the sparkle child is cut ([`crate::stone_effect`]), and the
/// result is registered under [`crate::stone_effect::stripped_id`]. That is one file read and one
/// inflate per colour per process, on the game thread, the first time a trail of that colour is
/// laid. Any failure is logged once and that colour keeps the stock stone rather than drawing
/// nothing. An id that is not a Prism Stone is returned unchanged.
fn stripped_stone(
    stripped: &mut std::collections::HashMap<u32, Option<u32>>,
    system: usize,
    stock: u32,
) -> u32 {
    if !ds2_rva::PRISM_STONE_SFX_IDS.contains(&stock) {
        return stock;
    }
    let resolved = *stripped.entry(stock).or_insert_with(|| {
        let id = crate::stone_effect::stripped_id(stock);
        let built = std::env::current_exe()
            .map_err(|error| format!("no executable path: {error}"))
            .and_then(|exe| {
                let path = exe
                    .parent()
                    .ok_or("the executable has no directory")?
                    .join(crate::stone_effect::BUNDLE_RELATIVE_PATH);
                std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))
            })
            .and_then(|dcx| crate::stone_effect::member_from_bundle(&dcx, stock))
            .and_then(|ffx| crate::stone_effect::strip_sparkles(&ffx, id))
            // SAFETY: game thread, mid-simulation -- `lay_markers` is only reached from the nav
            // tick -- and `system` is the live `KatanaSfxSystem` that call just resolved.
            .and_then(|bytes| unsafe { sfx::register_effect(system, id, bytes) });
        match built {
            Ok(()) => {
                log(format_args!(
                    "Prism Stone {stock}: laying the sparkle-free copy, registered as effect {id}"
                ));
                Some(id)
            }
            Err(why) => {
                log(format_args!(
                    "Prism Stone {stock}: could not build the sparkle-free copy ({why}); this \
                     colour keeps the stock stone"
                ));
                None
            }
        }
    });
    resolved.unwrap_or(stock)
}

/// Where a self-check run has got to.
///
/// A state machine rather than a pile of booleans because the states are genuinely sequential and
/// the illegal combinations -- narrating a route that never arrived, reporting one twice -- are
/// the ones a diagnostic must not produce. A log that contradicts itself is worse than no log.
///
/// # The stage that used to sit between these two, and why it is gone
///
/// `Watching` held the trail still: once stones were down, `lay_markers` returned early for
/// several seconds so the diagnostic could sample the same stones three times and say "7/7 alive
/// at t=10s" about a set that had not changed underneath it.
///
/// A path is never allowed to freeze. There is a player at one end of every route this crate
/// plans -- the local one, at minimum -- and a player moves, so a trail that stops being re-laid
/// is a trail going out of date while somebody watches it. The measurement was worth having once
/// and the freeze that bought it is not worth having ever; a diagnostic does not get to suspend
/// the feature it observes, which is the same rule that took `SELF_CHECK_DONE` away from the draw
/// side.
#[derive(Debug, Default, PartialEq)]
enum Check {
    /// The config does not ask for one.
    #[default]
    Off,
    /// Armed, and waiting for the planner to answer. Carries seconds spent waiting, against
    /// [`SELF_CHECK_DEADLINE_SECONDS`].
    WaitingForRoute(f32),
    /// Everything has been said. The route and the trail carry on exactly as they would for a
    /// real player; only the narration stops.
    Done,
}

impl Default for Tick {
    fn default() -> Self {
        Self {
            nav_system: 0,
            area: u32::MAX,
            lanes: Vec::new(),
            turn: 0,
            said_planner: false,
            said_quality: false,
            said_route: false,
            said_no_route: false,
            said_audit_scope: false,
            said_full: false,
            said_snap: false,
            check: Check::Off,
            stripped: std::collections::HashMap::new(),
        }
    }
}

static TICK: Mutex<Option<Tick>> = Mutex::new(None);

/// Install the detour on `NvNavigationSystem::Update`.
///
/// `false` means no route and no trail this session, already logged. The arrow is unaffected --
/// it needs none of this.
///
/// # Safety
///
/// Installs a native code detour. Call once, from the loader's install position.
pub(crate) unsafe fn install() -> bool {
    use core::ffi::c_void;

    use ds2_hook::{MH_ApplyQueued, MH_Initialize, MH_STATUS, MhHook};

    let site = match ds2_game_base::mem::game_rva(ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE) {
        Ok(site) => site,
        Err(error) => {
            log(format_args!("tick: no game module -- {error}"));
            return false;
        }
    };
    // THE RVA IS A NUMBER UNTIL SOMETHING CHECKS IT. A build these were not read from puts the
    // detour in the middle of whatever else lives there, and the first symptom would be a crash
    // on the first frame of gameplay in someone's session.
    let mut prologue = [0u8; ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE_PROLOGUE.len()];
    // SAFETY: a resolved RVA inside the loaded image; `read_bytes` reports an unmapped page
    // rather than faulting.
    let read = unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) };
    if !read || prologue != ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE_PROLOGUE {
        log(format_args!(
            "tick: 0x{site:016x} is not NvNavigationSystem::Update -- found {prologue:02x?}, \
             wanted {:02x?}. No route and no trail; the arrow still draws.",
            ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE_PROLOGUE
        ));
        return false;
    }
    // SAFETY: MinHook's own initialiser; idempotent, and `crate::render` may already have run it.
    // SAFETY: `MH_Initialize` takes no arguments and is safe to call again on an already-
    // initialised library, which the status below distinguishes.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!("tick: MH_Initialize said {status:?}"));
        return false;
    }
    // NOT `ds2_hook::register_union_hook`. The union's shared handler signature is four `usize`,
    // and this target's second argument is a float in `xmm1` -- a volatile register the
    // dispatcher is entitled to clobber. Nothing else in this workspace hooks this address, so
    // there is no collision for the union to arbitrate and no reason to pay for one.
    //
    // SAFETY: `site` has been proven to hold this function's own prologue, and `nav_update`
    // matches its ABI. The trampoline is stored before the hook is enabled, so the detour can
    // never run without one.
    // SAFETY: the target is an RVA this crate validated against the prologue it expects before
    // reaching here, and the detour is a `'static` fn item of the matching ABI.
    let hook = match unsafe { MhHook::new(site as *mut c_void, nav_update as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!("tick: MH_CreateHook said {status:?}"));
            return false;
        }
    };
    ORIGINAL.store(hook.trampoline() as usize, Ordering::Release);
    if let Ok(mut state) = TICK.lock() {
        *state = Some(Tick::default());
    }
    // SAFETY: enabling the hook created immediately above.
    if let Err(status) = unsafe { hook.queue_enable() } {
        log(format_args!("tick: queue_enable said {status:?}"));
        return false;
    }
    // SAFETY: applies this DLL's queued hooks; other features queue theirs the same way.
    if unsafe { MH_ApplyQueued() } != MH_STATUS::MH_OK {
        log(format_args!("tick: MH_ApplyQueued refused"));
        return false;
    }
    log(format_args!(
        "tick: NvNavigationSystem::Update hooked at 0x{site:016x} -- routes and markers can run \
         on the game's own tick"
    ));
    true
}

/// The detour. Runs the engine's tick, then ours.
///
/// Never panics and never propagates: this is the game's simulation step, and an unwind through
/// a `extern "system"` boundary into engine code is undefined behaviour rather than an error
/// message.
unsafe extern "system" fn nav_update(nav_system: usize, delta: f32) {
    let raw = ORIGINAL.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is MinHook's trampoline for this exact function, stored before the hook
        // was enabled, and the signature is the one the prologue check proved.
        let original: NavUpdate = unsafe { std::mem::transmute::<usize, NavUpdate>(raw) };
        // SAFETY: `original` is the trampoline MinHook produced for this target, so calling it runs the
        // bytes the detour displaced. The arguments are this detour's own, passed through untouched.
        unsafe { original(nav_system, delta) };
    }
    // AFTER the original, always. Creating a planner pushes onto the head of the intrusive list
    // the original walks; doing it from inside that walk corrupts the walk.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        work(nav_system, delta);
    }));
}

/// One tick of this crate's own work, on the game thread, with no engine lock held.
fn work(nav_system: usize, delta: f32) {
    let Ok(mut guard) = TICK.try_lock() else {
        return;
    };
    let Some(state) = guard.as_mut() else {
        return;
    };

    // THE MAP CHANGED, AND THE STONES ARE DELIBERATELY LEAKED. Sixteen kilobytes of leak per
    // warp, worst case, and it is the correct trade rather than laziness.
    //
    // A stone's control block is an intrusive node in an FX effect's controller list -- the head
    // is `node + 0xf8` and the links live INSIDE the block. Two things could be true across an
    // area boundary and this code cannot tell which:
    //
    //   - the FX manager was torn down with the map, in which case putting the stone out reads a
    //     freed manager;
    //   - the manager survived (it hangs off `KatanaSfxSystem`, a `GameManagerImp` singleton) and
    //     the NODE was recycled into somebody else's effect, in which case putting the stone out
    //     kills the wrong thing.
    //
    // Extinguishing is wrong in the first case, dropping is wrong in the second -- freed memory
    // the engine still has a pointer to is the worse of the two. `forget` is wrong in NEITHER:
    // the bytes stay allocated and untouched, so whatever the engine still believes about them
    // stays true, and nothing is called. It costs a box per stone until the process exits.
    //
    // The planner is a different case and is torn down properly: the `NvNavigationSystem` is the
    // same object across areas, so the planner is still linked and still ours -- but its world
    // pointer was bound at creation from an area that is gone, so it is retired and remade.
    let area = current_map_index();
    if state.nav_system != nav_system || state.area != area {
        let same_system = state.nav_system == nav_system;
        let mut stranded = 0usize;
        for mut lane in state.lanes.drain(..) {
            if lane.planner != 0 && same_system {
                // SAFETY: game thread, and `retire` only sets the byte the next tick acts on.
                unsafe { navquery::retire(nav_system, lane.planner) };
            }
            for handle in lane.trail.take_all() {
                stranded += 1;
                core::mem::forget(handle);
            }
        }
        if stranded > 0 {
            log(format_args!(
                "tick: map changed -- {stranded} marker(s) stranded; their storage is leaked on \
                 purpose rather than freed under an engine that may still hold a pointer to it"
            ));
        }
        // THE ANSWERS GO WITH THE LANES, and forgetting this is a route drawn through a map that
        // is gone. The same players are usually still in the session across a warp, so the lanes
        // are reopened under the same target addresses on the next pass -- and an answer matched
        // by target would be handed straight back out, made of points from the area behind you,
        // for however many ticks the fresh search takes to land.
        if let Ok(mut answers) = ANSWERS.try_lock() {
            answers.clear();
        }
        state.nav_system = nav_system;
        state.area = area;
        state.said_planner = false;
    }

    // WHO IS WANTED, read once, so the whole tick is inside one answer rather than half in each.
    let wanted: Vec<Wanted> = match WANTED.try_lock() {
        Ok(slot) => slot.clone(),
        // Another thread is mid-write. Not "nobody is wanted" -- standing down on a contended
        // lock would put the trail out and take it up again on the next frame, which is a
        // flicker with no cause a reader could find.
        Err(_) => return,
    };

    // NOTHING IS WANTED: the overlay is off, you are alone, or everyone else is close enough that
    // `near_suppress_meters` says you already know where they are.
    //
    // Two things follow, and the second is the one that would otherwise be a bug you only notice
    // an hour into a session. Every trail goes out -- leaving stones burning behind a feature that
    // has stopped running is exactly the litter the stop call was hunted down to avoid. And the
    // stale answers are cleared, so the draw side cannot keep drawing a route to somebody who is
    // no longer being routed to.
    //
    // Nothing is created here either. This crate's contract is that it does not touch the game
    // unless you turn it on, and linking an object into the engine's update list is touching the
    // game, so a session with the overlay off gets a detour that reads two pointers and returns.
    if wanted.is_empty() {
        let mut out = 0usize;
        for lane in state.lanes.drain(..) {
            out += close_lane(nav_system, lane);
        }
        if out > 0 {
            log(format_args!(
                "markers: nothing to route to -- putting {out} stone(s) out"
            ));
        }
        if let Ok(mut answers) = ANSWERS.try_lock() {
            answers.clear();
        }
        return;
    }

    open_and_close_lanes(state, &wanted, nav_system);
    forget_stale_answers(&state.lanes);
    if state.lanes.is_empty() {
        // Every planner the engine would give us was refused. Nothing to poll and nothing to lay.
        return;
    }

    // ARM OR DISARM THE SELF-CHECK BEFORE ANYTHING READS IT, so one tick is entirely inside one
    // state rather than half in each.
    let wanted_check = SELF_CHECK.try_lock().is_ok_and(|on| *on);
    if !wanted_check {
        state.check = Check::Off;
    } else if state.check == Check::Off {
        state.check = Check::WaitingForRoute(0.0);
        log(format_args!(
            "self-check: armed -- routing to the nearest non-player character and laying the \
             trail along whatever comes back"
        ));
    }

    let cadence = CADENCE.try_lock().map(|slot| *slot).unwrap_or(Cadence {
        move_meters: 2.0,
        min_seconds: 0.25,
        max_seconds: 3.0,
    });
    for index in 0..state.lanes.len() {
        poll_or_request(state, index, &wanted, cadence, delta);
    }
    if lay_markers(state) {
        report_first_stones(state);
    }
    watch_stones(state, delta);
}

/// Put a lane's stones out and hand its planner back to the engine.
///
/// Returns how many stones were extinguished, for the caller's log line.
///
/// # Safety at the call site, not in the signature
///
/// Every call is from `work`, on the game thread, in the area the stones were spawned in -- the
/// map-change branch runs first and would have stranded them instead. A lane closed anywhere else
/// would be extinguishing effects under a map that has gone.
fn close_lane(nav_system: usize, mut lane: Lane) -> usize {
    if lane.planner != 0 {
        // SAFETY: game thread; the planner is one `create_planner` returned, and `retire` only
        // sets the byte the engine's own next tick acts on.
        unsafe { navquery::retire(nav_system, lane.planner) };
    }
    let out = lane.trail.take_all();
    let count = out.len();
    for handle in out {
        // SAFETY: game thread, same area they were spawned in, no engine lock held.
        unsafe { handle.extinguish() };
    }
    count
}

/// Bring the lanes into line with who the draw side is asking about.
///
/// # Why a target that left takes its planner with it
///
/// A lane is not free to leave lying about. Its planner stays on the navigation system's update
/// list and is stepped by the engine every frame for as long as it is there, so a session that
/// saw twenty phantoms come and go would be stepping twenty dead planners and burning a trail's
/// worth of effects on each. Closing the lane is what makes "one per target" a bound rather than
/// a description of the first few seconds.
fn open_and_close_lanes(state: &mut Tick, wanted: &[Wanted], nav_system: usize) {
    let mut closed = 0usize;
    let mut stones = 0usize;
    let mut index = 0;
    while index < state.lanes.len() {
        // A lane whose planner went unreadable is closed here too, and it is the same closure:
        // the target is still wanted, so the loop below opens a fresh lane for it on this very
        // pass. That is why `Poll::Lost` only has to zero the pointer.
        let live = state.lanes[index].planner != 0;
        if live
            && wanted
                .iter()
                .any(|ask| ask.target == state.lanes[index].target)
        {
            index += 1;
            continue;
        }
        let lane = state.lanes.remove(index);
        closed += usize::from(live);
        stones += close_lane(nav_system, lane);
    }
    if closed > 0 {
        log(format_args!(
            "tick: {closed} target(s) left the session -- their planners are retired and \
             {stones} stone(s) are out"
        ));
    }

    for ask in wanted {
        if let Some(lane) = state
            .lanes
            .iter_mut()
            .find(|lane| lane.target == ask.target)
        {
            // The colour follows the palette slot, which can change when somebody else leaves.
            // Stones already down keep the old one until the trail is next torn down; repainting
            // them would mean extinguishing a whole good trail to change its hue.
            lane.effect_id = ask.effect_id;
            continue;
        }
        if state.lanes.len() >= MAX_LANES {
            break;
        }
        // SAFETY: game thread, after the original's list walk has finished -- this pushes onto
        // the head of the list that walk traverses, which is why `work` calls it from here and
        // not from inside the detour's own body.
        let Some(planner) = (unsafe { navquery::create_planner(nav_system) }) else {
            // The factory refused. Not fatal and not permanent: the next tick asks again, and
            // the lanes that did open carry on.
            break;
        };
        if !state.said_planner {
            state.said_planner = true;
            log(format_args!(
                "tick: route planner 0x{planner:016x} linked -- {} object(s) on the navigation \
                 system's list. One of these is created per target being routed to.",
                navquery::listed_objects(nav_system).unwrap_or(-1)
            ));
        }
        state
            .lanes
            .push(Lane::new(ask.target, planner, ask.effect_id));
    }
}

/// Give up on a self-check that is never going to arrive.
///
/// Read-only apart from the deadline itself.
fn watch_stones(state: &mut Tick, delta: f32) {
    // THE BACKSTOP, AND IT EXISTS BECAUSE A HUNG DIAGNOSTIC IS WORSE THAN A FAILED ONE. Several
    // ordinary conditions leave the check waiting for something that is never coming -- markers
    // switched off, no SFX system yet, a route nobody answers -- and each of those returns early
    // from somewhere else in this file without ever reaching the finish. Waiting forever would
    // also mean routing to an NPC forever, which is a feature the player did not ask for.
    if let Check::WaitingForRoute(waited) = &mut state.check {
        *waited += delta.max(0.0);
        if *waited > SELF_CHECK_DEADLINE_SECONDS {
            let markers = MARKERS.try_lock().ok().and_then(|slot| *slot);
            let nothing_to_place = state.lanes.iter().all(|lane| lane.effect_id == 0);
            log(format_args!(
                "self-check: gave up after {SELF_CHECK_DEADLINE_SECONDS:.0}s without laying a \
                 stone. {}",
                match markers {
                    None =>
                        "No marker settings reached the tick at all -- the overlay may never \
                             have drawn a frame.",
                    Some(_) if nothing_to_place =>
                        "`marker_effect_id` is 0, so there was never anything to place: the \
                         route half of the check ran and the trail half could not. Set it to 833 \
                         (a Prism Stone) and run again.",
                    Some(_) =>
                        "Settings were present, so look above for the route lines: either \
                                no route arrived or no spawn returned a handle.",
                }
            ));
            if let Ok(mut done) = SELF_CHECK_DONE.try_lock() {
                *done = true;
            }
            state.check = Check::Done;
        }
    }
}

/// Say what the first pass of stones did, then stand the narration down.
///
/// # What this replaced, and what was given up with it
///
/// A three-sample watch: hold the trail still, count how many stones were still alive at t=1s, 3s
/// and 10s, and report whether the effect lingers or bursts. It answered a real question, and the
/// only way it could answer it was to stop the trail being re-laid underneath it -- fresh stones
/// are trivially alive, so a moving trail makes the number meaningless.
///
/// A path does not get to freeze. So the measurement goes rather than the movement, and nothing
/// here pretends to make it any more: this reports the one pass it can honestly describe. Whether
/// a Prism Stone lingers is a question for a scratch experiment that owns its own effects, not for
/// the feature's own trail.
fn report_first_stones(state: &mut Tick) {
    let placed: usize = state.lanes.iter().map(|lane| lane.trail.placed()).sum();
    // SAFETY: game thread, in the area the stones were spawned in -- the area-change branch at
    // the top of `work` has already run this tick and would have emptied the trails otherwise.
    let alive = state
        .lanes
        .iter()
        .flat_map(|lane| lane.trail.handles())
        .filter(|handle| unsafe { handle.alive() })
        .count();
    log(format_args!(
        "self-check: {placed} stone(s) down across {} route(s), {alive} of them alive as the pass \
         ended{}. The trail carries on from here -- re-planned as you move, torn down and laid \
         again when the route changes -- so this is the last line, not the last of the stones.",
        state.lanes.len(),
        match (alive, placed) {
            (0, 0) => " -- nothing was ever placed, so this says nothing at all",
            (0, _) => " -- none of them survived the frame they were spawned on",
            (a, p) if a == p => " -- all of them",
            _ => " -- some died on the spawning frame and some did not",
        }
    ));
    if let Ok(mut done) = SELF_CHECK_DONE.try_lock() {
        *done = true;
    }
    state.check = Check::Done;
}

/// Advance one lane's search: poll what is in flight, or ask again if the route has gone stale.
///
/// Takes an index rather than a `&mut Lane` because the one-shot log flags live on [`Tick`] and a
/// borrow of the lane would lock the rest of it away. The destructuring below is what keeps those
/// two borrows disjoint without copying anything.
fn poll_or_request(
    state: &mut Tick,
    index: usize,
    wanted: &[Wanted],
    cadence: Cadence,
    delta: f32,
) {
    let Tick {
        lanes,
        said_route,
        said_no_route,
        said_audit_scope,
        said_snap,
        check,
        ..
    } = state;
    let Some(lane) = lanes.get_mut(index) else {
        return;
    };
    let Some(ask) = wanted.iter().find(|ask| ask.target == lane.target) else {
        // The lane outlived its question by a frame. `open_and_close_lanes` closes it on the next
        // tick; there is nothing to poll for in the meantime.
        return;
    };
    lane.pace.observe(ask.from, delta);
    lane.quiet_for = (lane.quiet_for - delta.max(0.0)).max(0.0);

    let target = lane.target;
    if let Some(waited) = lane.pending {
        // SAFETY: game thread; the planner is this lane's own and has not been retired.
        match unsafe { navquery::poll(lane.planner) } {
            Poll::Pending if waited < PENDING_TICK_BUDGET => {
                lane.pending = Some(waited + 1);
            }
            Poll::Pending => {
                // Not "no way to walk there" -- "the planner stopped being stepped". Different
                // failure, and it must not be reported as the first one.
                log(format_args!(
                    "tick: a search for 0x{target:x} went {PENDING_TICK_BUDGET} ticks without an \
                     answer -- abandoning it"
                ));
                lane.pending = None;
                lane.route = None;
                lane.fresh = true;
                publish(target, None);
            }
            Poll::Failed => {
                lane.pending = None;
                if matches!(check, Check::WaitingForRoute(_)) {
                    // FAILED on a live character standing on the navmesh is itself a finding, and
                    // a loud one: it means the snap or the planner is wrong, not that the world
                    // is. The real trail treats this as ordinary and says so once; the
                    // self-check must not let it pass quietly.
                    log(format_args!(
                        "self-check: the planner said NO ROUTE to 0x{target:x}. Read the `guard` \
                         part of the `snap` line above -- it names the condition. `guard \
                         REFUSES` is the engine rejecting the pair before expanding a node, and \
                         the reason is printed. `guard OK` means the search really ran and \
                         really found nothing, and THEN the capability mask (0x{:03x}) and the \
                         budget ({:.0}) are worth looking at. A MISS at either end is the snap, \
                         and the planner is innocent.",
                        ds2_rva::NV_ROUTE_PLANNER_CAPABILITY_DEFAULT,
                        ds2_rva::NV_ROUTE_MAX_COST_LONG_RANGE
                    ));
                }
                if !*said_no_route {
                    *said_no_route = true;
                    log(format_args!(
                        "tick: no walkable route to 0x{target:x} -- the arrow stands in, which is \
                         the same fallback the Elden Ring crate uses"
                    ));
                }
                lane.route = None;
                lane.fresh = true;
                publish(target, None);
            }
            Poll::Ready { points, segments } => {
                lane.pending = None;
                if matches!(check, Check::WaitingForRoute(_)) {
                    // BOTH NUMBERS. A route of forty segments that decodes to two points means
                    // the decoder is wrong; two and two means the walk really is that short. One
                    // number cannot tell those apart, and this is the only place both are in
                    // hand.
                    log(format_args!(
                        "self-check: READY -- {segments} segment(s) decoded to {} point(s), \
                         {:.1} m of path to 0x{target:x}",
                        points.len(),
                        path_length(&crate::navpath::positions(&points))
                    ));
                }
                if !*said_route {
                    *said_route = true;
                    log(format_args!(
                        "tick: first walkable route -- {segments} segment(s), {} point(s) to \
                         0x{target:x}",
                        points.len()
                    ));
                }
                // ASK THE ENGINE WHAT IT THINKS OF THE ROUTE IT JUST GAVE US.
                //
                // Once per request, not once per tick: `Poll::Ready` fires on the pass that
                // clears the lane's `pending`, so this is the fresh-route edge and not a poll
                // loop.
                //
                // SAFETY: game thread -- this is the `NvNavigationSystem::Update` detour -- and
                // the planner has just reported READY without FAILED, so the route embedded at
                // `+0x48` is the one `poll` decoded a moment ago.
                if let Some(report) = unsafe {
                    navquery::audit(lane.planner + ds2_rva::NV_ROUTE_PLANNER_ROUTE_OFFSET)
                } {
                    describe_audit(&report, said_audit_scope);
                }
                // BIND THE GUESSES TO THE GROUND BEFORE ANYBODY LAYS A STONE ON THEM. Done once
                // per fresh route rather than per tick: it costs one engine snap per sample, and
                // a route nothing has replaced cannot have changed shape.
                //
                // SAFETY: game thread -- this is the `NvNavigationSystem::Update` detour -- and
                // the world is the one the route was just planned through.
                let points = unsafe { bind_to_ground(points) };
                // The draw side gets bare positions; the trail keeps the flags, because it is the
                // only consumer that has to know which spans the engine actually computed.
                publish(target, Some(crate::navpath::positions(&points)));
                lane.route = Some(points);
                lane.fresh = true;
            }
            Poll::Lost => {
                log(format_args!(
                    "tick: a planner became unreadable -- its lane is closed and remade"
                ));
                lane.pending = None;
                // Zeroed rather than retired: the object could not be read, so handing it back to
                // the engine would be calling a method on something already gone. The lane is
                // dropped whole by the caller's next pass, which is the only cost -- one leaked
                // planner on a navigation system that is itself about to be replaced.
                lane.planner = 0;
            }
        }
        return;
    }

    if lane.planner == 0 {
        // Lost on a previous tick. `open_and_close_lanes` drops a lane without a planner at the
        // top of the next tick and opens a fresh one for the same target.
        return;
    }

    // THE ROUTE IS RE-ASKED ON MOVEMENT, NOT ON A CLOCK.
    //
    // The old rule was a half-second cooldown, taken from `0x14042ee40` -- the engine's own AI
    // reloads `ChrAiNavimeshCtrl + 0x2c0` with `0.5f` and refreshes its route bind when that
    // expires. Sound for an NPC, wrong here in both directions at once: it spends a navmesh
    // search twice a second on a player standing still, and it leaves a sprinting one up to nine
    // metres ahead of the route their own trail is laid along.
    //
    // `crate::trail::Cadence` has the whole argument, including where velocity went.
    if !lane.pace.due(ask.from, ask.to, cadence) {
        return;
    }

    // SAFETY: game thread. The allocator underneath the snap is lazily built and unguarded; this
    // is the one place in this crate entitled to touch it.
    let (from, to) = unsafe {
        (
            navquery::snap_reporting(ask.from),
            navquery::snap_reporting(ask.to),
        )
    };
    lane.pace.requested(ask.from, ask.to);

    // PRINT THE IDS BEFORE ASKING, AND PRINT THE ENGINE'S OWN GUARD BESIDE THEM.
    //
    // This line used to name the capability mask and the cost budget as the things to look at
    // when two good ids would not connect. That was WRONG, and it cost a session: neither is
    // reachable from the branch that refuses. `NvRoutePlanner`'s step resolves each end's graph
    // by hashing `id | 0x1ffff`, and when the two graphs differ it enters `0x140bb4310`, which
    // sets READY|FAILED -- the thing `poll` reports as NO ROUTE -- unless all four of
    // `area(start) == area(goal)`, both graphs resolved, and both link counts positive. Not one
    // node is expanded before that decision, so no mask and no budget can be involved in it.
    //
    // So the guard is read and printed. A refusal now names the false condition instead of
    // handing the reader a suspect list.
    //
    // Once per self-check run rather than twice a second, because a line that repeats sixty
    // times is a line nobody reads.
    if matches!(check, Check::WaitingForRoute(_)) && !*said_snap {
        *said_snap = true;
        // SAFETY: game thread, and the world exists -- the snaps above just walked it.
        let guard = match (from.id, to.id) {
            // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
            // offset this crate validated before installing. The callee's own contract asks for exactly
            // that live object, and reads inside it go through the fault-tolerant readers.
            (Some(start), Some(goal)) => unsafe { navquery::guard(start, goal) },
            _ => None,
        };
        log(format_args!(
            "self-check: snap start={} goal={} | world holds {} graph(s); {} | {}",
            describe_snap(&from),
            describe_snap(&to),
            from.graphs,
            describe_key(&from),
            describe_guard(guard.as_ref()),
        ));
    }

    let (Some(start), Some(goal)) = (from.id, to.id) else {
        // One of them is off the navmesh -- in a lift shaft, mid-fall, beyond the twenty-metre
        // snap radius, or in a map whose graph is not resident. A complete answer, and the
        // arrow's cue.
        lane.route = None;
        lane.fresh = true;
        publish(target, None);
        return;
    };
    // SAFETY: game thread; the planner is this lane's own, and the request clears both result
    // bits as it sets the pending one, so re-requesting over a search still in flight is safe.
    if unsafe {
        navquery::request(
            lane.planner,
            start,
            goal,
            ds2_rva::NV_ROUTE_MAX_COST_LONG_RANGE,
        )
    } {
        lane.pending = Some(0);
    }
}

/// Metres between the samples taken across a gap the engine never expanded.
///
/// One marker spacing would leave a stone on every sample and nothing between them to interpolate
/// wrongly; rather less than that is chosen so the line still bends with the hill between stones.
const GROUND_SAMPLE_METERS: f32 = 1.5;

/// A gap wider than this is a span the engine did not expand, and is worth binding to the ground.
///
/// Comfortably above the widest step inside a real expanded polyline -- the ones measured live ran
/// 0.18 m to 2.69 m -- so an expanded run is never resampled and never pays for a snap.
const GROUND_GAP_METERS: f32 = 4.0;

/// Most samples inserted across one gap, so a route across a whole map cannot explode the point
/// count or the number of engine calls one tick makes.
const GROUND_SAMPLES_PER_GAP: usize = 48;

/// Pull the straight guesses in a route down onto the navmesh.
///
/// # The hill
///
/// A route is a short expanded polyline near you and then portal midpoints tens of metres apart,
/// because `0x140bb4ac0` expands exactly one segment per plan. The line between two portals is an
/// interpolation nothing computed. On level ground it passes for a path; standing on a hill above
/// the character it is a straight line through open air, which is what was reported on
/// 2026-09-24 -- "it jumps over the air instead of binding to the ground" -- across a 32.5 m chord
/// that descended three metres.
///
/// So every wide gap is sampled and each sample is replaced by the centre of the navmesh triangle
/// underneath it. The result is one continuous line with the same two ends it had before, and no
/// point on it that the navmesh does not hold.
///
/// A sample the mesh cannot answer for keeps its interpolated position rather than being dropped:
/// a hole in the trail is worse than a point that is merely as wrong as it was before.
///
/// # Safety
///
/// Game thread only. Every sample calls the engine's snap, which is what `crate::navquery`'s whole
/// module header is about.
unsafe fn bind_to_ground(
    points: Vec<crate::navpath::RoutePoint>,
) -> Vec<crate::navpath::RoutePoint> {
    use crate::geometry::{add_scaled, length, normalize, sub};
    use crate::navpath::RoutePoint;

    if points.len() < 2 {
        return points;
    }
    let mut out: Vec<RoutePoint> = Vec::with_capacity(points.len());
    out.push(points[0]);
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let span = sub(to.at, from.at);
        let gap = length(span);
        // An expanded run is left exactly as the engine built it. Only the guesses are touched.
        if to.ground || !gap.is_finite() || gap <= GROUND_GAP_METERS {
            out.push(to);
            continue;
        }
        let Some(direction) = normalize(span) else {
            out.push(to);
            continue;
        };
        let steps = ((gap / GROUND_SAMPLE_METERS).floor() as usize).min(GROUND_SAMPLES_PER_GAP);
        for step in 1..steps {
            let along = add_scaled(from.at, direction, step as f32 * GROUND_SAMPLE_METERS);
            // SAFETY: game thread, and the world exists -- the route this came from was planned
            // through the same snap a moment ago.
            let at = unsafe { navquery::ground_under(along) }.unwrap_or(along);
            out.push(RoutePoint { at, ground: true });
        }
        out.push(to);
    }
    out
}

/// Publish one lane's answer for the draw side, replacing any earlier one about that target.
fn publish(target: u64, route: Option<Vec<[f32; 3]>>) {
    let Ok(mut slot) = ANSWERS.try_lock() else {
        return;
    };
    match slot.iter_mut().find(|answer| answer.target == target) {
        Some(answer) => answer.route = route,
        None => slot.push(Answer { target, route }),
    }
}

/// Throw away the answers about targets no lane is serving any more.
///
/// The draw side matches an answer to a player by address, so an answer that outlives its lane is
/// a route waiting to be drawn to whoever lands on that `PlayerCtrl` allocation next -- which is
/// the one thing [`answer_for`]'s matching exists to prevent, arriving by the back door. Run
/// straight after the lanes are squared up, where the list of who is still being served is in
/// hand.
fn forget_stale_answers(lanes: &[Lane]) {
    let Ok(mut slot) = ANSWERS.try_lock() else {
        return;
    };
    slot.retain(|answer| lanes.iter().any(|lane| lane.target == answer.target));
}

/// Put down this tick's share of every lane's trail, and put out what has been walked past.
///
/// # The budgets are for the overlay, not for one trail
///
/// `max_markers` and `markers_per_pass` are divided between the open lanes rather than applied to
/// each. Adding a target therefore divides the trail rather than multiplying the engine's work,
/// which is the difference between a second invader costing nothing and a fourth one costing four
/// times the spawns on the frame the simulation runs from. A player who wants longer trails with
/// several targets raises `max_markers`; nothing here decides that for them.
///
/// The spare from an uneven division goes to the lane whose `turn` it is, and the turn rotates,
/// so three lanes sharing three stones a pass do not starve the third one forever.
///
/// Returns `true` on the pass where a self-check's first laying finished, which is the caller's
/// cue to print the one line the check still has to print. Nothing about the trail changes on
/// that pass or after it.
#[must_use]
fn lay_markers(state: &mut Tick) -> bool {
    // NOTHING SUSPENDS THIS. A guard used to stand here that returned early while the self-check
    // sampled its stones, so that "7/7 alive at t=10s" was a statement about a set of stones
    // nothing had touched since they were laid.
    //
    // There is a player at one end of every route this file plans -- the local one, at minimum --
    // and a player moves. A trail that stops being re-laid is therefore a trail going out of date
    // in front of somebody who is looking at it and following it. No measurement is worth that,
    // and the diagnostic does not get to suspend the feature it observes. See `Check`.
    let Ok(slot) = MARKERS.try_lock() else {
        return false;
    };
    let Some(markers) = *slot else {
        // No settings yet, or the trail is off. Anything already down stays down: switching the
        // trail off mid-route should not make the stones behind you vanish, and putting them out
        // is a decision for `forget`, not for a missing config read.
        return false;
    };
    drop(slot);
    let Some(system) = sfx::system() else {
        return false;
    };

    let lanes = state.lanes.len().max(1);
    // Divided, not shared out greedily: a lane that cannot use its share this pass does not lend
    // it, because lending is what lets one long trail eat the frame budget every tick and leave
    // the others drawing nothing.
    let share = Spacing {
        max_markers: (markers.spacing.max_markers / lanes).max(1),
        per_pass: (markers.spacing.per_pass / lanes).max(1),
        ..markers.spacing
    };
    // The whole per-pass number is still the ceiling across all lanes, so an uneven division
    // cannot round its way past the budget it came from.
    let mut frame_budget = markers.spacing.per_pass.max(1);
    state.turn = state.turn.wrapping_add(1);
    let start = state.turn % lanes;

    let checking = matches!(state.check, Check::WaitingForRoute(_));
    let Tick {
        lanes: open,
        said_quality,
        said_full,
        stripped,
        ..
    } = state;
    let count = open.len();
    let mut laid_any = false;
    for step in 0..count {
        let lane = &mut open[(start + step) % count];
        if frame_budget == 0 {
            break;
        }
        if lane.effect_id == 0 {
            continue;
        }
        let Some(route) = lane.route.clone() else {
            // The planner said there is no way to walk there, or has not answered yet. Stones
            // already down stay down -- see `Trail::follows` on why a momentary absence of a
            // route is not evidence that the trail is wrong.
            continue;
        };
        // "Behind you" and "still on the route" are questions about WHERE the line goes, so they
        // take the positions alone. Only `plan` needs to know which spans are real ground.
        let line = crate::navpath::positions(&route);

        for handle in lane
            .trail
            .retire_behind(&line, markers.spacing.keep_behind_meters)
        {
            // SAFETY: game thread, same area as the spawn, no engine lock held.
            unsafe { handle.extinguish() };
        }

        // THE ROUTE MOVED, SO THE TRAIL COMES DOWN WHOLE. Checked once per fresh route rather
        // than every tick: the test walks every stone against every segment, and a route nothing
        // has replaced cannot have moved away from them.
        if lane.fresh {
            lane.fresh = false;
            if !lane.trail.follows(&line, share.on_route_tolerance()) {
                let out = lane.trail.take_all();
                let torn = out.len();
                for handle in out {
                    // SAFETY: game thread, same area as the spawn, no engine lock held.
                    unsafe { handle.extinguish() };
                }
                if lane.quiet_for <= 0.0 {
                    lane.quiet_for = TEARDOWN_QUIET_SECONDS;
                    log(format_args!(
                        "markers: the route to 0x{:x} moved off its own trail -- {torn} stone(s) \
                         out, laying again from your feet at {:.1} m/s",
                        lane.target,
                        lane.pace.speed()
                    ));
                }
            }
        }

        if !*said_quality {
            *said_quality = true;
            let level = sfx::quality(system).unwrap_or(u32::MAX);
            log(format_args!(
                "markers: effect {} on KatanaSfxSystem 0x{system:016x}, quality {level}{}",
                lane.effect_id,
                if level >= ds2_rva::KATANA_SFX_QUALITY_DROP_THRESHOLD {
                    " -- at or above the threshold at which the engine silently discards spawns"
                } else {
                    ""
                }
            ));
        }

        let this_pass = Spacing {
            per_pass: share.per_pass.min(frame_budget),
            ..share
        };
        let planned = lane.trail.plan(&route, this_pass);
        if planned.is_empty() && !*said_full && lane.trail.placed() >= share.max_markers {
            *said_full = true;
            log(format_args!(
                "markers: a trail is at its budget -- {} stone(s) down, and no more will be \
                 placed until you walk past some. The {} configured are divided between the \
                 {count} route(s) being drawn; raise `max_markers` if the trails stop short.",
                lane.trail.placed(),
                markers.spacing.max_markers
            ));
        }
        let mut attempted = 0usize;
        let mut took = 0usize;
        for (index, at) in planned.iter().enumerate() {
            // The direction a stone faces is the direction the trail runs at that point. The next
            // planned stone is the best available "onwards"; the last one borrows the one before
            // it, so the end of a trail does not suddenly point at nothing.
            let ahead = planned
                .get(index + 1)
                .or_else(|| index.checked_sub(1).and_then(|back| planned.get(back)));
            let direction = ahead.map_or([0.0, 1.0, 0.0], |next| crate::geometry::sub(*next, *at));
            attempted += 1;
            frame_budget = frame_budget.saturating_sub(1);
            // ONE COLOUR PER STONE WHILE CHECKING. The Prism Stone has seven ids, one per colour,
            // and sweeping them along the line answers in a single run which of the seven
            // actually appear -- for the cost of a modulo. A real trail uses one id throughout,
            // because a trail that changes colour as it goes reads as several trails -- and with
            // several trails on screen at once that is now literally true of the ones beside it.
            let stock = if checking {
                ds2_rva::PRISM_STONE_SFX_IDS
                    [lane.trail.placed().wrapping_add(index) % ds2_rva::PRISM_STONE_SFX_IDS.len()]
            } else {
                lane.effect_id
            };
            // The sparkle-free copy of that colour, so the trail is the glow alone.
            let id = stripped_stone(stripped, system, stock);
            if checking {
                // SAFETY: game thread, mid-simulation, no engine lock held.
                let attempt = unsafe { sfx::spawn_reporting(system, id, *at, direction) };
                log(format_args!("self-check: {}", attempt.describe(id)));
                match attempt.handle {
                    Some(handle) => {
                        took += 1;
                        lane.trail.remember(*at, handle);
                    }
                    // THE SAME REFUSAL RULE, AND LEAVING IT OUT OF THIS BRANCH KEPT THE FOUNTAIN.
                    //
                    // The first fix recorded refusals only on the ordinary path. A re-measure
                    // still found 600 spawns on one patch, and the id had changed to 839 -- a
                    // colour-sweep id, which only this branch emits. The diagnostic was running
                    // the identical loop beside the fixed one. Two spawn call sites, one rule.
                    None => {
                        if lane.trail.refuse(*at, share.on_route_tolerance()) {
                            log(format_args!(
                                "self-check: the engine refused every id offered at \
                                 {:.1},{:.1},{:.1} -- giving that spot up so the trail can get \
                                 past it",
                                at[0], at[1], at[2]
                            ));
                        }
                    }
                }
                continue;
            }
            // SAFETY: game thread, mid-simulation, no engine lock held -- which is the whole
            // reason this call is here and not in the `Present` detour.
            match unsafe { sfx::spawn(system, id, *at, direction) } {
                Some(handle) => {
                    took += 1;
                    lane.trail.remember(*at, handle);
                }
                // A REFUSAL IS RECORDED, AND NOT RECORDING IT WAS A PARTICLE FOUNTAIN.
                //
                // Measured live: 600 spawns in 40 seconds, all at one point, because a site the
                // engine would not take stayed the first untaken candidate forever -- and with
                // the per-pass budget divided between lanes that is the whole budget, so the
                // trail also stopped dead there. See `Trail::refused`.
                None => {
                    if lane.trail.refuse(*at, share.on_route_tolerance()) {
                        log(format_args!(
                            "markers: the engine refused effect {id} at {:.1},{:.1},{:.1} -- \
                             giving that spot up and carrying the trail on past it. {} site(s) \
                             written off on this trail so far.",
                            at[0],
                            at[1],
                            at[2],
                            lane.trail.refused()
                        ));
                    }
                }
            }
        }
        if attempted > 0 {
            laid_any = true;
        }
        if checking && attempted > 0 {
            log(format_args!(
                "self-check: laid {took}/{attempted} stone(s) this pass -- {} down of at most {}",
                lane.trail.placed(),
                share.max_markers
            ));
        }
    }

    // THE NARRATION STOPS ONCE THE TRAIL IS AS LONG AS IT IS GOING TO GET, and the trail does
    // not. "As long as it is going to get" is a pass that placed nothing while something is down
    // -- either the budget is full or every candidate is already taken, and both mean this first
    // laying is over. What follows is the ordinary life of a trail: re-planned as the player
    // moves, retired behind them, torn down and laid again when the route changes. The check has
    // nothing left to say about any of that and says nothing.
    let down: usize = open.iter().map(|lane| lane.trail.placed()).sum();
    checking && !laid_any && down > 0
}

/// What the map key did, in terms that name the cause rather than the symptom.
///
/// Three outcomes matter and they used to be one sentence. The SENTINEL case is the one worth
/// spelling out: `MapManager + 0x170` reads `0xffffffff` whenever the player's map entity does
/// not resolve, and the key built from it (`0x3fffffff`) is well-formed and matches nothing, so
/// a reader sees a plausible hex number and a failed lookup and has no reason to suspect the
/// field. Saying "the engine says there is no player map entity" is the difference between a
/// five-minute answer and an afternoon.
fn describe_key(report: &navquery::SnapReport) -> String {
    match report.map_index {
        None => "the MapManager could not be read at all".to_string(),
        Some(ds2_rva::MAP_INDEX_NONE) => format!(
            "MapManager+0x170 is the NO-PLAYER-ENTITY sentinel (0x{:08x}), so no key was built. \
             The engine's own `!= -1` guard cannot catch this -- the key builder's range is \
             0x00ffffff..=0x3fffffff. The sweep {}",
            ds2_rva::MAP_INDEX_NONE,
            match report.chosen_key {
                Some(found) => format!("found a graph anyway, carrying 0x{found:08x}"),
                None => "found nothing either".to_string(),
            }
        ),
        Some(index) => {
            let key = report.key;
            match (report.chosen_key, report.keyed_hit) {
                // The point of the sweep, said out loud: what the key SHOULD have been.
                (Some(found), false) => format!(
                    "map index {index} gave key 0x{key:08x} which matched NO graph, and the one \
                     the sweep chose carries 0x{found:08x} -- THAT is the key this map wants"
                ),
                (Some(found), true) if found != key => format!(
                    "map index {index} gave key 0x{key:08x} which matched a graph, but the sweep \
                     chose a DIFFERENT one carrying 0x{found:08x}"
                ),
                (Some(_), true) => {
                    format!("map index {index} gave key 0x{key:08x} and the sweep agreed")
                }
                (None, _) => format!(
                    "map index {index} gave key 0x{key:08x}, and no graph accepted the player at \
                     all -- so this is the snap radius or the filter, not the key"
                ),
            }
        }
    }
}

/// One end of a snap, as a reader needs to see it.
///
/// Hex, because a navigation-graph id is a packed bitfield -- index in `0..14`, kind in `15..16`,
/// parts key in `17..29`, validity in `30..31` -- and decimal hides every one of those
/// boundaries. `0xffffffff` is spelled out as a miss rather than printed as a number, because
/// that is the single most important thing this line can say.
fn describe_snap(report: &navquery::SnapReport) -> String {
    match (report.id, report.chosen) {
        // "SWEEP SLOT", NOT "GRAPH". This index is which entry of the world's eight-pointer
        // array the sweep happened to pick, and printing it as "graph 0" made two ids in
        // genuinely different graphs read as though they shared one -- which is the exact
        // distinction the planner refuses on. The graph identity is in the guard's keys.
        (Some(id), Some(index)) => format!(
            "0x{id:08x} (sweep slot {index}, {:.1} m off)",
            report.distance_squared.max(0.0).sqrt()
        ),
        (Some(id), None) => format!("0x{id:08x}"),
        (None, _) => "MISS (0xffffffff -- nothing within the snap radius on any graph)".to_string(),
    }
}

/// Print what the engine's own traversal test said about the route it just handed over.
///
/// # Why the raw attribute words are always printed
///
/// A verdict with no evidence under it cannot be checked by whoever reads the log next, and the
/// vocabulary of this table is documented nowhere -- it gets learned by watching which values
/// turn up on routes that behave and routes that do not. Two live words so far: `0x00000007`
/// (type `0`, capacity `7`) on ordinary floor and `0x00000047` (type `0x40`, capacity `7`) on a
/// gated edge.
///
/// `scope_said` is flipped the first time through: the verdict repeats per route, the paragraph
/// naming what the audit cannot see does not.
fn describe_audit(report: &navquery::Audit, scope_said: &mut bool) {
    let widest = match report.widest {
        Some(size) => format!("size class {size} of {}", ds2_rva::NAVI_SIZE_CLASSES - 1),
        // NOT "there is no way through". Every character in the game is size class 0..6 and the
        // smallest of those admits the most nodes, so a route that refuses even class 0 is a
        // wrong attribute table, and saying otherwise would blame the world for a bug here.
        None => "NONE -- not even the smallest agent, which reads as a wrong attribute table \
                 rather than as a fact about the world"
            .to_string(),
    };
    let words: Vec<String> = report
        .nodes
        .iter()
        .take(10)
        .map(|node| format!("0x{:08x}(t{:x}/c{})", node.attrs, node.kind, node.capacity))
        .collect();
    log(format_args!(
        "tick: route audit -- {} node(s) read{}. Widest agent this route admits: {widest}. The \
         request asked as size class 0, the smallest the format has. attrs: {}{}",
        report.nodes.len(),
        if report.unreadable == 0 {
            String::new()
        } else {
            format!(", {} unreadable", report.unreadable)
        },
        words.join(" "),
        if report.nodes.len() > 10 {
            format!(" ... {} more", report.nodes.len() - 10)
        } else {
            String::new()
        }
    ));
    for node in report.nodes.iter().filter(|node| node.notable()).take(12) {
        let verdict = if node.level != node.below {
            if node.below {
                "ASYMMETRIC, usable only when the edge sits BELOW you (a drop, not a climb)"
            } else {
                "ASYMMETRIC, usable only when the edge sits LEVEL WITH OR ABOVE you"
            }
        } else if node.level {
            // NOT "we are cheating here". `0x14042ee40` ORs `0x7f8` onto EVERY character's size,
            // so the feature bits are identical for us and for every NPC in the game.
            "gated type, passable both ways at 0x7f8, which is what every NPC has"
        } else {
            "IMPASSABLE even to the smallest agent"
        };
        let (x, y, z) = node
            .node
            .at
            .map_or((f32::NAN, f32::NAN, f32::NAN), |at| (at[0], at[1], at[2]));
        log(format_args!(
            "tick:   node 0x{:08x} (segment {}{} at {x:.2}, {y:.2}, {z:.2}) attrs 0x{:08x} \
             type 0x{:x} capacity {} -- {verdict}",
            node.node.id, node.node.segment, node.node.side, node.attrs, node.kind, node.capacity
        ));
    }
    if !*scope_said {
        *scope_said = true;
        log(format_args!(
            "tick:   Scope: the attribute word is read LIVE, so anything that rewrites it -- and \
             this engine has an NvNaviGraphCostUpdater and a MapObjNaviGraphLocationComponent, \
             both runtime-shaped -- shows up above. What is NOT covered is the gate layer \
             (NvNaviGraphGate, NvNaviGatePathFindingTask) or collision geometry never registered \
             with the navigation graph. Neither is ruled out by a clean line above."
        ));
    }
}

/// The planner's entry guard, as the four conditions it actually is.
///
/// Reads as `guard OK` or `guard REFUSES: <the false ones>`, so a NO ROUTE that follows has its
/// cause on the line above it rather than a list of suspects.
fn describe_guard(guard: Option<&navquery::Guard>) -> String {
    let Some(guard) = guard else {
        return "guard NOT READ (one end missed, or the world was not there to ask)".to_string();
    };
    let mut wrong = Vec::new();
    if guard.start_area != guard.goal_area {
        wrong.push(format!(
            "different areas (0x{:x} vs 0x{:x})",
            guard.start_area, guard.goal_area
        ));
    }
    if guard.start_graph == 0 {
        wrong.push(format!(
            "start key 0x{:08x} is in no graph",
            guard.start_key
        ));
    }
    if guard.goal_graph == 0 {
        wrong.push(format!("goal key 0x{:08x} is in no graph", guard.goal_key));
    }
    if guard.start_graph != 0 && guard.start_links <= 0 {
        wrong.push(format!("start graph has {} links", guard.start_links));
    }
    if guard.goal_graph != 0 && guard.goal_links <= 0 {
        wrong.push(format!("goal graph has {} links", guard.goal_links));
    }
    let keys = format!(
        "keys 0x{:08x}/0x{:08x}{}",
        guard.start_key,
        guard.goal_key,
        if guard.start_key == guard.goal_key {
            " (same graph)"
        } else {
            " (CROSS-GRAPH)"
        }
    );
    if wrong.is_empty() {
        format!("guard OK -- {keys}")
    } else {
        format!("guard REFUSES -- {keys}: {}", wrong.join("; "))
    }
}

/// Total length of a polyline, in metres. For the self-check's one-line route summary.
fn path_length(points: &[[f32; 3]]) -> f32 {
    points
        .windows(2)
        .map(|pair| crate::geometry::length(crate::geometry::sub(pair[1], pair[0])))
        .sum()
}

/// The map index the player is currently in, or [`u32::MAX`] when there is no world.
///
/// Keyed on rather than on the `NvNavigationSystem` pointer because that pointer is a singleton
/// that outlives a map change while the FX node pool underneath every marker does not.
fn current_map_index() -> u32 {
    let Some(manager) = navquery::game_manager() else {
        return u32::MAX;
    };
    // SAFETY: a live `GameManagerImp`; both reads refuse an unmapped page.
    unsafe {
        safe_read(manager + ds2_rva::GAME_MANAGER_MAP_MANAGER_OFFSET)
            .and_then(|map| safe_read_map_index(map))
            .unwrap_or(u32::MAX)
    }
}

/// A non-null pointer read, or `None`.
///
/// # Safety
///
/// `at` must be an address it is legal to attempt to read; the reader itself faults safely.
unsafe fn safe_read(at: usize) -> Option<usize> {
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
    // moved or was freed answers None rather than faulting.
    let value = unsafe { ds2_game_base::mem::safe_read_usize(at)? };
    (value != 0).then_some(value)
}

/// The player's map index on a `MapManager`.
///
/// [`ds2_rva::MAP_INDEX_NONE`] is deliberately NOT filtered out here. This value is only used to
/// notice that the map changed, and a load screen reading the sentinel and then reading a real
/// index really is two changes -- which costs one extra trail teardown, on a path that forgets
/// handles rather than touching them, during a load when the effects are gone anyway.
///
/// # Safety
///
/// `map_manager` must be a live `MapManager`; the reader itself faults safely.
unsafe fn safe_read_map_index(map_manager: usize) -> Option<u32> {
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    unsafe {
        ds2_game_base::mem::safe_read_u32(
            map_manager + ds2_rva::MAP_MANAGER_PLAYER_MAP_INDEX_OFFSET,
        )
    }
}
