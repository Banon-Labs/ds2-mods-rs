//! The fade from black that holds the title flow back.
//!
//! `FeOperatorTitle::v4` starts a 2.0 s fade from black and then, in its state 3, waits every
//! frame until that fade has finished before it starts the title flow. There is nothing on the
//! screen during it: the logos and warnings it would reveal are already skipped. So this detours
//! the operator's update and, when it is in that state, finishes the fade on the spot the way the
//! game's own fade start does for a zero duration -- remaining time `0.0`, current opacity set to
//! the target -- and lets the original run. The original then sees no fade running and starts the
//! flow the same frame.
//!
//! Writing only the remaining time would leave the current opacity where it was, and nothing
//! would move it on again: the screen would stay black. Setting it to the target is the second
//! half of what `0x140b23fe0` does for a zero duration.

use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// `void update(FeOperatorTitle *this)`: one argument, `this` in RCX, no return value.
type UpdateFn = unsafe extern "system" fn(*mut u8);

static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
/// Times the fade was cut short. Logged each time, since it happens once per trip to the title.
static SKIPPED: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn detour(this: *mut u8) {
    // SAFETY: `this` is the live `FeOperatorTitle` the game passed, and the reads and writes are
    // at offsets `ds2-rva` records from the function's own disassembly.
    unsafe { finish_fade_if_waiting(this) };
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, with this signature.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
        // SAFETY: the original, called with the argument the game passed.
        unsafe { original(this) };
    }
}

/// If the operator is waiting on the boot fade, finish the fade now.
///
/// # Safety
///
/// `this` must be the live `FeOperatorTitle`.
unsafe fn finish_fade_if_waiting(this: *mut u8) {
    if this.is_null() {
        return;
    }
    // SAFETY: the state is the `u32` the update's own first instruction reads.
    let state = unsafe {
        this.add(ds2_rva::FE_OPERATOR_TITLE_STATE_OFFSET)
            .cast::<u32>()
            .read()
    };
    if state != ds2_rva::FE_OPERATOR_TITLE_STATE_FADE_WAIT {
        return;
    }
    let Ok(base) = ds2_game_base::mem::game_module_base() else {
        return;
    };
    // SAFETY: guarded reads of the manager global and its fade pointer; `None` on a null or
    // unmapped hop.
    let fade = unsafe {
        ds2_game_base::mem::safe_read_usize(base + ds2_rva::GAME_MANAGER_IMP as usize)
            .filter(|&manager| manager != 0)
            .and_then(|manager| {
                ds2_game_base::mem::safe_read_usize(
                    manager + ds2_rva::GAME_MANAGER_SCREEN_FADE_OFFSET,
                )
            })
            .filter(|&fade| fade != 0)
    };
    let Some(fade) = fade else {
        return;
    };
    let fade = fade as *mut f32;
    // SAFETY: the fade object's three floats, at the offsets `0x140b23fe0` and `0x14039ab20` use;
    // the game reads and writes them on this same thread every frame.
    unsafe {
        let remaining = fade.byte_add(ds2_rva::SCREEN_FADE_REMAINING_OFFSET).read();
        if remaining <= 0.0 {
            return;
        }
        let current = fade.byte_add(ds2_rva::SCREEN_FADE_CURRENT_OFFSET).read();
        let target = fade.byte_add(ds2_rva::SCREEN_FADE_TARGET_OFFSET).read();
        fade.byte_add(ds2_rva::SCREEN_FADE_REMAINING_OFFSET)
            .write(0.0);
        fade.byte_add(ds2_rva::SCREEN_FADE_CURRENT_OFFSET)
            .write(target);
        let n = SKIPPED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} skipped screen=title-fade remaining={remaining:.3}s current={current} \
             target={target} count={n}"
        ));
    }
}

/// Detour the title operator's update. True when the hook is live.
///
/// # Safety
///
/// Patches executable memory; call from the same position as the rest of this crate's install,
/// with MinHook already initialised.
pub(crate) unsafe fn install(base: usize) -> bool {
    let site = base + ds2_rva::FE_OPERATOR_TITLE_UPDATE as usize;
    let replacement: UpdateFn = detour;
    // SAFETY: the target is the recorded prologue of an ordinary function, not an Arxan redirect,
    // and the detour has its exact signature.
    let hook = match unsafe { MhHook::new(site as *mut c_void, replacement as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} hook-failed screen=title-fade va=0x{site:016x} \
                 stage=MH_CreateHook status={status:?}"
            ));
            return false;
        }
    };
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: the site `MhHook::new` just registered.
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} hook-failed screen=title-fade va=0x{site:016x} stage=MH_EnableHook \
             status={status:?}"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} hooked screen=title-fade rva=0x{:08x} va=0x{site:016x}",
        ds2_rva::FE_OPERATOR_TITLE_UPDATE
    ));
    true
}
