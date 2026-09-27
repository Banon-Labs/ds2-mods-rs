//! A clock that cannot be unplugged, and the drawers that paint the frame before it is shown.
//!
//! # Why the clock exists
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
//! # Why there is one `Present` hook and these are tables
//!
//! A second detour on the same prologue from the same DLL is a second MinHook registration on one
//! address, which MinHook answers with `MH_ERROR_ALREADY_CREATED`. So [`crate`] owns the one
//! detour, and everything that wants a tick or a turn at the back buffer registers here.
//!
//! Every table is fixed-size, so running one is a handful of atomic loads and never takes a lock
//! or allocates inside `Present`. The clock runs first, before any drawer can decline to, so a
//! consumer's clock does not stop because a drawer had nothing to draw.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Signature of a per-frame consumer. No arguments: a tick is a tick.
pub type FrameFn = fn();

/// How many consumers the clock can carry.
///
/// Four was the original size and four were in use by the time the save picker wanted one:
/// the input harness, the net-effects key, the picker, and one spare.
pub const FRAME_HOOK_SLOTS: usize = 8;

/// The consumers, each stored as a `usize` because a `fn` pointer is not an `Atomic` type. `0` is
/// an empty slot.
static FRAME_HOOKS: [AtomicUsize; FRAME_HOOK_SLOTS] =
    [const { AtomicUsize::new(0) }; FRAME_HOOK_SLOTS];

/// Frames on which at least one consumer ran. Read by whoever wants to know the clock is live.
static TICKS: AtomicUsize = AtomicUsize::new(0);

/// Put `raw` in the first free slot of `table`. `true` when it is there now, including when it
/// already was.
fn add_to(table: &[AtomicUsize], raw: usize) -> bool {
    for slot in table {
        match slot.compare_exchange(0, raw, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(existing) if existing == raw => return true,
            Err(_) => {}
        }
    }
    false
}

/// Register a function to be called once per `Present`, after every consumer registered before it.
///
/// Returns `true` when `hook` is in the table, including when it already was: registering the
/// same function twice keeps one slot, so it still runs once a frame. Returns `false` when every
/// slot holds some other function; the caller should say so in its log rather than assume it
/// ticks.
pub fn add_frame_hook(hook: FrameFn) -> bool {
    add_to(&FRAME_HOOKS, hook as usize)
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

/// Signature of a drawer: the raw `IDXGISwapChain*` being presented, borrowed for the call.
pub type DrawerFn = fn(*mut core::ffi::c_void);

/// How many drawers can paint a frame. `ds2-invasion-path`'s route lines are the one today.
pub const DRAWER_SLOTS: usize = 4;

/// The drawers, run after the clock and before the imgui panels, in the order they were added.
static DRAWERS: [AtomicUsize; DRAWER_SLOTS] = [const { AtomicUsize::new(0) }; DRAWER_SLOTS];

/// Register a drawer. Same `true`/`false` contract as [`add_frame_hook`].
///
/// A drawer owns its own pipeline state and must put back everything it changes; the imgui panels
/// draw after every drawer, so they are always on top.
pub fn add_drawer(drawer: DrawerFn) -> bool {
    add_to(&DRAWERS, drawer as usize)
}

/// Call every drawer with the swap chain being presented. A null chain is never handed on.
pub fn run_drawers(swap_chain: *mut core::ffi::c_void) {
    if swap_chain.is_null() {
        return;
    }
    for slot in &DRAWERS {
        let raw = slot.load(Ordering::Acquire);
        if raw == 0 {
            continue;
        }
        // SAFETY: a nonzero slot only ever holds a `DrawerFn` stored by `add_drawer`.
        let drawer: DrawerFn = unsafe { std::mem::transmute::<usize, DrawerFn>(raw) };
        drawer(swap_chain);
    }
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
    fn filler_d() {
        order().push(6);
    }
    fn filler_e() {
        order().push(7);
    }
    fn filler_f() {
        order().push(8);
    }
    fn filler_g() {
        order().push(9);
    }

    fn clear() {
        for slot in FRAME_HOOKS.iter().chain(&DRAWERS) {
            slot.store(0, Ordering::Release);
        }
        FIRST.store(0, Ordering::Relaxed);
        SECOND.store(0, Ordering::Relaxed);
        order().clear();
    }

    static DRAWER_SEEN: AtomicUsize = AtomicUsize::new(0);

    fn drawer_a(chain: *mut core::ffi::c_void) {
        DRAWER_SEEN.fetch_add(chain as usize, Ordering::Relaxed);
    }

    fn drawer_b(chain: *mut core::ffi::c_void) {
        DRAWER_SEEN.fetch_add((chain as usize) << 4, Ordering::Relaxed);
    }

    /// Drawers run in order, once each, and a null swap chain is never handed on.
    #[test]
    fn every_drawer_gets_the_chain_and_null_reaches_none() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        clear();
        DRAWER_SEEN.store(0, Ordering::Relaxed);
        run_drawers(0x10 as *mut core::ffi::c_void);
        assert_eq!(
            DRAWER_SEEN.load(Ordering::Relaxed),
            0,
            "empty table calls nothing"
        );
        assert!(add_drawer(drawer_a));
        assert!(add_drawer(drawer_a));
        assert!(add_drawer(drawer_b));
        run_drawers(core::ptr::null_mut());
        assert_eq!(DRAWER_SEEN.load(Ordering::Relaxed), 0);
        run_drawers(0x10 as *mut core::ffi::c_void);
        assert_eq!(DRAWER_SEEN.load(Ordering::Relaxed), 0x110);
        clear();
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
        for hook in [
            first, filler_a, filler_b, filler_c, filler_d, filler_e, filler_f, filler_g,
        ] {
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
