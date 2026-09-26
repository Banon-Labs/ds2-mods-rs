//! `GameManagerImp`, the root singleton most engine services hang off, and its screen fade.
//!
//! Only the fields a crate in this repo reads are named. The rest of the object is padding, so a
//! `GameManagerImp` is never built or copied here: it is reached through
//! [`GameManagerImp::instance`] as a pointer into the game's own memory, and each child pointer is
//! read through a fault-safe accessor rather than a dereference of the whole struct.
//!
//! `docs/DS2-BOOT-WORK.md` has the disassembly behind the screen fade's layout.

use core::ptr::NonNull;

use super::chr::PlayerCtrl;

/// `GameManagerImp`, the singleton at [`ds2_rva::GAME_MANAGER_IMP`].
///
/// Source of name: RTTI. Not registered with DLRF, so nothing in the image states its size; the
/// padding below ends at the last field this repo reads, not at the end of the object.
#[repr(C)]
pub struct GameManagerImp {
    _unk0000: [u8; 0xd0],
    /// The local player, null on the title screen and during a load. The game's own
    /// `PlayerParam` getter takes this hop first.
    pub player_ctrl: Option<NonNull<PlayerCtrl>>,
    _unk00d8: [u8; 0x1088],
    /// The black overlay the game fades in and out of, read by the game's fade start and by its
    /// "is a fade running" query. Created during `GameManagerImp` setup, so null only before it.
    pub screen_fade: Option<NonNull<ScreenFade>>,
}

/// The screen fade: a black overlay's opacity, where it is heading, and how long it has left.
///
/// A prefix of the object: these are the fields the game's fade start, updater and "is a fade
/// running" query use, and the object may extend past them. So it is only ever read and written
/// through a pointer into the game's memory, never copied out whole.
///
/// Starting a fade to clear writes the duration to [`ScreenFade::remaining`] and `0.0` to
/// [`ScreenFade::target`], and for a duration that is not positive also writes `0.0` to
/// [`ScreenFade::current`] at once. A fade counts as running while `remaining > 0.0`. Read live
/// on 2026-09-26 in the world with `scripts/frida/title-fade.js`: all three `0.0`, a finished fade.
#[repr(C)]
pub struct ScreenFade {
    /// Current opacity of the overlay, `0.0` clear to `1.0` black.
    pub current: f32,
    /// The opacity the fade is moving towards.
    pub target: f32,
    /// Seconds left before `current` reaches `target`.
    pub remaining: f32,
}

#[cfg(windows)]
impl GameManagerImp {
    /// The live `GameManagerImp`, or `None` before the game has created it.
    ///
    /// Read with a fault-safe read, so an unresolvable module base or an unmapped global answers
    /// `None` instead of faulting.
    pub fn instance() -> Option<NonNull<Self>> {
        let base = ds2_game_base::mem::game_module_base().ok()?;
        // SAFETY: `safe_read_usize` validates the range in the kernel and answers `None` for an
        // unmapped one; the value is the singleton pointer the game keeps at this global.
        let manager = unsafe {
            ds2_game_base::mem::safe_read_usize(base + ds2_rva::GAME_MANAGER_IMP as usize)
        }?;
        NonNull::new(manager as *mut Self)
    }

    /// The local player's `PlayerCtrl`, or `None` when the pointer is null or unreadable.
    pub fn player_ctrl(manager: NonNull<Self>) -> Option<NonNull<PlayerCtrl>> {
        let field = manager.as_ptr() as usize + core::mem::offset_of!(Self, player_ctrl);
        // SAFETY: a fault-safe read of one pointer-sized field; `None` on an unmapped address.
        let player = unsafe { ds2_game_base::mem::safe_read_usize(field) }?;
        NonNull::new(player as *mut PlayerCtrl)
    }

    /// The screen fade `manager` points at, or `None` when the pointer is null or unreadable.
    pub fn screen_fade(manager: NonNull<Self>) -> Option<NonNull<ScreenFade>> {
        let field = manager.as_ptr() as usize + core::mem::offset_of!(Self, screen_fade);
        // SAFETY: a fault-safe read of one pointer-sized field; `None` on an unmapped address.
        let fade = unsafe { ds2_game_base::mem::safe_read_usize(field) }?;
        NonNull::new(fade as *mut ScreenFade)
    }
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::{GameManagerImp, ScreenFade};

    // Each value is the offset `ds2-rva` held before these types replaced it, as read from the
    // game's fade start and fade-running query and confirmed live by `scripts/frida/title-fade.js`.

    // Read live on 2026-09-26 by `scripts/frida/player-ctrl.js`: a non-null pointer whose vtable
    // is PlayerCtrl's.
    #[test]
    fn game_manager_player_ctrl_is_at_0xd0() {
        assert_eq!(offset_of!(GameManagerImp, player_ctrl), 0xd0);
    }

    #[test]
    fn game_manager_screen_fade_is_at_0x1160() {
        assert_eq!(offset_of!(GameManagerImp, screen_fade), 0x1160);
    }

    #[test]
    fn screen_fade_fields_are_current_target_remaining() {
        assert_eq!(offset_of!(ScreenFade, current), 0x00);
        assert_eq!(offset_of!(ScreenFade, target), 0x04);
        assert_eq!(offset_of!(ScreenFade, remaining), 0x08);
    }
}
