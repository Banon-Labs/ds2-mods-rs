//! The window-message half of the block: mouse clicks.
//!
//! Clicks do not reach the game through the DirectInput mouse [`crate::device`] blanks. The window
//! procedure hands each mouse event to `ds2_rva::MOUSE_EVENT_FOLD`, which folds it into each input
//! block's button word, so a block that only blanked the devices left a left-click attacking.
//!
//! During a block, a press is rewritten into the release of the same button before the fold runs,
//! and a wheel step into nothing. Rewriting rather than dropping means a button already held when
//! the block starts is let go instead of sticking, and a release is always passed through for the
//! same reason. Moves are passed through: they place the menu pointer and press nothing.
//!
//! Measured with `scripts/frida/click-block.js`: a right-press posted to the game window set
//! bit `0x2` of both blocks' `+0x228`; the same press, rewritten, left it clear on both.

use core::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use ds2_game_base::mem::read_bytes;
use ds2_hook::{MH_EnableHook, MH_STATUS, MhHook};

use crate::log::harness_log;

/// `fn(block, event)`, both in registers, no return value the caller reads.
type FoldFn = unsafe extern "system" fn(*mut u8, *mut u8);

/// The original fold, published before the site is patched.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Presses and wheel steps a block has turned away, for `status`.
static SUPPRESSED: AtomicU64 = AtomicU64::new(0);

/// The release that matches a press, or `None` for anything that is not a press.
const fn release_of(event_type: u32) -> Option<u32> {
    match event_type {
        0 | 2 => Some(1),
        3 | 4 => Some(5),
        6 | 7 => Some(8),
        _ => None,
    }
}

/// The type of a wheel step.
const WHEEL: u32 = 10;

unsafe extern "system" fn detour_fold(block: *mut u8, event: *mut u8) {
    if !event.is_null() && crate::is_blocking() {
        // SAFETY: `event` is the event the message handler is folding, and every offset is one
        // the fold itself reads.
        unsafe {
            let kind = event.add(ds2_rva::MOUSE_EVENT_TYPE_OFFSET).cast::<u32>();
            let current = kind.read_unaligned();
            if let Some(release) = release_of(current) {
                kind.write_unaligned(release);
                let modifiers = event.add(ds2_rva::MOUSE_EVENT_MODIFIERS_OFFSET);
                modifiers.write(modifiers.read() & 0xf0);
                SUPPRESSED.fetch_add(1, Ordering::Relaxed);
            } else if current == WHEEL {
                event
                    .add(ds2_rva::MOUSE_EVENT_WHEEL_OFFSET)
                    .cast::<i32>()
                    .write_unaligned(0);
                SUPPRESSED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the arguments are
        // this detour's own.
        unsafe { std::mem::transmute::<usize, FoldFn>(trampoline)(block, event) };
    }
}

/// How many presses and wheel steps blocks have suppressed so far.
pub(crate) fn suppressed() -> u64 {
    SUPPRESSED.load(Ordering::Relaxed)
}

/// Hook the fold. `true` when the detour is live.
///
/// # Safety
///
/// `base` is the loaded game image, MinHook is initialised, and Arxan has already been neutered.
pub(crate) unsafe fn install(base: usize) -> bool {
    let address = base + ds2_rva::MOUSE_EVENT_FOLD as usize;
    let mut found = [0u8; 5];
    // SAFETY: `read_bytes` reports an unmapped page rather than faulting.
    if !unsafe { read_bytes(address, &mut found) } {
        harness_log!("hook-refused site=mouse-event va=0x{address:016x} reason=unreadable");
        return false;
    }
    if found != ds2_rva::MOUSE_EVENT_FOLD_PROLOGUE {
        let expected = ds2_rva::MOUSE_EVENT_FOLD_PROLOGUE;
        harness_log!(
            "hook-refused site=mouse-event va=0x{address:016x} reason=prologue-moved \
             expected={expected:02x?} found={found:02x?}"
        );
        return false;
    }
    let detour: FoldFn = detour_fold;
    // SAFETY: the prologue was checked above, and the detour has the target's ABI.
    let hook = match unsafe { MhHook::new(address as *mut c_void, detour as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            harness_log!(
                "hook-failed site=mouse-event va=0x{address:016x} stage=MH_CreateHook \
                 status={status:?}"
            );
            return false;
        }
    };
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: the address `MhHook::new` above registered.
    let status = unsafe { MH_EnableHook(address as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        harness_log!(
            "hook-failed site=mouse-event va=0x{address:016x} stage=MH_EnableHook \
             status={status:?}"
        );
        return false;
    }
    harness_log!(
        "hooked site=mouse-event rva=0x{:08x} va=0x{address:016x}",
        ds2_rva::MOUSE_EVENT_FOLD
    );
    true
}

#[cfg(test)]
mod tests {
    use super::release_of;

    #[test]
    fn every_press_becomes_its_own_release() {
        assert_eq!(release_of(0), Some(1));
        assert_eq!(release_of(2), Some(1));
        assert_eq!(release_of(3), Some(5));
        assert_eq!(release_of(4), Some(5));
        assert_eq!(release_of(6), Some(8));
        assert_eq!(release_of(7), Some(8));
    }

    #[test]
    fn releases_moves_and_wheel_are_not_presses() {
        for kind in [1, 5, 8, 9, 10] {
            assert_eq!(release_of(kind), None);
        }
    }
}
