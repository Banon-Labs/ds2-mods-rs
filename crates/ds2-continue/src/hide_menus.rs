//! Keep the title screen and the character list off the screen while the shortcut walks past them.
//!
//! # The two screens need two different mechanisms, and that is the whole story
//!
//! | screen | what it is | how it is hidden |
//! | --- | --- | --- |
//! | character list | `FeGroupTitleDataList` | the frontend's own [`ds2_rva::FE_GROUP_CLOSE`] |
//! | title screen | `FeSceneTitle` | posed hidden, [`ds2_rva::FE_SCENE_TITLE_POSE_HIDDEN`] |
//!
//! The character list is closed and stays closed because the shortcut leaves it slowly -- it goes
//! on to `LoadProfile`, so the close animation has frames to finish in.
//!
//! The title screen does not, and that is what defeated the first two attempts at it. Its own close
//! (`0x1400f3590`) works exactly as advertised; it just *animates*, over a span the substate's ~25ms
//! residency cannot cover. Calling it every frame from the update kept restarting a fade that never
//! reached its end, so the menu was visible the entire time -- which is precisely what was reported
//! from the last run, and it was the close being slow rather than the close being wrong.
//!
//! # What the title screen actually is
//!
//! One object. `[`[`ds2_rva::FE_TITLE_CONTEXT`]`] + 0x80` is a `FeSceneTitle`, and it carries the
//! logo, the PRESS ANY BUTTON prompt and the six menu rows together -- which is why substate `0x17`
//! and substate `0x47` both poll it, and why a player experiences them as one screen. Hiding it
//! therefore hides the logo and the menu in a single call, rather than needing a cover placed over
//! them.
//!
//! Two crates had disagreed about that pointer -- `ds2-dialog-skip` called it the title scene,
//! this one called it the top-menu group -- and the disagreement is what hid the answer for two
//! attempts. Its constructor settles it; see [`ds2_rva::FE_SCENE_TITLE_POSE_HIDDEN`].
//!
//! # It leaves nothing behind
//!
//! The pose is a playback position and nothing else: no open flag is cleared, no group is torn
//! down, and no counter is moved. So there is no restore to get wrong, and a player who returns to
//! the title screen from in-game gets it back from the game's own next open. That is deliberate --
//! the two earlier attempts here both failed by changing state, one of them fatally.
//!
//! # It patches nothing
//!
//! There is no detour in this module. Both calls are made from detours the crate already owns for
//! the shortcut itself, so the feature adds no patched sites.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// `FeGroupBase::close(group)` and `FeGroupBase::v2(scene)` share a shape: one argument, and the
/// body reads only `this`.
type OneArgFn = unsafe extern "system" fn(*mut u8);

/// Whether `[continue] hide_menus` asked for this.
static ENABLED: AtomicU32 = AtomicU32::new(0);

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// One bit per screen, so the first action on each is logged and the rest are silent. The title
/// pose is re-asserted every frame the top menu is resident.
static LOGGED: AtomicU32 = AtomicU32::new(0);

const LOG_BIT_TITLE: u32 = 1;
const LOG_BIT_DATA_LIST: u32 = 2;

/// Turn the hiding on. Call before [`crate::install`].
pub fn set_enabled(enabled: bool) {
    ENABLED.store(u32::from(enabled), Ordering::Release);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire) != 0
}

pub(crate) fn set_module_base(base: usize) {
    MODULE_BASE.store(base, Ordering::Release);
}

/// Resolve `[`[`ds2_rva::FE_TITLE_CONTEXT`]`] + offset`, the way the game's own methods open.
///
/// `FeSubStateTitleTopMenu::v3` is `mov rax,[0x14160de10]; mov rbx,[rax+0x80]`, and
/// `FeSubStateTitleLoadDataList::v1` is the same pair with `+0x98`. Reading the receiver the way
/// the game reads it is the check that was skipped by both failed attempts here.
///
/// # Safety
///
/// Must run on a game thread with the image mapped. Every dereference is null-checked.
unsafe fn title_context_slot(offset: usize) -> Option<(usize, *mut u8)> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return None;
    }
    let context = (base + ds2_rva::FE_TITLE_CONTEXT as usize) as *const *mut u8;
    // SAFETY: the same load the game makes at the head of both substate methods.
    let context = unsafe { context.read() };
    if context.is_null() {
        return None;
    }
    // SAFETY: `context` is the live title context and the offset is the one the game indexes.
    let object = unsafe { context.add(offset).cast::<*mut u8>().read() };
    if object.is_null() {
        return None;
    }
    Some((base, object))
}

/// Pose the title screen hidden -- logo, prompt and menu rows together.
///
/// Idempotent, so it is meant to be called every frame the top menu is resident: the play writes a
/// playback position and re-writing the same one changes nothing.
///
/// # Safety
///
/// Must run on a game thread, after the original of whichever detour calls it, so the scene exists.
/// The call itself is the game's own [`ds2_rva::FE_SCENE_TITLE_POSE_HIDDEN`], which null-checks the
/// scene it dereferences before touching it.
pub(crate) unsafe fn pose_title_hidden() {
    if !enabled() {
        return;
    }
    let Some((base, scene)) = (unsafe { title_context_slot(ds2_rva::FE_TITLE_CONTEXT_TOP_MENU_GROUP_OFFSET) })
    else {
        return;
    };
    // The `FeSceneTitle` open byte, logged raw on the first pose. It is the field that told the
    // last attempt it had the wrong class -- the generic close reads `+0x30`, which on this object
    // is not a boolean and read 248. On the right class this reads 1 while the screen is up, so a
    // 1 here is the receiver confirming itself.
    // SAFETY: `scene` is a live `FeSceneTitle`, which is larger than this offset.
    let open = unsafe { scene.add(0xf1).read_volatile() };
    // SAFETY: an ordinary function entry in the mapped image, called with the single argument its
    // own body establishes. It is `FeSceneTitle::v2` minus that override's counter increment, so it
    // changes a playback position and nothing else.
    let pose: OneArgFn = unsafe {
        std::mem::transmute::<usize, OneArgFn>(base + ds2_rva::FE_SCENE_TITLE_POSE_HIDDEN as usize)
    };
    unsafe { pose(scene) };

    if LOGGED.fetch_or(LOG_BIT_TITLE, Ordering::Relaxed) & LOG_BIT_TITLE == 0 {
        log(format_args!(
            "{LOG_PREFIX} hide-menu title-screen pose=0x{:08x} sequence=0x{:x} open={open} posed",
            ds2_rva::FE_SCENE_TITLE_POSE_HIDDEN,
            ds2_rva::FE_SCENE_TITLE_SEQUENCE_HIDDEN
        ));
    }
}

/// Close the character list's group, if it is open.
///
/// Unlike the title screen this really is a close: the list is left slowly enough for the close
/// sequence to finish, and it was confirmed hidden on screen.
///
/// # Safety
///
/// Must run on a game thread with the image mapped, and after the original of whichever detour
/// calls it -- the group has to exist before it can be closed. Every dereference is null-checked.
pub(crate) unsafe fn close_data_list() {
    if !enabled() {
        return;
    }
    let Some((base, group)) =
        (unsafe { title_context_slot(ds2_rva::FE_TITLE_CONTEXT_DATA_LIST_GROUP_OFFSET) })
    else {
        return;
    };
    // The byte the generic close treats as "is it open". Logged raw rather than as a boolean, for
    // the reason the title pose logs its own: a field that is not what you think it is will say so.
    // SAFETY: `group` is a live object at least this large.
    let flag = unsafe {
        group
            .add(ds2_rva::FE_GROUP_OPEN_FLAG_OFFSET)
            .read_volatile()
    };
    // SAFETY: an ordinary function entry in the mapped image, called with the single argument its
    // own body establishes -- the same call the game's own substate makes on this same pointer.
    let close: OneArgFn = unsafe {
        std::mem::transmute::<usize, OneArgFn>(base + ds2_rva::FE_GROUP_CLOSE as usize)
    };
    unsafe { close(group) };

    if LOGGED.fetch_or(LOG_BIT_DATA_LIST, Ordering::Relaxed) & LOG_BIT_DATA_LIST == 0 {
        log(format_args!(
            "{LOG_PREFIX} hide-menu data-list close=0x{:08x} flag={flag} called",
            ds2_rva::FE_GROUP_CLOSE
        ));
    }
}
