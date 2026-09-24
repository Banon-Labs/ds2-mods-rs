//! What the harness is currently pretending the hardware produced.
//!
//! # Why every field is an `Option`
//!
//! Authoring is per-field, not per-device. A run that wants the camera to turn wants ONE axis
//! written and everything else left exactly as the player's hardware left it -- otherwise the
//! act of turning the camera would also centre the movement stick and release every button, and
//! the measurement would be of a different situation than the one anyone asked about. `None`
//! means "the harness has no opinion about this field", and the detour then does not touch it.
//!
//! Suppression is the separate, louder decision: see [`crate::drive`]. Blocking zeroes the
//! device first and authoring stamps on top, so an authored field survives a block and an
//! un-authored one does not.
//!
//! # Why the published copy is atomics rather than a `Mutex`
//!
//! Every reader and every writer is the game thread -- the detours are the tick, and the command
//! file is polled from inside one of them -- so there is no contention to arbitrate. What there
//! IS, is a lock that would be held inside a per-frame detour on the engine's input path, which
//! is the kind of thing that turns a panic somewhere else into a hung game. Relaxed atomics have
//! no such failure mode and cost nothing here.

use core::sync::atomic::{AtomicU64, Ordering};

/// How many pad axes there are, restated from the address table so the array length and the
/// offsets can never disagree.
pub const AXIS_COUNT: usize = ds2_rva::PAD_DEVICE_AXIS_COUNT;

/// Sentinel bit for "this slot holds a value". A slot of exactly `0` is `None`; a slot with this
/// bit set carries an `f32`'s bits in its low 32.
///
/// A plain `0` cannot be confused with a stored value, because a stored `0.0` encodes as
/// `PRESENT | 0` and never as bare zero.
const PRESENT: u64 = 1 << 32;

/// Encode an optional `f32` into one slot.
const fn encode(value: Option<f32>) -> u64 {
    match value {
        None => 0,
        Some(v) => PRESENT | v.to_bits() as u64,
    }
}

/// Decode one slot.
const fn decode(slot: u64) -> Option<f32> {
    if slot & PRESENT == 0 {
        None
    } else {
        Some(f32::from_bits(slot as u32))
    }
}

/// Encode an optional `u16` button mask into one slot, with the same sentinel.
const fn encode_buttons(value: Option<u16>) -> u64 {
    match value {
        None => 0,
        Some(v) => PRESENT | v as u64,
    }
}

/// Decode a button-mask slot.
const fn decode_buttons(slot: u64) -> Option<u16> {
    if slot & PRESENT == 0 {
        None
    } else {
        Some(slot as u16)
    }
}

/// One frame's worth of authored device state.
///
/// Units are the engine's own, which is the whole reason this struct carries floats rather than
/// stick counts: `ds2_rva::PAD_AXIS_FULL_SCALE` documents that every backend normalises an axis
/// to `-1.0..=1.0` before anything downstream sees it, so that is the range a caller works in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Authored {
    /// Pad axes, indexed by `ds2_rva::PAD_AXIS_*`.
    pub axes: [Option<f32>; AXIS_COUNT],
    /// Pad button bitmask (`XINPUT_GAMEPAD.wButtons` layout).
    pub buttons: Option<u16>,
    /// Left and right triggers, `0.0..=1.0`.
    pub triggers: [Option<f32>; 2],
    /// How far to move the authored cursor this frame, in PIXELS of client-space travel.
    ///
    /// A delta, not a coordinate, and the distinction is load-bearing. DARK SOULS II's camera
    /// reads an ABSOLUTE cursor position (`ds2_rva::WINDOWS_MOUSE_DEVICE_POSITION_OFFSET`) and
    /// `parseCameraInput` differences two successive values of it, so a constant position is one
    /// frame of motion and then stillness. `crate::device` keeps a virtual cursor and adds this
    /// to it each frame, which is what turns a delta into the moving position the consumer
    /// wants.
    pub mouse: Option<[f32; 2]>,
}

impl Authored {
    /// No opinion about anything. What the harness publishes when it is not driving.
    pub const NOTHING: Self = Self {
        axes: [None; AXIS_COUNT],
        buttons: None,
        triggers: [None, None],
        mouse: None,
    };

    /// Is the harness authoring anything at all?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::NOTHING
    }

    /// One axis held, nothing else touched.
    #[must_use]
    pub fn axis(index: usize, value: f32) -> Self {
        let mut authored = Self::NOTHING;
        if let Some(slot) = authored.axes.get_mut(index) {
            *slot = Some(value);
        }
        authored
    }

    /// One frame of mouse motion, in pixels; nothing else touched.
    #[must_use]
    pub fn mouse_delta(dx: f32, dy: f32) -> Self {
        Self {
            mouse: Some([dx, dy]),
            ..Self::NOTHING
        }
    }

    /// Clamp every axis and trigger into the range the engine's own normalisation produces.
    ///
    /// Called on the way into [`Self::publish`] rather than left to callers, because a value
    /// outside it is one no hardware could have produced, and the point of writing the device
    /// object instead of synthesising OS input is that the result is indistinguishable from
    /// hardware. A caller that asks for `10.0` gets `1.0` and a log line, not a stick the game
    /// has never seen.
    #[must_use]
    pub fn clamped(mut self) -> Self {
        let full = ds2_rva::PAD_AXIS_FULL_SCALE;
        for value in self.axes.iter_mut().flatten() {
            *value = value.clamp(-full, full);
        }
        for value in self.triggers.iter_mut().flatten() {
            *value = value.clamp(0.0, full);
        }
        self
    }
}

/// The published copy the detours read. Written by [`Authored::publish`], read by
/// [`Authored::current`].
static AXES: [AtomicU64; AXIS_COUNT] = [const { AtomicU64::new(0) }; AXIS_COUNT];
static BUTTONS: AtomicU64 = AtomicU64::new(0);
static TRIGGERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static MOUSE_X: AtomicU64 = AtomicU64::new(0);
static MOUSE_Y: AtomicU64 = AtomicU64::new(0);

impl Authored {
    /// Make this the state the detours stamp onto the device objects from now on.
    pub fn publish(self) {
        let clamped = self.clamped();
        for (slot, value) in AXES.iter().zip(clamped.axes) {
            slot.store(encode(value), Ordering::Relaxed);
        }
        BUTTONS.store(encode_buttons(clamped.buttons), Ordering::Relaxed);
        for (slot, value) in TRIGGERS.iter().zip(clamped.triggers) {
            slot.store(encode(value), Ordering::Relaxed);
        }
        let (x, y) = match clamped.mouse {
            Some([x, y]) => (Some(x), Some(y)),
            None => (None, None),
        };
        MOUSE_X.store(encode(x), Ordering::Relaxed);
        MOUSE_Y.store(encode(y), Ordering::Relaxed);
    }

    /// What was last published.
    #[must_use]
    pub fn current() -> Self {
        let mut axes = [None; AXIS_COUNT];
        for (out, slot) in axes.iter_mut().zip(AXES.iter()) {
            *out = decode(slot.load(Ordering::Relaxed));
        }
        let mouse = match (
            decode(MOUSE_X.load(Ordering::Relaxed)),
            decode(MOUSE_Y.load(Ordering::Relaxed)),
        ) {
            (Some(x), Some(y)) => Some([x, y]),
            // Half a mouse delta is not a mouse delta. The two slots are only ever written
            // together, so this arm is unreachable in practice and is here so that a future
            // change that breaks the pairing fails closed rather than writing a garbage axis.
            _ => None,
        };
        Self {
            axes,
            buttons: decode_buttons(BUTTONS.load(Ordering::Relaxed)),
            triggers: [
                decode(TRIGGERS[0].load(Ordering::Relaxed)),
                decode(TRIGGERS[1].load(Ordering::Relaxed)),
            ],
            mouse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published copy is process-global, which is exactly what it is for and a hazard for a
    /// test harness that runs threads in parallel. Every test that touches it takes this first.
    static SERIALIZE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_stored_zero_is_not_an_absent_value() {
        // The bug this encoding exists to make impossible: centring a stick is `Some(0.0)`, and
        // if that encoded as a bare zero the detour would read "no opinion" and leave the
        // player's own stick in place -- so a release would silently never happen.
        assert_eq!(decode(encode(Some(0.0))), Some(0.0));
        assert_eq!(decode(encode(None)), None);
        assert_ne!(encode(Some(0.0)), encode(None));
    }

    #[test]
    fn every_float_survives_the_round_trip() {
        for value in [-1.0f32, -0.5, 0.0, 0.25, 1.0, f32::MIN_POSITIVE] {
            assert_eq!(decode(encode(Some(value))), Some(value));
        }
    }

    #[test]
    fn a_full_button_mask_survives_the_round_trip() {
        assert_eq!(decode_buttons(encode_buttons(Some(0))), Some(0));
        assert_eq!(decode_buttons(encode_buttons(Some(0xffff))), Some(0xffff));
        assert_eq!(decode_buttons(encode_buttons(None)), None);
    }

    #[test]
    fn publishing_round_trips_through_the_atomics() {
        let _serialized = SERIALIZE.lock().unwrap_or_else(|error| error.into_inner());
        let authored = Authored {
            axes: [None, Some(0.0), None, Some(-0.75), None, None],
            buttons: Some(0x1000),
            triggers: [Some(1.0), None],
            mouse: Some([12.0, -3.0]),
        };
        authored.publish();
        assert_eq!(Authored::current(), authored);
        Authored::NOTHING.publish();
        assert!(Authored::current().is_empty());
    }

    #[test]
    fn out_of_range_values_are_clamped_to_what_hardware_could_produce() {
        let authored = Authored {
            axes: [Some(10.0), Some(-10.0), None, None, None, None],
            triggers: [Some(5.0), Some(-1.0)],
            ..Authored::NOTHING
        }
        .clamped();
        assert_eq!(authored.axes[0], Some(1.0));
        assert_eq!(authored.axes[1], Some(-1.0));
        assert_eq!(authored.triggers[0], Some(1.0));
        assert_eq!(
            authored.triggers[1],
            Some(0.0),
            "a trigger is 0.0..=1.0 in the engine's own normalisation, never negative"
        );
    }

    #[test]
    fn the_axis_array_is_as_long_as_the_address_table_says() {
        // Not a tautology: `axes` is a fixed-size array and this is the assertion that its size
        // is the one the six `movss` in the pad poll write, rather than a number typed here.
        assert_eq!(AXIS_COUNT, 6);
        const { assert!(ds2_rva::PAD_AXIS_RIGHT_X < AXIS_COUNT) };
        const { assert!(ds2_rva::PAD_AXIS_RIGHT_Y < AXIS_COUNT) };
    }
}
