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
//! # The one thing that is NOT proven
//!
//! **That vtable slot 4 is opacity.** The evidence is that the factory calls it twice with
//! `0.0f` immediately after construction, before anything could have shown the screen, and that
//! `FeOperatorNowLoading`'s override forwards the float to both the scene at `+0x10` and the group
//! at `+0x270` -- the shape of an opacity that has to reach every layer.
//!
//! That is an inference from a call site. This crate has already paid for exactly that kind of
//! reasoning once: `MOFmodSoundManager::v0` contains the image's only `EventSystem::update` call
//! and is still not a frame pump. So this ships announcing what it did rather than asserting what
//! happened, and every call logs the value it passed.
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

/// `FeOperatorBase::v4(this, float)` -- the slot the factory calls with `0.0f`.
///
/// MSVC `__thiscall` on x64 puts `this` in `rcx` and a `float` second argument in `xmm1`, which is
/// what `extern "system"` describes.
type OperatorOpacityFn = unsafe extern "system" fn(*mut u8, f32);

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

/// What [`set_opacity`] managed to do, so a call that goes nowhere says where it stopped instead
/// of vanishing into a silent early return.
enum Applied {
    NoModule,
    NoGameManager,
    NoFrontendRoot,
    NoOperator,
    NoVtable,
    Called,
}

impl std::fmt::Display for Applied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoModule => "no-module-base",
            Self::NoGameManager => "no-game-manager",
            Self::NoFrontendRoot => "no-frontend-root",
            Self::NoOperator => "no-operator",
            Self::NoVtable => "no-vtable",
            Self::Called => "called",
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

/// Walk to `FeOperatorNowLoading` and call its opacity slot.
///
/// # Safety
///
/// Must run on a game thread with the image mapped. Every dereference is null-checked, the same
/// way the game's own accessors check.
unsafe fn set_opacity(value: f32) -> Applied {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return Applied::NoModule;
    }
    let manager = (base + ds2_rva::GAME_MANAGER_IMP as usize) as *const *mut u8;
    // SAFETY: the RVA names a pointer-sized global in the mapped image.
    let manager = unsafe { manager.read() };
    if manager.is_null() {
        return Applied::NoGameManager;
    }
    // SAFETY: `manager` is the live GameManagerImp.
    let root = unsafe { field(manager, ds2_rva::GAME_MANAGER_FRONTEND_ROOT_OFFSET) };
    if root.is_null() {
        return Applied::NoFrontendRoot;
    }
    // SAFETY: `root` is the frontend object the factory populated.
    let operator = unsafe { field(root, ds2_rva::FRONTEND_NOW_LOADING_OPERATOR_OFFSET) };
    if operator.is_null() {
        return Applied::NoOperator;
    }
    // SAFETY: `operator` is a live C++ object, so its first word is its vtable.
    let vtable = unsafe { operator.cast::<*mut u8>().read() };
    if vtable.is_null() {
        return Applied::NoVtable;
    }
    // SAFETY: slot 4 of an FeOperatorBase vtable, the slot the game's own factory calls with a
    // float at 0x14050030b.
    let slot = unsafe {
        vtable
            .add(ds2_rva::FE_OPERATOR_OPACITY_VTABLE_SLOT * std::mem::size_of::<usize>())
            .cast::<usize>()
            .read()
    };
    if slot == 0 {
        return Applied::NoVtable;
    }
    // SAFETY: the slot holds `FeOperatorNowLoading::v4`, whose signature `OperatorOpacityFn`
    // describes; this is the identical call the factory makes.
    let opacity = unsafe { std::mem::transmute::<usize, OperatorOpacityFn>(slot) };
    unsafe { opacity(operator, value) };
    Applied::Called
}

/// Raise the cover. Idempotent, and a no-op once it has been dropped.
///
/// # Safety
///
/// As [`set_opacity`].
pub(crate) unsafe fn show() {
    if !enabled() || DROPPED.load(Ordering::Acquire) != 0 {
        return;
    }
    let first = SHOWN.swap(1, Ordering::AcqRel) == 0;
    // SAFETY: called from a game thread, after the original of whichever detour invoked it.
    let applied = unsafe { set_opacity(1.0) };
    if first {
        log(format_args!(
            "{LOG_PREFIX} loading-screen shown opacity=1 {applied}"
        ));
    }
}

/// Re-assert the cover, without logging. For the title detours that already run every frame their
/// substate is resident.
///
/// # Safety
///
/// As [`set_opacity`].
pub(crate) unsafe fn reassert() {
    if !enabled() || SHOWN.load(Ordering::Acquire) == 0 || DROPPED.load(Ordering::Acquire) != 0 {
        return;
    }
    // SAFETY: as `show`.
    unsafe { set_opacity(1.0) };
}

/// Drop the cover and refuse to raise it again.
///
/// # Safety
///
/// As [`set_opacity`].
pub(crate) unsafe fn hide(reason: &str) {
    if !enabled() || DROPPED.swap(1, Ordering::AcqRel) == 1 {
        return;
    }
    if SHOWN.load(Ordering::Acquire) == 0 {
        return;
    }
    // SAFETY: as `show`.
    let applied = unsafe { set_opacity(0.0) };
    SHOWN.store(0, Ordering::Release);
    log(format_args!(
        "{LOG_PREFIX} loading-screen hidden by={reason} opacity=0 {applied}"
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
