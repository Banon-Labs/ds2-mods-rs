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
//! hook, one function, and a small fixed table of consumers it calls in turn.
//!
//! The consumers are called before anything in the overlay can decline to run -- before the state
//! lock, before the enabled check -- so a consumer's clock does not stop because the overlay had
//! nothing to draw.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Signature of a per-frame consumer. No arguments: a tick is a tick.
pub type FrameFn = fn();

/// How many consumers the clock can carry.
///
/// A fixed table rather than a growable list, so running it is a handful of atomic loads and
/// never takes a lock or allocates inside `Present`.
pub const FRAME_HOOK_SLOTS: usize = 4;

/// The consumers, each stored as a `usize` because a `fn` pointer is not an `Atomic` type. `0` is
/// an empty slot.
static FRAME_HOOKS: [AtomicUsize; FRAME_HOOK_SLOTS] =
    [const { AtomicUsize::new(0) }; FRAME_HOOK_SLOTS];

/// Frames on which at least one consumer ran. Read by whoever wants to know the clock is live.
static TICKS: AtomicUsize = AtomicUsize::new(0);

/// Register a function to be called once per `Present`, after every consumer registered before it.
///
/// Returns `true` when `hook` is in the table, including when it already was: registering the
/// same function twice keeps one slot, so it still runs once a frame. Returns `false` when every
/// slot holds some other function; the caller should say so in its log rather than assume it
/// ticks.
pub fn add_frame_hook(hook: FrameFn) -> bool {
    let raw = hook as usize;
    for slot in &FRAME_HOOKS {
        match slot.compare_exchange(0, raw, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(existing) if existing == raw => return true,
            Err(_) => {}
        }
    }
    false
}

/// Run every registered consumer, in the order they were added. Called at the top of the
/// `Present` detour.
pub fn run_frame_hook() {
    let mut ran = false;
    for slot in &FRAME_HOOKS {
        let raw = slot.load(Ordering::Acquire);
        if raw == 0 {
            continue;
        }
        ran = true;
        // SAFETY: a nonzero slot only ever holds a `FrameFn` stored by `add_frame_hook` above.
        let hook: FrameFn = unsafe { std::mem::transmute::<usize, FrameFn>(raw) };
        hook();
    }
    if ran {
        TICKS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Signature of the one overlay that draws onto the frame: the raw `IDXGISwapChain*` being
/// presented, borrowed for the call.
pub type PresentOverlayFn = fn(*mut core::ffi::c_void);

/// The overlay drawn last in the `Present` detour, or `0`.
///
/// One slot, because a second imgui context on the same swap chain is a second input subclass
/// on the same window. `ds2-net-effects`' selector is the one user: it renders hudhook's imgui
/// frame through this instead of a `Present` hook of its own, which MinHook would refuse on this
/// address (`MH_ERROR_ALREADY_CREATED`).
static PRESENT_OVERLAY: AtomicUsize = AtomicUsize::new(0);

/// Put `overlay` in the slot. `true` when it is there now, including when it already was;
/// `false` when some other function holds the slot.
pub fn set_present_overlay(overlay: PresentOverlayFn) -> bool {
    let raw = overlay as usize;
    match PRESENT_OVERLAY.compare_exchange(0, raw, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => true,
        Err(existing) => existing == raw,
    }
}

/// Call the overlay with the swap chain being presented. Called by the `Present` detour after its
/// own drawing and immediately before the real `Present`, so the overlay is on top.
pub fn run_present_overlay(swap_chain: *mut core::ffi::c_void) {
    let raw = PRESENT_OVERLAY.load(Ordering::Acquire);
    if raw == 0 || swap_chain.is_null() {
        return;
    }
    // SAFETY: a nonzero slot only ever holds a `PresentOverlayFn` stored by `set_present_overlay`.
    let overlay: PresentOverlayFn = unsafe { std::mem::transmute::<usize, PresentOverlayFn>(raw) };
    overlay(swap_chain);
}

/// How many frames ran at least one consumer. Zero after a session that drew frames means nothing
/// was ever registered, which is a different problem from a consumer that ran and did nothing.
#[must_use]
pub fn ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU32;

    static SERIALIZE: Mutex<()> = Mutex::new(());
    static FIRST: AtomicU32 = AtomicU32::new(0);
    static SECOND: AtomicU32 = AtomicU32::new(0);
    static ORDER: Mutex<Vec<u8>> = Mutex::new(Vec::new());

    fn order() -> std::sync::MutexGuard<'static, Vec<u8>> {
        ORDER.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn first() {
        FIRST.fetch_add(1, Ordering::Relaxed);
        order().push(1);
    }

    fn second() {
        SECOND.fetch_add(1, Ordering::Relaxed);
        order().push(2);
    }

    // Distinct bodies, so an optimiser cannot fold them into one address and make the table
    // look less full than it is.
    fn filler_a() {
        order().push(3);
    }
    fn filler_b() {
        order().push(4);
    }
    fn filler_c() {
        order().push(5);
    }

    fn clear() {
        for slot in &FRAME_HOOKS {
            slot.store(0, Ordering::Release);
        }
        FIRST.store(0, Ordering::Relaxed);
        SECOND.store(0, Ordering::Relaxed);
        order().clear();
    }

    static OVERLAY_SEEN: AtomicUsize = AtomicUsize::new(0);

    fn overlay_a(chain: *mut core::ffi::c_void) {
        OVERLAY_SEEN.store(chain as usize, Ordering::Relaxed);
    }

    fn overlay_b(_: *mut core::ffi::c_void) {
        OVERLAY_SEEN.store(usize::MAX, Ordering::Relaxed);
    }

    /// One overlay slot: the first holder keeps it, a re-register is a no-op, a null swap chain
    /// is never handed on.
    #[test]
    fn the_present_overlay_slot_has_one_holder() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        PRESENT_OVERLAY.store(0, Ordering::Release);
        run_present_overlay(0x1234 as *mut core::ffi::c_void);
        assert_eq!(
            OVERLAY_SEEN.load(Ordering::Relaxed),
            0,
            "empty slot calls nothing"
        );
        assert!(set_present_overlay(overlay_a));
        assert!(set_present_overlay(overlay_a));
        assert!(!set_present_overlay(overlay_b));
        run_present_overlay(core::ptr::null_mut());
        assert_eq!(OVERLAY_SEEN.load(Ordering::Relaxed), 0);
        run_present_overlay(0x1234 as *mut core::ffi::c_void);
        assert_eq!(OVERLAY_SEEN.load(Ordering::Relaxed), 0x1234);
        PRESENT_OVERLAY.store(0, Ordering::Release);
    }

    #[test]
    fn an_empty_table_is_inert() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        clear();
        let before = ticks();
        run_frame_hook();
        // A `Present` detour in a build with no consumer neither dereferences a null function
        // pointer nor counts a tick.
        assert_eq!(ticks(), before);
    }

    #[test]
    fn every_consumer_runs_once_per_frame_in_the_order_added() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        clear();
        assert!(add_frame_hook(first));
        assert!(add_frame_hook(second));
        let before = ticks();
        run_frame_hook();
        run_frame_hook();
        assert_eq!(FIRST.load(Ordering::Relaxed), 2);
        assert_eq!(SECOND.load(Ordering::Relaxed), 2);
        assert_eq!(ticks(), before + 2);
        assert_eq!(*order(), vec![1, 2, 1, 2]);
        clear();
    }

    #[test]
    fn adding_the_same_consumer_twice_keeps_one_slot() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        clear();
        assert!(add_frame_hook(first));
        assert!(add_frame_hook(first));
        run_frame_hook();
        assert_eq!(FIRST.load(Ordering::Relaxed), 1);
        clear();
    }

    #[test]
    fn a_full_table_refuses_a_new_consumer_and_keeps_the_old_ones() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        clear();
        for hook in [first, filler_a, filler_b, filler_c] {
            assert!(add_frame_hook(hook));
        }
        assert!(!add_frame_hook(second));
        assert!(
            add_frame_hook(first),
            "a consumer already in a full table is still in it"
        );
        run_frame_hook();
        assert_eq!(FIRST.load(Ordering::Relaxed), 1);
        assert_eq!(SECOND.load(Ordering::Relaxed), 0);
        clear();
    }
}
