//! The detour itself, created by the first registration.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::{game_rva, read_bytes};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::registry::{Registry, TickFn, When};

/// `NET_SESSION_UPDATE(this, f32 delta)`. The delta is a float in `xmm1`; declaring it as an
/// integer would compile and hand the original whatever that register held.
type NetSessionUpdate = unsafe extern "system" fn(usize, f32);

static REGISTRY: Registry = Registry::new();

/// Trampoline back to the real update, published before the site is patched.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// `None` until the first registration tries to hook; then whether that worked, and what it said.
static HOOKED: Mutex<Option<Result<usize, String>>> = Mutex::new(None);

/// Register `callback` to run `when` relative to the net session update, creating the one shared
/// detour on the first call.
///
/// Returns the detour's address, or why there is no detour. A failure is sticky: later callers get
/// the same message rather than another attempt at a site that already refused.
///
/// # Errors
///
/// When the site does not hold the net session update's prologue, MinHook refuses it, or the
/// callback list is full. Nothing is patched in the first two cases.
///
/// # Safety
///
/// The first call patches executable memory in the loaded game image. Call from the loader's
/// post-Arxan install position.
pub unsafe fn register(when: When, callback: TickFn) -> Result<usize, String> {
    let mut hooked = HOOKED.lock().unwrap_or_else(|poison| poison.into_inner());
    if hooked.is_none() {
        // SAFETY: the caller's contract.
        *hooked = Some(unsafe { hook() });
    }
    let site = hooked
        .clone()
        .unwrap_or_else(|| Err("unreachable".to_string()))?;
    REGISTRY
        .add(when, callback)
        .map_err(|error| format!("{} {error}", crate::LOG_PREFIX))?;
    Ok(site)
}

unsafe fn hook() -> Result<usize, String> {
    let prefix = crate::LOG_PREFIX;
    let site = game_rva(ds2_rva::NET_SESSION_UPDATE)
        .map_err(|error| format!("{prefix} no module base -- {error}"))?;
    let expected = ds2_rva::NET_SESSION_UPDATE_PROLOGUE;
    let mut found = [0u8; ds2_rva::NET_SESSION_UPDATE_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; `read_bytes` reports an unmapped page rather
    // than faulting.
    let read = unsafe { read_bytes(site, &mut found) };
    if !read || found != expected {
        return Err(format!(
            "{prefix} NET_SESSION_UPDATE at 0x{site:016x} read={read} saw={found:02x?} \
             want={expected:02x?} -- not the net session update on this build, nothing patched"
        ));
    }
    // SAFETY: MinHook's own initialiser; ALREADY_INITIALIZED means another crate got there first.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        return Err(format!("{prefix} MH_Initialize said {status:?}"));
    }
    // SAFETY: the site matched its recorded prologue, and the detour has the same ABI.
    let hook = unsafe { MhHook::new(site as *mut c_void, detour as *mut c_void) }
        .map_err(|status| format!("{prefix} MH_CreateHook said {status:?}"))?;
    ORIGINAL.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: an address `MhHook::new` accepted just above.
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        return Err(format!("{prefix} MH_EnableHook said {status:?}"));
    }
    Ok(site)
}

/// The one detour: Before callbacks, the original, After callbacks.
unsafe extern "system" fn detour(this: usize, delta: f32) {
    REGISTRY.run(When::Before, this);
    let raw = ORIGINAL.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: MinHook's trampoline for this exact function and ABI.
        let original: NetSessionUpdate =
            unsafe { std::mem::transmute::<usize, NetSessionUpdate>(raw) };
        // SAFETY: forwarding the game's own arguments.
        unsafe { original(this, delta) };
    }
    REGISTRY.run(When::After, this);
}
