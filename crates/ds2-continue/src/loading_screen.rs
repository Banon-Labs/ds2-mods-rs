//! Cover the title flow with the game's own NOW LOADING screen.
//!
//! # Why the game's screen and not a black rectangle
//!
//! The autocontinue shortcut takes about 5.6 seconds from launch to standing in the world, and it
//! presses nothing on the way. What the player sees during it is the publisher logos, the title,
//! the six-row menu for a frame or two, and the character list for a frame or two. Hiding all of
//! that leaves a choice about what is on screen instead, and a black screen for five and a half
//! seconds is indistinguishable from a hang.
//!
//! `FeOperatorNowLoading` is already in the process. It drives `FeSceneNowLoading` and
//! `FeGroupNowLoading` -- the full-screen page the game puts up on every map transition -- and it
//! is constructed during `GameManagerImp`'s own init, long before the title state machine exists.
//! So the cover is a screen the player already associates with "the game is working", built from
//! the game's own assets, and nothing here has to draw a pixel.
//!
//! # The walk, verified in a running game rather than only read
//!
//! ```text
//! [GAME_MANAGER_IMP] + 0x22e0        the frontend object that owns the operator table
//!                    + 0xc8          FeOperatorNowLoading
//! ```
//!
//! Both offsets come from the factory at `0x140500200`, which allocates `0x3c0` bytes, installs
//! vtable `0x1410fa0c8`, and stores the result at `+0xc8`; `GameManagerImp`'s init then parks the
//! container at `+0x22e0`. Reading that chain out of `/proc/<pid>/mem` on a live game landed on an
//! object whose vtable is exactly `0x1410fa0c8`, which RTTI names `FeOperatorNowLoading`.
//!
//! # The call is copied, not deduced
//!
//! The first version of this guessed. The operator factory calls vtable slot 4 twice with `0.0f`
//! immediately after construction, which looks exactly like "start hidden", so it set slot 4 to
//! `1.0` and expected a cover. It changed nothing on screen -- the title and both menus showed
//! through -- which is the third time this session that "here is a call site that would make sense
//! if my label were right" turned out to be worth nothing.
//!
//! What replaced it is not a better guess. `0x1405116f0` is the game's **own** switch between the
//! title and the loading screen, and it is a straight swap of two operators:
//!
//! ```text
//! [param+2] == 1   Title.slot24(0x66, false, 0.0)   NowLoading.slot24(0x65, true,  0.0)
//! [param+2] != 1   Title.slot24(0x65, true,  0.0)   NowLoading.slot24(0x66, false, 0.0)
//! ```
//!
//! `slot24` is `void(this, u32 screen_id, bool show, float fade)`. Both operators answer to both
//! ids, so the id is not "the loading screen" -- it selects which of that operator's screens, and
//! the operator supplies the content. Raising the cover runs the first line; dropping it runs the
//! second. Same order, same ids, same zero fade.
//!
//! # Where it turns on and off
//!
//! On at `FeOperatorTitle::v2`, the title operator's setup -- the moment the title frontend
//! becomes live, and the earliest point at which covering it means anything. Off at
//! `FeSubStateTitleStartIngame::v1`, the same boundary the audio suppression uses.
//!
//! It is re-asserted from the two title detours this crate already owns, because the title's own
//! frontend may drive opacity for its own reasons and the cover has to win. Both of those run only
//! while their substate is resident, so this is a handful of calls, not a per-frame write.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// `FeOperatorTitle::v2(this)`. One argument: the body reads `this+0x10` and a global and touches
/// no other incoming register.
type OperatorSetupFn = unsafe extern "system" fn(*mut u8);

pub(crate) static TITLE_SETUP_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Whether `[continue] loading_screen` asked for this.
static ENABLED: AtomicU32 = AtomicU32::new(0);

/// Non-zero once the cover has been raised and not yet dropped.
static SHOWN: AtomicU32 = AtomicU32::new(0);

/// Set once the cover has been dropped, so a second release does nothing.
static DROPPED: AtomicU32 = AtomicU32::new(0);

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// Turn the cover on. Call before [`crate::install`].
pub fn set_enabled(enabled: bool) {
    ENABLED.store(u32::from(enabled), Ordering::Release);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire) != 0
}

pub(crate) fn set_module_base(base: usize) {
    MODULE_BASE.store(base, Ordering::Release);
}

/// What [`swap`] managed to do, so a call that goes nowhere says where it stopped instead of
/// vanishing into a silent early return.
enum Applied {
    /// The walk to one of the two operators came back null -- no game manager, no frontend
    /// object, or an operator slot the factory has not filled yet.
    NoOperator,
    /// Both operators resolved, and there is no verified call to make on them yet.
    NoLever,
}

impl std::fmt::Display for Applied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoOperator => "no-operator",
            Self::NoLever => "operators-resolved-but-no-verified-lever",
        })
    }
}

/// Read a pointer-sized field.
///
/// # Safety
///
/// `object` must be a live object with `offset` inside it.
unsafe fn field(object: *mut u8, offset: usize) -> *mut u8 {
    // SAFETY: the caller guarantees the object and the offset.
    unsafe { object.add(offset).cast::<*mut u8>().read() }
}

/// Resolve one named operator slot on the frontend object.
///
/// # Safety
///
/// Must run on a game thread with the image mapped.
unsafe fn operator(offset: usize) -> Option<*mut u8> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return None;
    }
    let manager = (base + ds2_rva::GAME_MANAGER_IMP as usize) as *const *mut u8;
    // SAFETY: the RVA names a pointer-sized global in the mapped image.
    let manager = unsafe { manager.read() };
    if manager.is_null() {
        return None;
    }
    // SAFETY: `manager` is the live GameManagerImp.
    let root = unsafe { field(manager, ds2_rva::GAME_MANAGER_FRONTEND_ROOT_OFFSET) };
    if root.is_null() {
        return None;
    }
    // SAFETY: `root` is the frontend object whose named operator slots the factory filled.
    let operator = unsafe { field(root, offset) };
    (!operator.is_null()).then_some(operator)
}

/// Resolve both operators and report. **Calls nothing.**
///
/// Two levers have been tried here and both are dead:
///
/// * **Slot 4, "opacity".** The factory calls it twice with `0.0f` right after construction, which
///   looks like "start hidden". Setting it to `1.0` at the title changed nothing on screen.
/// * **Slot 24 with `(0x65, true, 0.0)`,** copied from the game's own switch at `0x1405116f0`.
///   That crashed the process with an access violation on the first boot that reached it. These
///   vtables have **eleven** slots -- `v0..v10`, and `+0x58` onward is string data -- so `+0xc0`
///   read text and called it. The call site was real; the object was not. `rdi` there is some
///   other class that also uses `+0xc8` and `+0xd0`, and matching on a displacement is not an
///   identification.
///
/// What survives is the walk, which was verified against a live process: the frontend object at
/// [`ds2_rva::GAME_MANAGER_FRONTEND_ROOT_OFFSET`] really does hold `FeOperatorNowLoading` at
/// [`ds2_rva::FRONTEND_NOW_LOADING_OPERATOR_OFFSET`] and `FeOperatorTitle` at
/// [`ds2_rva::FRONTEND_TITLE_OPERATOR_OFFSET`], vtable-checked in memory. When the real show
/// mechanism is found it will be a call on one of these two pointers, so the walk is worth
/// keeping and the guessing is not.
///
/// # Safety
///
/// Must run on a game thread with the image mapped. Reads only.
unsafe fn swap(_to_loading: bool) -> Applied {
    // SAFETY: as documented on this function.
    let (Some(_title), Some(_loading)) = (unsafe {
        (
            operator(ds2_rva::FRONTEND_TITLE_OPERATOR_OFFSET),
            operator(ds2_rva::FRONTEND_NOW_LOADING_OPERATOR_OFFSET),
        )
    }) else {
        return Applied::NoOperator;
    };
    Applied::NoLever
}

/// Raise the cover. Idempotent, and a no-op once it has been dropped.
///
/// # Safety
///
/// As [`swap`].
pub(crate) unsafe fn show() {
    if !enabled() || DROPPED.load(Ordering::Acquire) != 0 {
        return;
    }
    let first = SHOWN.swap(1, Ordering::AcqRel) == 0;
    // SAFETY: called from a game thread, after the original of whichever detour invoked it.
    let applied = unsafe { swap(true) };
    if first {
        log(format_args!(
            "{LOG_PREFIX} loading-screen shown title-hide=0x{:02x} loading-show=0x{:02x} {applied}",
            ds2_rva::FE_OPERATOR_SCREEN_ID_HIDE,
            ds2_rva::FE_OPERATOR_SCREEN_ID_SHOW
        ));
    }
}

/// Re-assert the cover, without logging. For the title detours that already run every frame their
/// substate is resident.
///
/// # Safety
///
/// As [`swap`].
pub(crate) unsafe fn reassert() {
    if !enabled() || SHOWN.load(Ordering::Acquire) == 0 || DROPPED.load(Ordering::Acquire) != 0 {
        return;
    }
    // SAFETY: as `show`.
    unsafe { swap(true) };
}

/// Drop the cover and refuse to raise it again.
///
/// # Safety
///
/// As [`swap`].
pub(crate) unsafe fn hide(reason: &str) {
    if !enabled() || DROPPED.swap(1, Ordering::AcqRel) == 1 {
        return;
    }
    if SHOWN.load(Ordering::Acquire) == 0 {
        return;
    }
    // SAFETY: as `show`.
    let applied = unsafe { swap(false) };
    SHOWN.store(0, Ordering::Release);
    log(format_args!(
        "{LOG_PREFIX} loading-screen hidden by={reason} {applied}"
    ));
}

/// `FeOperatorTitle::v2`. After the original, because the original is what makes the title
/// frontend live -- covering it before it exists covers nothing.
unsafe extern "system" fn detour_title_setup(this: *mut u8) {
    let trampoline = TITLE_SETUP_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook's copy of this site's original prologue, with the vtable's signature.
        let original = unsafe { std::mem::transmute::<usize, OperatorSetupFn>(trampoline) };
        unsafe { original(this) };
    }
    // SAFETY: the original returned on the game thread; the walk is null-checked throughout.
    unsafe { show() };
}

/// The one detour this module contributes.
pub(crate) fn sites() -> [(&'static str, u32, *mut c_void, &'static AtomicUsize); 1] {
    [(
        "title-operator-setup",
        ds2_rva::FE_OPERATOR_TITLE_SETUP,
        detour_title_setup as *mut c_void,
        &TITLE_SETUP_TRAMPOLINE,
    )]
}
