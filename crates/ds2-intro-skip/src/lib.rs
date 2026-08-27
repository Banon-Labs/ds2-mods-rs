//! Skip DARK SOULS II's boot screens by telling each one it has already finished.
//!
//! The publisher logo, the unauthorised-copying warning and the user-policy screen are not video
//! files -- `Game/movie/` holds one file and it is the attract-loop cinematic. They are three
//! `FeSubState*` objects driven by `FeStateTitle`, each with an `enter` (vtable slot 1), an
//! `update` (slot 3) and a small integer phase counter it advances through. `docs/DS2-TITLE-FLOW.md`
//! has the full trace.
//!
//! # Why this is not a patch so much as a shortcut the game already knows
//!
//! Every one of the three has a shipped path where `enter` writes the terminal phase and returns
//! having done nothing else:
//!
//! * `FeSubStateTitleLogo` does it when its scene reference at `+0x18` is null -- "there is no
//!   logo to show".
//! * `FeSubStateWarningNoCopy` does it when a virtual on the `0x1416751f8` singleton returns
//!   nonzero.
//! * `FeSubStateTitleUserPolicy` does it when the persisted `[sys+0x136d]` flag says the policy
//!   was already accepted.
//!
//! So this hooks `enter`, lets the original run, and then writes that same terminal phase. The
//! transition is one the game performs on itself; nothing here invents a state.
//!
//! # Why the original still runs
//!
//! Skipping it entirely would be a cleaner "no logo at all", and it is tempting because two of the
//! three shipped paths return before touching their scene. It is not done, because `leave`
//! (slot 2) closes the scene reference the original `enter` opened, and a close without a matching
//! open is an unbalanced lifetime on an object this crate does not own. Letting `enter` run keeps
//! open and close symmetric and costs at most a frame of the logo before `update` sees the
//! terminal phase. If that frame ever turns out to be visible, the fix is to reproduce the skip
//! path's fast-out animation, not to drop `enter`.
//!
//! # The phase offset is per class and is not a shared base field
//!
//! `FeSubStateTitleLogo` keeps its phase at `+0x20`; the other two keep theirs at `+0x10`. Each
//! offset was read from the field that class's own `update` switches on. Writing one offset for
//! all three would put a `4` into an unrelated member of two of them, which is exactly the kind of
//! plausible-looking wrong that this repo keeps finding in ported code.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};

/// Prefix on every line this crate writes to the loader log. Distinct from `ds2-loader:` so a
/// reader can tell which component spoke, and so a log filter can select one without the other.
pub const LOG_PREFIX: &str = "ds2-intro-skip:";
