//! A clock that cannot be unplugged.
//!
//! # Why this exists
//!
//! `ds2-input-harness` used to advance its state machine from inside the device polls it
//! detours, because those run once a frame and it was already there. That was wrong in a way
//! that only a live session showed: the tick elected the keyboard poll as its owner, the game
//! stopped calling it, and the harness went completely deaf -- three fresh commands produced no
//! log line at all while the process was alive. Unplug a controller and you get the same silence
//! for a different reason.
//!
//! Anything whose heartbeat is an input device cannot be the thing that proves something about a
//! session where input is the variable. `Present` is not an input device: it runs every frame
//! regardless of what is plugged in, of what has focus, and of whether the player has touched
//! anything for an hour.
//!
//! # Why the seam lives here rather than a second `Present` hook
//!
//! This crate already detours `IDXGISwapChain::Present`. A second detour on the same prologue
//! from the same DLL is a second MinHook registration on one address, which is the collision
//! `ds2-hook`'s union exists to arbitrate and which is not worth arbitrating for one call. One
//! hook, one function, one extra function-pointer load per frame.
//!
//! The hook is called BEFORE anything in the overlay can decline to run -- before the state
//! lock, before the enabled check -- so a consumer's clock does not stop because the overlay had
//! nothing to draw.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Signature of a per-frame consumer. No arguments: a tick is a tick.
pub type FrameFn = fn();

/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
static FRAME_HOOK: AtomicUsize = AtomicUsize::new(0);

/// Frames the hook has been run. Not used here; read by whoever wants to know the clock is live.
static TICKS: AtomicUsize = AtomicUsize::new(0);

/// Install a function to be called once per `Present`.
///
/// One consumer, last writer wins. There is exactly one today and a registry for a second would
/// be machinery with no user.
pub fn set_frame_hook(hook: FrameFn) {
    FRAME_HOOK.store(hook as usize, Ordering::Release);
}

/// Run the installed hook, if there is one. Called at the top of the `Present` detour.
pub fn run_frame_hook() {
    let raw = FRAME_HOOK.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }
    TICKS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `raw` is only ever a `FrameFn` stored by `set_frame_hook` above.
    let hook: FrameFn = unsafe { std::mem::transmute::<usize, FrameFn>(raw) };
    hook();
}

/// How many times the hook has run. Zero after a session that drew frames means the hook was
/// never installed, which is a different problem from one that ran and did nothing.
#[must_use]
pub fn ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    static SERIALIZE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static CALLS: AtomicU32 = AtomicU32::new(0);

    fn counter() {
        CALLS.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn an_uninstalled_hook_is_inert() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        FRAME_HOOK.store(0, Ordering::Release);
        run_frame_hook();
        // Nothing to assert but the absence of a crash: the point is that a `Present` detour in
        // a build with no consumer must not dereference a null function pointer.
    }

    #[test]
    fn an_installed_hook_runs_once_per_call() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        CALLS.store(0, Ordering::Relaxed);
        set_frame_hook(counter);
        run_frame_hook();
        run_frame_hook();
        assert_eq!(CALLS.load(Ordering::Relaxed), 2);
        FRAME_HOOK.store(0, Ordering::Release);
    }
}
