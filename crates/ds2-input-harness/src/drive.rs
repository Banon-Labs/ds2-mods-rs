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

use core::fmt;

use crate::authored::{AXIS_COUNT, Authored};
use crate::command::Command;
use crate::log::harness_log;
use crate::turn::{Magnitudes, Outcome, Turn};

/// Which input the harness pushes when it is asked to turn the camera.
///
/// # Why this is a choice and not a constant
///
/// The first version of this crate drove pad axis 3 on the strength of `sThumbRX` landing there,
/// and a live probe found that axis moving the camera at most 6.35 degrees over thirty frames at
/// full deflection -- camera-follow drift, not a stick, because nothing was plugged in. Which
/// input turns the camera is a property of the session, not of the binary, so it is selectable
/// and the `probe` command measures it rather than anyone asserting it.
///
/// **With a controller connected, axis 3 is the camera's heading: measured, not inferred.** A
/// probe on 2026-09-26, with an Xbox controller plugged in and a confirmed `block` in force, moved
/// the yaw 128.88 degrees on `pad3` against 18.95 on `pad0` (the character walking, the camera
/// following), 7.62 on `pad1`, under half a degree on `pad2` and `pad4`, and 0.00 on both mouse
/// channels. An earlier unblocked run gave the same ranking (494.78 on `pad3`). So `channel pad3`
/// is the channel to use in a session with a pad. The default stays `mouse-x` for the session
/// without one; whether the mouse moves the camera at all when no pad is present is still
/// unmeasured.
///
/// **The pad channels work without the window having focus**, so driving the game never needs
/// to take focus from the person at the desk. A probe with the game window unfocused from launch
/// to finish turned the yaw 733.33 degrees on `pad3`. The game's own
/// `Ext.UserInput.CooperativeLevel.SetForeGround.Pad` option (input manager `+0x16e`) read 0
/// live, so the pad poll does not skip itself when the window is inactive. The keyboard's option
/// (`+0x16f`) read 1, and its poll stops being called at all while unfocused, so the keyboard
/// channel cannot drive an unfocused game. `scripts/frida/input-focus.js` reads those bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Mouse X. **The default**: it needs no hardware, and its chain from the DirectInput
    /// mouse's X float to the camera is traced in `docs/DS2-MOUSE-LOOK.md` and measured with
    /// `scripts/frida/mouse-look-author.js`. It needs the game window focused; the pad does not.
    MouseX,
    /// Mouse Y. The same chain, the other component; pitch rather than heading, so a yaw loop
    /// driving it should expect `NoResponse`.
    MouseY,
    /// One of the six normalised pad axes. Right for a session with a controller in it.
    PadAxis(usize),
}

impl Channel {
    /// What `turn` drives until something says otherwise.
    pub const DEFAULT: Self = Self::MouseX;

    /// Every channel a sweep tries, in the order it tries them: the two mouse components first,
    /// because they cost no hardware, then the pad axes.
    pub const SWEEP: [Self; 2 + AXIS_COUNT] = [
        Self::MouseX,
        Self::MouseY,
        Self::PadAxis(0),
        Self::PadAxis(1),
        Self::PadAxis(2),
        Self::PadAxis(3),
        Self::PadAxis(4),
        Self::PadAxis(5),
    ];

    /// How hard this channel may be pushed, in its own unit.
    #[must_use]
    pub const fn limits(self) -> Magnitudes {
        match self {
            Self::MouseX | Self::MouseY => Magnitudes::MOUSE,
            Self::PadAxis(_) => Magnitudes::PAD,
        }
    }

    /// One frame of this channel held at `value`.
    #[must_use]
    pub fn authored(self, value: f32) -> Authored {
        match self {
            Self::MouseX => Authored::mouse_delta(value, 0.0),
            Self::MouseY => Authored::mouse_delta(0.0, value),
            Self::PadAxis(index) => Authored::axis(index, value),
        }
    }

    /// Parse a channel name: `mouse-x`, `mouse-y`, or `pad0`..`pad5`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.to_ascii_lowercase().as_str() {
            "mouse-x" | "mousex" | "mouse" => Some(Self::MouseX),
            "mouse-y" | "mousey" => Some(Self::MouseY),
            other => {
                let index: usize = other.strip_prefix("pad")?.parse().ok()?;
                (index < AXIS_COUNT).then_some(Self::PadAxis(index))
            }
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MouseX => write!(formatter, "mouse-x"),
            Self::MouseY => write!(formatter, "mouse-y"),
            Self::PadAxis(index) => write!(formatter, "pad{index}"),
        }
    }
}

/// Frames the probe leaves each channel released between sweeps, so one channel's motion does
/// not bleed into the next one's measurement. The camera in this engine springs back toward the
/// player, so "release and wait" is not the same as "stop instantly".
const PROBE_SETTLE_FRAMES: u32 = 15;

/// A sweep of every channel, measuring what each one does to the camera's yaw.
#[derive(Clone, Copy, Debug)]
struct Probe {
    /// Index into [`Channel::SWEEP`].
    index: usize,
    /// Frames still to hold the current channel.
    holding: u32,
    /// Frames still to wait with it released before moving on.
    settling: u32,
    /// How long each channel is held.
    hold_frames: u32,
    /// Yaw when this channel's hold began.
    start_yaw: f32,
    /// Accumulated yaw travel for this channel, wrap-corrected.
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
    /// A closed-loop camera turn on [`Session::channel`].
    Turning { turn: Turn, channel: Channel },
    /// A channel sweep.
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
    /// What `turn` drives.
    channel: Channel,
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
            channel: Channel::DEFAULT,
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

    /// What `turn` will drive.
    #[must_use]
    pub const fn channel(&self) -> Channel {
        self.channel
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
                    "status: block-frames-left={} activity={} channel={} yaw={}",
                    self.block_frames_left,
                    self.activity_name(),
                    self.channel,
                    match yaw {
                        Some(value) => format!("{value:.2}"),
                        None => "none (no camera source installed)".to_owned(),
                    }
                );
            }
            Command::SetChannel(channel) => {
                self.channel = channel;
                harness_log!("channel: `turn` now drives {channel}");
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
                harness_log!(
                    "mouse: adding ({dx}, {dy}) counts per frame to the DirectInput mouse for \
                     {frames} frames"
                );
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
                    let channel = self.channel;
                    self.activity = Activity::Turning {
                        turn: Turn::begin(degrees, yaw, budget, channel.limits()),
                        channel,
                    };
                    harness_log!(
                        "turn: {degrees} degrees on {channel}, from yaw {yaw:.2}, budget \
                         {budget} frames -- closed loop on the camera's own yaw"
                    );
                }
                None => harness_log!(
                    "turn REFUSED: no camera yaw source is installed, so a turn could not be \
                     measured and would be a guess. Enable [invasion_path] (it publishes the \
                     yaw of the camera it draws through) or use `axis`/`mouse` for an open-loop \
                     hold."
                ),
            },
            Command::Probe { frames } => match yaw {
                Some(yaw) => {
                    self.activity = Activity::Probing(Probe {
                        index: 0,
                        holding: frames,
                        settling: 0,
                        hold_frames: frames,
                        start_yaw: yaw,
                        travelled: 0.0,
                        last_yaw: yaw,
                    });
                    harness_log!(
                        "probe: holding each of the {} channels for {frames} frames and \
                         reporting the yaw it moved",
                        Channel::SWEEP.len()
                    );
                }
                None => harness_log!(
                    "probe REFUSED: no camera yaw source is installed, and a probe with nothing \
                     to measure is just a handful of presses."
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
            // a cutscene), and an input held through that is exactly the stuck-input failure
            // every budget in this crate exists to prevent.
            Activity::Turning { turn, .. } if yaw.is_none() => {
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
            Activity::Turning { turn, channel } => {
                let channel = *channel;
                // Unreachable with `None`: the guarded arm above took that case.
                let yaw = yaw.unwrap_or_default();
                match turn.step(yaw) {
                    Outcome::Driving(value) => return channel.authored(value),
                    Outcome::Reached { frames, travelled } => harness_log!(
                        "turn REACHED in {frames} frames on {channel}: the camera's own yaw \
                         moved {travelled:.2} degrees"
                    ),
                    Outcome::Timeout { travelled } => harness_log!(
                        "turn TIMED OUT on {channel}: the camera's yaw moved {travelled:.2} \
                         degrees before the frame budget ran out -- released"
                    ),
                    Outcome::NoResponse { frames } => harness_log!(
                        "turn NO RESPONSE: {channel} was pushed for {frames} frames and the \
                         camera's yaw did not move. That channel is not the camera in this \
                         session -- run `probe` to find out which one is, then `channel <name>`."
                    ),
                }
                *activity = Activity::Idle;
                Authored::NOTHING
            }
            Activity::Probing(_) => Self::probe_frame(activity, yaw.unwrap_or_default()),
        }
    }

    /// One frame of a channel sweep.
    ///
    /// The measurement is taken over the hold AND the settle that follows it, not just the hold.
    /// That is deliberate: this camera springs back toward the player when an input is released,
    /// so "how far did it move while I pushed" and "where did it end up" are different numbers,
    /// and the second is the one that answers whether a channel controls the camera.
    fn probe_frame(activity: &mut Activity, yaw: f32) -> Authored {
        let Activity::Probing(probe) = activity else {
            return Authored::NOTHING;
        };
        probe.travelled += crate::turn::wrap_degrees(yaw - probe.last_yaw);
        probe.last_yaw = yaw;
        let channel = Channel::SWEEP[probe.index];

        if probe.holding > 0 {
            probe.holding -= 1;
            if probe.holding == 0 {
                probe.settling = PROBE_SETTLE_FRAMES;
            }
            return channel.authored(channel.limits().max);
        }

        probe.settling -= 1;
        if probe.settling > 0 {
            return Authored::NOTHING;
        }
        harness_log!(
            "probe: {channel} held at +{} for {} frames left the camera's yaw {:.2} degrees from \
             where it started ({:.2} -> {:.2})",
            channel.limits().max,
            probe.hold_frames,
            probe.travelled,
            probe.start_yaw,
            yaw
        );
        probe.index += 1;
        if probe.index >= Channel::SWEEP.len() {
            harness_log!("probe complete -- `channel <name>` points `turn` at the winner");
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
            Activity::Turning { .. } => "turn",
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
    fn the_default_channel_is_the_one_that_needs_no_hardware() {
        // The correction this whole revision is: pad axis 3 was the default and a session with
        // nothing plugged in could never turn the camera with it.
        assert_eq!(Session::new().channel(), Channel::MouseX);
    }

    #[test]
    fn channel_names_round_trip() {
        for channel in Channel::SWEEP {
            assert_eq!(Channel::parse(&channel.to_string()), Some(channel));
        }
        assert_eq!(Channel::parse("MOUSE-X"), Some(Channel::MouseX));
        assert_eq!(Channel::parse("pad3"), Some(Channel::PadAxis(3)));
        assert_eq!(
            Channel::parse("pad6"),
            None,
            "there are six axes; a seventh index would be a write past the block"
        );
        assert_eq!(Channel::parse("elbow"), None);
    }

    #[test]
    fn a_mouse_channel_authors_a_delta_and_a_pad_channel_an_axis() {
        assert_eq!(
            Channel::MouseX.authored(12.0).mouse,
            Some([12.0, 0.0]),
            "mouse-x must not also move y, or every turn would also pitch"
        );
        assert_eq!(Channel::MouseY.authored(12.0).mouse, Some([0.0, 12.0]));
        assert_eq!(Channel::PadAxis(3).authored(0.5).axes[3], Some(0.5));
        assert!(Channel::PadAxis(3).authored(0.5).mouse.is_none());
    }

    #[test]
    fn a_hold_releases_itself_when_its_frames_run_out() {
        let mut session = Session::new();
        session.accept(
            Command::Axis {
                index: 3,
                value: 1.0,
                frames: 3,
            },
            None,
        );
        for expected in 0..3 {
            let frame = session.frame(None);
            assert_eq!(
                frame.authored.axes[3],
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
    fn a_turn_drives_the_mouse_and_stops_when_the_yaw_arrives() {
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
            if let Some([dx, _]) = frame.authored.mouse {
                // A plant at a fifth of a degree per count.
                yaw = crate::turn::wrap_degrees(yaw + dx * 0.2);
            }
            frames += 1;
            assert!(frames < 500, "the turn should have finished long ago");
        }
        assert!(
            (yaw - 30.0).abs() <= 6.0,
            "camera ended at {yaw}, which is not where 30 degrees is"
        );
    }

    #[test]
    fn a_turn_follows_the_channel_it_was_told_to_use() {
        let mut session = Session::new();
        session.accept(Command::SetChannel(Channel::PadAxis(3)), None);
        session.accept(
            Command::Turn {
                degrees: 30.0,
                budget: 50,
            },
            Some(0.0),
        );
        let frame = session.frame(Some(0.0));
        assert!(
            frame.authored.axes[3].is_some(),
            "after `channel pad3` the turn must push pad3, not the mouse"
        );
        assert!(frame.authored.mouse.is_none());
    }

    #[test]
    fn a_turn_that_loses_its_camera_releases_the_input() {
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
            "an input held through a camera that stopped answering is the stuck-input failure"
        );
        assert!(!session.busy());
    }

    #[test]
    fn a_probe_tries_every_channel_and_then_stops() {
        let mut session = Session::new();
        let mut yaw = 0.0f32;
        session.accept(Command::Probe { frames: 4 }, Some(yaw));
        let mut seen_axes = [false; AXIS_COUNT];
        let (mut seen_mouse_x, mut seen_mouse_y) = (false, false);
        let mut frames = 0;
        while session.busy() {
            let frame = session.frame(Some(yaw));
            for (index, value) in frame.authored.axes.iter().enumerate() {
                if value.is_some() {
                    seen_axes[index] = true;
                }
            }
            if let Some([dx, dy]) = frame.authored.mouse {
                seen_mouse_x |= dx != 0.0;
                seen_mouse_y |= dy != 0.0;
                // Only mouse-x does anything, which is what a probe is for finding out.
                yaw = crate::turn::wrap_degrees(yaw + dx * 0.2);
            }
            frames += 1;
            assert!(frames < 2000, "the probe should have finished long ago");
        }
        assert!(
            seen_mouse_x && seen_mouse_y,
            "both mouse components must be tried"
        );
        assert!(
            seen_axes.iter().all(|touched| *touched),
            "every pad axis must be tried, or the probe is not a survey: {seen_axes:?}"
        );
    }
}
