//! The row's own bookkeeping: whether a latched request is still real, and whether a queued field
//! was ever put on screen.
//!
//! Both are lessons `er-mods-rs` paid for with a row that looked dead. A press latches "a field is
//! open", and anything that strands that latch leaves every later press refused as a repeat. And a
//! press that queues a field nothing ever submits is, on screen, identical to a row that does
//! nothing. The runtime supplies the facts -- which dialog, whether a field or window is live,
//! how many pump passes have run -- and these functions decide.

/// Frames a queued field may wait for its submit before [`SubmitWatch`] reports it.
///
/// The submit normally lands a frame or two after the press, so this is far past that and still
/// close enough to the press that the report sits beside it in the log.
pub const AWAITING_SUBMIT_REPORT_FRAMES: usize = 180;

/// What the runtime knows about an "already open" latch when the row is pressed.
///
/// `owner` values are opaque identities (in practice the quit dialog the latch was taken on);
/// zero means none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Latch {
    /// The dialog the latch was taken against.
    pub latched_owner: usize,
    /// Whether a field job is live right now.
    pub field_live: bool,
    /// Whether the field's window has been seen running.
    pub window_seen: bool,
    /// Whether the runtime has already judged the field abandoned (closed without a report).
    pub abandoned: bool,
}

impl Latch {
    /// Whether this latch is debris that a press from `pressing_owner` may clear.
    ///
    /// Stale when it was taken against a different dialog -- that dialog is gone, and nothing will
    /// ever arrive to clear a latch on it -- or when no field and no window back it, or when the
    /// runtime has already seen the field abandoned. A stale flag must never outrank a player
    /// pressing the row. A live field on the pressing dialog is not stale: clearing that would
    /// stack a second field on the first.
    #[must_use]
    pub const fn is_stale(&self, pressing_owner: usize) -> bool {
        if self.latched_owner != 0 && self.latched_owner != pressing_owner {
            return true;
        }
        if !self.field_live && !self.window_seen {
            return true;
        }
        self.abandoned
    }
}

/// Why a queued field has not been submitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StallCause {
    /// The pump that submits fields has never run, so its hook is not installed and waiting will
    /// not help.
    PumpNeverRan,
    /// The pump runs and the submit is being refused -- usually a previous job still owning the
    /// dialog's queue.
    SubmitRefused,
}

/// A report that a queued field has waited [`AWAITING_SUBMIT_REPORT_FRAMES`] frames.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stall {
    /// Frames the field has been waiting.
    pub frames: usize,
    /// Pump passes this session, which is what separates the two causes.
    pub pump_passes: usize,
    /// Which of the two it is.
    pub cause: StallCause,
}

/// Per-frame watchdog for a field that was queued and never submitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SubmitWatch {
    waited: usize,
}

impl SubmitWatch {
    /// A watch with nothing waiting.
    #[must_use]
    pub const fn new() -> Self {
        Self { waited: 0 }
    }

    /// One frame. `awaiting` is whether a press is queued without a submit; `pump_passes` is the
    /// session's pump count, never reset.
    ///
    /// Reports once per wait, on the frame the wait reaches the threshold. A frame with nothing
    /// awaiting resets the count, so the next press gets its own report.
    pub fn tick(&mut self, awaiting: bool, pump_passes: usize) -> Option<Stall> {
        if !awaiting {
            self.waited = 0;
            return None;
        }
        self.waited += 1;
        (self.waited == AWAITING_SUBMIT_REPORT_FRAMES).then_some(Stall {
            frames: self.waited,
            pump_passes,
            cause: if pump_passes == 0 {
                StallCause::PumpNeverRan
            } else {
                StallCause::SubmitRefused
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported: a latch held against a dialog that was since rebuilt swallowed presses until the
    /// tab happened to be rebuilt again.
    #[test]
    fn a_latch_taken_against_another_dialog_is_stale() {
        let latch = Latch {
            latched_owner: 0x1ad3_a180,
            field_live: true,
            window_seen: true,
            abandoned: false,
        };
        assert!(latch.is_stale(0x1085_4580));
    }

    /// Ported: a request that died before it opened has nothing left to clear it.
    #[test]
    fn a_latch_with_no_field_behind_it_is_stale() {
        let latch = Latch {
            latched_owner: 0xdead_beef,
            field_live: false,
            window_seen: false,
            abandoned: false,
        };
        assert!(latch.is_stale(0xdead_beef));
    }

    /// Ported: the guard must not eat a genuine double press.
    #[test]
    fn a_live_field_on_this_dialog_is_not_stale() {
        let latch = Latch {
            latched_owner: 0xabc_1000,
            field_live: true,
            window_seen: true,
            abandoned: false,
        };
        assert!(!latch.is_stale(0xabc_1000));
    }

    #[test]
    fn an_abandoned_field_is_stale_even_on_this_dialog() {
        let latch = Latch {
            latched_owner: 0xabc_1000,
            field_live: true,
            window_seen: true,
            abandoned: true,
        };
        assert!(latch.is_stale(0xabc_1000));
    }

    #[test]
    fn the_watch_reports_once_at_the_threshold() {
        let mut watch = SubmitWatch::new();
        let reports: Vec<Stall> = (0..AWAITING_SUBMIT_REPORT_FRAMES * 3)
            .filter_map(|_| watch.tick(true, 5))
            .collect();
        assert_eq!(
            reports,
            vec![Stall {
                frames: AWAITING_SUBMIT_REPORT_FRAMES,
                pump_passes: 5,
                cause: StallCause::SubmitRefused
            }]
        );
    }

    /// Ported: zero pump passes means the hook is missing, a different bug from a refused submit.
    #[test]
    fn a_pump_that_never_ran_is_named_as_such() {
        let mut watch = SubmitWatch::new();
        let stall = (0..AWAITING_SUBMIT_REPORT_FRAMES)
            .filter_map(|_| watch.tick(true, 0))
            .next_back();
        assert_eq!(stall.map(|s| s.cause), Some(StallCause::PumpNeverRan));
    }

    #[test]
    fn a_submit_resets_the_watch_for_the_next_press() {
        let mut watch = SubmitWatch::new();
        for _ in 0..AWAITING_SUBMIT_REPORT_FRAMES - 1 {
            assert_eq!(watch.tick(true, 1), None);
        }
        assert_eq!(watch.tick(false, 1), None);
        let again = (0..AWAITING_SUBMIT_REPORT_FRAMES)
            .filter_map(|_| watch.tick(true, 1))
            .count();
        assert_eq!(again, 1, "the next press gets its own report");
    }
}
