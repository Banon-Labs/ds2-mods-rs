//! Installing the one detour, and the answer it writes.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, safe_read_i32, safe_read_u16, safe_read_usize};
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

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// One message box this mod is willing to suppress.
///
/// A vtable address rather than a function address, because the hook is on the *shared* `enter` and
/// the only thing that distinguishes one dialog from another at that point is the object's vptr.
struct Dialog {
    name: &'static str,
    vtable_rva: u32,
}

/// The boot dialogs, allowlisted by vtable.
///
/// All four are message boxes the title flow owns, and all four have the inert `ret 0` handlers
/// that [`handlers_are_inert`] re-checks at runtime.
///
/// **`common-window` is first because it is the one that actually appears.** It was left out of
/// the first version on the strength of its generic-sounding name, and the run that followed
/// logged `seen screen=<not-allowlisted> vtable=0x00000001410bcff8` while the three named ones
/// never fired at all. Reading it put it back: its vtable is referenced at exactly one site in the
/// whole image, inside `FeStateTitle`'s substate-table builder, so there is one instance and it
/// belongs to the title flow -- no in-game prompt can be an instance of it. See
/// [`ds2_rva::FE_DIALOG_VTABLE_COMMON_WINDOW`] for the derivation.
///
/// The other three are network-failure and offline notices that do not appear on a machine whose
/// checks succeed. They stay listed because a run where they DO appear costs nothing extra, and a
/// screen that never occurs costs nothing at all.
///
/// Deliberately NOT here: `FeSubStateTitleDeleteProfile`, which shares the same update and has a
/// real slot-8 body -- [`ds2_rva::FE_DIALOG_VTABLE_DELETE_PROFILE_DO_NOT_ANSWER`].
const DIALOGS: [Dialog; 4] = [
    Dialog {
        name: "common-window",
        vtable_rva: ds2_rva::FE_DIALOG_VTABLE_COMMON_WINDOW,
    },
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

/// Trampoline back to the original `enter`, published before the site is patched so a detour that
/// fires immediately cannot read a zero and fail to show a dialog it meant to leave alone.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The live module base, resolved once at install.
///
/// Cached because the detour runs every frame for every open dialog, and `GetModuleHandleA` on
/// that path would be a syscall per frame to re-learn something that cannot change.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// How many dialogs have been suppressed. Reported so a run that suppressed nothing is
/// distinguishable from a run where no dialog ever came up.
static SUPPRESSED: AtomicUsize = AtomicUsize::new(0);

/// Vtables already named in a "seen, left alone" line.
///
/// `enter` runs once per appearance rather than once per frame, so this matters less than it did
/// when the hook was on the update -- but the one reusable notice box is entered repeatedly with
/// different messages, and without this each re-entry would repeat the same declined-dialog line.
/// Sized generously against the six classes that share this enter; a seventh from a future build
/// simply goes unreported rather than overflowing anything.
static REPORTED: [AtomicUsize; 8] = [const { AtomicUsize::new(0) }; 8];

/// `void enter(this)` -- `this` in RCX, no other argument.
///
/// Unlike the family's `update`, which takes a frame delta in XMM1, `enter` reads no incoming
/// float: `0x140104db0` touches RCX and nothing else before its first call. That was checked
/// rather than assumed, because the update next door does take one and a detour that dropped it
/// would corrupt a timer with no diagnostic.
type EnterFn = unsafe extern "system" fn(*mut u8);

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

/// Decide whether this dialog should be prevented from ever appearing.
///
/// Returns `true` when the caller must NOT run the original `enter` -- no window is created, and
/// the object is left in the state the game itself leaves it in once such a box has been closed.
///
/// # Safety
///
/// `this` is the substate the game is about to enter. Every read is fault-tolerant; the two writes
/// happen only after the object has been identified by vptr, confirmed to have inert handlers, and
/// confirmed to be a one-button box.
unsafe fn suppress(this: *mut u8) -> bool {
    if this.is_null() {
        return false;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return false;
    }
    let object = this as usize;

    let Some(vptr) = (unsafe { safe_read_usize(object) }) else {
        return false;
    };
    let Some(dialog) = DIALOGS
        .iter()
        .find(|dialog| vptr == base + dialog.vtable_rva as usize)
    else {
        report_once(
            vptr,
            format_args!(
                "{LOG_PREFIX} seen screen=<not-allowlisted> vtable=0x{vptr:016x} rva=0x{:08x} \
                 kind={} caption=0x{:x} options={} action=shown",
                vptr.wrapping_sub(base),
                unsafe { safe_read_i32(object + ds2_rva::FE_DIALOG_KIND_OFFSET) }.unwrap_or(-1),
                unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CAPTION_OFFSET) }.unwrap_or(0),
                unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_OPTIONS_OFFSET) }
                    .map_or(-1, |raw| raw as i16),
            ),
        );
        return false;
    };
    if !unsafe { handlers_are_inert(vptr, base) } {
        report_once(
            vptr,
            format_args!(
                "{LOG_PREFIX} seen screen={} vtable=0x{vptr:016x} action=shown \
                 reason=handlers-not-inert",
                dialog.name
            ),
        );
        return false;
    }

    let Some(raw_options) = (unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_OPTIONS_OFFSET) })
    else {
        return false;
    };
    let options = raw_options as i16;

    // THIS MOD SUPPRESSES NOTICES. IT NEVER ANSWERS A QUESTION.
    //
    // A negative option count is the game's own marker for a one-button acknowledgement box: the
    // update's input path can only ever produce a cancel for one, and the closed phase it computes
    // can only ever be 3. There is no second outcome, so removing the box removes a keypress and
    // nothing else. A non-negative count means a real decision with a real affirmative, and this
    // mod shows it and steps back -- whatever it is asking, the player is the one being asked.
    //
    // That turns the safety argument from a list of class names into a property of the object in
    // front of the code. The allowlist still encodes intent and the inertness check still refuses a
    // box whose answer would run code, but THIS is the condition that makes "silently decided
    // something on the player's behalf" structurally impossible rather than merely unlikely. Every
    // dialog it declines says so in the log, which is how a two-option boot dialog would come to
    // light -- as a line to read, not as a choice already made.
    if options >= 0 {
        report_once(
            vptr,
            format_args!(
                "{LOG_PREFIX} seen screen={} vtable=0x{vptr:016x} options={options} \
                 action=shown reason=has-a-real-choice",
                dialog.name
            ),
        );
        return false;
    }

    // The state the game itself leaves such a box in once it has been closed. `leave` closes the
    // window ONLY when the phase is 1, so writing the closed phase here is also what keeps a
    // `leave` that follows from closing a window this never opened.
    //
    // SAFETY: the object was just read at three of its own offsets through fault-tolerant reads
    // that all succeeded, so it is mapped and at least as large as the fields the game itself
    // writes here.
    unsafe {
        this.add(ds2_rva::FE_DIALOG_RESULT_OFFSET)
            .write(ds2_rva::FE_DIALOG_RESULT_CANCEL);
        this.add(ds2_rva::FE_DIALOG_PHASE_OFFSET)
            .write(ds2_rva::FE_DIALOG_PHASE_CLOSED_CANCEL);
        // What the original `enter` would have zeroed. Nothing reads it in the closed phase; it is
        // written so the object is not left carrying a stale timer from a previous appearance.
        this.add(ds2_rva::FE_DIALOG_ELAPSED_OFFSET)
            .cast::<u32>()
            .write(0);
    }

    let total = SUPPRESSED.fetch_add(1, Ordering::Relaxed) + 1;
    // `kind` and `caption` are logged per appearance, not once, because the ONE notice object is
    // re-entered with different messages -- the first suppressed run showed kind=6/caption=0x20 and
    // then kind=70/caption=0x47 through the same vtable. Logging only the class would have made
    // two different notices look like one repeated event.
    log(format_args!(
        "{LOG_PREFIX} suppressed screen={} kind={} caption=0x{:x} options={options} total={total}",
        dialog.name,
        unsafe { safe_read_i32(object + ds2_rva::FE_DIALOG_KIND_OFFSET) }.unwrap_or(-1),
        unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CAPTION_OFFSET) }.unwrap_or(0),
    ));
    true
}

/// The detour: for a suppressible notice, return without ever creating the window.
///
/// THIS IS THE ONE PLACE THE ORIGINAL IS DELIBERATELY NOT CALLED, and it is why the boxes stop
/// appearing rather than merely closing themselves. An earlier version hooked the shared `update`
/// instead and wrote the result byte a press writes; that worked -- every box answered itself -- but
/// the box still had to be drawn first, so the player watched a dialog flash past instead of
/// pressing a button. Preventing it means not opening it.
///
/// Skipping an `enter` is normally the wrong shape, and `ds2-intro-skip` deliberately does not do
/// it: there `leave` closes what `enter` opened unconditionally, so a skipped open leaves an
/// unbalanced close. Here `leave` closes only when the phase is 1 (`0x1401050a6`), so a box that
/// was never opened is never closed, and the pairing stays balanced.
unsafe extern "system" fn detour_enter(this: *mut u8) {
    if unsafe { suppress(this) } {
        return;
    }
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one every caller of the shared `enter` uses.
        let original: EnterFn = unsafe { std::mem::transmute::<usize, EnterFn>(trampoline) };
        unsafe { original(this) };
    }
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Whether the shared `enter` is now detoured. There is exactly one hook, so this is the whole
    /// story -- unlike `ds2-intro-skip`, this feature cannot land partially.
    pub installed: bool,
}

/// Detour `FeSubStateCommonWindowBase::v1`. Call from the post-Arxan callback, never `DllMain`.
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

    let site = base + ds2_rva::FE_DIALOG_ENTER as usize;
    let hook = match unsafe { MhHook::new(site as *mut c_void, detour_enter as *mut c_void) } {
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
        ds2_rva::FE_DIALOG_ENTER,
        DIALOGS.len()
    ));
    Outcome { installed: true }
}
