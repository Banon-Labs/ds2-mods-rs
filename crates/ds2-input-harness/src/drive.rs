//! One frame at a time: what the harness is doing, and what that means for the device objects.
//!
//! This is the whole of the harness's decision-making and it is deliberately free of Windows,
//! MinHook and game memory. It is handed the camera's yaw (or `None` when nothing can supply
//! one) and hands back the authored state plus whether to blank the human -- so every branch in
//! here is reachable from `cargo test` with no game running.
//!
//! # The two things that are always true
//!
//! 1. **Everything is frame-bounded.** There is no activity without a budget and no block
//!    without a deadline. A harness that wedges stops pressing on its own.
//! 2. **Blocking and authoring are separate.** Blocking zeroes what the hardware produced;
//!    authoring stamps values on top. They compose, so "block everything and turn the camera" is
//!    one block plus one turn rather than a special case.

use crate::authored::{AXIS_COUNT, Authored};
use crate::command::Command;
use crate::log::harness_log;
use crate::turn::{MAX_MAGNITUDE, Outcome, Turn};

/// The axis a `turn` drives.
///
/// The right stick's X, because that is the axis XInput's `sThumbRX` lands on
/// (`ds2_rva::PAD_AXIS_RIGHT_X`). **That it is bound to the camera is NOT established
/// statically** -- the binding lives in the `DLUI` mapper and is a runtime fact. `probe`
/// measures it, and a `turn` against an axis that moves nothing reports `NoResponse` rather than
/// pretending. If `probe` says a different axis is the camera, this is the constant to change.
pub const TURN_AXIS: usize = ds2_rva::PAD_AXIS_RIGHT_X;

/// Frames the probe leaves the stick centred between axes, so one axis's motion does not bleed
/// into the next axis's measurement. The camera in this engine springs back toward the player,
/// so "release and wait" is not the same as "stop instantly".
const PROBE_SETTLE_FRAMES: u32 = 15;

/// A sweep of every pad axis, measuring what each one does to the camera's yaw.
#[derive(Clone, Copy, Debug)]
struct Probe {
    /// Which axis is being held right now.
    axis: usize,
    /// Frames still to hold it.
    holding: u32,
    /// Frames still to wait with the stick centred before the next axis.
    settling: u32,
    /// How long each axis is held.
    hold_frames: u32,
    /// Yaw when this axis's hold began.
    start_yaw: f32,
    /// Accumulated yaw travel for this axis, wrap-corrected.
    travelled: f32,
    /// The yaw seen on the previous frame.
    last_yaw: f32,
}

/// What the harness is doing right now.
#[derive(Clone, Copy, Debug)]
enum Activity {
    /// Nothing. The player's own hardware is in charge (unless a block is running).
    Idle,
    /// Holding a fixed authored state until the counter runs out.
    Hold {
        authored: Authored,
        frames_left: u32,
    },
    /// A closed-loop camera turn.
    Turning(Turn),
    /// An axis sweep.
    Probing(Probe),
}

/// What one frame of the harness amounts to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Values to stamp onto the device objects after the game's own poll has run.
    pub authored: Authored,
    /// Whether to blank what the hardware produced first.
    pub block: bool,
}

/// The harness's whole runtime state.
#[derive(Clone, Copy, Debug)]
pub struct Session {
    activity: Activity,
    /// Frames of suppression still owed. Zero means the player's input reaches the game.
    block_frames_left: u32,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A harness that is doing nothing and blocking nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            activity: Activity::Idle,
            block_frames_left: 0,
        }
    }

    /// Is the human's input currently being blanked?
    #[must_use]
    pub const fn blocking(&self) -> bool {
        self.block_frames_left > 0
    }

    /// Is the harness pressing anything?
    #[must_use]
    pub const fn busy(&self) -> bool {
        !matches!(self.activity, Activity::Idle)
    }

    /// Take a command. `yaw` is the camera's current yaw, if anything can supply one.
    ///
    /// A command that needs a yaw and has none is REFUSED here, at the moment it is asked for,
    /// rather than started and abandoned later -- so the log line that says "no camera" sits
    /// next to the request that needed one.
    pub fn accept(&mut self, command: Command, yaw: Option<f32>) {
        match command {
            Command::Block { frames } => {
                self.block_frames_left = frames;
                harness_log!("block: blanking every human input for {frames} frames");
            }
            Command::Unblock => {
                self.block_frames_left = 0;
                harness_log!("unblock: the player's own input reaches the game again");
            }
            Command::Release => {
                self.activity = Activity::Idle;
                harness_log!("release: authoring nothing (a running block is untouched)");
            }
            Command::Status => {
                harness_log!(
                    "status: block-frames-left={} activity={} yaw={}",
                    self.block_frames_left,
                    self.activity_name(),
                    match yaw {
                        Some(value) => format!("{value:.2}"),
                        None => "none (no camera source installed)".to_owned(),
                    }
                );
            }
            Command::Axis {
                index,
                value,
                frames,
            } => {
                self.activity = Activity::Hold {
                    authored: Authored::axis(index, value),
                    frames_left: frames,
                };
                harness_log!("axis: holding pad axis {index} at {value} for {frames} frames");
            }
            Command::Mouse { dx, dy, frames } => {
                self.activity = Activity::Hold {
                    authored: Authored::mouse_delta(dx, dy),
                    frames_left: frames,
                };
                harness_log!("mouse: holding delta ({dx}, {dy}) for {frames} frames");
            }
            Command::Buttons { mask, frames } => {
                self.activity = Activity::Hold {
                    authored: Authored {
                        buttons: Some(mask),
                        ..Authored::NOTHING
                    },
                    frames_left: frames,
                };
                harness_log!("buttons: holding mask 0x{mask:04x} for {frames} frames");
            }
            Command::Turn { degrees, budget } => match yaw {
                Some(yaw) => {
                    self.activity = Activity::Turning(Turn::begin(degrees, yaw, budget));
                    harness_log!(
                        "turn: {degrees} degrees on pad axis {TURN_AXIS}, from yaw {yaw:.2}, \
                         budget {budget} frames -- closed loop on the camera's own yaw"
                    );
                }
                None => harness_log!(
                    "turn REFUSED: no camera yaw source is installed, so a turn could not be \
                     measured and would be a guess. Enable [invasion_path] (it publishes the \
                     yaw of the camera it draws through) or use `axis` for an open-loop hold."
                ),
            },
            Command::Probe { frames } => match yaw {
                Some(yaw) => {
                    self.activity = Activity::Probing(Probe {
                        axis: 0,
                        holding: frames,
                        settling: 0,
                        hold_frames: frames,
                        start_yaw: yaw,
                        travelled: 0.0,
                        last_yaw: yaw,
                    });
                    harness_log!(
                        "probe: holding each of the {AXIS_COUNT} pad axes at +{MAX_MAGNITUDE} \
                         for {frames} frames and reporting the yaw it moved"
                    );
                }
                None => harness_log!(
                    "probe REFUSED: no camera yaw source is installed, and a probe with nothing \
                     to measure is just six stick presses."
                ),
            },
        }
    }

    /// Advance one frame and say what to write.
    pub fn frame(&mut self, yaw: Option<f32>) -> Frame {
        // Read BEFORE the decrement: `block 2` must blank two frames, and taking the budget
        // first spends one of them on the bookkeeping.
        let block = self.blocking();
        self.block_frames_left = self.block_frames_left.saturating_sub(1);
        // Taken out, advanced, put back. `Activity` is `Copy`, and this is what lets the sweep
        // replace the whole activity with `Idle` from inside a function that is also reading it.
        let mut activity = self.activity;
        let authored = Self::advance(&mut activity, yaw);
        self.activity = activity;
        Frame { authored, block }
    }

    /// One frame of whatever the harness is doing.
    fn advance(activity: &mut Activity, yaw: Option<f32>) -> Authored {
        match activity {
            Activity::Idle => Authored::NOTHING,
            Activity::Hold {
                authored,
                frames_left,
            } => {
                let held = *authored;
                *frames_left -= 1;
                if *frames_left == 0 {
                    *activity = Activity::Idle;
                    harness_log!("hold complete -- released");
                }
                held
            }
            // A turn or a sweep that loses its camera mid-drive must stop pressing, not keep
            // pushing blind. `None` here means whatever was resolving a camera stopped (a load,
            // a cutscene), and a stick held through that is exactly the stuck-input failure
            // every budget in this crate exists to prevent.
            Activity::Turning(turn) if yaw.is_none() => {
                harness_log!(
                    "turn ABANDONED after {} frames: the camera yaw source stopped answering, \
                     so there is nothing left to measure against",
                    turn.frames()
                );
                *activity = Activity::Idle;
                Authored::NOTHING
            }
            Activity::Probing(_) if yaw.is_none() => {
                harness_log!("probe ABANDONED: the camera yaw source stopped answering");
                *activity = Activity::Idle;
                Authored::NOTHING
            }
            Activity::Turning(turn) => {
                let yaw = yaw.unwrap_or_default();
                match turn.step(yaw) {
                    Outcome::Driving(value) => return Authored::axis(TURN_AXIS, value),
                    Outcome::Reached { frames, travelled } => harness_log!(
                        "turn REACHED in {frames} frames: the camera's own yaw moved \
                         {travelled:.2} degrees"
                    ),
                    Outcome::Timeout { travelled } => harness_log!(
                        "turn TIMED OUT: the camera's yaw moved {travelled:.2} degrees before \
                         the frame budget ran out -- released"
                    ),
                    Outcome::NoResponse { frames } => harness_log!(
                        "turn NO RESPONSE: pad axis {TURN_AXIS} was held for {frames} frames \
                         and the camera's yaw did not move. That axis is not the camera on this \
                         build with these settings -- run `probe` to find out which one is."
                    ),
                }
                *activity = Activity::Idle;
                Authored::NOTHING
            }
            Activity::Probing(_) => Self::probe_frame(activity, yaw.unwrap_or_default()),
        }
    }

    /// One frame of an axis sweep.
    ///
    /// The measurement is taken over the hold AND the settle that follows it, not just the hold.
    /// That is deliberate: this camera springs back toward the player when a stick is released,
    /// so "how far did it move while I pushed" and "where did it end up" are different numbers,
    /// and the second is the one that answers whether an axis controls the camera.
    fn probe_frame(activity: &mut Activity, yaw: f32) -> Authored {
        let Activity::Probing(probe) = activity else {
            return Authored::NOTHING;
        };
        probe.travelled += crate::turn::wrap_degrees(yaw - probe.last_yaw);
        probe.last_yaw = yaw;

        if probe.holding > 0 {
            probe.holding -= 1;
            if probe.holding == 0 {
                probe.settling = PROBE_SETTLE_FRAMES;
            }
            return Authored::axis(probe.axis, MAX_MAGNITUDE);
        }

        probe.settling -= 1;
        if probe.settling > 0 {
            return Authored::NOTHING;
        }
        harness_log!(
            "probe: pad axis {} held at +{MAX_MAGNITUDE} for {} frames left the camera's yaw \
             {:.2} degrees from where it started ({:.2} -> {:.2})",
            probe.axis,
            probe.hold_frames,
            probe.travelled,
            probe.start_yaw,
            yaw
        );
        probe.axis += 1;
        if probe.axis >= AXIS_COUNT {
            harness_log!("probe complete");
            *activity = Activity::Idle;
            return Authored::NOTHING;
        }
        probe.holding = probe.hold_frames;
        probe.start_yaw = yaw;
        probe.travelled = 0.0;
        Authored::NOTHING
    }

    /// A word for the log.
    const fn activity_name(&self) -> &'static str {
        match self.activity {
            Activity::Idle => "idle",
            Activity::Hold { .. } => "hold",
            Activity::Turning(_) => "turn",
            Activity::Probing(_) => "probe",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_session_presses_nothing_and_blocks_nothing() {
        let mut session = Session::new();
        let frame = session.frame(None);
        assert!(frame.authored.is_empty());
        assert!(!frame.block);
    }

    #[test]
    fn a_hold_releases_itself_when_its_frames_run_out() {
        let mut session = Session::new();
        session.accept(
            Command::Axis {
                index: TURN_AXIS,
                value: 1.0,
                frames: 3,
            },
            None,
        );
        for expected in 0..3 {
            let frame = session.frame(None);
            assert_eq!(
                frame.authored.axes[TURN_AXIS],
                Some(1.0),
                "frame {expected} should still be holding"
            );
        }
        assert!(
            session.frame(None).authored.is_empty(),
            "the frame after the budget must release, with no further command"
        );
        assert!(!session.busy());
    }

    #[test]
    fn a_block_expires_on_its_own() {
        let mut session = Session::new();
        session.accept(Command::Block { frames: 2 }, None);
        assert!(session.frame(None).block);
        assert!(session.frame(None).block);
        assert!(
            !session.frame(None).block,
            "a block that outlives its budget is a lockout"
        );
    }

    #[test]
    fn unblock_takes_effect_at_once() {
        let mut session = Session::new();
        session.accept(Command::Block { frames: 10_000 }, None);
        assert!(session.frame(None).block);
        session.accept(Command::Unblock, None);
        assert!(!session.frame(None).block);
    }

    #[test]
    fn release_stops_authoring_without_lifting_a_block() {
        let mut session = Session::new();
        session.accept(Command::Block { frames: 100 }, None);
        session.accept(
            Command::Axis {
                index: 0,
                value: 1.0,
                frames: 100,
            },
            None,
        );
        assert!(!session.frame(None).authored.is_empty());
        session.accept(Command::Release, None);
        let frame = session.frame(None);
        assert!(frame.authored.is_empty());
        assert!(
            frame.block,
            "release is about what the harness presses, not about what the player is allowed to"
        );
    }

    #[test]
    fn a_turn_without_a_camera_is_refused_rather_than_started() {
        let mut session = Session::new();
        session.accept(
            Command::Turn {
                degrees: 45.0,
                budget: 100,
            },
            None,
        );
        assert!(
            !session.busy(),
            "a turn with nothing to measure against must not start"
        );
        assert!(session.frame(None).authored.is_empty());
    }

    #[test]
    fn a_turn_drives_the_right_stick_and_stops_when_the_yaw_arrives() {
        let mut session = Session::new();
        let mut yaw = 0.0f32;
        session.accept(
            Command::Turn {
                degrees: 30.0,
                budget: 500,
            },
            Some(yaw),
        );
        let mut frames = 0;
        while session.busy() {
            let frame = session.frame(Some(yaw));
            if let Some(value) = frame.authored.axes[TURN_AXIS] {
                // A plant that turns two degrees per frame at full stick.
                yaw = crate::turn::wrap_degrees(yaw + value * 2.0);
            }
            frames += 1;
            assert!(frames < 500, "the turn should have finished long ago");
        }
        assert!(
            (yaw - 30.0).abs() <= 2.5,
            "camera ended at {yaw}, which is not where 30 degrees is"
        );
    }

    #[test]
    fn a_turn_that_loses_its_camera_releases_the_stick() {
        let mut session = Session::new();
        session.accept(
            Command::Turn {
                degrees: 90.0,
                budget: 500,
            },
            Some(0.0),
        );
        let _ = session.frame(Some(1.0));
        let frame = session.frame(None);
        assert!(
            frame.authored.is_empty(),
            "a stick held through a camera that stopped answering is the stuck-input failure"
        );
        assert!(!session.busy());
    }

    #[test]
    fn a_probe_holds_every_axis_in_turn_and_then_stops() {
        let mut session = Session::new();
        let mut yaw = 0.0f32;
        session.accept(Command::Probe { frames: 4 }, Some(yaw));
        let mut seen = [false; AXIS_COUNT];
        let mut frames = 0;
        while session.busy() {
            let frame = session.frame(Some(yaw));
            for (index, value) in frame.authored.axes.iter().enumerate() {
                if value.is_some() {
                    seen[index] = true;
                }
            }
            // Only axis 3 does anything, which is what a probe is for finding out.
            if let Some(value) = frame.authored.axes[3] {
                yaw = crate::turn::wrap_degrees(yaw + value);
            }
            frames += 1;
            assert!(frames < 1000, "the probe should have finished long ago");
        }
        assert!(
            seen.iter().all(|touched| *touched),
            "every axis must be tried, or the probe is not a survey: {seen:?}"
        );
    }
}
