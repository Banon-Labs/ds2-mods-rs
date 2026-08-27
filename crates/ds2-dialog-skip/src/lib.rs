//! Answer DARK SOULS II's title-flow message boxes the way a button press answers them.
//!
//! `ds2-intro-skip` removes the three boot screens; the flow then stops on message boxes that
//! wait for input, so the title menu still costs the player several presses. This crate presses
//! them. `docs/DS2-TITLE-FLOW.md` carries the full trace.
//!
//! # Writing one byte IS the button press
//!
//! Six classes draw these boxes and **all six share one `update`**,
//! `FeSubStateCommonWindowBase::v3` at [`ds2_rva::FE_DIALOG_UPDATE`]. Reading it settles the whole
//! design, because on the frame a button is pressed its only effect is a store into the result
//! byte at `+0x31`:
//!
//! ```text
//! if (input_pressed(ui)) {
//!     if ((int16)this->options < 0)      this->result = 1;   // one-button box
//!     else if (highlighted is confirm)   this->result = 2;
//!     else                               this->result = 1;
//! } else if (this->timeout > 0 && this->elapsed >= this->timeout) {
//!     this->result = 1;                                      // shipped auto-close
//! }
//! switch (this->result) {                                    // dispatch, reads nothing else
//!     case 0: return;                                        // still waiting
//!     case 1: this->vtable[8](this); break;
//!     case 2: this->vtable[9](this); break;
//! }
//! close_window(ui); this->phase = 2;
//! ```
//!
//! The dispatch consults the result byte and **nothing else about the press**. So this crate
//! stores that byte and lets the game's own dispatch run: the close, the animation, the phase
//! transition and the handler call are all the game's, unmodified. Nothing here reimplements a
//! transition, and nothing here closes a window it did not open.
//!
//! # The answer is computed, not chosen
//!
//! `+0x12` is a **signed** option count. Negative means a one-button acknowledgement box, where
//! the game itself will only ever produce `1`; non-negative means a real choice, where a press on
//! the affirmative produces `2`. So the value to write is a function of the object:
//!
//! ```text
//! result = if (int16)this->options < 0 { CANCEL } else { CONFIRM }
//! ```
//!
//! Picking a constant instead would be right for one box and silently wrong for the other kind.
//!
//! # Two locks, because one of these dialogs deletes a save profile
//!
//! `FeSubStateTitleDeleteProfile` shares the same `update`. A mod that answered "every common
//! window" would answer that one too, and it is reached exactly when a player has deliberately
//! asked to delete something. Two independent conditions must both hold before this crate writes
//! anything:
//!
//! 1. **An allowlist of vtables**, so only the three known boot dialogs are candidates.
//! 2. **A runtime inertness check** on the object's own vtable: slots 8 and 9 must still be
//!    `FeSubStateCommonWindowBase`'s `ret 0` stubs. If either has a body, answering the box would
//!    *do* something, and this crate declines.
//!
//! The second is the one that matters, because it is a property of the bytes in front of it
//! rather than a belief about a name. `DeleteProfile` overrides slot 8, so it fails that check on
//! its own merits even if it somehow reached the allowlist. Belt and braces on purpose: the first
//! lock encodes intent, the second enforces it.
//!
//! # A dialog it does not recognise is reported, never answered
//!
//! Which boxes actually appear at boot depends on the machine -- whether the network check failed,
//! whether the information fetch failed. Any common window this crate sees and does not act on is
//! logged once, by vtable address, with the reason. That way the set of boot dialogs is *measured*
//! across runs instead of assumed, and the failure mode of an incomplete allowlist is a log line
//! plus a button press, not a silently auto-answered dialog.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};

/// Prefix on every line this crate writes to the loader log. Distinct from `ds2-loader:` and from
/// `ds2-intro-skip:` so a reader can tell which component spoke, and so a run that boots badly can
/// be attributed to one feature rather than to "the mod".
pub const LOG_PREFIX: &str = "ds2-dialog-skip:";
