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

use ds2_game_base::mem::{game_module_base, safe_read_f32, safe_read_i32};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// Trampoline back to the original process-window `enter`.
static PROCESS_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// How many times the press gate has been forced. Reported so "the title screen never came up"
/// and "the skip never fired" cannot be confused.
static PRESSES: AtomicUsize = AtomicUsize::new(0);

/// How many process windows have had their minimum duration cleared.
static SHORTENED: AtomicUsize = AtomicUsize::new(0);

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

/// What [`install`] managed to do. Each hook is independent; one failing does not stop the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The press-any-button gate is now forced.
    pub press_any_button: bool,
    /// Process windows now close as soon as their work is done.
    pub process_windows: bool,
}

/// Install whichever of the two title-flow skips were asked for.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`) and before the title flow reaches these substates.
pub unsafe fn install(press_any_button: bool, process_windows: bool) -> Outcome {
    let mut outcome = Outcome {
        press_any_button: false,
        process_windows: false,
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

    if press_any_button {
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

    if process_windows {
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

    outcome
}
