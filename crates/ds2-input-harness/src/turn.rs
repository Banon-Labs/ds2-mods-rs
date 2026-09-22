//! Turning the camera by a stated number of degrees, by watching what actually happened.
//!
//! # Why this is closed-loop and not a multiplication
//!
//! "Hold the stick for N frames" is not a rotation. Degrees-per-frame-per-unit-of-stick is a
//! product of the game's camera-speed setting, its deadzone, its acceleration curve and the
//! frame rate, none of which this repo has measured and none of which a constant here could be
//! honest about. So nothing here converts degrees into frames. It pushes the axis, reads the
//! camera's own yaw back, and stops when the yaw has moved as far as was asked -- which needs no
//! sensitivity constant and is correct whatever the player's settings are.
//!
//! That also makes the drive SELF-VALIDATING in the sense `AGENTS.md` asks for: a run cannot
//! report "turned 45 degrees" unless the camera's own matrix said so. If the camera did not
//! move, the outcome says `NoResponse` and names how many frames it waited.
//!
//! # Why the axis polarity is discovered rather than declared
//!
//! Which way `+1.0` on an axis turns the camera is a mapper binding plus a per-player Y-inversion
//! setting, and `ds2-rva` deliberately records only that the float is "pad axis 3". So the
//! controller starts by pushing in an arbitrary direction, watches the sign of the yaw it got,
//! and flips if it was wrong. A wrong guess costs [`PROBE_FRAMES`] frames; a declared constant
//! would have cost a run that turned the camera the wrong way and reported success.
//!
//! # What is measured and what is assumed
//!
//! **Measured, per frame, from the game:** the camera's yaw. **Assumed:** nothing about the
//! relationship between axis and yaw except that it is monotonic within one drive -- which is
//! what lets the sign probe conclude anything. The magnitude floor below is a *choice*, and its
//! rationale is on the constant.

/// Degrees of residual error at which the turn is called done.
///
/// One degree is below what the overlay measurement this exists for can resolve, and chasing
/// tighter than the controller's own per-frame step would make it oscillate.
pub const TOLERANCE_DEGREES: f32 = 1.0;

/// A per-frame yaw change smaller than this is treated as the camera not having moved.
///
/// The camera drifts and springs on its own in this engine (it follows the player), so "exactly
/// zero" is the wrong test for "our axis did nothing".
pub const NOISE_DEGREES: f32 = 0.25;

/// How many frames to push before deciding which way the axis turns the camera.
///
/// Long enough that one frame of camera spring cannot outvote the drive, short enough that a
/// wrong initial guess is a blink.
pub const PROBE_FRAMES: u32 = 10;

/// Error at which the controller starts easing off, in degrees. Above it the channel is held at
/// its maximum.
pub const SLOW_SPAN_DEGREES: f32 = 30.0;

/// How hard the controller is allowed to push, in whatever unit its channel takes.
///
/// A pair rather than two constants because the two channels do not share a unit: a pad axis is
/// a fraction of full deflection and a mouse is pixels of cursor travel per frame. A single
/// `MAX_MAGNITUDE` would have been right for one of them and nonsense for the other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Magnitudes {
    /// Smallest push the controller will ask for.
    pub min: f32,
    /// Largest push the controller will ask for.
    pub max: f32,
}

impl Magnitudes {
    /// For a pad axis, in fractions of full deflection.
    ///
    /// **The floor is a choice, not a measurement.** DARK SOULS II's own right-stick deadzone
    /// has not been read out of the binary, so it is set above the value Microsoft publishes as
    /// the XInput right-stick deadzone (`XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE` is 8689 of 32767,
    /// about `0.265` of full scale) with room to spare. If a live run shows the camera stalling
    /// near the goal instead of converging, this floor is the first thing to raise -- the drive
    /// logs the magnitude it asked for, so a stall is visible rather than inferred.
    pub const PAD: Self = Self {
        min: 0.4,
        max: ds2_rva::PAD_AXIS_FULL_SCALE,
    };

    /// For the mouse, in pixels of authored cursor travel per frame.
    ///
    /// The floor is ONE PIXEL and that one is not a choice: the consumer differences two
    /// successive cursor positions (`ds2_rva::WINDOWS_MOUSE_DEVICE_POSITION_OFFSET`), so a
    /// one-pixel step is a real, indivisible mouse movement -- there is no deadzone to clear
    /// because there is no deadzone in a subtraction.
    ///
    /// The ceiling IS a choice. Degrees per pixel depends on the player's sensitivity setting,
    /// which this repo has not measured, so 25 is "brisk but not a spin" rather than a derived
    /// number. Getting it wrong costs convergence speed and nothing else: the controller eases
    /// off as the goal approaches and stops when the camera's own yaw says it has arrived.
    pub const MOUSE: Self = Self {
        min: 1.0,
        max: 25.0,
    };
}

/// Wrap an angle difference into `-180.0..=180.0`.
///
/// Every yaw comparison goes through this. Comparing raw yaws would make a turn across the
/// `+180 / -180` seam look like a 359-degree turn in the opposite direction, which is the classic
/// way a heading controller spins the wrong way exactly once per revolution.
#[must_use]
pub fn wrap_degrees(mut degrees: f32) -> f32 {
    while degrees > 180.0 {
        degrees -= 360.0;
    }
    while degrees < -180.0 {
        degrees += 360.0;
    }
    degrees
}

/// Which sign of the axis moves the yaw the way the caller asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Polarity {
    /// Not established yet; the probe is running.
    Unknown,
    /// `+magnitude` moves the yaw the way the request wants.
    Forward,
    /// `-magnitude` does.
    Reversed,
}

/// What the turn did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// Still going. The `f32` is the axis value to hold this frame.
    Driving(f32),
    /// The yaw moved as far as was asked. Release the axis.
    Reached {
        /// Frames the turn took.
        frames: u32,
        /// Degrees of yaw actually travelled, signed.
        travelled: f32,
    },
    /// The frame budget ran out with the goal unmet. Release the axis.
    Timeout {
        /// Degrees of yaw actually travelled, signed.
        travelled: f32,
    },
    /// The axis was pushed for [`PROBE_FRAMES`] frames and the camera did not move.
    ///
    /// **This is the answer to "is this axis the camera?", and it is a real answer rather than a
    /// failure to handle.** The axis is not bound to camera yaw on this build with these
    /// settings, and a run that reported success instead would be lying.
    NoResponse {
        /// Frames spent waiting for any movement at all.
        frames: u32,
    },
}

/// A turn in progress.
#[derive(Clone, Copy, Debug)]
pub struct Turn {
    /// What was asked for, kept unchanged. The polarity probe compares against THIS rather than
    /// against `remaining`, because `remaining` can legitimately change sign on an overshoot and
    /// a probe that read it then would conclude the axis was inverted when it was merely fast.
    requested: f32,
    /// Signed degrees still to travel.
    remaining: f32,
    /// Signed degrees travelled so far. Reported, and used by the polarity probe.
    travelled: f32,
    /// The yaw the last [`Turn::step`] saw, so a delta can be taken.
    last_yaw: f32,
    /// How far the camera moved on the previous frame, unsigned.
    ///
    /// This is the controller's only estimate of how fast the camera is, and it is a
    /// MEASUREMENT: it is what the game did, not a sensitivity constant. It is used for exactly
    /// one thing -- deciding when the goal is as close as one push can get it.
    last_step: f32,
    /// Which way to push. Discovered, not declared.
    polarity: Polarity,
    /// Frames elapsed.
    frames: u32,
    /// Frames allowed before giving up.
    budget: u32,
    /// How hard this channel may be pushed, in its own unit.
    limits: Magnitudes,
}

impl Turn {
    /// Begin a turn of `degrees` from the camera's current `yaw`.
    ///
    /// `budget` bounds the whole drive. It is not optional and there is no unbounded form: an
    /// authored channel with no budget is a stuck input, which is the failure mode every bounded
    /// action in this repo exists to prevent. `limits` carries the channel's unit -- see
    /// [`Magnitudes`].
    #[must_use]
    pub fn begin(degrees: f32, yaw: f32, budget: u32, limits: Magnitudes) -> Self {
        Self {
            requested: degrees,
            remaining: degrees,
            travelled: 0.0,
            last_yaw: yaw,
            last_step: 0.0,
            polarity: Polarity::Unknown,
            frames: 0,
            budget,
            limits,
        }
    }

    /// Signed degrees travelled so far.
    #[must_use]
    pub fn travelled(&self) -> f32 {
        self.travelled
    }

    /// Frames elapsed.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Advance one frame against a freshly read camera `yaw`.
    pub fn step(&mut self, yaw: f32) -> Outcome {
        let delta = wrap_degrees(yaw - self.last_yaw);
        self.last_yaw = yaw;
        self.travelled += delta;
        self.remaining -= delta;
        self.last_step = delta.abs();
        self.frames += 1;

        // DONE when the goal is inside tolerance, or when it is closer than half of what one
        // more frame of the camera's own observed speed would move it. Without the second
        // clause a fast camera oscillates around the goal for ever: every push overshoots by
        // more than the tolerance, so the error never gets small, and a turn that has plainly
        // arrived reports `Timeout`. The rate in that clause is measured, not assumed.
        let settle = TOLERANCE_DEGREES.max(self.last_step * 0.5);
        if self.remaining.abs() <= settle {
            return Outcome::Reached {
                frames: self.frames,
                travelled: self.travelled,
            };
        }

        if self.polarity == Polarity::Unknown && self.frames >= PROBE_FRAMES {
            if self.travelled.abs() < NOISE_DEGREES {
                return Outcome::NoResponse {
                    frames: self.frames,
                };
            }
            self.polarity =
                if self.travelled.is_sign_negative() == self.requested.is_sign_negative() {
                    Polarity::Forward
                } else {
                    Polarity::Reversed
                };
        }

        if self.frames >= self.budget {
            return Outcome::Timeout {
                travelled: self.travelled,
            };
        }

        let magnitude = (self.remaining.abs() / SLOW_SPAN_DEGREES * self.limits.max)
            .clamp(self.limits.min, self.limits.max);
        let direction = match self.polarity {
            // During the probe the request's own sign is as good a guess as any, and being wrong
            // costs `PROBE_FRAMES` frames.
            Polarity::Unknown | Polarity::Forward => {
                if self.remaining.is_sign_negative() {
                    -1.0
                } else {
                    1.0
                }
            }
            Polarity::Reversed => {
                if self.remaining.is_sign_negative() {
                    1.0
                } else {
                    -1.0
                }
            }
        };
        Outcome::Driving(direction * magnitude)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera that turns `gain` degrees per frame per unit of axis, in `sign`'s direction, and
    /// ignores anything inside `deadzone`.
    ///
    /// Deliberately crude: the controller must work without knowing any of these three numbers,
    /// so the test's job is to vary them and check that it still lands.
    struct Plant {
        yaw: f32,
        gain: f32,
        sign: f32,
        deadzone: f32,
    }

    impl Plant {
        fn apply(&mut self, axis: f32) {
            if axis.abs() <= self.deadzone {
                return;
            }
            self.yaw = wrap_degrees(self.yaw + self.sign * self.gain * axis);
        }
    }

    /// How close a turn can be expected to land against a given plant.
    ///
    /// Not [`TOLERANCE_DEGREES`] flat: a camera that moves `gain` degrees per frame at full
    /// stick cannot stop closer than about one of its own steps, and the controller knows it --
    /// that is exactly what the `settle` clause in [`Turn::step`] says. A test that demanded
    /// better would be demanding the controller lie.
    fn landing_bound(plant: &Plant) -> f32 {
        TOLERANCE_DEGREES.max(plant.gain * Magnitudes::PAD.max) + 0.001
    }

    /// Run a turn to completion against a plant, returning the final outcome.
    fn run(plant: &mut Plant, degrees: f32, budget: u32) -> Outcome {
        run_with(plant, degrees, budget, Magnitudes::PAD)
    }

    /// Same, for a channel whose unit is not a pad axis.
    fn run_with(plant: &mut Plant, degrees: f32, budget: u32, limits: Magnitudes) -> Outcome {
        let mut turn = Turn::begin(degrees, plant.yaw, budget, limits);
        loop {
            match turn.step(plant.yaw) {
                Outcome::Driving(push) => plant.apply(push),
                done => return done,
            }
        }
    }

    #[test]
    fn the_mouse_channel_lands_with_its_own_unit() {
        // Pixels per frame, not fractions of a stick: a plant whose gain is 0.2 degrees per
        // pixel is a plausible mouse sensitivity, and the controller must converge on it
        // without knowing that number.
        let mut plant = Plant {
            yaw: 0.0,
            gain: 0.2,
            sign: 1.0,
            deadzone: 0.0,
        };
        let bound = TOLERANCE_DEGREES.max(plant.gain * Magnitudes::MOUSE.max) + 0.001;
        let outcome = run_with(&mut plant, 90.0, 2000, Magnitudes::MOUSE);
        let Outcome::Reached { travelled, .. } = outcome else {
            panic!("expected Reached, got {outcome:?}");
        };
        assert!((travelled - 90.0).abs() <= bound, "travelled {travelled}");
    }

    #[test]
    fn a_one_pixel_floor_is_enough_for_a_mouse() {
        // The pad floor exists to clear a deadzone. A cursor difference has none, so the mouse
        // channel must be able to creep the last degree at one pixel a frame rather than
        // oscillating across it.
        assert_eq!(Magnitudes::MOUSE.min, 1.0);
        let mut plant = Plant {
            yaw: 0.0,
            gain: 0.2,
            sign: 1.0,
            // Anything at or below one pixel is swallowed -- so ONLY the floor gets through,
            // and the turn still has to land.
            deadzone: 0.99,
        };
        let outcome = run_with(&mut plant, 5.0, 2000, Magnitudes::MOUSE);
        assert!(
            matches!(outcome, Outcome::Reached { .. }),
            "got {outcome:?}"
        );
    }

    #[test]
    fn a_turn_lands_within_tolerance_whatever_the_gain() {
        for gain in [0.5f32, 2.0, 6.0] {
            let mut plant = Plant {
                yaw: 0.0,
                gain,
                sign: 1.0,
                deadzone: 0.0,
            };
            let bound = landing_bound(&plant);
            let outcome = run(&mut plant, 45.0, 2000);
            let Outcome::Reached { travelled, .. } = outcome else {
                panic!("gain {gain}: expected Reached, got {outcome:?}");
            };
            assert!(
                (travelled - 45.0).abs() <= bound,
                "gain {gain}: travelled {travelled}, bound {bound}"
            );
        }
    }

    #[test]
    fn an_inverted_axis_still_lands() {
        // The case a declared polarity constant would have got wrong half the time.
        let mut plant = Plant {
            yaw: 0.0,
            gain: 2.0,
            sign: -1.0,
            deadzone: 0.0,
        };
        let bound = landing_bound(&plant);
        let outcome = run(&mut plant, 90.0, 2000);
        let Outcome::Reached { travelled, .. } = outcome else {
            panic!("expected Reached, got {outcome:?}");
        };
        assert!((travelled - 90.0).abs() <= bound, "travelled {travelled}");
    }

    #[test]
    fn a_negative_request_turns_the_other_way() {
        let mut plant = Plant {
            yaw: 0.0,
            gain: 2.0,
            sign: 1.0,
            deadzone: 0.0,
        };
        let bound = landing_bound(&plant);
        let outcome = run(&mut plant, -60.0, 2000);
        let Outcome::Reached { travelled, .. } = outcome else {
            panic!("expected Reached, got {outcome:?}");
        };
        assert!((travelled + 60.0).abs() <= bound, "travelled {travelled}");
    }

    #[test]
    fn a_turn_across_the_seam_does_not_spin_the_long_way_round() {
        // Start at 170 degrees and ask for +40, which crosses +180 into -150.
        let mut plant = Plant {
            yaw: 170.0,
            gain: 3.0,
            sign: 1.0,
            deadzone: 0.0,
        };
        let bound = landing_bound(&plant);
        let outcome = run(&mut plant, 40.0, 2000);
        let Outcome::Reached { travelled, .. } = outcome else {
            panic!("expected Reached, got {outcome:?}");
        };
        assert!(
            (travelled - 40.0).abs() <= bound,
            "travelled {travelled}, bound {bound}"
        );
        assert!(
            (wrap_degrees(plant.yaw - (-150.0))).abs() <= bound,
            "ended at {}",
            plant.yaw
        );
    }

    #[test]
    fn an_axis_that_moves_nothing_is_reported_rather_than_waited_on() {
        // The axis is not bound to the camera: the plant swallows everything.
        let mut plant = Plant {
            yaw: 0.0,
            gain: 0.0,
            sign: 1.0,
            deadzone: 0.0,
        };
        let outcome = run(&mut plant, 45.0, 2000);
        assert_eq!(
            outcome,
            Outcome::NoResponse {
                frames: PROBE_FRAMES
            },
            "a dead axis must be named as dead, not run until the budget expires"
        );
    }

    #[test]
    fn a_deadzone_the_controller_cannot_clear_times_out_rather_than_lying() {
        // Everything the floor can ask for is swallowed, so no progress is possible. The turn
        // must report a timeout with what it actually travelled -- never `Reached`.
        let mut plant = Plant {
            yaw: 0.0,
            gain: 4.0,
            sign: 1.0,
            deadzone: 1.0,
        };
        let outcome = run(&mut plant, 45.0, 200);
        assert!(
            matches!(
                outcome,
                Outcome::NoResponse { .. } | Outcome::Timeout { .. }
            ),
            "got {outcome:?}"
        );
    }

    #[test]
    fn the_budget_is_honoured() {
        let mut plant = Plant {
            yaw: 0.0,
            // Slow enough that 30 frames cannot cover 180 degrees.
            gain: 0.5,
            sign: 1.0,
            deadzone: 0.0,
        };
        let outcome = run(&mut plant, 180.0, 30);
        let Outcome::Timeout { travelled } = outcome else {
            panic!("expected Timeout, got {outcome:?}");
        };
        assert!(travelled.abs() < 180.0);
    }

    #[test]
    fn wrapping_is_symmetric_and_idempotent() {
        assert_eq!(wrap_degrees(0.0), 0.0);
        assert_eq!(wrap_degrees(180.0), 180.0);
        assert_eq!(wrap_degrees(-180.0), -180.0);
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
        assert_eq!(wrap_degrees(720.0), 0.0);
        assert_eq!(wrap_degrees(wrap_degrees(359.0)), wrap_degrees(359.0));
    }

    #[test]
    fn a_turn_that_is_already_satisfied_ends_on_the_first_frame() {
        let mut turn = Turn::begin(0.0, 12.0, 100, Magnitudes::PAD);
        assert!(matches!(
            turn.step(12.0),
            Outcome::Reached { frames: 1, .. }
        ));
    }
}
