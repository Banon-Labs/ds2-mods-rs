//! Stop DARK SOULS II's title-flow notice boxes from ever being created.
//!
//! `ds2-intro-skip` removes the three boot screens; the flow then stops on message boxes that wait
//! for input, so the title menu still costs the player several presses. This crate removes the
//! boxes. `docs/DS2-TITLE-FLOW.md` carries the full trace.
//!
//! # One `enter`, and the box is simply never created
//!
//! Six classes draw these boxes and **all six funnel through one `enter`**,
//! `FeSubStateCommonWindowBase::v1` at [`ds2_rva::FE_DIALOG_ENTER`] -- including the two that
//! override `v1`, which format their message and then `call 0x140104db0` to do the showing. That
//! function is the only place a title message box comes into existence, so it is the only place one
//! can be prevented instead of dismissed.
//!
//! The detour returns without calling the original, and writes the state the game itself leaves
//! such a box in once closed: result `1`, phase `3`.
//!
//! # Why skipping `enter` does not leak a window
//!
//! `leave` closes the window **only when the phase is 1**:
//!
//! ```text
//! if (this->phase == 1) { this->vtable[10](this); close_window(ui); }
//! this->phase = 0;
//! ```
//!
//! So an `enter` that never ran and never opened anything pairs with a `leave` that never closes
//! anything. **This is the opposite call from the one `ds2-intro-skip` makes**, and deliberately so:
//! there `leave` closes unconditionally, which is why that crate lets the original `enter` run and
//! only rewrites the phase afterwards. The conditional here is what makes suppression sound, and it
//! was read out of `leave` rather than assumed from the sibling crate's shape.
//!
//! # What the first two versions did, and why this one is different
//!
//! Version one hooked the shared `update` instead and wrote the result byte at `+0x31` -- the byte
//! a button press writes, and the only thing the press-handling path writes, so the dispatch that
//! closes the box could not tell the difference. That worked: every box answered itself. But the
//! box still had to be **drawn** before it could answer itself, so the player watched dialogs flash
//! past instead of pressing buttons. Auto-advancing is not the same as not appearing.
//!
//! Version one also got the allowlist wrong, and that is worth keeping written down. It named three
//! classes chosen from their names -- the two `ServerFailWarn` boxes and the offline notice -- and
//! the run that followed logged `seen screen=<not-allowlisted> vtable=0x1410bcff8` while all three
//! named ones never fired. The dialog that actually appears is `FeSubStateCommonWindow`, left out
//! because "common" sounded like a box used all over the game. It is not: its vtable is referenced
//! at exactly one site in the whole image, inside `FeStateTitle`'s substate-table builder.
//!
//! # This mod suppresses notices. It never answers a question.
//!
//! `+0x12` is a **signed** option count. Negative is the game's own marker for a one-button
//! acknowledgement box: its input path can only ever produce a cancel, and the closed phase it
//! computes can only ever be 3, so removing the box removes a keypress and nothing else.
//! Non-negative means a real decision with a real affirmative -- and those are shown and left alone.
//!
//! That is the condition that carries the safety argument, because it is a property of the object
//! rather than a belief about a class name. Two further locks sit around it: an allowlist of
//! vtables, and a runtime check that slots 8 and 9 are still `FeSubStateCommonWindowBase`'s `ret 0`
//! stubs. `FeSubStateTitleDeleteProfile` shares this same `enter` and overrides slot 8, so it fails
//! that check on its own merits whatever the allowlist says.
//!
//! # A dialog it does not recognise is shown, and reported
//!
//! Which boxes appear depends on the machine -- whether the network check failed, whether the
//! information fetch failed. Anything this crate sees and does not suppress is logged once with its
//! vtable, kind, caption and option count, and then shown normally. The failure mode of an
//! incomplete allowlist is a log line plus a button press, which is exactly how the allowlist came
//! to be corrected in the first place.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;
#[cfg(windows)]
mod title;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};
#[cfg(windows)]
pub use title::{Outcome as TitleOutcome, Request as TitleRequest, install as install_title};

/// Prefix on every line this crate writes to the loader log. Distinct from `ds2-loader:` and from
/// `ds2-intro-skip:` so a reader can tell which component spoke, and so a run that boots badly can
/// be attributed to one feature rather than to "the mod".
pub const LOG_PREFIX: &str = "ds2-dialog-skip:";
