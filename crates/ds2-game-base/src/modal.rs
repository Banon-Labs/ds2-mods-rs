//! Whether one of this workspace's own modal windows is holding the thread it was opened on.
//!
//! Some features open an OS window from the game thread and wait for it -- `ds2-save-file`'s file
//! dialog is the one that exists -- and for as long as it is up the game presents no frames. A
//! watchdog that measures "frames stopped" cannot tell that from a freeze, and on 2026-09-26 one
//! did not: a load dialog left open for over 30 s produced a full main-thread stall report.
//!
//! So a feature that blocks on purpose says so here, with a guard held around the blocking call,
//! and a watchdog asks [`active`] before it calls anything a hang. A depth, not a flag, so a nested
//! window cannot clear the state of the one under it.

use std::sync::atomic::{AtomicU32, Ordering};

static DEPTH: AtomicU32 = AtomicU32::new(0);

/// Held for as long as a modal window of ours is up. Dropping it ends the period.
#[must_use = "the modal period ends when the guard is dropped"]
pub struct ModalGuard(());

impl Drop for ModalGuard {
    fn drop(&mut self) {
        DEPTH.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Mark the start of a modal period on the calling thread; it lasts until the guard drops.
pub fn enter() -> ModalGuard {
    DEPTH.fetch_add(1, Ordering::AcqRel);
    ModalGuard(())
}

/// Whether any modal period is open right now.
pub fn active() -> bool {
    DEPTH.load(Ordering::Acquire) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nested periods keep the state until the outermost one closes.
    #[test]
    fn nested_guards_close_in_order() {
        assert!(!active());
        let outer = enter();
        let inner = enter();
        assert!(active());
        drop(inner);
        assert!(active(), "the outer window is still up");
        drop(outer);
        assert!(!active());
    }
}
