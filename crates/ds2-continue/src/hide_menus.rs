//! Close the two menus the shortcut walks through, using the frontend's own close.
//!
//! # What this replaces, and why the first two attempts failed
//!
//! The goal was to hide the title flow behind the game's own NOW LOADING page. Two levers were
//! tried on `FeOperatorNowLoading` and both are dead:
//!
//! * **Vtable slot 4, read as opacity** because the operator factory calls it with `0.0f` right
//!   after construction. Setting it to `1.0` changed nothing on screen.
//! * **Vtable slot 24 with `(0x65, true, 0.0)`,** copied from the game's own switch at
//!   `0x1405116f0`. It crashed the process. Those operator vtables have **eleven** slots, so
//!   `+0xc0` read the string data that follows and called it.
//!
//! The second failure is the instructive one. The call site was real; the receiver was not.
//! `[rdi+0xc8]` and `[rdi+0xd0]` there are **scenes**, and `0x65`/`0x66` are **sequence ids**, not
//! operator screen ids -- the same family as the `0x67` `ds2-dialog-skip` already plays on
//! `FeSceneTitle`. Two different classes happened to keep something at `+0xc8` and `+0xd0`, and
//! matching a displacement was taken for an identification.
//!
//! # What is used instead
//!
//! `FeGroupBase::close` at [`ds2_rva::FE_GROUP_CLOSE`], which is the whole of the frontend's "make
//! this screen go away":
//!
//! ```c
//! scene = group->_0x08;
//! if (scene && group->_0x30) {
//!     play_sequence(scene, 0x68, 0, 0.0f);   // 0x66 open, 0x67 settled, 0x68 close
//!     scene->_0x18 += 1;
//!     group->_0x30 = 0;
//! }
//! ```
//!
//! Nothing here indexes a vtable. It calls one ordinary function, with a pointer loaded out of
//! `FeTitleContext` by exactly the two instructions the game's own code uses:
//! `FeSubStateTitleTopMenu::v3` opens `mov rax,[0x14160de10]; mov rbx,[rax+0x80]`, and
//! `FeSubStateTitleLoadDataList::v1` opens the same pair with `+0x98`. **The receiver is read the
//! way the game reads it**, which is the check that was skipped twice.
//!
//! # It patches nothing
//!
//! There is no detour here. The two calls are made from `detour_top_menu` and `detour_enter`,
//! which this crate already owns for the shortcut itself, so the feature adds no patched sites.
//!
//! # Why it is safe to call from a per-frame detour
//!
//! Both the open and the close test `group->_0x30` before doing anything, so a second close is a
//! no-op. That is the game's own guard, not one added here.
//!
//! # What it does not do
//!
//! It hides two menus. It does not put anything in their place, and it does not touch the logos or
//! the title screen. Covering the whole flow still wants the NOW LOADING page, and the call that
//! raises that is still unknown -- but it will be a sequence played on a scene, not a slot on an
//! operator.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// `FeGroupBase::close(group)`. One argument; the body reads only `this`.
type GroupCloseFn = unsafe extern "system" fn(*mut u8);

/// Whether `[continue] hide_menus` asked for this.
static ENABLED: AtomicU32 = AtomicU32::new(0);

static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// One bit per menu, so the first close of each is logged and the rest are silent. The top-menu
/// detour runs every frame its substate is resident.
static LOGGED: AtomicU32 = AtomicU32::new(0);

/// Which menu a close is aimed at.
#[derive(Clone, Copy)]
pub(crate) enum Menu {
    /// `FeGroupTitleTopMenu`, the six-row menu after PRESS ANY BUTTON.
    TopMenu,
    /// `FeGroupTitleDataList`, the character list.
    DataList,
}

impl Menu {
    /// Each menu's OWN close. They are different functions on different classes, and using the
    /// data list's on the top menu is what made the first attempt hide only one of the two.
    fn close_rva(self) -> u32 {
        match self {
            Self::TopMenu => ds2_rva::FE_GROUP_TITLE_TOP_MENU_CLOSE,
            Self::DataList => ds2_rva::FE_GROUP_CLOSE,
        }
    }

    fn offset(self) -> usize {
        match self {
            Self::TopMenu => ds2_rva::FE_TITLE_CONTEXT_TOP_MENU_GROUP_OFFSET,
            Self::DataList => ds2_rva::FE_TITLE_CONTEXT_DATA_LIST_GROUP_OFFSET,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::TopMenu => "top-menu",
            Self::DataList => "data-list",
        }
    }

    fn bit(self) -> u32 {
        match self {
            Self::TopMenu => 1,
            Self::DataList => 2,
        }
    }
}

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

/// Close one menu's group, if it is open.
///
/// # Safety
///
/// Must run on a game thread with the image mapped, and after the original of whichever detour
/// calls it -- the group has to exist before it can be closed. Every dereference is null-checked.
pub(crate) unsafe fn close(menu: Menu) {
    if !enabled() {
        return;
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let context = (base + ds2_rva::FE_TITLE_CONTEXT as usize) as *const *mut u8;
    // SAFETY: the same load the game makes at the head of both substate methods.
    let context = unsafe { context.read() };
    if context.is_null() {
        return;
    }
    // SAFETY: `context` is the live title context and the offset is the one the game indexes.
    let group = unsafe { context.add(menu.offset()).cast::<*mut u8>().read() };
    if group.is_null() {
        return;
    }
    // The byte the generic close treats as "is it open". Logged raw rather than as a boolean,
    // because it is only meaningful for a class that actually uses that layout -- and the one time
    // it read 248 instead of 0 or 1, that was the signal the wrong close was being used.
    // SAFETY: `group` is a live object at least this large.
    let flag = unsafe {
        group
            .add(ds2_rva::FE_GROUP_OPEN_FLAG_OFFSET)
            .read_volatile()
    };
    // SAFETY: an ordinary function entry in the mapped image, called with the single argument its
    // own body establishes -- the same call the game's own substate makes on this same pointer.
    let close: GroupCloseFn =
        unsafe { std::mem::transmute::<usize, GroupCloseFn>(base + menu.close_rva() as usize) };
    unsafe { close(group) };

    if LOGGED.fetch_or(menu.bit(), Ordering::Relaxed) & menu.bit() == 0 {
        log(format_args!(
            "{LOG_PREFIX} hide-menu {} close=0x{:08x} flag={flag} called",
            menu.name(),
            menu.close_rva()
        ));
    }
}
