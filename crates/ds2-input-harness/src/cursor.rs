//! Hiding the game's own pointer while a panel holds the input.
//!
//! The game decides the OS pointer's visibility once a frame, in `ds2_rva::INPUT_UPDATE`, from
//! the flag at `ds2_rva::INPUT_UPDATE_CURSOR_WANTED_OFFSET`, and calls `ShowCursor` on its own
//! thread until the display count agrees. So while [`crate::is_held`] is set the detour clears
//! that flag for the length of the call and puts the game's value back afterwards: the game
//! hides the pointer itself, on the thread that owns it, and shows it again the first frame after
//! the hold ends. The panel draws its own pointer meanwhile.
//!
//! Before this the pointer's visibility under the save picker depended on whatever the game last
//! wanted: hidden on one run, and on the next shown beside the panel's own (2026-09-27).

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ds2_game_base::mem::read_bytes;
use ds2_hook::{MH_EnableHook, MH_STATUS, MhHook};

use crate::log::harness_log;

/// `fn(this, dt)`: `this` in RCX, a float in XMM1. Whatever is left in RAX is passed back.
type UpdateFn = unsafe extern "system" fn(*mut u8, f32) -> u64;

/// The original update, published before the site is patched.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Whether the last call hid the pointer, so the change is logged once each way.
static HIDING: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn detour_update(this: *mut u8, dt: f32) -> u64 {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return 0;
    }
    // SAFETY: MinHook published this trampoline for exactly this site.
    let original = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
    let hide = crate::is_held() && !this.is_null();
    if HIDING.swap(hide, Ordering::AcqRel) != hide {
        harness_log!(
            "pointer {} -- {}",
            if hide { "hidden" } else { "restored" },
            if hide {
                "a panel holds the input and draws its own"
            } else {
                "the game decides again"
            }
        );
    }
    if !hide {
        // SAFETY: the game's own arguments, passed through.
        return unsafe { original(this, dt) };
    }
    // SAFETY: `this` is the object the update reads the flag from at this offset.
    let wanted = unsafe {
        this.add(ds2_rva::INPUT_UPDATE_CURSOR_WANTED_OFFSET)
            .cast::<u32>()
    };
    // SAFETY: as above; the update runs on this thread, so nothing else writes it meanwhile.
    let kept = unsafe { wanted.read_unaligned() };
    // SAFETY: as above.
    unsafe { wanted.write_unaligned(0) };
    // SAFETY: the game's own arguments, passed through.
    let result = unsafe { original(this, dt) };
    // SAFETY: as above.
    unsafe { wanted.write_unaligned(kept) };
    result
}

/// Hook the input update. `true` when the detour is live.
///
/// # Safety
///
/// `base` is the loaded game image, MinHook is initialised, and Arxan has already been neutered.
pub(crate) unsafe fn install(base: usize) -> bool {
    let address = base + ds2_rva::INPUT_UPDATE as usize;
    let mut found = [0u8; 5];
    // SAFETY: `read_bytes` reports an unmapped page rather than faulting.
    if !unsafe { read_bytes(address, &mut found) } {
        harness_log!("hook-refused site=pointer va=0x{address:016x} reason=unreadable");
        return false;
    }
    if found != ds2_rva::INPUT_UPDATE_PROLOGUE {
        let expected = ds2_rva::INPUT_UPDATE_PROLOGUE;
        harness_log!(
            "hook-refused site=pointer va=0x{address:016x} reason=prologue-moved \
             expected={expected:02x?} found={found:02x?}"
        );
        return false;
    }
    let detour: UpdateFn = detour_update;
    // SAFETY: the prologue was checked above, and the detour has the target's ABI.
    let hook = match unsafe { MhHook::new(address as *mut c_void, detour as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            harness_log!(
                "hook-failed site=pointer va=0x{address:016x} stage=MH_CreateHook \
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
            "hook-failed site=pointer va=0x{address:016x} stage=MH_EnableHook status={status:?}"
        );
        return false;
    }
    harness_log!(
        "hooked site=pointer rva=0x{:08x} va=0x{address:016x}",
        ds2_rva::INPUT_UPDATE
    );
    true
}
