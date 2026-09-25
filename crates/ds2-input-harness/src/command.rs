//! The instruction an agent outside the process sends, and the gate that stops it repeating.
//!
//! # The protocol, and why it is a file with a number on top
//!
//! `<Game>/ds2-input-harness-cmd.txt` holds two lines:
//!
//! ```text
//! <sequence number>
//! <command>
//! ```
//!
//! The command runs when the NUMBER changes, not when the file changes. That is what makes the
//! file a queue rather than a standing instruction: rewriting it with the same number is inert,
//! a command cannot fire twice because a frame happened to re-read the file, and a half-written
//! file is simply a parse that fails until the next poll. `../er-mods-rs` arrived at the same
//! shape for the same reason (`crates/er-input-harness/src/repl.rs`); this is that idea, not
//! that code.
//!
//! A file rather than an environment variable, because `steam -applaunch` hands the request to
//! an already-running Steam client over IPC and the game inherits THAT client's environment --
//! the lesson `ds2-loader`'s own config already records.
//!
//! # The grammar
//!
//! ```text
//! block <frames>              suppress every human input for N frames
//! unblock                     stop suppressing now
//! axis <index> <value> <frames>   hold one pad axis at a value
//! mouse <dx> <dy> <frames>    move the authored cursor N pixels per frame
//! buttons <hex> <frames>      hold a pad button mask for N frames
//! turn <degrees> [frames]     turn the camera, watching its own yaw (closed loop)
//! probe [frames]              hold each channel in turn and report the yaw each one moved
//! channel <name>              what `turn` drives: mouse-x, mouse-y, pad0..pad5
//! release                     stop authoring anything
//! status                      write the current state to the log
//! ```
//!
//! Every verb that presses anything takes a frame count, and there is no unbounded form of any
//! of them. That is the same rule `ds2-safe-input` states and for the same reason: an authored
//! input is only bounded if something guarantees the release.

/// Longest a `block` may last. About ten minutes at 60fps.
///
/// A cap rather than a preference: blocking is the one command that can leave a person unable to
/// pause or quit their own game, so "forever" is not on the menu and a harness that wedges
/// cannot hold the input path hostage for the rest of the session.
pub const MAX_BLOCK_FRAMES: u32 = 36_000;

/// Longest any authored press may be held.
pub const MAX_HOLD_FRAMES: u32 = MAX_BLOCK_FRAMES;

/// Frames a `turn` is allowed if the command does not say. Thirty seconds at 60fps, which is
/// far longer than any real turn and short enough that a wedged drive ends on its own.
pub const DEFAULT_TURN_BUDGET_FRAMES: u32 = 1_800;

/// Frames each axis is held for by `probe` if the command does not say.
pub const DEFAULT_PROBE_FRAMES: u32 = 30;

/// What the file asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Zero every human input the engine reads, for this many frames.
    Block {
        /// How long the zeroing lasts.
        frames: u32,
    },
    /// Stop zeroing.
    Unblock,
    /// Hold one pad axis (`ds2_rva::PAD_AXIS_*`) at `value` for `frames`.
    Axis {
        /// Which axis, as a `ds2_rva::PAD_AXIS_*` index.
        index: usize,
        /// What to hold it at.
        value: f32,
        /// For how long.
        frames: u32,
    },
    /// Move the authored cursor `dx`/`dy` PIXELS PER FRAME for `frames`.
    ///
    /// Not a one-off jump: the consumer differences two successive cursor positions, so a
    /// constant position is a single frame of motion followed by stillness. See
    /// [`crate::authored::Authored::mouse`].
    Mouse {
        /// Pixels per frame, horizontally.
        dx: f32,
        /// Pixels per frame, vertically.
        dy: f32,
        /// For how long.
        frames: u32,
    },
    /// Hold a pad button mask for `frames`.
    Buttons {
        /// The button bits to hold down.
        mask: u16,
        /// For how long.
        frames: u32,
    },
    /// Turn the camera by `degrees`, closed-loop, giving up after `budget` frames.
    Turn {
        /// How far to turn, signed.
        degrees: f32,
        /// The frame budget before it gives up.
        budget: u32,
    },
    /// Hold each channel in turn and report how far the camera's yaw moved for each.
    Probe {
        /// How long to hold each channel before moving to the next.
        frames: u32,
    },
    /// Point `turn` at a different input.
    SetChannel(crate::drive::Channel),
    /// Stop authoring anything. Does not lift a `block`.
    Release,
    /// Write the current state to the log.
    Status,
}

/// Why a line could not be used.
///
/// Reported rather than swallowed, and the rejected text is carried so the log can print what
/// was actually written instead of "invalid command".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Nothing but whitespace.
    Empty,
    /// The first word is not a verb.
    UnknownVerb(String),
    /// The verb is real but the arguments are not.
    BadArguments {
        /// The verb that was recognised.
        verb: &'static str,
        /// What its arguments should have looked like.
        expected: &'static str,
    },
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "empty command"),
            Self::UnknownVerb(word) => write!(formatter, "unknown verb {word:?}"),
            Self::BadArguments { verb, expected } => {
                write!(formatter, "{verb} expects {expected}")
            }
        }
    }
}

/// Clamp a frame count into `1..=`[`MAX_HOLD_FRAMES`].
///
/// Zero frames is not a press, it is a no-op that looks like one, so it becomes one frame rather
/// than being rejected -- the caller meant "briefly".
fn frames(raw: u32) -> u32 {
    raw.clamp(1, MAX_HOLD_FRAMES)
}

/// Parse one command line.
///
/// # Errors
///
/// Returns the reason the line could not be used, for the log.
pub fn parse(line: &str) -> Result<Command, ParseError> {
    let mut words = line.split_whitespace();
    let Some(verb) = words.next() else {
        return Err(ParseError::Empty);
    };
    let lowered = verb.to_ascii_lowercase();
    let rest: Vec<&str> = words.collect();

    match lowered.as_str() {
        "block" => {
            let raw: u32 = rest.first().and_then(|word| word.parse().ok()).ok_or(
                ParseError::BadArguments {
                    verb: "block",
                    expected: "<frames>",
                },
            )?;
            Ok(Command::Block {
                frames: raw.clamp(1, MAX_BLOCK_FRAMES),
            })
        }
        "unblock" => Ok(Command::Unblock),
        "release" => Ok(Command::Release),
        "status" => Ok(Command::Status),
        "channel" => rest
            .first()
            .and_then(|word| crate::drive::Channel::parse(word))
            .map(Command::SetChannel)
            .ok_or(ParseError::BadArguments {
                verb: "channel",
                expected: "mouse-x | mouse-y | pad0..pad5",
            }),
        "axis" => {
            let bad = ParseError::BadArguments {
                verb: "axis",
                expected: "<index> <value> <frames>",
            };
            let [index, value, held] = rest.as_slice() else {
                return Err(bad);
            };
            let index: usize = index.parse().map_err(|_| bad.clone())?;
            let value: f32 = value.parse().map_err(|_| bad.clone())?;
            let held: u32 = held.parse().map_err(|_| bad.clone())?;
            if index >= crate::authored::AXIS_COUNT || !value.is_finite() {
                return Err(bad);
            }
            Ok(Command::Axis {
                index,
                value,
                frames: frames(held),
            })
        }
        "mouse" => {
            let bad = ParseError::BadArguments {
                verb: "mouse",
                expected: "<dx> <dy> <frames>",
            };
            let [dx, dy, held] = rest.as_slice() else {
                return Err(bad);
            };
            let dx: f32 = dx.parse().map_err(|_| bad.clone())?;
            let dy: f32 = dy.parse().map_err(|_| bad.clone())?;
            let held: u32 = held.parse().map_err(|_| bad.clone())?;
            if !dx.is_finite() || !dy.is_finite() {
                return Err(bad);
            }
            Ok(Command::Mouse {
                dx,
                dy,
                frames: frames(held),
            })
        }
        "buttons" => {
            let bad = ParseError::BadArguments {
                verb: "buttons",
                expected: "<hex mask> <frames>",
            };
            let [mask, held] = rest.as_slice() else {
                return Err(bad);
            };
            let mask = mask.strip_prefix("0x").unwrap_or(mask);
            let mask = u16::from_str_radix(mask, 16).map_err(|_| bad.clone())?;
            let held: u32 = held.parse().map_err(|_| bad.clone())?;
            Ok(Command::Buttons {
                mask,
                frames: frames(held),
            })
        }
        "turn" => {
            let bad = ParseError::BadArguments {
                verb: "turn",
                expected: "<degrees> [budget frames]",
            };
            let degrees: f32 = rest
                .first()
                .ok_or(bad.clone())?
                .parse()
                .map_err(|_| bad.clone())?;
            if !degrees.is_finite() {
                return Err(bad);
            }
            let budget = match rest.get(1) {
                None => DEFAULT_TURN_BUDGET_FRAMES,
                Some(word) => frames(word.parse().map_err(|_| bad.clone())?),
            };
            Ok(Command::Turn { degrees, budget })
        }
        "probe" => {
            let held = match rest.first() {
                None => DEFAULT_PROBE_FRAMES,
                Some(word) => frames(word.parse().map_err(|_| ParseError::BadArguments {
                    verb: "probe",
                    expected: "[frames per axis]",
                })?),
            };
            Ok(Command::Probe { frames: held })
        }
        _ => Err(ParseError::UnknownVerb(verb.to_owned())),
    }
}

/// Split the two-line file into its sequence number and its command line.
///
/// Returns `None` for anything that is not "a number, then a line" -- including a file caught
/// half-written, which is the normal case rather than an error and is why this is an `Option`
/// and not a `Result`.
#[must_use]
pub fn split(contents: &str) -> Option<(u64, &str)> {
    let mut lines = contents.lines();
    let sequence: u64 = lines.next()?.trim().parse().ok()?;
    let command = lines.next()?.trim();
    if command.is_empty() {
        return None;
    }
    Some((sequence, command))
}

/// Remembers which sequence number has already been run.
#[derive(Clone, Copy, Debug, Default)]
pub struct SequenceGate {
    last: Option<u64>,
}

impl SequenceGate {
    /// Nothing seen yet. Same state [`Default`] produces, in a form a `static` can be built from.
    pub const NEW: Self = Self { last: None };

    /// Hand the gate a file's contents; get back the command to run, if it is a new one.
    ///
    /// The FIRST sequence number seen is accepted, whatever it is. That matters for a harness
    /// that attaches to a game whose command file is left over from a previous run -- the number
    /// is not reset, so a gate that demanded "greater than zero" would either refuse everything
    /// or replay a stale command. Accepting the first and then demanding a change is the rule
    /// that behaves the same either way.
    pub fn take<'a>(&mut self, contents: &'a str) -> Option<&'a str> {
        let (sequence, command) = split(contents)?;
        if self.last == Some(sequence) {
            return None;
        }
        self.last = Some(sequence);
        Some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verb_parses() {
        assert_eq!(parse("block 600"), Ok(Command::Block { frames: 600 }));
        assert_eq!(parse("unblock"), Ok(Command::Unblock));
        assert_eq!(parse("release"), Ok(Command::Release));
        assert_eq!(parse("status"), Ok(Command::Status));
        assert_eq!(
            parse("axis 3 -0.5 20"),
            Ok(Command::Axis {
                index: 3,
                value: -0.5,
                frames: 20
            })
        );
        assert_eq!(
            parse("mouse 12 -4 10"),
            Ok(Command::Mouse {
                dx: 12.0,
                dy: -4.0,
                frames: 10
            })
        );
        assert_eq!(
            parse("buttons 0x1000 5"),
            Ok(Command::Buttons {
                mask: 0x1000,
                frames: 5
            })
        );
        assert_eq!(
            parse("turn 45"),
            Ok(Command::Turn {
                degrees: 45.0,
                budget: DEFAULT_TURN_BUDGET_FRAMES
            })
        );
        assert_eq!(
            parse("turn -90 300"),
            Ok(Command::Turn {
                degrees: -90.0,
                budget: 300
            })
        );
        assert_eq!(
            parse("probe"),
            Ok(Command::Probe {
                frames: DEFAULT_PROBE_FRAMES
            })
        );
        assert_eq!(
            parse("channel pad3"),
            Ok(Command::SetChannel(crate::drive::Channel::PadAxis(3)))
        );
        assert_eq!(
            parse("channel mouse-x"),
            Ok(Command::SetChannel(crate::drive::Channel::MouseX))
        );
        assert!(parse("channel elbow").is_err());
        assert!(parse("channel").is_err());
    }

    #[test]
    fn a_verb_is_case_insensitive_and_whitespace_tolerant() {
        assert_eq!(parse("  TURN   45  "), parse("turn 45"));
    }

    #[test]
    fn a_block_cannot_outlast_the_cap() {
        assert_eq!(
            parse("block 99999999"),
            Ok(Command::Block {
                frames: MAX_BLOCK_FRAMES
            }),
            "a block long enough to strand the player at the keyboard is not a block, it is a lockout"
        );
        assert_eq!(parse("block 0"), Ok(Command::Block { frames: 1 }));
    }

    #[test]
    fn a_hold_cannot_outlast_the_cap() {
        let Ok(Command::Axis { frames, .. }) = parse("axis 3 1.0 99999999") else {
            panic!("expected an axis command");
        };
        assert_eq!(frames, MAX_HOLD_FRAMES);
    }

    #[test]
    fn an_axis_index_off_the_end_is_refused() {
        // The array is six long because six `movss` write it; a seventh index would be a write
        // into whatever follows the axis block on a live engine object.
        assert!(parse("axis 6 1.0 10").is_err());
        assert!(parse("axis 99 1.0 10").is_err());
    }

    #[test]
    fn a_non_finite_value_is_refused() {
        assert!(parse("axis 3 NaN 10").is_err());
        assert!(parse("axis 3 inf 10").is_err());
        assert!(parse("turn nan").is_err());
        assert!(parse("mouse inf 0 10").is_err());
    }

    #[test]
    fn junk_is_named_rather_than_swallowed() {
        assert_eq!(parse(""), Err(ParseError::Empty));
        assert_eq!(
            parse("wiggle 3"),
            Err(ParseError::UnknownVerb("wiggle".to_owned()))
        );
        assert!(matches!(
            parse("axis 3"),
            Err(ParseError::BadArguments { verb: "axis", .. })
        ));
    }

    #[test]
    fn the_gate_runs_a_command_once() {
        let mut gate = SequenceGate::default();
        assert_eq!(gate.take("7\nturn 45\n"), Some("turn 45"));
        assert_eq!(
            gate.take("7\nturn 45\n"),
            None,
            "the same number must not fire twice -- that is the whole point of the number"
        );
        assert_eq!(gate.take("8\nturn 45\n"), Some("turn 45"));
    }

    #[test]
    fn the_gate_accepts_whatever_number_it_first_sees() {
        // A command file left over from a previous session starts at some arbitrary number.
        let mut gate = SequenceGate::default();
        assert_eq!(gate.take("4212\nrelease\n"), Some("release"));
    }

    #[test]
    fn a_half_written_file_is_not_a_command() {
        let mut gate = SequenceGate::default();
        assert_eq!(gate.take(""), None);
        assert_eq!(gate.take("9"), None, "a number with no command line yet");
        assert_eq!(gate.take("9\n"), None);
        assert_eq!(gate.take("9\n   \n"), None);
        assert_eq!(gate.take("not-a-number\nturn 45\n"), None);
        // ...and the real thing still works afterwards.
        assert_eq!(gate.take("9\nturn 45\n"), Some("turn 45"));
    }
}
