//! Installing the three `enter` detours, and the detours themselves.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything else
/// rather than opening one of its own. Stored as a `usize` because a `fn` pointer is not an
/// `Atomic` type; only ever set from [`set_logger`].
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// One boot screen: where its `enter` is, where it keeps its phase, and what to call it in a log.
///
/// The phase offset travels WITH the address deliberately. These two numbers are only ever correct
/// as a pair -- `FeSubStateTitleLogo` is at `+0x20` while the other two are at `+0x10` -- and
/// pairing them in one struct makes a mismatched combination something you have to construct on
/// purpose rather than something you can fall into by reusing a constant.
struct Screen {
    name: &'static str,
    enter_rva: u32,
    phase_offset: usize,
}

/// The three screens, in the order they appear at boot.
const SCREENS: [Screen; 3] = [
    Screen {
        name: "warning-no-copy",
        enter_rva: ds2_rva::FE_SUBSTATE_WARNING_NO_COPY_ENTER,
        phase_offset: ds2_rva::FE_SUBSTATE_PHASE_OFFSET,
    },
    Screen {
        name: "logo",
        enter_rva: ds2_rva::FE_SUBSTATE_TITLE_LOGO_ENTER,
        phase_offset: ds2_rva::FE_SUBSTATE_TITLE_LOGO_PHASE_OFFSET,
    },
    Screen {
        name: "user-policy",
        enter_rva: ds2_rva::FE_SUBSTATE_TITLE_USER_POLICY_ENTER,
        phase_offset: ds2_rva::FE_SUBSTATE_PHASE_OFFSET,
    },
];

/// Trampolines back to the original `enter`, one per entry in [`SCREENS`], published before the
/// site is patched so a detour that fires immediately cannot read a zero.
static TRAMPOLINES: [AtomicUsize; SCREENS.len()] = [const { AtomicUsize::new(0) }; SCREENS.len()];

/// How many times each detour has fired. Reported so a run that skipped nothing is distinguishable
/// from a run where the screens never came up -- without it, "no logo appeared" and "the hook never
/// installed" look identical from the outside.
static FIRED: [AtomicUsize; SCREENS.len()] = [const { AtomicUsize::new(0) }; SCREENS.len()];

/// A substate `enter`: `void enter(this)`, `this` in RCX.
///
/// This signature is known rather than assumed. All three overrides read their `this` out of RCX
/// and touch no other argument register, and every one of them returns without a value.
type EnterFn = unsafe extern "system" fn(*mut u8);

/// Run the original `enter`, then tell the substate it is already finished.
///
/// # Safety
///
/// `this` is the substate the game is entering; `index` must be a valid index into [`SCREENS`] and
/// [`TRAMPOLINES`]. The phase field is a `u32` at a class-specific offset that this crate reads out
/// of `ds2-rva`, and writing it is the whole point.
unsafe fn skip(index: usize, this: *mut u8) {
    let trampoline = TRAMPOLINES[index].load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is
        // the one every override implements.
        let original: EnterFn = unsafe { std::mem::transmute::<usize, EnterFn>(trampoline) };
        unsafe { original(this) };
    }
    if this.is_null() {
        return;
    }
    let screen = &SCREENS[index];
    // SAFETY: the original has just run against this same pointer, so the object is live and at
    // least as large as the phase field the game itself writes at this offset.
    unsafe {
        this.add(screen.phase_offset)
            .cast::<u32>()
            .write(ds2_rva::TITLE_SUBSTATE_PHASE_DONE);
    }
    let n = FIRED[index].fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} skipped screen={} phase-offset=0x{:x} value={} count={n}",
        screen.name,
        screen.phase_offset,
        ds2_rva::TITLE_SUBSTATE_PHASE_DONE,
    ));
}

// One detour per screen rather than a shared body: MinHook hands a detour no way to learn which
// site it was reached from, so the index has to be baked into the function.
unsafe extern "system" fn detour_warning_no_copy(this: *mut u8) {
    unsafe { skip(0, this) }
}
unsafe extern "system" fn detour_logo(this: *mut u8) {
    unsafe { skip(1, this) }
}
unsafe extern "system" fn detour_user_policy(this: *mut u8) {
    unsafe { skip(2, this) }
}

const DETOURS: [EnterFn; SCREENS.len()] = [detour_warning_no_copy, detour_logo, detour_user_policy];

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Screens whose `enter` is now detoured.
    pub installed: usize,
    /// Screens that were attempted. Always [`SCREENS`]`.len()`; carried so a caller reporting
    /// "2 of 3" does not have to know the total from somewhere else.
    pub attempted: usize,
}

/// Detour the `enter` of each boot screen. Call from the post-Arxan callback, never `DllMain`.
///
/// A screen that fails to hook is logged and skipped over; the others still install. Partial is
/// the right failure mode here -- two screens gone is better than none, and the count in the
/// returned [`Outcome`] says plainly that it was partial rather than reporting success.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`) and before the title flow reaches these substates, which in practice
/// means the loader's Arxan callback.
pub unsafe fn install() -> Outcome {
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome {
                installed: 0,
                attempted: SCREENS.len(),
            };
        }
    };

    // MinHook is statically linked into this DLL, so nothing else shares this instance and
    // ALREADY_INITIALIZED can only mean this ran twice. Treat it as success.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome {
            installed: 0,
            attempted: SCREENS.len(),
        };
    }

    let mut installed = 0;
    for (index, screen) in SCREENS.iter().enumerate() {
        let site = base + screen.enter_rva as usize;
        let detour = DETOURS[index];
        let hook = match unsafe { MhHook::new(site as *mut c_void, detour as *mut c_void) } {
            Ok(hook) => hook,
            Err(status) => {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed screen={} va=0x{site:016x} stage=MH_CreateHook \
                     status={status:?}",
                    screen.name
                ));
                continue;
            }
        };
        // Published BEFORE the site is patched, so a detour cannot observe a zero and silently
        // decline to call the original.
        TRAMPOLINES[index].store(hook.trampoline() as usize, Ordering::Release);
        let status = unsafe { MH_EnableHook(site as *mut c_void) };
        if status != MH_STATUS::MH_OK {
            log(format_args!(
                "{LOG_PREFIX} hook-failed screen={} va=0x{site:016x} stage=MH_EnableHook \
                 status={status:?}",
                screen.name
            ));
            continue;
        }
        // The handle simply falls out of scope here. `MhHook` has no `Drop`, so that does NOT
        // remove the hook -- the patch stays for the life of the process, which is what is wanted.
        // Two earlier versions of this line were wrong in opposite directions: `mem::forget` on
        // the assumption that dropping would unhook, then an explicit `drop` to say so. Clippy
        // rejected both, correctly, because neither does anything to a type with no destructor.
        installed += 1;
        log(format_args!(
            "{LOG_PREFIX} hooked screen={} rva=0x{:08x} va=0x{site:016x} phase-offset=0x{:x}",
            screen.name, screen.enter_rva, screen.phase_offset
        ));
    }

    log(format_args!(
        "{LOG_PREFIX} install installed={installed}/{}",
        SCREENS.len()
    ));
    Outcome {
        installed,
        attempted: SCREENS.len(),
    }
}
