//! Controller/menu INTENT that automation is allowed to emit, and nothing wider.
//!
//! # Why the vocabulary is a whitelist
//!
//! The restriction is deliberate, not an oversight. An automation layer that can inject any key
//! and any mouse motion can do anything a player can, including things nobody asked it for, and
//! the failure mode is a stuck button in a live session -- the game keeps reading a press that no
//! finger is making. So the surface is nine buttons ([`SafeButton`]), each action is bounded in
//! GAME FRAMES rather than host sleeps (so nothing here depends on wall-clock timing, pointer
//! focus or window activation), and every action is validated before a backend ever sees it.
//!
//! A [`SafeInputBackend`] maps that intent onto a concrete mechanism and MUST release every held
//! button when it is dropped or when a sequence fails. That obligation is the other half of the
//! design: a bounded action is only bounded if something guarantees the release.
//!
//! # What a backend would attach to
//!
//! No backend exists in this repo yet, and none has been written or tested against the game.
//! What is verified is only that both backend shapes have somewhere to attach: `DarkSoulsII.exe`
//! imports `DINPUT8.dll` (one thunk, `DirectInput8Create`) and `XINPUT1_3.dll` (two thunks).
//! Nothing further about how DARK SOULS II reads input has been established, and nothing in this
//! crate assumes anything about it.
//!
//! # No game knowledge
//!
//! This crate is host-side logic: a whitelist, a frame budget, a trait and a recording backend.
//! No addresses, no offsets, no structure layouts, no `windows` dependency -- which is why
//! [`RecordingBackend`] makes the whole thing provable with `cargo test` on a Linux host, with no
//! game running.

use std::fmt;

/// The `max_hold_frames` a [`SafeInputConfig`] starts with.
pub const DEFAULT_MAX_HOLD_FRAMES: u16 = 30;
/// The shortest an action may last. An action of no frames is one no backend can express.
pub const MIN_ACTION_FRAMES: u16 = 1;

/// Whitelisted logical inputs that the automation layer is allowed to emit.
///
/// This crate intentionally models controller/menu intent instead of exposing
/// arbitrary key or mouse injection. Backends map these buttons to a concrete
/// injection mechanism and must release all held buttons when dropped or when a
/// sequence fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SafeButton {
    /// Accept the highlighted row.
    Confirm,
    /// Back out one level.
    Cancel,
    /// Open or close the pause menu.
    Start,
    /// Move the cursor up one row.
    DpadUp,
    /// Move the cursor down one row.
    DpadDown,
    /// Move the cursor left one column.
    DpadLeft,
    /// Move the cursor right one column.
    DpadRight,
    /// Previous tab.
    LeftBumper,
    /// Next tab.
    RightBumper,
}

impl SafeButton {
    /// The name this button goes by in a log line -- stable, lowercase, underscore-separated.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Cancel => "cancel",
            Self::Start => "start",
            Self::DpadUp => "dpad_up",
            Self::DpadDown => "dpad_down",
            Self::DpadLeft => "dpad_left",
            Self::DpadRight => "dpad_right",
            Self::LeftBumper => "left_bumper",
            Self::RightBumper => "right_bumper",
        }
    }
}

/// A bounded input action. Durations are expressed in game frames so callers do
/// not depend on host sleeps, pointer focus, or wall-clock mouse polling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafeInputAction {
    /// Press and release, `frames` apart.
    Tap {
        /// Which button.
        button: SafeButton,
        /// How long it stays down, in game frames.
        frames: u16,
    },
    /// Press and keep held for `frames`, without the release.
    Hold {
        /// Which button.
        button: SafeButton,
        /// How long it stays down, in game frames.
        frames: u16,
    },
    /// Let one held button up.
    Release {
        /// Which button.
        button: SafeButton,
    },
    /// Let every held button up -- what a failed sequence must end with.
    ReleaseAll,
}

impl SafeInputAction {
    /// A [`Self::Tap`], with `frames` checked against the config's bounds.
    ///
    /// # Errors
    ///
    /// [`SafeInputError`] when `frames` is outside what the config allows.
    pub fn tap(
        button: SafeButton,
        frames: u16,
        config: SafeInputConfig,
    ) -> Result<Self, SafeInputError> {
        config.validate_frames(frames)?;
        Ok(Self::Tap { button, frames })
    }

    /// A [`Self::Hold`], with `frames` checked against the config's bounds.
    ///
    /// # Errors
    ///
    /// [`SafeInputError`] when `frames` is outside what the config allows.
    pub fn hold(
        button: SafeButton,
        frames: u16,
        config: SafeInputConfig,
    ) -> Result<Self, SafeInputError> {
        config.validate_frames(frames)?;
        Ok(Self::Hold { button, frames })
    }
}

/// The bounds every action is checked against before a backend ever sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SafeInputConfig {
    /// The longest a button may be held, in game frames. A held button that outlives the run is
    /// what leaves the player unable to move, so the ceiling is a safety limit rather than taste.
    pub max_hold_frames: u16,
}

impl Default for SafeInputConfig {
    fn default() -> Self {
        Self {
            max_hold_frames: DEFAULT_MAX_HOLD_FRAMES,
        }
    }
}

impl SafeInputConfig {
    /// Check a frame count against both ends of the allowed range.
    ///
    /// # Errors
    ///
    /// `ZeroFrameAction` below [`MIN_ACTION_FRAMES`], `FramesExceedLimit` above
    /// [`Self::max_hold_frames`].
    pub fn validate_frames(self, frames: u16) -> Result<(), SafeInputError> {
        if frames < MIN_ACTION_FRAMES {
            return Err(SafeInputError::ZeroFrameAction);
        }
        if frames > self.max_hold_frames {
            return Err(SafeInputError::FramesExceedLimit {
                frames,
                max: self.max_hold_frames,
            });
        }
        Ok(())
    }
}

/// Why an action was refused, or how the backend failed to deliver it.
#[derive(Debug, Eq, PartialEq)]
pub enum SafeInputError {
    /// An action that would last no frames at all, which no backend can express.
    ZeroFrameAction,
    /// A hold longer than the config allows.
    FramesExceedLimit {
        /// What was asked for.
        frames: u16,
        /// The ceiling it exceeded.
        max: u16,
    },
    /// The backend refused or failed, in its own words.
    Backend(String),
}

impl fmt::Display for SafeInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroFrameAction => write!(formatter, "input action must last at least one frame"),
            Self::FramesExceedLimit { frames, max } => write!(
                formatter,
                "input action frame count {frames} exceeds safety limit {max}"
            ),
            Self::Backend(message) => write!(formatter, "input backend failed: {message}"),
        }
    }
}

impl std::error::Error for SafeInputError {}

/// Whatever actually delivers an action to the game.
///
/// An implementor must release every held button when it is dropped or when a sequence fails --
/// this crate bounds what can be asked for, and the backend is what makes the bound true.
pub trait SafeInputBackend {
    /// Deliver one already-validated action.
    ///
    /// # Errors
    ///
    /// `SafeInputError::Backend` carrying whatever the injection mechanism said.
    fn apply(&mut self, action: SafeInputAction) -> Result<(), SafeInputError>;
}

/// Safe facade around an input backend. It validates bounded actions and offers
/// no mouse movement or arbitrary key injection API.
pub struct SafeInputController<B> {
    backend: B,
    config: SafeInputConfig,
}

impl<B> SafeInputController<B>
where
    B: SafeInputBackend,
{
    /// Wrap a backend, with the bounds every action through it will be checked against.
    pub fn new(backend: B, config: SafeInputConfig) -> Self {
        Self { backend, config }
    }

    /// Press and release one button.
    ///
    /// # Errors
    ///
    /// The validation error when `frames` is out of bounds, or the backend's own.
    pub fn tap(&mut self, button: SafeButton, frames: u16) -> Result<(), SafeInputError> {
        self.backend
            .apply(SafeInputAction::tap(button, frames, self.config)?)
    }

    /// Hold one button down for `frames`.
    ///
    /// # Errors
    ///
    /// The validation error when `frames` is out of bounds, or the backend's own.
    pub fn hold(&mut self, button: SafeButton, frames: u16) -> Result<(), SafeInputError> {
        self.backend
            .apply(SafeInputAction::hold(button, frames, self.config)?)
    }

    /// Let one held button up.
    ///
    /// # Errors
    ///
    /// The backend's own -- there is nothing to validate.
    pub fn release(&mut self, button: SafeButton) -> Result<(), SafeInputError> {
        self.backend.apply(SafeInputAction::Release { button })
    }

    /// Let every held button up. What a failed sequence must end with.
    ///
    /// # Errors
    ///
    /// The backend's own -- there is nothing to validate.
    pub fn release_all(&mut self) -> Result<(), SafeInputError> {
        self.backend.apply(SafeInputAction::ReleaseAll)
    }

    /// Give the backend back, so a caller can shut it down on its own terms.
    pub fn into_backend(self) -> B {
        self.backend
    }
}

/// Deterministic backend used by tests and trace-only integrations.
#[derive(Default, Debug)]
pub struct RecordingBackend {
    /// Every action it was handed, in order, so a test asserts on the sequence rather than on
    /// anything a game did with it.
    pub actions: Vec<SafeInputAction>,
}

impl SafeInputBackend for RecordingBackend {
    fn apply(&mut self, action: SafeInputAction) -> Result<(), SafeInputError> {
        self.actions.push(action);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MAX_HOLD_FRAMES: u16 = 3;
    const TEST_VALID_TAP_FRAMES: u16 = 2;
    const TEST_TOO_MANY_HOLD_FRAMES: u16 = 4;
    const TEST_ZERO_FRAMES: u16 = 0;

    #[test]
    fn controller_allows_only_bounded_actions() {
        let backend = RecordingBackend::default();
        let mut controller = SafeInputController::new(
            backend,
            SafeInputConfig {
                max_hold_frames: TEST_MAX_HOLD_FRAMES,
            },
        );

        controller
            .tap(SafeButton::Confirm, TEST_VALID_TAP_FRAMES)
            .unwrap();
        let error = controller
            .hold(SafeButton::DpadDown, TEST_TOO_MANY_HOLD_FRAMES)
            .unwrap_err();
        controller.release_all().unwrap();

        assert_eq!(
            error,
            SafeInputError::FramesExceedLimit {
                frames: TEST_TOO_MANY_HOLD_FRAMES,
                max: TEST_MAX_HOLD_FRAMES,
            }
        );
        assert_eq!(
            controller.into_backend().actions,
            vec![
                SafeInputAction::Tap {
                    button: SafeButton::Confirm,
                    frames: TEST_VALID_TAP_FRAMES,
                },
                SafeInputAction::ReleaseAll,
            ]
        );
    }

    #[test]
    fn zero_frame_tap_is_rejected() {
        let error = SafeInputAction::tap(
            SafeButton::Confirm,
            TEST_ZERO_FRAMES,
            SafeInputConfig::default(),
        )
        .unwrap_err();
        assert_eq!(error, SafeInputError::ZeroFrameAction);
    }
}
