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
//! original(nav_system, delta)     <-- the engine steps our planner, among everything else
//! then, ours:
//!   1. has the map changed?        forget the planner and the trail, extinguishing nothing
//!   2. no planner?                 create one; the engine will step it from next tick
//!   3. a search in flight?         poll it -- FAILED before READY, always
//!   4. idle, and someone asked?    snap both ends, request
//!   5. a route to follow?          lay up to `markers_per_pass` stones along it
//! ```
//!
//! Step 2 is after the original on purpose and not as a matter of taste: creating a planner
//! pushes onto the head of the intrusive list the original is in the middle of walking.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::log::log;
use crate::navquery::{self, Poll};
use crate::sfx;
use crate::trail::{Spacing, Trail};

/// `NvNavigationSystem::Update(NvNavigationSystem*, f32 delta)`.
///
/// The delta really is a float in `xmm1` -- see
/// [`ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE`]. Declaring it as an integer would compile and would
/// hand the engine whatever was left in that register.
type NavUpdate = unsafe extern "system" fn(usize, f32);

/// Trampoline back to the real `NvNavigationSystem::Update`.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Seconds between route requests.
///
/// **The engine's own number.** `0x14042ee40` reloads `ChrAiNavimeshCtrl + 0x2c0` with
/// `0x3f000000` -- `0.5f` -- every time that timer expires, and refreshes the AI's route bind
/// only when it has. Re-planning faster than the game's own AI does would be spending a
/// navmesh search per frame to redraw a line that has not meaningfully moved.
const REPLAN_SECONDS: f32 = 0.5;

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

/// What the draw side wants a route for. Written every frame, read when the tick is idle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wanted {
    /// Where the local player is.
    pub(crate) from: [f32; 3],
    /// Where the player being routed to is.
    pub(crate) to: [f32; 3],
    /// That player's `PlayerCtrl` address, so an answer can be matched to the question.
    pub(crate) target: u64,
}

/// The marker settings, forwarded from the config the draw side owns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Markers {
    /// `0` means no trail. See [`ds2_rva::PRISM_STONE_SFX_IDS`] for what a real one is.
    pub(crate) effect_id: u32,
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
/// When it is, three things change and nothing else does: spawns go through
/// `crate::sfx::spawn_reporting` instead of `spawn`, the route's arrival is described in the log,
/// and the stones are watched on `crate::selfcheck`'s schedule and then taken down. The route,
/// the snap, the planner, the spacing and the budgets are the SAME code the real trail uses --
/// which is the point. A self-check that ran its own private path would prove only that its own
/// private path works.
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

/// The question, or `None` when the overlay has nothing to route to.
static WANTED: Mutex<Option<Wanted>> = Mutex::new(None);
/// The marker settings, or `None` before the overlay has run once.
static MARKERS: Mutex<Option<Markers>> = Mutex::new(None);
/// The most recent answer.
static ANSWER: Mutex<Option<Answer>> = Mutex::new(None);

/// Ask for a route. Called from the draw side; returns immediately and does no engine work.
pub(crate) fn ask(wanted: Option<Wanted>) {
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

/// The newest answer, if it is about `target`.
///
/// Matched by target rather than returned blind: the roster is re-sorted every frame, and a route
/// drawn to the player it was NOT planned for is a confident line to the wrong person -- which is
/// worse than the arrow it replaced.
pub(crate) fn answer_for(target: u64) -> Option<Option<Vec<[f32; 3]>>> {
    let slot = ANSWER.try_lock().ok()?;
    let answer = slot.as_ref()?;
    (answer.target == target).then(|| answer.route.clone())
}

/// Everything the tick owns. Touched from the tick and nowhere else.
struct Tick {
    /// The `NvNavigationSystem` the planner below belongs to.
    nav_system: usize,
    /// The map area the trail below was laid in.
    area: u32,
    /// The planner, or `0` before one has been made.
    planner: usize,
    /// Which player the in-flight search is for, and how long it has been in flight.
    pending: Option<(u64, u32)>,
    /// Seconds left before another request may be made.
    cooldown: f32,
    /// The stones on the ground.
    trail: Trail<sfx::Handle>,
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
}

/// Where a self-check run has got to.
///
/// A four-state machine rather than a pile of booleans because the states are genuinely
/// sequential and the illegal combinations -- watching stones that were never placed, reporting a
/// route twice -- are the ones a diagnostic must not produce. A log that contradicts itself is
/// worse than no log.
#[derive(Debug, Default, PartialEq)]
enum Check {
    /// The config does not ask for one.
    #[default]
    Off,
    /// Armed, and waiting for the planner to answer. Carries seconds spent waiting, against
    /// [`SELF_CHECK_DEADLINE_SECONDS`].
    WaitingForRoute(f32),
    /// Stones are down; watching them on `crate::selfcheck`'s schedule.
    Watching(crate::selfcheck::Schedule),
    /// Everything has been said. The draw side has been told to stand down.
    Done,
}

impl Default for Tick {
    fn default() -> Self {
        Self {
            nav_system: 0,
            area: u32::MAX,
            planner: 0,
            pending: None,
            cooldown: 0.0,
            trail: Trail::default(),
            said_planner: false,
            said_quality: false,
            said_route: false,
            said_no_route: false,
            said_audit_scope: false,
            said_full: false,
            said_snap: false,
            check: Check::Off,
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
        if state.planner != 0 && state.nav_system == nav_system {
            // SAFETY: game thread, and `retire` only sets the byte the next tick acts on.
            unsafe { navquery::retire(nav_system, state.planner) };
        }
        let stranded = state.trail.take_all();
        if !stranded.is_empty() {
            log(format_args!(
                "tick: map changed -- {} marker(s) stranded; their storage is leaked on purpose \
                 rather than freed under an engine that may still hold a pointer to it",
                stranded.len()
            ));
        }
        for handle in stranded {
            core::mem::forget(handle);
        }
        state.nav_system = nav_system;
        state.area = area;
        state.planner = 0;
        state.pending = None;
        state.cooldown = 0.0;
        state.said_planner = false;
    }

    // NOTHING IS WANTED: the overlay is off, you are alone, or the only other player is close
    // enough that `near_suppress_meters` says you already know where they are.
    //
    // Two things follow, and the second is the one that would otherwise be a bug you only notice
    // an hour into a session. The trail goes OUT -- leaving stones burning behind a feature that
    // has stopped running is exactly the litter the stop call was hunted down to avoid. And the
    // stale answer is cleared, so the draw side cannot keep drawing a route to somebody who is no
    // longer being routed to.
    //
    // Nothing is CREATED here either. This crate's contract is that it does not touch the game
    // unless you turn it on, and linking an object into the engine's update list is touching the
    // game, so a session with the overlay off gets a detour that reads two pointers and returns.
    if state.pending.is_none() && WANTED.try_lock().is_ok_and(|slot| slot.is_none()) {
        let out = state.trail.take_all();
        if !out.is_empty() {
            log(format_args!(
                "markers: nothing to route to -- putting {} stone(s) out",
                out.len()
            ));
        }
        for handle in out {
            // SAFETY: game thread, same area they were spawned in, no engine lock held.
            unsafe { handle.extinguish() };
        }
        if let Ok(mut answer) = ANSWER.try_lock() {
            *answer = None;
        }
        state.cooldown = 0.0;
        return;
    }

    if state.planner == 0 {
        // SAFETY: game thread, after the original's list walk has finished.
        let Some(planner) = (unsafe { navquery::create_planner(nav_system) }) else {
            return;
        };
        state.planner = planner;
        if !state.said_planner {
            state.said_planner = true;
            log(format_args!(
                "tick: route planner 0x{planner:016x} linked -- {} object(s) on the navigation \
                 system's list",
                navquery::listed_objects(nav_system).unwrap_or(-1)
            ));
        }
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

    state.cooldown = (state.cooldown - delta.max(0.0)).max(0.0);
    poll_or_request(state);
    lay_markers(state);
    watch_stones(state, delta);
}

/// Sample the self-check's stones on schedule, then take them down.
///
/// Everything here is read-only except the last step, which hands the draw side the "stand down"
/// it needs to stop nominating a target -- and that is what makes the ordinary teardown path
/// run, rather than a second one written for the diagnostic.
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
            log(format_args!(
                "self-check: gave up after {SELF_CHECK_DEADLINE_SECONDS:.0}s without laying a \
                 stone. {}",
                match markers {
                    None =>
                        "No marker settings reached the tick at all -- the overlay may never \
                             have drawn a frame.",
                    Some(markers) if markers.effect_id == 0 =>
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
        return;
    }

    let Check::Watching(schedule) = &mut state.check else {
        return;
    };
    let Some(sample) = schedule.advance(delta) else {
        if !schedule.finished() {
            return;
        }
        // Every sample is in. Tell the draw side to stop asking; the stand-down branch at the
        // top of `work` puts the stones out on the next tick.
        log(format_args!(
            "self-check: done -- standing down, which puts the stones out through the same path \
             the real trail uses"
        ));
        if let Ok(mut done) = SELF_CHECK_DONE.try_lock() {
            *done = true;
        }
        state.check = Check::Done;
        return;
    };
    let placed = state.trail.placed();
    // SAFETY: game thread, in the area the stones were spawned in -- the area-change branch at
    // the top of `work` has already run this tick and would have emptied the trail otherwise.
    let alive = state
        .trail
        .handles()
        .filter(|handle| unsafe { handle.alive() })
        .count();
    log(format_args!(
        "self-check: t={:.1}s{} -- {alive}/{placed} stone(s) still alive{}",
        sample.scheduled,
        if sample.late() {
            format!(
                " (LATE: actually taken at {:.1}s, the frame stalled)",
                sample.actual
            )
        } else {
            String::new()
        },
        match (alive, placed) {
            (0, 0) => " -- nothing was ever placed, so this says nothing about lingering",
            (0, _) => " -- they did NOT linger: a burst, not a marker",
            (a, p) if a == p => " -- all of them; this is what a trail needs",
            _ => " -- some died and some did not, which is not a property of the id alone",
        }
    ));
}

/// Advance the one search this crate ever has in flight.
fn poll_or_request(state: &mut Tick) {
    if let Some((target, waited)) = state.pending {
        // SAFETY: game thread; the planner is ours and has not been retired.
        match unsafe { navquery::poll(state.planner) } {
            Poll::Pending if waited < PENDING_TICK_BUDGET => {
                state.pending = Some((target, waited + 1));
            }
            Poll::Pending => {
                // Not "no way to walk there" -- "the planner stopped being stepped". Different
                // failure, and it must not be reported as the first one.
                log(format_args!(
                    "tick: a search for 0x{target:x} went {PENDING_TICK_BUDGET} ticks without an \
                     answer -- abandoning it"
                ));
                state.pending = None;
                publish(target, None);
            }
            Poll::Failed => {
                state.pending = None;
                if matches!(state.check, Check::WaitingForRoute(_)) {
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
                if !state.said_no_route {
                    state.said_no_route = true;
                    log(format_args!(
                        "tick: no walkable route to 0x{target:x} -- the arrow stands in, which is \
                         the same fallback the Elden Ring crate uses"
                    ));
                }
                publish(target, None);
            }
            Poll::Ready { points, segments } => {
                state.pending = None;
                if matches!(state.check, Check::WaitingForRoute(_)) {
                    // BOTH NUMBERS. A route of forty segments that decodes to two points means
                    // the decoder is wrong; two and two means the walk really is that short. One
                    // number cannot tell those apart, and this is the only place both are in
                    // hand.
                    log(format_args!(
                        "self-check: READY -- {segments} segment(s) decoded to {} point(s), \
                         {:.1} m of path to 0x{target:x}",
                        points.len(),
                        path_length(&points)
                    ));
                }
                if !state.said_route {
                    state.said_route = true;
                    log(format_args!(
                        "tick: first walkable route -- {segments} segment(s), {} point(s) to \
                         0x{target:x}",
                        points.len()
                    ));
                }
                // ASK THE ENGINE WHAT IT THINKS OF THE ROUTE IT JUST GAVE US.
                //
                // Once per request, not once per tick: `Poll::Ready` fires on the pass that
                // clears `state.pending`, so this is the fresh-route edge and not a poll loop.
                //
                // SAFETY: game thread -- this is the `NvNavigationSystem::Update` detour -- and
                // the planner has just reported READY without FAILED, so the route embedded at
                // `+0x48` is the one `poll` decoded a moment ago.
                if let Some(report) = unsafe {
                    navquery::audit(state.planner + ds2_rva::NV_ROUTE_PLANNER_ROUTE_OFFSET)
                } {
                    describe_audit(&report, &mut state.said_audit_scope);
                }
                publish(target, Some(points));
            }
            Poll::Lost => {
                log(format_args!(
                    "tick: the planner became unreadable -- making another"
                ));
                state.pending = None;
                state.planner = 0;
            }
        }
        return;
    }

    if state.cooldown > 0.0 {
        return;
    }
    let Ok(slot) = WANTED.try_lock() else {
        return;
    };
    let Some(wanted) = *slot else {
        return;
    };
    drop(slot);

    // SAFETY: game thread. The allocator underneath the snap is lazily built and unguarded; this
    // is the one place in this crate entitled to touch it.
    let (from, to) = unsafe {
        (
            navquery::snap_reporting(wanted.from),
            navquery::snap_reporting(wanted.to),
        )
    };
    state.cooldown = REPLAN_SECONDS;

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
    if matches!(state.check, Check::WaitingForRoute(_)) && !state.said_snap {
        state.said_snap = true;
        // SAFETY: game thread, and the world exists -- the snaps above just walked it.
        let guard = match (from.id, to.id) {
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
        publish(wanted.target, None);
        return;
    };
    // SAFETY: game thread; the planner is ours, and the request clears both result bits as it
    // sets the pending one, so re-requesting over a search still in flight is safe.
    if unsafe {
        navquery::request(
            state.planner,
            start,
            goal,
            ds2_rva::NV_ROUTE_MAX_COST_LONG_RANGE,
        )
    } {
        state.pending = Some((wanted.target, 0));
    }
}

/// Publish an answer for the draw side.
fn publish(target: u64, route: Option<Vec<[f32; 3]>>) {
    if let Ok(mut slot) = ANSWER.try_lock() {
        *slot = Some(Answer { target, route });
    }
}

/// Put down this tick's share of the trail, and put out what has been walked past.
fn lay_markers(state: &mut Tick) {
    // THE WATCH IS AN EXPERIMENT AND THE TRAIL MUST HOLD STILL FOR IT. The route keeps being
    // re-planned every half second while the stones are being watched, so without this the
    // ordinary trail would go on placing fresh stones -- which are trivially alive -- and
    // retiring old ones as the target moves. Both corrupt the one number the watch exists to
    // produce: "7/7 alive at t=10s" would mean nothing if three of the seven were laid at t=9.
    if matches!(state.check, Check::Watching(_)) {
        return;
    }
    let Ok(slot) = MARKERS.try_lock() else {
        return;
    };
    let Some(markers) = *slot else {
        // No settings yet, or the trail is off. Anything already down stays down: switching the
        // trail off mid-route should not make the stones behind you vanish, and putting them out
        // is a decision for `forget`, not for a missing config read.
        return;
    };
    drop(slot);
    if markers.effect_id == 0 {
        return;
    }
    let Ok(answer) = ANSWER.try_lock() else {
        return;
    };
    let route = match answer.as_ref().and_then(|answer| answer.route.clone()) {
        Some(route) => route,
        None => return,
    };
    drop(answer);

    for handle in state
        .trail
        .retire_behind(&route, markers.spacing.keep_behind_meters)
    {
        // SAFETY: game thread, same area as the spawn, no engine lock held.
        unsafe { handle.extinguish() };
    }

    let Some(system) = sfx::system() else {
        return;
    };
    if !state.said_quality {
        state.said_quality = true;
        let level = sfx::quality(system).unwrap_or(u32::MAX);
        log(format_args!(
            "markers: effect {} on KatanaSfxSystem 0x{system:016x}, quality {level}{}",
            markers.effect_id,
            if level >= ds2_rva::KATANA_SFX_QUALITY_DROP_THRESHOLD {
                " -- AT OR ABOVE THE THRESHOLD AT WHICH THE ENGINE SILENTLY DISCARDS SPAWNS"
            } else {
                ""
            }
        ));
    }

    let planned = state.trail.plan(&route, markers.spacing);
    if planned.is_empty() && !state.said_full && state.trail.placed() >= markers.spacing.max_markers
    {
        state.said_full = true;
        log(format_args!(
            "markers: trail at its budget -- {} stone(s) down, and no more will be placed until \
             you walk past some. Raise `max_markers` if the trail is stopping short.",
            state.trail.placed()
        ));
    }
    let checking = matches!(state.check, Check::WaitingForRoute(_));
    let mut attempted = 0usize;
    let mut took = 0usize;
    for (index, at) in planned.iter().enumerate() {
        // The direction a stone faces is the direction the trail runs at that point. The next
        // planned stone is the best available "onwards"; the last one borrows the one before it,
        // so the end of a trail does not suddenly point at nothing.
        let ahead = planned
            .get(index + 1)
            .or_else(|| index.checked_sub(1).and_then(|back| planned.get(back)));
        let direction = ahead.map_or([0.0, 1.0, 0.0], |next| crate::geometry::sub(*next, *at));
        attempted += 1;
        // ONE COLOUR PER STONE WHILE CHECKING. The Prism Stone has seven ids, one per colour, and
        // sweeping them along the line answers in a single run which of the seven actually
        // appear -- for the cost of a modulo. The real trail uses the configured id throughout,
        // because a trail that changes colour as it goes reads as several trails.
        let id = if checking {
            ds2_rva::PRISM_STONE_SFX_IDS
                [state.trail.placed().wrapping_add(index) % ds2_rva::PRISM_STONE_SFX_IDS.len()]
        } else {
            markers.effect_id
        };
        if checking {
            // SAFETY: game thread, mid-simulation, no engine lock held.
            let attempt = unsafe { sfx::spawn_reporting(system, id, *at, direction) };
            log(format_args!("self-check: {}", attempt.describe(id)));
            if let Some(handle) = attempt.handle {
                took += 1;
                state.trail.remember(*at, handle);
            }
            continue;
        }
        // SAFETY: game thread, mid-simulation, no engine lock held -- which is the whole reason
        // this call is here and not in the `Present` detour.
        if let Some(handle) = unsafe { sfx::spawn(system, id, *at, direction) } {
            took += 1;
            state.trail.remember(*at, handle);
        }
    }

    // THE SELF-CHECK STOPS PLACING ONCE THE TRAIL IS AS LONG AS IT IS GOING TO GET, and starts
    // watching instead. "As long as it is going to get" is a pass that placed nothing while
    // something is down -- either the budget is full or every candidate is already taken, and
    // both mean the laying is over.
    if checking && attempted > 0 {
        log(format_args!(
            "self-check: laid {took}/{attempted} stone(s) this pass -- {} down of at most {}",
            state.trail.placed(),
            markers.spacing.max_markers
        ));
    }
    if checking && attempted == 0 && state.trail.placed() > 0 {
        log(format_args!(
            "self-check: trail complete at {} stone(s); watching them at t={:?}s",
            state.trail.placed(),
            crate::selfcheck::SAMPLE_SECONDS
        ));
        state.check = Check::Watching(crate::selfcheck::Schedule::default());
    }
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
    unsafe {
        ds2_game_base::mem::safe_read_u32(
            map_manager + ds2_rva::MAP_MANAGER_PLAYER_MAP_INDEX_OFFSET,
        )
    }
}
