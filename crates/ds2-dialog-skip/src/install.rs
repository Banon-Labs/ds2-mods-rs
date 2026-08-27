//! Installing the one detour, and the answer it writes.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, safe_read_u8, safe_read_u16, safe_read_usize};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything
/// else. Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
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

/// One message box this mod is willing to answer.
///
/// A vtable address rather than a function address, because the hook is on the *shared* update and
/// the only thing that distinguishes one dialog from another at that point is the object's vptr.
struct Dialog {
    name: &'static str,
    vtable_rva: u32,
}

/// The boot dialogs, allowlisted by vtable.
///
/// All three are message boxes the title flow raises on its own during startup, and all three have
/// the inert `ret 0` handlers that [`handlers_are_inert`] re-checks at runtime. Deliberately NOT
/// here: `FeSubStateCommonWindow`, which is the generic box used all over the game rather than a
/// boot screen, and `FeSubStateTitleDeleteProfile`, which is
/// [`ds2_rva::FE_DIALOG_VTABLE_DELETE_PROFILE_DO_NOT_ANSWER`] and has a real slot-8 body.
const DIALOGS: [Dialog; 3] = [
    Dialog {
        name: "online-check-fail-warn",
        vtable_rva: ds2_rva::FE_DIALOG_VTABLE_ONLINE_CHECK_FAIL_WARN,
    },
    Dialog {
        name: "information-fail-warn",
        vtable_rva: ds2_rva::FE_DIALOG_VTABLE_INFORMATION_FAIL_WARN,
    },
    Dialog {
        name: "offline-mode-window",
        vtable_rva: ds2_rva::FE_DIALOG_VTABLE_OFFLINE_MODE_WINDOW,
    },
];

/// Trampoline back to the original update, published before the site is patched so a detour that
/// fires immediately cannot read a zero and drop a frame of the game's own logic.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The live module base, resolved once at install.
///
/// Cached because the detour runs every frame for every open dialog, and `GetModuleHandleA` on
/// that path would be a syscall per frame to re-learn something that cannot change.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// How many dialogs have been answered. Reported so a run that answered nothing is
/// distinguishable from a run where no dialog ever appeared.
static ANSWERED: AtomicUsize = AtomicUsize::new(0);

/// Vtables already named in a "seen, not answered" line.
///
/// The hook is on an update that runs every frame, so an unrecognised dialog would otherwise write
/// a log line per frame for as long as it is on screen. Reporting each distinct vptr once keeps
/// the evidence and drops the flood. Sized generously against the six classes that share this
/// update; a seventh from a future build simply goes unreported rather than overflowing anything.
static REPORTED: [AtomicUsize; 8] = [const { AtomicUsize::new(0) }; 8];

/// `void update(this, float delta)` -- `this` in RCX, the frame delta in XMM1.
///
/// **THE FLOAT IS NOT OPTIONAL.** `FeSubStateCommonWindowBase::v3` opens its phase-1 branch with
/// `addss xmm1, [rcx+0x18]` against an XMM1 it never initialises, so the register is an incoming
/// argument and the field at `+0x18` is an elapsed-time accumulator. A detour declared as taking
/// only `this` would be free to clobber XMM1 before reaching the trampoline, and the dialog's
/// timer would then accumulate whatever happened to be in that register. `FeSubStateTitleLogo::v3`
/// does the same thing to its own `+0x28`, which is what establishes this as the family's
/// signature rather than a quirk of one function.
type UpdateFn = unsafe extern "system" fn(*mut u8, f32);

/// Are this object's decision handlers still the base class's `ret 0` stubs?
///
/// The dispatch inside the update calls slot 8 for a cancel and slot 9 for a confirm. If both are
/// the inert stubs, answering the box closes it and has no other effect -- which is the property
/// that makes synthesising an answer safe, and it is checked here against the bytes in front of us
/// rather than assumed from the class name. `FeSubStateTitleDeleteProfile` overrides slot 8 and
/// fails this on its own merits.
///
/// # Safety
///
/// `vptr` may be anything; every read goes through a fault-tolerant reader that returns `None` on
/// unmapped memory rather than faulting.
unsafe fn handlers_are_inert(vptr: usize, base: usize) -> bool {
    let slot = |index: usize| unsafe { safe_read_usize(vptr + index * size_of::<usize>()) };
    slot(ds2_rva::FE_DIALOG_SLOT_ON_CANCEL)
        == Some(base + ds2_rva::FE_DIALOG_INERT_ON_CANCEL as usize)
        && slot(ds2_rva::FE_DIALOG_SLOT_ON_CONFIRM)
            == Some(base + ds2_rva::FE_DIALOG_INERT_ON_CONFIRM as usize)
}

/// Log a vtable once, however many frames it is on screen for.
fn report_once(vptr: usize, args: std::fmt::Arguments<'_>) {
    for slot in &REPORTED {
        match slot.compare_exchange(0, vptr, Ordering::AcqRel, Ordering::Acquire) {
            // Claimed a free slot: this is the first sighting.
            Ok(_) => {
                log(args);
                return;
            }
            // Already recorded, by us on an earlier frame or by another thread just now.
            Err(existing) if existing == vptr => return,
            Err(_) => {}
        }
    }
}

/// Press the button, if this object is a dialog this mod is willing to press for.
///
/// # Safety
///
/// `this` is the substate the game is about to update. Every read is fault-tolerant; the single
/// write happens only after the object has been identified by vptr, confirmed to be waiting for
/// input, and confirmed to have inert handlers.
unsafe fn answer(this: *mut u8) {
    if this.is_null() {
        return;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let object = this as usize;

    // Only phase 1 waits for input. In every other phase the update returns without dispatching,
    // so a result written now would be read on some later frame in a state we have not reasoned
    // about -- and after a real press, it would overwrite the player's own answer.
    if unsafe { safe_read_u8(object + ds2_rva::FE_DIALOG_PHASE_OFFSET) }
        != Some(ds2_rva::FE_DIALOG_PHASE_WAITING)
    {
        return;
    }
    if unsafe { safe_read_u8(object + ds2_rva::FE_DIALOG_RESULT_OFFSET) }
        != Some(ds2_rva::FE_DIALOG_RESULT_NONE)
    {
        return;
    }

    let Some(vptr) = (unsafe { safe_read_usize(object) }) else {
        return;
    };
    let Some(dialog) = DIALOGS
        .iter()
        .find(|dialog| vptr == base + dialog.vtable_rva as usize)
    else {
        report_once(
            vptr,
            format_args!(
                "{LOG_PREFIX} seen screen=<not-allowlisted> vtable=0x{vptr:016x} \
                 rva=0x{:08x} action=left-alone",
                vptr.wrapping_sub(base)
            ),
        );
        return;
    };
    if !unsafe { handlers_are_inert(vptr, base) } {
        report_once(
            vptr,
            format_args!(
                "{LOG_PREFIX} seen screen={} vtable=0x{vptr:016x} action=left-alone \
                 reason=handlers-not-inert",
                dialog.name
            ),
        );
        return;
    }

    // The answer is a function of the object, not a constant. A negative option count is the
    // game's own marker for a one-button acknowledgement box, and on those its input path only
    // ever produces a cancel -- writing a confirm there would call a handler the game never
    // would.
    let Some(raw_options) = (unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_OPTIONS_OFFSET) })
    else {
        return;
    };
    let options = raw_options as i16;
    let value = if options < 0 {
        ds2_rva::FE_DIALOG_RESULT_CANCEL
    } else {
        ds2_rva::FE_DIALOG_RESULT_CONFIRM
    };

    // SAFETY: the object was just read at three of its own offsets through fault-tolerant reads
    // that all succeeded, so it is mapped and at least as large as the byte the game itself writes
    // here on every button press.
    unsafe { this.add(ds2_rva::FE_DIALOG_RESULT_OFFSET).write(value) };

    let total = ANSWERED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} answered screen={} options={options} result={value} total={total}",
        dialog.name
    ));
}

/// The detour: answer first, then run the game's own update unchanged.
///
/// ANSWERING BEFORE THE ORIGINAL IS THE POINT. The update's dispatch reads the result byte on the
/// same call, so a value written here is consumed on this very frame -- the box closes through the
/// game's own path without waiting a frame for the next tick. Writing it afterwards would work too
/// and would simply be one frame slower; nothing else about the ordering matters, because the two
/// touch no other shared state.
unsafe extern "system" fn detour_update(this: *mut u8, delta: f32) {
    unsafe { answer(this) };
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is
        // the one every substate update implements. `delta` is forwarded rather than reconstructed
        // so the dialog's own timer keeps accumulating real frame time.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
        unsafe { original(this, delta) };
    }
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Whether the shared update is now detoured. There is exactly one hook, so this is the whole
    /// story -- unlike `ds2-intro-skip`, this feature cannot land partially.
    pub installed: bool,
}

/// Detour `FeSubStateCommonWindowBase::v3`. Call from the post-Arxan callback, never `DllMain`.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`) and before the title flow raises its first message box, which in
/// practice means the loader's Arxan callback.
pub unsafe fn install() -> Outcome {
    let base = match game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome { installed: false };
        }
    };
    // Published before the site is patched: the detour reads it on its very first frame and
    // declines to act if it is still zero.
    MODULE_BASE.store(base, Ordering::Release);

    // MinHook is statically linked into this DLL, so nothing else shares this instance and
    // ALREADY_INITIALIZED can only mean this ran twice. Treat it as success.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome { installed: false };
    }

    let site = base + ds2_rva::FE_DIALOG_UPDATE as usize;
    let hook = match unsafe { MhHook::new(site as *mut c_void, detour_update as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed va=0x{site:016x} stage=MH_CreateHook status={status:?}"
            ));
            return Outcome { installed: false };
        }
    };
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} install-failed va=0x{site:016x} stage=MH_EnableHook status={status:?}"
        ));
        return Outcome { installed: false };
    }
    // The handle falls out of scope here. `MhHook` has no `Drop`, so that does NOT remove the
    // hook -- the patch stays for the life of the process, which is what is wanted.

    log(format_args!(
        "{LOG_PREFIX} install ok rva=0x{:08x} va=0x{site:016x} dialogs={}",
        ds2_rva::FE_DIALOG_UPDATE,
        DIALOGS.len()
    ));
    Outcome { installed: true }
}
