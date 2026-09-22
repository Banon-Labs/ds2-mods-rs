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
    said_full: bool,
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
            said_full: false,
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

    // THE MAP CHANGED, AND THE HANDLES DID NOT SURVIVE IT. Every stone's control block points at
    // an FX node in a pool the old area owned. Extinguishing one now would read through a freed
    // manager, so the handles are DROPPED rather than put out -- which is safe by construction,
    // because `sfx::Handle` has no `Drop` that calls the engine. The effects went with the map.
    //
    // The planner is a different case: the `NvNavigationSystem` is the same object across areas,
    // so the planner is still linked and still ours, but its world pointer was bound at creation
    // from an area that is gone. Retire it properly and make another.
    let area = current_area();
    if state.nav_system != nav_system || state.area != area {
        if state.planner != 0 && state.nav_system == nav_system {
            // SAFETY: game thread, and `retire` only sets the byte the next tick acts on.
            unsafe { navquery::retire(nav_system, state.planner) };
        }
        let dropped = state.trail.take_all();
        if !dropped.is_empty() {
            log(format_args!(
                "tick: map changed -- {} marker(s) went with it",
                dropped.len()
            ));
        }
        drop(dropped);
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

    state.cooldown = (state.cooldown - delta.max(0.0)).max(0.0);
    poll_or_request(state);
    lay_markers(state);
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
                if !state.said_no_route {
                    state.said_no_route = true;
                    log(format_args!(
                        "tick: no walkable route to 0x{target:x} -- the arrow stands in, which is \
                         the same fallback the Elden Ring crate uses"
                    ));
                }
                publish(target, None);
            }
            Poll::Ready(points) => {
                state.pending = None;
                if !state.said_route {
                    state.said_route = true;
                    log(format_args!(
                        "tick: first walkable route -- {} point(s) to 0x{target:x}",
                        points.len()
                    ));
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
    let (start, goal) = unsafe { (navquery::snap(wanted.from), navquery::snap(wanted.to)) };
    state.cooldown = REPLAN_SECONDS;
    let (Some(start), Some(goal)) = (start, goal) else {
        // One of them is off the navmesh -- in a lift shaft, mid-fall, or beyond the twenty-metre
        // snap radius. A complete answer, and the arrow's cue.
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
    for (index, at) in planned.iter().enumerate() {
        // The direction a stone faces is the direction the trail runs at that point. The next
        // planned stone is the best available "onwards"; the last one borrows the one before it,
        // so the end of a trail does not suddenly point at nothing.
        let ahead = planned
            .get(index + 1)
            .or_else(|| index.checked_sub(1).and_then(|back| planned.get(back)));
        let direction = ahead.map_or([0.0, 1.0, 0.0], |next| crate::geometry::sub(*next, *at));
        // SAFETY: game thread, mid-simulation, no engine lock held -- which is the whole reason
        // this call is here and not in the `Present` detour.
        if let Some(handle) = unsafe { sfx::spawn(system, markers.effect_id, *at, direction) } {
            state.trail.remember(*at, handle);
        }
    }
}

/// The map area the world is currently in, or [`u32::MAX`] when there is no world.
///
/// Keyed on rather than on the `NvNavigationSystem` pointer because that pointer is a singleton
/// that outlives a map change while the FX node pool underneath every marker does not.
fn current_area() -> u32 {
    let Some(manager) = navquery::game_manager() else {
        return u32::MAX;
    };
    // SAFETY: a live `GameManagerImp`; both reads refuse an unmapped page.
    unsafe {
        safe_read(manager + ds2_rva::GAME_MANAGER_MAP_MANAGER_OFFSET)
            .and_then(|map| safe_read_area(map))
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

/// The area word on a `MapManager`.
///
/// # Safety
///
/// `map_manager` must be a live `MapManager`; the reader itself faults safely.
unsafe fn safe_read_area(map_manager: usize) -> Option<u32> {
    unsafe { ds2_game_base::mem::safe_read_u32(map_manager + ds2_rva::MAP_MANAGER_AREA_OFFSET) }
}
