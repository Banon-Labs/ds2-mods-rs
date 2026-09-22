//! When the self-check looks at its stones, and what it is trying to tell apart.
//!
//! # Why a schedule rather than a single look
//!
//! The question the trail actually needs answered is **does a Prism Stone linger, or does it
//! flash once?** -- and that is not a question one observation can answer. A stone checked
//! immediately is alive whether it lingers or not; a stone checked once, late, cannot tell
//! "it ended normally" from "it never started". Three looks, spread out, give a shape: alive at
//! all three is lingering, alive at the first and gone by the second is a burst, dead at the
//! first is a spawn that produced nothing the engine kept.
//!
//! # Why the times are what they are
//!
//! One second is long enough that a frame or two of engine latency has passed and short enough
//! that a one-shot burst is still going. Three seconds is past the end of essentially every
//! one-shot effect in this engine. Ten seconds is long enough that a player at the keyboard has
//! had time to look at the ground, and it is the number that would have to hold for a trail to
//! be usable at all -- a marker that dies before you have walked to it is not a marker.
//!
//! They are literals rather than settings. The question is fully determined: it is "does this
//! outlive a burst", not "how long would you like to wait", and a knob here would only let a
//! future reader mistune the one experiment the module exists to run.
//!
//! # Lateness is reported, not hidden
//!
//! A stalled frame -- a load, a shader compile, a breakpoint -- can leave two samples due at
//! once. This takes one per tick and reports both the time it was SCHEDULED for and the time it
//! actually happened, so a reader can see that "t=1.0s" was really taken at 4.2s and discount it
//! accordingly. Silently relabelling a late sample would turn a stall into a false finding about
//! how long effects last.

// Windows-only in practice; ungated so the schedule stays host-testable.
#![cfg_attr(not(windows), allow(dead_code))]

/// Seconds after the stones go down at which their liveness is sampled.
pub(crate) const SAMPLE_SECONDS: [f32; 3] = [1.0, 3.0, 10.0];

/// One due observation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    /// The time this sample was meant to be taken at.
    pub(crate) scheduled: f32,
    /// The time it is actually being taken at. Equal to `scheduled` on a healthy frame.
    pub(crate) actual: f32,
}

impl Sample {
    /// Did the frame run late enough that this reading should be read with suspicion?
    ///
    /// Half a second: comfortably more than any ordinary frame, comfortably less than the gap
    /// between two samples, so a `true` here really is a stall rather than jitter.
    pub(crate) fn late(&self) -> bool {
        self.actual - self.scheduled > 0.5
    }
}

/// Walks [`SAMPLE_SECONDS`] as time passes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Schedule {
    elapsed: f32,
    taken: usize,
}

impl Schedule {
    /// Advance by `delta` seconds and return a sample if one has come due.
    ///
    /// At most one per call. Several overdue at once are returned over consecutive calls, each
    /// carrying its own real `actual`, rather than being collapsed or dropped.
    pub(crate) fn advance(&mut self, delta: f32) -> Option<Sample> {
        if delta.is_finite() && delta > 0.0 {
            self.elapsed += delta;
        }
        let scheduled = *SAMPLE_SECONDS.get(self.taken)?;
        if self.elapsed < scheduled {
            return None;
        }
        self.taken += 1;
        Some(Sample {
            scheduled,
            actual: self.elapsed,
        })
    }

    /// Every sample has been taken and there is nothing left to wait for.
    pub(crate) fn finished(&self) -> bool {
        self.taken >= SAMPLE_SECONDS.len()
    }

    /// Seconds since the schedule started.
    ///
    /// Test-only, and deliberately so: the production path learns the elapsed time from
    /// [`Sample::actual`], which is the reading it is about to write down. A second way to ask
    /// the same question is a second thing that can disagree with the log.
    #[cfg(test)]
    pub(crate) fn elapsed(&self) -> f32 {
        self.elapsed
    }

    /// How many samples have been taken. Test-only, for the same reason as [`Schedule::elapsed`].
    #[cfg(test)]
    pub(crate) fn taken(&self) -> usize {
        self.taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sixty ticks of a sixtieth of a second each.
    fn run(schedule: &mut Schedule, seconds: f32) -> Vec<Sample> {
        let mut out = Vec::new();
        let step = 1.0 / 60.0;
        let mut left = seconds;
        while left > 0.0 {
            if let Some(sample) = schedule.advance(step) {
                out.push(sample);
            }
            left -= step;
        }
        out
    }

    #[test]
    fn nothing_is_due_before_the_first_sample_time() {
        let mut schedule = Schedule::default();
        assert!(run(&mut schedule, 0.9).is_empty());
        assert!(!schedule.finished());
    }

    #[test]
    fn each_sample_comes_due_once_and_in_order() {
        let mut schedule = Schedule::default();
        let taken = run(&mut schedule, 11.0);
        let scheduled: Vec<f32> = taken.iter().map(|sample| sample.scheduled).collect();
        assert_eq!(scheduled, SAMPLE_SECONDS.to_vec());
        assert!(schedule.finished());
    }

    #[test]
    fn a_finished_schedule_stops_producing_samples() {
        let mut schedule = Schedule::default();
        run(&mut schedule, 11.0);
        assert!(run(&mut schedule, 30.0).is_empty());
    }

    /// The stall case: one enormous delta leaves all three due. None may be dropped, and none may
    /// be relabelled -- a reading taken at 20 s must not be reported as the one-second reading
    /// without saying so.
    #[test]
    fn a_stall_delivers_every_overdue_sample_and_admits_it_was_late() {
        let mut schedule = Schedule::default();
        let mut taken = Vec::new();
        for _ in 0..SAMPLE_SECONDS.len() {
            taken.push(schedule.advance(20.0).expect("a sample was due"));
        }
        assert_eq!(taken.len(), 3);
        assert_eq!(
            taken.iter().map(|s| s.scheduled).collect::<Vec<f32>>(),
            SAMPLE_SECONDS.to_vec()
        );
        assert!(
            taken.iter().all(Sample::late),
            "a sample taken twenty seconds late reported itself as on time: {taken:?}"
        );
        assert!(schedule.finished());
    }

    #[test]
    fn an_on_time_sample_does_not_claim_to_be_late() {
        let mut schedule = Schedule::default();
        let taken = run(&mut schedule, 1.2);
        assert_eq!(taken.len(), 1);
        assert!(!taken[0].late(), "{:?}", taken[0]);
    }

    /// A negative or non-finite delta is a clock that has gone wrong, and the schedule must not
    /// travel backwards or become `NaN` because of it.
    #[test]
    fn a_broken_clock_does_not_move_the_schedule() {
        let mut schedule = Schedule::default();
        schedule.advance(-5.0);
        schedule.advance(f32::NAN);
        schedule.advance(f32::INFINITY);
        assert_eq!(schedule.elapsed(), 0.0);
        assert_eq!(schedule.taken(), 0);
    }
}
