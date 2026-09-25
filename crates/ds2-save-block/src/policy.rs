//! What to do with one frame of the game's save request, decided without touching the game.
//!
//! The whole feature is this file plus five memory writes. Keeping the decision here, over a plain
//! struct of five numbers, is what lets the awkward cases -- a permit that expires, a permitted save
//! that has begun, kind 14's one-frame deferral -- be tested on the machine the code is written on
//! rather than in a session with a character in it.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Frames a permit stays open, once armed: 600, which is ten seconds at 60 fps.
///
/// Twice `ds2_save_file::export`'s own deadline, and that relationship is the point. The export row
/// asks the game to save, waits up to 300 menu frames for the container's stamp to change, and
/// copies whatever is there when the wait ends. A permit shorter than that wait could erase the very
/// request the row is waiting for and hand the player their previous save; a permit far longer than
/// it leaves a window in which the game's own autosave would also be let through.
pub const PERMIT_FRAMES: u32 = 600;

// Both directions are failures, so both are asserted where the number is.
const _: () = assert!(PERMIT_FRAMES >= 300, "shorter than the export row waits");
const _: () = assert!(
    PERMIT_FRAMES <= 60 * 60,
    "a minute of unguarded saving is not a permit"
);

/// Frames remaining on the permit. Zero means the game is not allowed to save.
static PERMIT: AtomicU32 = AtomicU32::new(0);

/// Requests erased since the process started, for the log line and for a caller that wants to say
/// whether the feature ever did anything.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Let the next save through, and the one after it if the first has not started yet.
///
/// Called by the two rows that ask the game to persist on purpose -- `Save Game to File` and
/// `Load Character from File` -- immediately before they call `SaveLoadSystem::RequestSave`. Safe to
/// call when nothing is installed: it moves a counter nobody reads.
///
/// Arming is idempotent in the only sense that matters: a second call re-opens the full window
/// rather than extending a partly spent one, so two rows pressed in quick succession both get their
/// save.
pub fn permit_save() {
    PERMIT.store(PERMIT_FRAMES, Ordering::Release);
}

/// Frames left on the permit. `0` when the game's own saving is being suppressed.
pub fn permit_remaining() -> u32 {
    PERMIT.load(Ordering::Acquire)
}

/// How many save requests have been erased.
pub fn dropped_requests() -> u64 {
    DROPPED.load(Ordering::Acquire)
}

/// The five fields of `SaveLoadSystem` this feature reads, as they were at the top of one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// `+0x1a2`: something asked for a save.
    pub wanted: bool,
    /// `+0x1a9`: kind 14 asked for one, to be turned into a real request next frame.
    pub deferred: bool,
    /// `+0x1a6`: a save has been started and not yet collected.
    pub in_flight: bool,
    /// `+0x68`: the lowest kind asked for since the last save.
    pub kind: i32,
}

/// What the detour does with the frame it is looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// A permit is open. Touch nothing: the save that is pending is the one a menu row asked for.
    Pass,
    /// Nobody asked for a save. Hold the autosave accumulator at zero and leave the rest alone.
    HoldTimer,
    /// A request is pending and no permit is open. Erase it, and hold the accumulator.
    Erase {
        /// The kind the requester asked for, carried so the log names what was refused.
        kind: i32,
        /// Whether this was kind 14's deferred flag rather than an ordinary request.
        deferred: bool,
    },
}

/// One frame's decision, and what the permit counter becomes.
///
/// The permit closes early the moment the save it was armed for has actually begun, which is the
/// frame `in_flight` is set and both request flags are clear -- exactly the state the game leaves
/// behind when it starts a save. Without that, a permit armed for one save would stay open for the
/// rest of its ten seconds and let the next bonfire through with it.
pub fn decide(frame: Frame, permit: u32) -> (Decision, u32) {
    if permit == 0 {
        let decision = if frame.wanted || frame.deferred {
            Decision::Erase {
                kind: frame.kind,
                deferred: frame.deferred,
            }
        } else {
            Decision::HoldTimer
        };
        return (decision, 0);
    }
    let started = frame.in_flight && !frame.wanted && !frame.deferred;
    let remaining = if started { 0 } else { permit - 1 };
    (Decision::Pass, remaining)
}

/// Run [`decide`] against the live permit counter and store what it becomes.
///
/// Separate from `decide` so the state machine stays a function of its arguments: this is the only
/// place the counter is read and written on the game thread.
pub fn step(frame: Frame) -> Decision {
    let (decision, remaining) = decide(frame, PERMIT.load(Ordering::Acquire));
    PERMIT.store(remaining, Ordering::Release);
    if matches!(decision, Decision::Erase { .. }) {
        DROPPED.fetch_add(1, Ordering::AcqRel);
    }
    decision
}

/// Whether this erasure is one the log should carry.
///
/// Requests arrive at the rate the player picks things up, and `addItemToInventory` asks for one per
/// item, so a line each would bury the log in a long session. The first is worth a line because it
/// proves the feature is live; after that, one every [`LOG_EVERY`] says the same thing for the price
/// of a line every few minutes.
pub fn should_log(dropped: u64) -> bool {
    dropped == 1 || dropped.is_multiple_of(LOG_EVERY)
}

/// Erasures between log lines, after the first.
pub const LOG_EVERY: u64 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Frame = Frame {
        wanted: false,
        deferred: false,
        in_flight: false,
        kind: ds2_rva_save_kind_none(),
    };

    /// The idle kind, spelled without the dependency: this module builds on hosts where `ds2-rva`
    /// is not a dependency, and the value only has to be *a* kind for these tests.
    const fn ds2_rva_save_kind_none() -> i32 {
        15
    }

    #[test]
    fn nothing_pending_only_holds_the_timer() {
        assert_eq!(decide(IDLE, 0), (Decision::HoldTimer, 0));
    }

    #[test]
    fn an_ordinary_request_is_erased_with_its_kind() {
        let frame = Frame {
            wanted: true,
            kind: 2,
            ..IDLE
        };
        assert_eq!(
            decide(frame, 0),
            (
                Decision::Erase {
                    kind: 2,
                    deferred: false
                },
                0
            )
        );
    }

    #[test]
    fn kind_fourteens_deferred_flag_is_erased_too() {
        let frame = Frame {
            deferred: true,
            kind: 14,
            ..IDLE
        };
        assert_eq!(
            decide(frame, 0),
            (
                Decision::Erase {
                    kind: 14,
                    deferred: true
                },
                0
            )
        );
    }

    #[test]
    fn a_permit_passes_the_frame_through_and_spends_one() {
        let frame = Frame {
            wanted: true,
            kind: 2,
            ..IDLE
        };
        assert_eq!(
            decide(frame, PERMIT_FRAMES),
            (Decision::Pass, PERMIT_FRAMES - 1)
        );
    }

    #[test]
    fn a_permit_closes_the_moment_the_save_it_allowed_has_begun() {
        let started = Frame {
            in_flight: true,
            ..IDLE
        };
        assert_eq!(decide(started, PERMIT_FRAMES), (Decision::Pass, 0));
    }

    #[test]
    fn a_save_still_in_flight_with_a_second_request_behind_it_keeps_the_permit() {
        let frame = Frame {
            wanted: true,
            in_flight: true,
            kind: 2,
            ..IDLE
        };
        assert_eq!(decide(frame, 10), (Decision::Pass, 9));
    }

    #[test]
    fn an_expiring_permit_goes_back_to_erasing() {
        let frame = Frame {
            wanted: true,
            kind: 2,
            ..IDLE
        };
        let (decision, remaining) = decide(frame, 1);
        assert_eq!((decision, remaining), (Decision::Pass, 0));
        assert!(matches!(decide(frame, remaining).0, Decision::Erase { .. }));
    }

    #[test]
    fn arming_reopens_the_whole_window() {
        permit_save();
        assert_eq!(permit_remaining(), PERMIT_FRAMES);
        let _ = step(IDLE);
        assert_eq!(permit_remaining(), PERMIT_FRAMES - 1);
        permit_save();
        assert_eq!(permit_remaining(), PERMIT_FRAMES);
        // Leave the counter as the rest of the process expects to find it.
        while permit_remaining() > 0 {
            let _ = step(IDLE);
        }
    }

    #[test]
    fn the_log_takes_the_first_and_then_one_in_a_hundred() {
        assert!(should_log(1));
        assert!(!should_log(2));
        assert!(!should_log(99));
        assert!(should_log(100));
        assert!(should_log(1_000));
    }
}
