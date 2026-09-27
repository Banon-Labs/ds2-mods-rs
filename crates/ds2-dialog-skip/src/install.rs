//! Installing the one detour, and the answer it writes.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    kinds: Kinds,
}

/// Which appearances of an allowlisted class may be suppressed.
#[derive(Clone, Copy, Debug)]
enum Kinds {
    /// A class with one message of its own, so the class names the message. The three
    /// network/offline classes each override the message getter with their own text.
    Any,
    /// A class the title re-enters with many messages, told apart only by kind (its substate id).
    /// Only these kinds are suppressed; every other appearance is shown.
    Only(&'static [i32]),
}

/// The `common-window` kinds with evidence of being boxes this crate may take off the screen.
///
/// Each one is cited, with its message and the run that logged it, on its `ds2-rva` constant.
/// Kind 62 is the offline prompt: listed so the offline answer can still reach it, and it is
/// still shown on any run that has not been configured to play offline, because it has two edges.
///
/// Deliberately absent, with the message the image gives each: 82 "Failed to save game." and 88
/// "Failed to load character data." A failure the player is not told about is a press that did
/// nothing, which is exactly what 88's suppression looked like.
const COMMON_WINDOW_KINDS: &[i32] = &[
    ds2_rva::FE_COMMON_WINDOW_KIND_NOT_EXITED_PROPERLY,
    ds2_rva::FE_COMMON_WINDOW_KIND_SERVICE_UNAVAILABLE,
    ds2_rva::FE_COMMON_WINDOW_KIND_NO_NEW_INFORMATION,
];

/// Whether an appearance of kind `kind` is on its class's allowlist.
fn kind_allowed(kinds: Kinds, kind: i32) -> bool {
    match kinds {
        Kinds::Any => true,
        Kinds::Only(listed) => listed.contains(&kind),
    }
}

/// Whether this appearance may be suppressed: on the allowlist, or the login-refused notice on a
/// run that has said it plays under Seamless Co-op.
fn kind_suppressible(kinds: Kinds, kind: i32, login_refusal_expected: bool) -> bool {
    kind_allowed(kinds, kind)
        || (login_refusal_expected
            && matches!(kinds, Kinds::Only(_))
            && kind == ds2_rva::FE_COMMON_WINDOW_KIND_LOGIN_REFUSED)
}

/// Whether the official server's login refusal
/// ([`ds2_rva::FE_COMMON_WINDOW_KIND_LOGIN_REFUSED`]) may be suppressed.
///
/// Off unless the loader turns it on, and it turns it on only for a `[seamless]` run. Under
/// Seamless Co-op the game still tries the official login at boot, and the refusal is expected:
/// co-op runs over the mod's own network. The box has one button, and the only way out of it is
/// the offline window, so suppressing it saves one keypress and does not change where the game goes.
/// On any other run it stays on the screen, because there a refusal would be news.
static SUPPRESS_LOGIN_REFUSAL: AtomicBool = AtomicBool::new(false);

/// Allow the login-refused notice to be suppressed. The loader relays `[seamless] enabled`.
pub fn set_suppress_login_refusal(enabled: bool) {
    SUPPRESS_LOGIN_REFUSAL.store(enabled, Ordering::Release);
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
        kinds: Kinds::Only(COMMON_WINDOW_KINDS),
    },
    Dialog {
        name: "online-check-fail-warn",
        vtable_rva: ds2_rva::FE_DIALOG_VTABLE_ONLINE_CHECK_FAIL_WARN,
        kinds: Kinds::Any,
    },
    Dialog {
        name: "information-fail-warn",
        vtable_rva: ds2_rva::FE_DIALOG_VTABLE_INFORMATION_FAIL_WARN,
        kinds: Kinds::Any,
    },
    Dialog {
        name: "offline-mode-window",
        vtable_rva: ds2_rva::FE_DIALOG_VTABLE_OFFLINE_MODE_WINDOW,
        kinds: Kinds::Any,
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

/// Whether a two-option box may be answered toward `FeSubStateOfflineModeWindow`.
///
/// **Off unless something turns it on**, and the default matters: with this clear, this crate's
/// rule is exactly what it always was -- suppress one-outcome notices, never answer a question.
/// The loader sets it from `[offline] enabled`, so the only run in which a real choice gets
/// answered is one that has already been configured to play offline, which is what makes the
/// answer the player's own rather than this mod's.
static ANSWER_OFFLINE_PROMPT: AtomicBool = AtomicBool::new(false);

/// Allow the offline prompt to be answered. Call before [`install`].
///
/// Separate from the config plumbing on purpose: this crate owns the `enter` hook and therefore
/// has to own the decision, but it must not own the POLICY -- whether this run is an offline run
/// is `ds2-offline`'s business and the loader's to relay. A setter keeps the dependency pointing
/// that way rather than making a dialog crate read an unrelated feature's config section.
pub fn set_answer_offline_prompt(enabled: bool) {
    ANSWER_OFFLINE_PROMPT.store(enabled, Ordering::Release);
}

/// Nesting depth of [`hold`] calls. Zero means this crate answers boxes as usual.
static HELD: AtomicUsize = AtomicUsize::new(0);

/// Stop answering anything until [`release`] is called. The measurement that forced this:
///
/// ```text
/// ds2-continue:    data-list phase=1->2 slot=0 action=0->2 dest=0x57-LoadProfile
/// ds2-dialog-skip: suppressed screen=common-window kind=88 cancel-dest=0x55 confirm-dest=0xffff \
///                  edge=only-edge result=1
/// ds2-continue:    data-list phase=1->2 ...                     <- and round again, twelve times
/// ```
///
/// This comment used to call kind 88 a character confirm being answered "no". The image says
/// otherwise: kind 88 is built with message `0x38271`, "Failed to load character data."
/// ([`ds2_rva::FE_COMMON_WINDOW_KIND_LOAD_CHARACTER_FAILED`]). The loop above was a load failing
/// twelve times with the failure hidden each time. Kind 88 is now off the allowlist and always
/// shown, so that case no longer needs a hold.
///
/// The hold stays for what it still does: a flow deliberately driving the title --
/// `ds2-save-file`'s character swap -- gets every box, allowlisted or not, for the duration.
///
/// Paired with [`release`], and counted rather than boolean so two flows holding at once cannot
/// have the first release re-arm it under the second.
pub fn hold() -> usize {
    let depth = HELD.fetch_add(1, Ordering::AcqRel) + 1;
    log(format_args!(
        "{LOG_PREFIX} held depth={depth} -- boxes are the player's to answer until released"
    ));
    depth
}

/// Undo one [`hold`].
pub fn release() -> usize {
    let previous = HELD.fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
        Some(held.saturating_sub(1))
    });
    let depth = previous.unwrap_or(0).saturating_sub(1);
    log(format_args!("{LOG_PREFIX} released depth={depth}"));
    depth
}

/// Whether [`hold`] is in force.
pub fn held() -> bool {
    HELD.load(Ordering::Acquire) != 0
}

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
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
    // moved or was freed answers None rather than faulting.
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
    // A flow that is driving the title has taken its questions back. First, before anything is
    // read: while held, this crate has no opinion about any box at all.
    if held() {
        return false;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return false;
    }
    let object = this as usize;

    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads

    // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that

    // moved or was freed answers None rather than faulting.

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
                 kind={} cancel-dest=0x{:02x} confirm-dest=0x{:02x} action=shown",
                vptr.wrapping_sub(base),
                // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
                // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
                // moved or was freed answers None rather than faulting.
                unsafe { safe_read_i32(object + ds2_rva::FE_DIALOG_KIND_OFFSET) }.unwrap_or(-1),
                // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
                // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
                // moved or was freed answers None rather than faulting.
                unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CANCEL_DEST_OFFSET) }
                    .map_or(-1, |raw| raw as i16),
                // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
                // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
                // moved or was freed answers None rather than faulting.
                unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CONFIRM_DEST_OFFSET) }
                    .map_or(-1, |raw| raw as i16),
            ),
        );
        return false;
    };
    // The class is allowlisted; the message may not be. `common-window` is one class entered with
    // many messages, so the kind decides, and a kind nobody has vouched for is shown and logged
    // every time it appears -- a new message is a line to read, never a box answered unseen.
    // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one.
    let kind = unsafe { safe_read_i32(object + ds2_rva::FE_DIALOG_KIND_OFFSET) }.unwrap_or(-1);
    if !kind_suppressible(
        dialog.kinds,
        kind,
        SUPPRESS_LOGIN_REFUSAL.load(Ordering::Acquire),
    ) {
        log(format_args!(
            "{LOG_PREFIX} seen screen={} kind={kind} cancel-dest=0x{:02x} confirm-dest=0x{:02x} \
             action=shown reason=kind-not-allowlisted",
            dialog.name,
            // SAFETY: as above.
            unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CANCEL_DEST_OFFSET) }
                .map_or(-1, |raw| raw as i16),
            // SAFETY: as above.
            unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CONFIRM_DEST_OFFSET) }
                .map_or(-1, |raw| raw as i16),
        ));
        return false;
    }
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
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

    // THE TWO EDGES, read from the object rather than reasoned about from button labels. `v5`
    // (`0x140104f30`) publishes one transition per edge: the cancel destination unconditionally,
    // the confirm destination only when it is non-negative. See
    // `ds2_rva::FE_DIALOG_CANCEL_DEST_OFFSET`.
    let Some(cancel_dest) =
        // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
        // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
        // moved or was freed answers None rather than faulting.
        (unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CANCEL_DEST_OFFSET) })
    else {
        return false;
    };
    let cancel_dest = cancel_dest as i16;
    let Some(confirm_dest) =
        // SAFETY: `safe_read_*` accepts any address and fails closed on an unmapped one -- it reads
        // through `ReadProcessMemory`, which validates the range in the kernel. A game structure that
        // moved or was freed answers None rather than faulting.
        (unsafe { safe_read_u16(object + ds2_rva::FE_DIALOG_CONFIRM_DEST_OFFSET) })
    else {
        return false;
    };
    let confirm_dest = confirm_dest as i16;

    // THIS MOD SUPPRESSES NOTICES, AND ANSWERS EXACTLY ONE QUESTION.
    //
    // The rule used to be "never answer a question", and the comment here used to say that
    // declining would be how "a two-option boot dialog would come to light -- as a line to read,
    // not as a choice already made". That happened. The line was
    //
    //   seen screen=common-window vtable=0x1410bcff8 options=42 action=shown reason=has-a-real-choice
    //
    // and the box is `The DARK SOULS II service is not available ... Select "CANCEL" to start the
    // game in offline mode`. So the mechanism worked exactly as designed, and what it surfaced is a
    // question this mod already knows the answer to on an offline run.
    //
    // THE `options=42` IN THAT LINE WAS NEVER AN OPTION COUNT. `+0x12` is the confirm edge's
    // destination substate id, and 42 is `0x2a`, `FeSubStateOfflineModeWindow`. See
    // `ds2_rva::FE_DIALOG_CONFIRM_DEST_OFFSET`.
    //
    // WHICH EDGE TO TAKE, and the whole safety argument for answering anything at all.
    //
    // A negative confirm destination means the game published only one transition, so the box has
    // exactly one outcome and removing it removes a keypress. That is the original rule and it is
    // unchanged.
    //
    // A two-edge box is a real question and is shown -- UNLESS exactly one of its two destinations
    // is `FeSubStateOfflineModeWindow` and this run has been asked to play offline. Then the
    // question is one this mod already knows the answer to, because the answer is the mod's whole
    // configuration, and taking that edge is pressing the button rather than faking a state.
    //
    // "EXACTLY one" is doing real work. If both edges led there the choice would be meaningless
    // and answering it would be noise; if neither does, this is some other question and none of
    // this mod's business. Requiring exactly one also means the code never has to know which
    // BUTTON is which -- see `ds2_rva::FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW` for why that matters,
    // and for the run where reasoning from the labels would have retried the login instead.
    let offline = ds2_rva::FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW;
    let (result, closed_phase, edge) = if confirm_dest < 0 {
        (
            ds2_rva::FE_DIALOG_RESULT_CANCEL,
            ds2_rva::FE_DIALOG_PHASE_CLOSED_CANCEL,
            "only-edge",
        )
    } else if !ANSWER_OFFLINE_PROMPT.load(Ordering::Acquire) {
        report_once(
            vptr,
            format_args!(
                "{LOG_PREFIX} seen screen={} vtable=0x{vptr:016x} cancel-dest=0x{:02x} \
                 confirm-dest=0x{:02x} action=shown reason=has-a-real-choice",
                dialog.name, cancel_dest, confirm_dest
            ),
        );
        return false;
    } else if confirm_dest == offline && cancel_dest != offline {
        (
            ds2_rva::FE_DIALOG_RESULT_CONFIRM,
            ds2_rva::FE_DIALOG_PHASE_CLOSED_CONFIRM,
            "confirm-goes-offline",
        )
    } else if cancel_dest == offline && confirm_dest != offline {
        (
            ds2_rva::FE_DIALOG_RESULT_CANCEL,
            ds2_rva::FE_DIALOG_PHASE_CLOSED_CANCEL,
            "cancel-goes-offline",
        )
    } else {
        report_once(
            vptr,
            format_args!(
                "{LOG_PREFIX} seen screen={} vtable=0x{vptr:016x} cancel-dest=0x{:02x} \
                 confirm-dest=0x{:02x} action=shown reason=has-a-real-choice",
                dialog.name, cancel_dest, confirm_dest
            ),
        );
        return false;
    };

    // The state the game itself leaves such a box in once it has been closed. `leave` closes the
    // window ONLY when the phase is 1, so writing the closed phase here is also what keeps a
    // `leave` that follows from closing a window this never opened.
    //
    // SAFETY: the object was just read at three of its own offsets through fault-tolerant reads
    // that all succeeded, so it is mapped and at least as large as the fields the game itself
    // writes here.
    unsafe {
        this.add(ds2_rva::FE_DIALOG_RESULT_OFFSET).write(result);
        this.add(ds2_rva::FE_DIALOG_PHASE_OFFSET)
            .write(closed_phase);
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
        "{LOG_PREFIX} suppressed screen={} kind={kind} cancel-dest=0x{cancel_dest:02x} \
         confirm-dest=0x{confirm_dest:02x} edge={edge} result={result} phase={closed_phase} \
         total={total}",
        dialog.name,
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
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    if unsafe { suppress(this) } {
        return;
    }
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one every caller of the shared `enter` uses.
        let original: EnterFn = unsafe { std::mem::transmute::<usize, EnterFn>(trampoline) };
        // SAFETY: `original` is the trampoline MinHook produced for this target, so calling it runs the
        // bytes the detour displaced. The arguments are this detour's own, passed through untouched.
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
    // SAFETY: `MH_Initialize` takes no arguments and is documented as safe to call again on an
    // already-initialised library, which the status below distinguishes.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome { installed: false };
    }

    let site = base + ds2_rva::FE_DIALOG_ENTER as usize;
    // SAFETY: the target is an RVA this crate validated against the prologue it expects before
    // reaching here, and the detour is a `'static` fn item of the matching ABI.
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
    // SAFETY: the target is the address `MhHook::new` above already registered with MinHook.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn common_window() -> Kinds {
        DIALOGS
            .iter()
            .find(|dialog| dialog.name == "common-window")
            .expect("common-window is allowlisted")
            .kinds
    }

    #[test]
    fn a_common_window_is_suppressed_only_for_its_vouched_kinds() {
        let kinds = common_window();
        // The two boot notices, and the offline prompt the offline answer needs to reach.
        assert!(kind_allowed(kinds, 6));
        assert!(kind_allowed(kinds, 70));
        assert!(kind_allowed(kinds, 62));
    }

    #[test]
    fn the_load_and_save_failures_are_always_shown() {
        let kinds = common_window();
        assert!(!kind_allowed(
            kinds,
            ds2_rva::FE_COMMON_WINDOW_KIND_LOAD_CHARACTER_FAILED
        ));
        assert!(!kind_allowed(
            kinds,
            ds2_rva::FE_COMMON_WINDOW_KIND_SAVE_FAILED
        ));
    }

    #[test]
    fn an_unknown_or_unreadable_kind_is_shown() {
        let kinds = common_window();
        assert!(!kind_allowed(kinds, 90));
        assert!(!kind_allowed(kinds, -1));
    }

    #[test]
    fn the_login_refusal_is_suppressed_only_on_a_seamless_run() {
        let kinds = common_window();
        let refused = ds2_rva::FE_COMMON_WINDOW_KIND_LOGIN_REFUSED;
        assert!(!kind_suppressible(kinds, refused, false));
        assert!(kind_suppressible(kinds, refused, true));
        // The flag vouches for that one message and nothing else.
        assert!(!kind_suppressible(
            kinds,
            ds2_rva::FE_COMMON_WINDOW_KIND_LOAD_CHARACTER_FAILED,
            true
        ));
        assert!(!kind_suppressible(kinds, 90, true));
    }

    #[test]
    fn a_single_message_class_is_suppressed_whatever_its_kind() {
        assert!(kind_allowed(Kinds::Any, 42));
        assert!(kind_allowed(Kinds::Any, -1));
    }
}
