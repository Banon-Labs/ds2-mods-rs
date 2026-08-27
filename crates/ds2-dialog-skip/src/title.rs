//! The things that still stop the title flow once the notice boxes are gone.
//!
//! They are different in kind, and the difference decides the cut in each case.
//!
//! # The title screen
//!
//! Three separate stops, and the order they were understood in matters because the first two look
//! like the same thing and are not:
//!
//! 1. **The wait.** Phase 1 of `FeSubStateTitleMain::v3` will not look for a press until the scene
//!    reports its settled sequence is up, so forcing the press poll alone skips nothing visible.
//! 2. **The scene's intro sequence.** `enter` starts sequence `0x66` and nothing in the phase
//!    machine stops it. Putting the scene into its settled state (`0x67`) instead is what makes the
//!    menu usable **as soon as its data is available rather than paced by an animation**.
//! 3. **The activation flourish**, phases 2 and 3, which run after the press is taken.
//!
//! (2) and (3) share one detour on `FeSubStateTitleMain::v3` and are separately switchable, because
//! they mutate different state and a boot failure has to be attributable to one of them.
//!
//! # Press any button
//!
//! `FeSubStateTitleMain::v3`'s phase-1 branch ticks the title scene, waits for the title sequence
//! to be up, and then asks [`ds2_rva::FE_TITLE_MAIN_PRESS_ANY_BUTTON`] whether a button was
//! pressed. That poll has **exactly one caller in the whole image**, so forcing it true is a
//! change to one gate rather than to input handling. The sequence gate is left alone, and the
//! game's own phase-1 body -- which is what prepares the top menu -- runs in full. Forcing the
//! substate's terminal phase instead would skip that setup, which is why the phase is untouched
//! here even though `ds2-intro-skip` writes phases for the boot screens.
//!
//! # Process windows
//!
//! These are the "please wait" boxes that resolve on their own. **They wrap real asynchronous
//! work** -- a network check, a server login, a system-data save, a profile load -- started by
//! `enter` through vtable slot 8 and waited on by `update` through slot 10. Suppressing one would
//! skip the wait, not just the window, so nothing here touches the phase or the slots.
//!
//! What it does touch is [`ds2_rva::FE_PROCESS_WINDOW_MIN_DURATION_OFFSET`], a minimum display
//! time the update enforces *before* it will even ask whether the work is done. Zeroing it removes
//! that floor and nothing else: the window still stays up for exactly as long as the operation
//! really takes, and it cannot outrun it. A window that was lingering after its work finished
//! stops lingering; a window whose work is genuinely slow is unaffected.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ds2_game_base::mem::{
    game_module_base, safe_read_f32, safe_read_i32, safe_read_u8, safe_read_usize,
};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// Trampoline back to the original process-window `enter`.
static PROCESS_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Trampoline back to the ORIGINAL title-sequence gate.
///
/// The detour replaces that gate outright -- it reports a press-ready scene on the first frame so
/// the flow does not pace itself to the logo animation. That is the whole point of the feature, and
/// it is also what puts a menu on screen while the scene is still animating in. Keeping the
/// trampoline means the honest answer is still *askable*: [`title_sequence_settled`] calls it, and
/// nothing else does, so forcing the flow and knowing whether the animation has finished stop being
/// the same question.
static SEQUENCE_GATE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Trampoline back to `FeSubStateTitleMain`'s original update.
static TITLE_MAIN_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The live module base, resolved at install for the title-scene lookup.
static MODULE_BASE_TITLE: AtomicUsize = AtomicUsize::new(0);

/// Set once the title scene has been pushed to its settled state.
static IDLE_FORCED: AtomicUsize = AtomicUsize::new(0);

/// Whether the shared `FeSubStateTitleMain::v3` detour should settle the scene.
static SETTLE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether that same detour should write the terminal phase over the activation flourish.
static ANIMATION_ENABLED: AtomicBool = AtomicBool::new(false);

/// How many activation animations were cut short.
static ANIMATIONS_SKIPPED: AtomicUsize = AtomicUsize::new(0);

/// How many process windows were hidden outright.
static HIDDEN: AtomicUsize = AtomicUsize::new(0);

/// How many times the title-sequence gate has been forced.
static SEQUENCE_GATES: AtomicUsize = AtomicUsize::new(0);

/// How many times the press gate has been forced. Reported so "the title screen never came up"
/// and "the skip never fired" cannot be confused.
static PRESSES: AtomicUsize = AtomicUsize::new(0);

/// How many process windows have had their minimum duration cleared.
static SHORTENED: AtomicUsize = AtomicUsize::new(0);

/// `void update(this, float delta)` -- `this` in RCX, the frame delta in XMM1.
///
/// The float is carried for the same reason the dialog update's is: `FeSubStateTitleMain::v3`
/// accumulates a delta into its idle timer, and a detour that dropped XMM1 would feed it garbage.
type UpdateFn = unsafe extern "system" fn(*mut u8, f32);

/// Report that the title sequence is up, always.
///
/// This is the gate the press poll sits behind: `FeSubStateTitleMain::v3`'s phase 1 will not even
/// look for a press until `0x1400f37f0` says the scene's currently-playing sequence is `0x67`, the
/// idle "press any button" state. **That wait IS the title-logo animation and the prompt animating
/// in** -- forcing the press poll alone skips nothing visible, because the poll is never reached
/// until the animation has finished on its own.
///
/// Like the press poll it has **exactly one caller in the whole image**, at `0x1400fee5b` inside
/// that same update, so this reaches one gate rather than the sequence system.
///
/// # Cutting an animation short is something the press path already does
///
/// The body this unblocks opens `lea rcx,[rbx+0x18]; call 0x140afe8a0` -- it finishes the running
/// sequence before starting the transition. So the game already handles being told to proceed
/// while a sequence is mid-flight; this just makes that happen on the first frame instead of after
/// the animation has played out.
///
/// The original is never called: it reads scene state and returns a verdict, so running it and
/// discarding the answer would be the same as not running it.
unsafe extern "system" fn detour_sequence_gate(scene: *mut u8) -> u8 {
    let _ = scene;
    let total = SEQUENCE_GATES.fetch_add(1, Ordering::Relaxed) + 1;
    if total <= 3 {
        log(format_args!(
            "{LOG_PREFIX} forced screen=title-main gate=title-sequence total={total}"
        ));
    }
    1
}

/// `void enter(this)` -- `this` in RCX. Same shape as the dialog `enter`.
///
/// There is deliberately no matching alias for the press gate's signature: nothing ever calls
/// through to the original, so there is no trampoline to type. The detour's own declaration --
/// `unsafe extern "system" fn(*mut u8) -> u8` -- is the only place that shape is needed. `u8`
/// rather than `bool` because the caller tests `al` for non-zero, and a Rust `bool` over FFI
/// carries a validity requirement this code has no reason to take on.
type EnterFn = unsafe extern "system" fn(*mut u8);

/// Report a button press, always.
///
/// THE ORIGINAL IS NEVER CALLED, and there is nothing to preserve by calling it: it reads global
/// input state and returns a verdict, touching nothing. Running it and discarding the answer would
/// be the same as not running it.
unsafe extern "system" fn detour_press_gate(_ignored: *mut u8) -> u8 {
    let total = PRESSES.fetch_add(1, Ordering::Relaxed) + 1;
    // Logged only for the first few, not every frame: the gate is polled once per frame while the
    // title screen is up, and it is consulted again on later visits to the title.
    if total <= 3 {
        log(format_args!(
            "{LOG_PREFIX} pressed screen=title-main gate=press-any-button total={total}"
        ));
    }
    1
}

/// Run the original `enter`, then remove the window's artificial minimum display time.
///
/// Ordering matters here in the opposite direction from the dialog detour: the original must run
/// FIRST, because it is what starts the work and writes the phase and the duration this then
/// edits. Zeroing beforehand would be overwritten.
unsafe extern "system" fn detour_process_enter(this: *mut u8) {
    let trampoline = PROCESS_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is
        // the one all six sharing classes implement.
        let original: EnterFn = unsafe { std::mem::transmute::<usize, EnterFn>(trampoline) };
        unsafe { original(this) };
    }
    if this.is_null() {
        return;
    }
    let object = this as usize;

    // Phase 1 is the only phase that reads the minimum duration. `enter` sets phase 3 outright
    // when slot 8 reported there was nothing to do -- no window was shown, and there is nothing to
    // shorten.
    if unsafe { safe_read_i32(object + ds2_rva::FE_PROCESS_WINDOW_PHASE_OFFSET) }
        != Some(ds2_rva::FE_PROCESS_WINDOW_PHASE_SHOWING)
    {
        return;
    }
    // Written as a positive test rather than a negated one so a NaN duration falls through to
    // "leave it alone" instead of to "write zero over it".
    let Some(previous) =
        (unsafe { safe_read_f32(object + ds2_rva::FE_PROCESS_WINDOW_MIN_DURATION_OFFSET) })
            .filter(|previous| *previous > 0.0)
    else {
        return;
    };

    // SAFETY: the object was just read at two of its own offsets through fault-tolerant reads that
    // both succeeded, so it is mapped and at least as large as the field the game's own
    // constructor writes here.
    unsafe {
        this.add(ds2_rva::FE_PROCESS_WINDOW_MIN_DURATION_OFFSET)
            .cast::<f32>()
            .write(0.0)
    };

    let total = SHORTENED.fetch_add(1, Ordering::Relaxed) + 1;
    // The previous value is logged because it is the evidence for whether this was worth doing at
    // all: a floor of 0.5s that the work beats every time is exactly the lingering the player sees,
    // and a floor already shorter than the work would show up here as a number that explains why
    // nothing visibly changed.
    log(format_args!(
        "{LOG_PREFIX} shortened screen=process-window kind={} min-duration={previous:.3}->0 \
         total={total}",
        unsafe { safe_read_i32(object + ds2_rva::FE_PROCESS_WINDOW_KIND_OFFSET) }.unwrap_or(-1),
    ));
}

/// The one function that draws a "please wait" box. Four register arguments, no stack arguments.
///
/// All four are carried even though the body appears to use only RCX: it keeps RCX and forwards
/// RDX, R8 and R9 untouched into `0x1405105f0`. They are `u64` rather than narrower types so that
/// forwarding reproduces even the upper bits the callers leave undefined -- six call sites set only
/// `r9b` and `r8d`, and one sets only `r8b` and `r9d`.
type ShowProcessWindowFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u64) -> i32;

/// Trampoline back to the original `show_process_window`.
static SHOW_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Address of the byte that is nonzero while `FeOperatorTitle` is running. Resolved once at
/// install so the detour does not repeat the module lookup on a drawing path.
static TITLE_ACTIVE_FLAG: AtomicUsize = AtomicUsize::new(0);

/// Draw nothing, but only while the title flow is running.
///
/// **THE GATE IS THE GAME'S OWN FLAG**, not a timer and not a notion of "still booting" invented
/// here. `FeOperatorTitle` sets [`ds2_rva::FE_OPERATOR_TITLE_ACTIVE`] on setup and clears it on
/// teardown, and the game reads it itself. Hiding every process window unconditionally would take
/// the in-game "Saving..." indicator with it, which a player is entitled to see; gating on this
/// keeps the change to the boot sequence.
///
/// Returning `0` is the function's own answer when there is no window manager to draw on
/// (`xor eax,eax; ret`), and no caller uses the return value -- all seven ignore EAX and write
/// their own phase field next.
unsafe extern "system" fn detour_show_process_window(
    ui: *mut c_void,
    caption: *mut c_void,
    arg3: u64,
    arg4: u64,
) -> i32 {
    let flag = TITLE_ACTIVE_FLAG.load(Ordering::Acquire);
    let in_title_flow =
        flag != 0 && unsafe { safe_read_u8(flag) }.is_some_and(|active| active != 0);
    if in_title_flow {
        let total = HIDDEN.fetch_add(1, Ordering::Relaxed) + 1;
        // Only the first few: several of these open during one boot and the count carries the rest.
        if total <= 8 {
            log(format_args!(
                "{LOG_PREFIX} hidden screen=process-window total={total}"
            ));
        }
        return 0;
    }
    let trampoline = SHOW_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return 0;
    }
    // SAFETY: MinHook published this trampoline for this site, and all four registers are forwarded
    // exactly as received.
    let original: ShowProcessWindowFn =
        unsafe { std::mem::transmute::<usize, ShowProcessWindowFn>(trampoline) };
    unsafe { original(ui, caption, arg3, arg4) }
}

/// `void play_settled(scene)` -- `FeSceneTitle` in RCX, the only argument its own callers pass.
type PlaySettledFn = unsafe extern "system" fn(*mut u8);

/// Put the title scene straight into its settled state, once.
///
/// `FeSubStateTitleMain::v1` starts sequence `0x66` on the scene and nothing in the phase machine
/// stops it. Forcing the press gate alone makes the gate report a state the scene is not in, so the
/// flow advances while that sequence keeps running underneath. This plays `0x67` -- the settled
/// state the gate is actually waiting to observe -- so the scene is *put into* that state rather
/// than skipped past it.
///
/// The effect that matters, and is confirmed in-game: **the menu becomes usable as soon as its data
/// is available instead of being paced by an animation.** A separate open question is that the
/// title text is still seen animating; see `docs/DS2-TITLE-FLOW.md`. That is a question about the
/// remaining animation, not a reason to drop this call.
///
/// # Safety
///
/// Reads the scene through fault-tolerant reads and calls a game function with the same single
/// argument its own call sites pass. Runs at most once per process.
unsafe fn force_title_settled(base: usize) {
    if IDLE_FORCED.swap(1, Ordering::AcqRel) != 0 {
        return;
    }
    let Some(globals) = (unsafe { safe_read_usize(base + ds2_rva::FE_TITLE_GLOBALS as usize) })
    else {
        return;
    };
    let Some(scene) = (unsafe { safe_read_usize(globals + ds2_rva::FE_TITLE_SCENE_OFFSET) }) else {
        return;
    };
    if scene == 0 {
        return;
    }
    // SAFETY: resolved from the live module base, called with the scene pointer its own call sites
    // pass, and guarded to run once.
    let play_settled: PlaySettledFn = unsafe {
        std::mem::transmute::<usize, PlaySettledFn>(
            base + ds2_rva::FE_SCENE_TITLE_PLAY_IDLE as usize,
        )
    };
    unsafe { play_settled(scene as *mut u8) };
    log(format_args!(
        "{LOG_PREFIX} settled screen=title-main scene=0x{scene:x} sequence=0x67"
    ));
}

/// Run the title screen's update, then cut the activation animation short if that is all that is
/// left.
///
/// Phase 1 is where every decision lives: it waits for the press and, on one, runs the whole
/// top-menu setup. Phases 2 and 3 are the flourish afterwards -- phase 2 is a pure wait that does
/// nothing but advance, and phase 3 waits for the same sequence and then advances. So observing
/// EITHER of them after the original has run means the setup is already done and only the animation
/// remains, which is what makes writing the terminal phase here a skip of the animation rather than
/// of anything load-bearing.
///
/// The phase is read AFTER the original rather than before, deliberately: before the call it is
/// still 1 and says nothing about whether the press was taken this frame.
unsafe extern "system" fn detour_title_main_update(this: *mut u8, delta: f32) {
    let trampoline = TITLE_MAIN_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site, and `delta` is forwarded so the
        // idle timer that decides the attract-movie timeout keeps accumulating real frame time.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
        unsafe { original(this, delta) };
    }
    // The title screen updates every frame from well before the menu exists, which makes it the
    // one clock the menu's own log lines can be stamped against. Nothing else here reads it.
    crate::menu::tick_frame();
    if this.is_null() {
        return;
    }
    // The sequence this replaces was started by `enter`, so the earliest update is the first
    // opportunity to put the scene into its settled state instead.
    let base = MODULE_BASE_TITLE.load(Ordering::Acquire);
    if base != 0 && SETTLE_ENABLED.load(Ordering::Acquire) {
        unsafe { force_title_settled(base) };
    }
    if !ANIMATION_ENABLED.load(Ordering::Acquire) {
        return;
    }
    let object = this as usize;
    let phase = unsafe { safe_read_i32(object + ds2_rva::FE_TITLE_MAIN_PHASE_OFFSET) };
    if phase != Some(ds2_rva::FE_TITLE_MAIN_PHASE_ANIMATING)
        && phase != Some(ds2_rva::FE_TITLE_MAIN_PHASE_ANIMATING_LATE)
    {
        return;
    }
    // PHASE 3 ALSO CALLS `0x140afe8a0` ON THE `+0x38` HANDLE before writing this phase, and an
    // earlier version of this detour reproduced that call on the theory that it snaps a running
    // sequence to its end. IT DOES NOT, and the call was removed rather than left in on the chance
    // it helped. `0x140afe8a0` tail-calls `0x1409d5610`, whose body compares `[handle]` against a
    // global and, on mismatch, builds a record tagged `0x4d4f4d53` ("SMOM") and reports it --
    // handle validation or telemetry, not playback control. In a live run it returned success and
    // the title text animated in exactly as before, which is what prompted reading the body instead
    // of inferring the meaning from where it is called.
    //
    // SAFETY: the phase was just read from this object without faulting, and this writes the same
    // field the original writes at `0x1400fedf7`.
    unsafe {
        this.add(ds2_rva::FE_TITLE_MAIN_PHASE_OFFSET)
            .cast::<i32>()
            .write(ds2_rva::FE_TITLE_MAIN_PHASE_DONE)
    };
    let total = ANIMATIONS_SKIPPED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} advanced screen=title-main phase={}->{} total={total}",
        phase.unwrap_or(-1),
        ds2_rva::FE_TITLE_MAIN_PHASE_DONE
    ));
}

/// What [`install`] managed to do. Each hook is independent; one failing does not stop the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The press-any-button gate is now forced.
    pub press_any_button: bool,
    /// Process windows now close as soon as their work is done.
    pub process_windows: bool,
    /// The title screen's activation animation is now cut short.
    pub title_animation: bool,
    /// Process windows are now not drawn at all while the title flow is running.
    pub hide_process_windows: bool,
    /// The wait for the title logo/prompt animation is now bypassed.
    pub title_sequence_gate: bool,
    /// The title scene is now put into its settled state on the first update.
    pub title_settle: bool,
    /// How many of the two floor sites were hooked.
    ///
    /// A count rather than a bool so a partial install is visible. The two are not worth the same
    /// today -- `0x44`'s floor measured at zero -- so *which* one installed is the question, and
    /// the `hooked floor=` lines are what answer it.
    pub substate_floors: usize,
}

// ============================================================================================
// THE ONE-SECOND FLOORS (`ds2-mods-rs-wxl`). Worth 875ms of a 6.8s boot, measured -- 13%.
// That is HALF of the ~1.86s this comment first predicted. The missing half is at the bottom.
//
// `FeSubStateTitleSteamLoadSystemData` and `FeSubStateTitleInformation` each keep their own
// elapsed timer, accumulate the frame delta into it, and refuse to advance until it passes
// `[0x1410ac698]` -- which is `1.0f`. Run 6 caught both by watching their phase fields: 0x05 sat
// in phase 4 for 879ms after `SaveLoadSystem` had already gone idle, and 0x44 sat in phase 2 for
// 985ms.
//
// THE CONSTANT IS NOT THE FIX. `0x1410ac698` carries 2042 RIP-relative references from 1548
// functions -- it is MSVC's pooled `1.0f` for the whole image, not a tunable belonging to these
// two. What is patched here is each substate's OWN elapsed field, set at `enter` so the game's own
// `comiss` passes the first time that branch is reached. The comparison and the transition both
// stay the game's.
//
// AND ONLY THE FLOOR GOES. `0x44`'s phase 2 tests two things in order -- the download job first
// (`call [r14->vtable+0x28]; test al,al; jne return`), the timer second -- so a satisfied timer
// cannot outrun an unfinished job. `0x05`'s phase 4 is only reachable once the storage service has
// already reported idle. Neither skips work; both skip waiting.
//
// AND THAT ORDERING IS WHY THE SAVING IS 875ms AND NOT 1.86s. `0x05`'s phase 4 fell from 879ms to
// 6ms. `0x44`'s fell from 985ms to 981.7ms -- nothing -- because the job, not the timer, was
// always what held it (`ds2-mods-rs-umo`). ITS ENTRY STAYS ANYWAY, and not out of sentiment: the
// job takes ~982ms against a 1000ms floor, so the two are within 20ms of each other, and the day
// `umo` shortens that job this floor is what binds instead. Measured harmless until then --
// `was=0` on every run, and the original `enter` runs first.
// ============================================================================================

/// A substate whose `enter` should leave its elapsed timer already past the floor.
struct Floor {
    name: &'static str,
    enter_rva: u32,
    /// Offset of the `f32` that class's own branch compares. Travels WITH the address because the
    /// two are only correct as a pair -- `0x18` on one, `0x5a24` on the other -- and pairing them
    /// makes a mismatched combination something to construct on purpose rather than fall into.
    elapsed_offset: usize,
}

const FLOORS: [Floor; 2] = [
    Floor {
        name: "steam-load-system-data",
        enter_rva: ds2_rva::FE_SUBSTATE_STEAM_LOAD_SYSTEM_DATA_ENTER,
        elapsed_offset: ds2_rva::FE_SUBSTATE_STEAM_LOAD_SYSTEM_DATA_ELAPSED_OFFSET,
    },
    Floor {
        name: "title-information",
        enter_rva: ds2_rva::FE_SUBSTATE_TITLE_INFORMATION_ENTER,
        elapsed_offset: ds2_rva::FE_SUBSTATE_TITLE_INFORMATION_ELAPSED_OFFSET,
    },
];

static FLOOR_TRAMPOLINES: [AtomicUsize; FLOORS.len()] =
    [const { AtomicUsize::new(0) }; FLOORS.len()];
static FLOOR_FIRED: [AtomicUsize; FLOORS.len()] = [const { AtomicUsize::new(0) }; FLOORS.len()];

/// A substate `enter`: `void enter(this)`, `this` in RCX.
type FloorEnterFn = unsafe extern "system" fn(*mut u8);

/// Run the original `enter`, then set the elapsed timer past the floor.
///
/// AFTER the original, not before. Both classes touch this field around entry -- `0x05`'s
/// constructor zeroes it and `0x44` resets it to zero on the way out of phase 2 -- so a write
/// placed first could be overwritten with nothing to say it had been. A floor still in place looks
/// exactly like a floor that was never hooked, which is the failure this ordering avoids.
///
/// # Safety
///
/// `this` is the substate being entered; `index` must be a valid index into [`FLOORS`].
unsafe fn lift_floor(index: usize, this: *mut u8) {
    let floor = &FLOORS[index];
    let trampoline = FLOOR_TRAMPOLINES[index].load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site, and the signature is the one
        // every substate `enter` implements.
        let original: FloorEnterFn =
            unsafe { std::mem::transmute::<usize, FloorEnterFn>(trampoline) };
        unsafe { original(this) };
    }
    if this.is_null() {
        return;
    }
    // What the original left, read before it is overwritten. If this is ever not 0 the field is
    // not what this crate believes it is, and the log is where that shows up -- rather than in a
    // boot that mysteriously failed to get faster.
    // SAFETY: the original has just run against this pointer, so the object is live and at least
    // as large as the field the game itself writes at this offset.
    let left_by_original = unsafe { this.add(floor.elapsed_offset).cast::<f32>().read() };
    // SAFETY: same object, same field, read successfully immediately above.
    unsafe {
        this.add(floor.elapsed_offset)
            .cast::<f32>()
            .write(ds2_rva::FE_SUBSTATE_FLOOR_ELAPSED);
    }
    let n = FLOOR_FIRED[index].fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} floor-lifted screen={} elapsed-offset=0x{:x} was={left_by_original} now={} \
         count={n}",
        floor.name,
        floor.elapsed_offset,
        ds2_rva::FE_SUBSTATE_FLOOR_ELAPSED,
    ));
}

// One detour per site: MinHook gives a detour no way to learn which site reached it.
unsafe extern "system" fn detour_floor_steam_load(this: *mut u8) {
    unsafe { lift_floor(0, this) }
}
unsafe extern "system" fn detour_floor_information(this: *mut u8) {
    unsafe { lift_floor(1, this) }
}

const FLOOR_DETOURS: [FloorEnterFn; FLOORS.len()] =
    [detour_floor_steam_load, detour_floor_information];

/// What the caller asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Force the press-any-button poll.
    pub press_any_button: bool,
    /// Hook process windows at all.
    pub process_windows: bool,
    /// Hide process windows outright rather than only clearing their minimum display time.
    pub hide_process_windows: bool,
    /// Cut the title screen's activation animation short once its setup has run.
    pub title_animation: bool,
    /// Force the gate that waits for the title logo/prompt animation before a press is accepted.
    pub title_sequence_gate: bool,
    /// Put the title scene into its settled state instead of letting its intro sequence play out.
    ///
    /// Shares the `FeSubStateTitleMain::v3` detour with [`Self::title_animation`], so the hook goes
    /// in when EITHER is asked for. Kept as its own key regardless: it mutates game state that the
    /// other does not touch, and every switch in this repo exists so a boot failure is attributable
    /// to one line.
    pub title_settle: bool,
    /// Remove the one-second floors on `0x05 SteamLoadSystemData` and `0x44 Information`.
    ///
    /// Measured at 879 ms and 985 ms, both spent after the work they were waiting for had already
    /// finished. Its own key rather than riding [`Self::process_windows`] because it is a
    /// different mechanism on different classes: neither of those two derives
    /// `FeSubStateProcessWindowBase`, so neither has a `min_duration` field to zero -- they inline
    /// the `1.0f` threshold instead, which is why the existing fix never reached them.
    pub substate_floors: bool,
}

/// Install whichever of the two title-flow skips were asked for.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`) and before the title flow reaches these substates.
pub unsafe fn install(request: Request) -> Outcome {
    let mut outcome = Outcome {
        press_any_button: false,
        process_windows: false,
        title_animation: false,
        title_settle: false,
        hide_process_windows: false,
        title_sequence_gate: false,
        substate_floors: 0,
    };
    let base = match game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} title-install-failed stage=module-base error={error}"
            ));
            return outcome;
        }
    };
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} title-install-failed stage=MH_Initialize status={status:?}"
        ));
        return outcome;
    }

    if request.title_sequence_gate {
        let site = base + ds2_rva::FE_TITLE_MAIN_SEQUENCE_GATE as usize;
        // No trampoline: the detour replaces the gate outright rather than fronting it.
        match unsafe { MhHook::new(site as *mut c_void, detour_sequence_gate as *mut c_void) } {
            Ok(hook) => {
                // Stored even though the detour never calls it: `title_sequence_settled` asks the
                // original whether the logo has finished, which is a different question from the
                // one the detour answers.
                SEQUENCE_GATE_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
                let status = unsafe { MH_EnableHook(site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    outcome.title_sequence_gate = true;
                    log(format_args!(
                        "{LOG_PREFIX} hooked gate=title-sequence rva=0x{:08x} va=0x{site:016x}",
                        ds2_rva::FE_TITLE_MAIN_SEQUENCE_GATE
                    ));
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} hook-failed gate=title-sequence va=0x{site:016x} \
                         stage=MH_EnableHook status={status:?}"
                    ));
                }
            }
            Err(status) => log(format_args!(
                "{LOG_PREFIX} hook-failed gate=title-sequence va=0x{site:016x} \
                 stage=MH_CreateHook status={status:?}"
            )),
        }
    }

    if request.substate_floors {
        for (index, floor) in FLOORS.iter().enumerate() {
            let site = base + floor.enter_rva as usize;
            let hook = match unsafe {
                MhHook::new(site as *mut c_void, FLOOR_DETOURS[index] as *mut c_void)
            } {
                Ok(hook) => hook,
                Err(status) => {
                    log(format_args!(
                        "{LOG_PREFIX} hook-failed floor={} va=0x{site:016x} stage=MH_CreateHook \
                         status={status:?}",
                        floor.name
                    ));
                    continue;
                }
            };
            // Published BEFORE the site is patched: a detour that read a zero here would skip the
            // original, and for an `enter` that means the substate never initialises at all.
            FLOOR_TRAMPOLINES[index].store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(site as *mut c_void) };
            if status != MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed floor={} va=0x{site:016x} stage=MH_EnableHook \
                     status={status:?}",
                    floor.name
                ));
                continue;
            }
            outcome.substate_floors += 1;
            log(format_args!(
                "{LOG_PREFIX} hooked floor={} rva=0x{:08x} va=0x{site:016x} elapsed-offset=0x{:x}",
                floor.name, floor.enter_rva, floor.elapsed_offset
            ));
        }
    }

    if request.press_any_button {
        let site = base + ds2_rva::FE_TITLE_MAIN_PRESS_ANY_BUTTON as usize;
        // No trampoline is stored: the detour replaces the poll outright rather than fronting it.
        match unsafe { MhHook::new(site as *mut c_void, detour_press_gate as *mut c_void) } {
            Ok(_) => {
                let status = unsafe { MH_EnableHook(site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    outcome.press_any_button = true;
                    log(format_args!(
                        "{LOG_PREFIX} hooked gate=press-any-button rva=0x{:08x} va=0x{site:016x}",
                        ds2_rva::FE_TITLE_MAIN_PRESS_ANY_BUTTON
                    ));
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} hook-failed gate=press-any-button va=0x{site:016x} \
                         stage=MH_EnableHook status={status:?}"
                    ));
                }
            }
            Err(status) => log(format_args!(
                "{LOG_PREFIX} hook-failed gate=press-any-button va=0x{site:016x} \
                 stage=MH_CreateHook status={status:?}"
            )),
        }
    }

    if request.process_windows {
        let site = base + ds2_rva::FE_PROCESS_WINDOW_ENTER as usize;
        match unsafe { MhHook::new(site as *mut c_void, detour_process_enter as *mut c_void) } {
            Ok(hook) => {
                // Published BEFORE the site is patched, so a detour cannot observe a zero and skip
                // the original -- which here would mean never starting the work the window covers.
                PROCESS_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
                let status = unsafe { MH_EnableHook(site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    outcome.process_windows = true;
                    log(format_args!(
                        "{LOG_PREFIX} hooked gate=process-window rva=0x{:08x} va=0x{site:016x}",
                        ds2_rva::FE_PROCESS_WINDOW_ENTER
                    ));
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} hook-failed gate=process-window va=0x{site:016x} \
                         stage=MH_EnableHook status={status:?}"
                    ));
                }
            }
            Err(status) => log(format_args!(
                "{LOG_PREFIX} hook-failed gate=process-window va=0x{site:016x} \
                 stage=MH_CreateHook status={status:?}"
            )),
        }
    }

    if request.hide_process_windows {
        // Published before the site is patched: a detour that fired with a zero here would read no
        // flag, conclude it is not in the title flow, and draw the window it was meant to hide.
        TITLE_ACTIVE_FLAG.store(
            base + ds2_rva::FE_OPERATOR_TITLE_ACTIVE as usize,
            Ordering::Release,
        );
        let site = base + ds2_rva::FE_SHOW_PROCESS_WINDOW as usize;
        match unsafe {
            MhHook::new(
                site as *mut c_void,
                detour_show_process_window as *mut c_void,
            )
        } {
            Ok(hook) => {
                SHOW_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
                let status = unsafe { MH_EnableHook(site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    outcome.hide_process_windows = true;
                    log(format_args!(
                        "{LOG_PREFIX} hooked gate=show-process-window rva=0x{:08x} \
                         va=0x{site:016x} flag-rva=0x{:08x}",
                        ds2_rva::FE_SHOW_PROCESS_WINDOW,
                        ds2_rva::FE_OPERATOR_TITLE_ACTIVE
                    ));
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} hook-failed gate=show-process-window va=0x{site:016x} \
                         stage=MH_EnableHook status={status:?}"
                    ));
                }
            }
            Err(status) => log(format_args!(
                "{LOG_PREFIX} hook-failed gate=show-process-window va=0x{site:016x} \
                 stage=MH_CreateHook status={status:?}"
            )),
        }
    }
    // ONE HOOK, TWO BEHAVIOURS. `title_animation` writes the terminal phase over the activation
    // flourish; `title_settle` puts the scene into its settled state. Both live in the same
    // `FeSubStateTitleMain::v3` detour, so the hook goes in when either is asked for and each
    // behaviour is gated on its own flag inside -- which is what keeps them independently
    // switchable without patching the same site twice.
    if request.title_animation || request.title_settle {
        MODULE_BASE_TITLE.store(base, Ordering::Release);
        // Published before the site is patched: a detour that fired first would otherwise read
        // `false` for both and do nothing on the one frame that matters.
        SETTLE_ENABLED.store(request.title_settle, Ordering::Release);
        ANIMATION_ENABLED.store(request.title_animation, Ordering::Release);
        let site = base + ds2_rva::FE_TITLE_MAIN_UPDATE as usize;
        match unsafe { MhHook::new(site as *mut c_void, detour_title_main_update as *mut c_void) } {
            Ok(hook) => {
                // Published BEFORE the site is patched: this detour's whole job happens AFTER the
                // original, so a zero trampoline would mean the title screen stopped updating.
                TITLE_MAIN_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
                let status = unsafe { MH_EnableHook(site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    outcome.title_animation = request.title_animation;
                    outcome.title_settle = request.title_settle;
                    log(format_args!(
                        "{LOG_PREFIX} hooked gate=title-main-update rva=0x{:08x} va=0x{site:016x} \
                         animation={} settle={}",
                        ds2_rva::FE_TITLE_MAIN_UPDATE,
                        request.title_animation,
                        request.title_settle
                    ));
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} hook-failed gate=title-animation va=0x{site:016x} \
                         stage=MH_EnableHook status={status:?}"
                    ));
                }
            }
            Err(status) => log(format_args!(
                "{LOG_PREFIX} hook-failed gate=title-animation va=0x{site:016x} \
                 stage=MH_CreateHook status={status:?}"
            )),
        }
    }

    outcome
}

/// `bool is_settled(scene)` -- the gate's real signature, taking the scene its callers pass.
type SequenceGateFn = unsafe extern "system" fn(*mut u8) -> u8;

/// Has the title scene finished animating in?
///
/// `None` when the question cannot be asked -- the gate was not hooked, or the scene is not up yet.
/// A caller that cannot get an answer must not treat that as "still animating", because the failure
/// would then be indistinguishable from a permanent one.
///
/// # Safety
///
/// Calls the game's own predicate with the single argument its own call site passes, through the
/// trampoline MinHook published for it.
pub unsafe fn title_sequence_settled() -> Option<bool> {
    let trampoline = SEQUENCE_GATE_TRAMPOLINE.load(Ordering::Acquire);
    let base = MODULE_BASE_TITLE.load(Ordering::Acquire);
    if trampoline == 0 || base == 0 {
        return None;
    }
    let globals = unsafe { safe_read_usize(base + ds2_rva::FE_TITLE_GLOBALS as usize) }?;
    let scene = unsafe { safe_read_usize(globals + ds2_rva::FE_TITLE_SCENE_OFFSET) }?;
    if scene == 0 {
        return None;
    }
    // SAFETY: MinHook published this trampoline for this site, and the scene pointer comes from the
    // field the gate's own caller reads at `0x1400fedc9`.
    let original: SequenceGateFn =
        unsafe { std::mem::transmute::<usize, SequenceGateFn>(trampoline) };
    Some(unsafe { original(scene as *mut u8) } != 0)
}
