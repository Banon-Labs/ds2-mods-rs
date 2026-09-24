//! Publishing the heading of the camera this overlay actually drew through.
//!
//! # Why the overlay is the one that publishes it
//!
//! `ds2-input-harness` needs a yaw to close a loop around: it turns the camera by pushing a
//! stick and watching what happened, and without a reading it can only hold a stick for a
//! guessed number of frames. It could resolve a camera of its own -- and then the two
//! resolutions could disagree, and the measurement "does the overlay track the camera?" would be
//! taken against a camera the overlay is not using. That is exactly the bug the measurement
//! exists to find.
//!
//! So the number comes from here, from the same [`crate::geometry::Camera`] the frame was drawn
//! with, one call at the single point where a camera is in hand. Nothing in this crate consumes
//! it; nothing in this crate changes because of it.
//!
//! # Staleness is the whole of the difficulty
//!
//! A frozen yaw is worse than no yaw: a closed loop fed one would push the stick until its
//! budget expired and report a timeout, when the truth is that the camera stopped being
//! resolvable (a load, a cutscene, the overlay toggled off). So every publication bumps a stamp,
//! and the reader reports `None` once the stamp has stood still for [`STALE_AFTER_READS`]
//! consecutive reads.
//!
//! The window is in READS, not in frames or milliseconds, because the reader is the harness and
//! it reads exactly once per input poll. A small window is therefore a small number of frames
//! without needing a clock -- and a window rather than a single read tolerates the overlay's
//! `Present` and the engine's input poll being one frame out of step with each other, which
//! they are free to be.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Consecutive reads with an unchanged stamp after which the yaw is called stale.
///
/// Eight is a seventh of a second at 60fps: long enough that `Present` and the input poll
/// drifting apart by a frame or two is invisible, short enough that a closed loop notices its
/// camera has gone before it has pushed a stick anywhere interesting.
pub const STALE_AFTER_READS: u64 = 8;

/// Last published heading, as `f32` bits.
static YAW_BITS: AtomicU32 = AtomicU32::new(0);

/// Bumped on every publication. The reader watches this, not the yaw: a camera that is being
/// resolved every frame and happens to be perfectly still publishes the same yaw and a
/// different stamp, and that is a live reading rather than a stale one.
static STAMP: AtomicU64 = AtomicU64::new(0);

/// The stamp the last read saw, and how many reads have seen it unchanged.
static LAST_SEEN_STAMP: AtomicU64 = AtomicU64::new(0);
static READS_SINCE_CHANGE: AtomicU64 = AtomicU64::new(0);

/// Record the heading of the camera this frame was drawn through.
///
/// Call once per frame, from wherever a camera is in hand. Cheap enough to be unconditional:
/// two relaxed stores.
pub fn publish(yaw_degrees: f32) {
    YAW_BITS.store(yaw_degrees.to_bits(), Ordering::Relaxed);
    STAMP.fetch_add(1, Ordering::Relaxed);
}

/// The heading of the camera the overlay is currently drawing through, or `None` if it has not
/// been refreshed in the last [`STALE_AFTER_READS`] reads.
///
/// This is the function the loader hands to `ds2_input_harness::set_yaw_source`. It is stateful
/// -- it remembers what the previous read saw -- and is therefore meant for exactly one caller.
#[must_use]
pub fn current() -> Option<f32> {
    let stamp = STAMP.load(Ordering::Relaxed);
    if stamp == 0 {
        // Nothing has ever been published: the overlay is off, or no frame has drawn yet.
        return None;
    }
    if LAST_SEEN_STAMP.swap(stamp, Ordering::Relaxed) == stamp {
        if READS_SINCE_CHANGE.fetch_add(1, Ordering::Relaxed) + 1 >= STALE_AFTER_READS {
            return None;
        }
    } else {
        READS_SINCE_CHANGE.store(0, Ordering::Relaxed);
    }
    Some(f32::from_bits(YAW_BITS.load(Ordering::Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published copy is process-global by design and shared by every test in this module.
    static SERIALIZE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Put the module back to "nothing published", which `current` reports as `None`.
    fn reset() {
        STAMP.store(0, Ordering::Relaxed);
        LAST_SEEN_STAMP.store(0, Ordering::Relaxed);
        READS_SINCE_CHANGE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn nothing_published_is_no_yaw() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        reset();
        assert_eq!(current(), None);
    }

    #[test]
    fn a_fresh_publication_reads_back() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        reset();
        publish(-37.5);
        assert_eq!(current(), Some(-37.5));
    }

    #[test]
    fn a_still_camera_that_is_still_being_resolved_is_not_stale() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        reset();
        // The same yaw, republished every frame: a camera that is not moving, not a camera that
        // has gone. Reporting `None` here would abandon every turn that started while the
        // player was standing still.
        for _ in 0..(STALE_AFTER_READS * 4) {
            publish(12.0);
            assert_eq!(current(), Some(12.0));
        }
    }

    #[test]
    fn a_camera_that_stops_being_published_goes_stale() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        reset();
        publish(12.0);
        assert_eq!(current(), Some(12.0));
        // Nothing republishes. The reader tolerates a short gap, then gives up.
        for read in 1..STALE_AFTER_READS {
            assert_eq!(
                current(),
                Some(12.0),
                "read {read} is still inside the window"
            );
        }
        assert_eq!(
            current(),
            None,
            "past the window, the reading is not evidence"
        );
    }

    #[test]
    fn publishing_again_revives_a_stale_reading() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        reset();
        publish(1.0);
        for _ in 0..(STALE_AFTER_READS * 2) {
            let _ = current();
        }
        assert_eq!(current(), None);
        publish(2.0);
        assert_eq!(current(), Some(2.0));
    }
}
