//! The two things that still stop the title flow once the notice boxes are gone.
//!
//! They are different in kind, and the difference decides the cut in each case.
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
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, safe_read_f32, safe_read_i32, safe_read_u8};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// Trampoline back to the original process-window `enter`.
static PROCESS_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Trampoline back to `FeSubStateTitleMain`'s original update.
static TITLE_MAIN_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

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
unsafe extern "system" fn detour_sequence_gate(_scene: *mut u8) -> u8 {
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
    if this.is_null() {
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
}

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
        hide_process_windows: false,
        title_sequence_gate: false,
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
            Ok(_) => {
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
    if request.title_animation {
        let site = base + ds2_rva::FE_TITLE_MAIN_UPDATE as usize;
        match unsafe { MhHook::new(site as *mut c_void, detour_title_main_update as *mut c_void) } {
            Ok(hook) => {
                // Published BEFORE the site is patched: this detour's whole job happens AFTER the
                // original, so a zero trampoline would mean the title screen stopped updating.
                TITLE_MAIN_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
                let status = unsafe { MH_EnableHook(site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    outcome.title_animation = true;
                    log(format_args!(
                        "{LOG_PREFIX} hooked gate=title-animation rva=0x{:08x} va=0x{site:016x}",
                        ds2_rva::FE_TITLE_MAIN_UPDATE
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
