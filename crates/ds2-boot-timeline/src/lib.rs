//! Time every step of the title boot flow, so the cost of each one is a number rather than a guess.
//!
//! `docs/DS2-BOOT-WORK.md` names the thirteen substates between process start and the top menu and
//! proves their ordering statically. It contains **no durations**, because static analysis cannot
//! produce one. This crate is the missing half: one run, one timestamped line per substate entered
//! and left, and a closing total when the top menu appears.
//!
//! Three separate things need that log and none of them can start without it: whether the storage
//! and network chains are worth overlapping (`ds2-mods-rs-7on`), whether `0x38 SaveSystemData`'s
//! redundant-looking write costs anything worth cutting (`ds2-mods-rs-cz6`), and the weight table a
//! loading bar has to be driven from (`ds2-mods-rs-aim`).
//!
//! # Why it hooks the flow and not each substate
//!
//! Hooking each class's `enter` is the obvious design and it is the one that has already failed
//! once in this repo. `ds2-dialog-skip` first hooked `FeSubStateProcessWindowBase::v1` and hid
//! exactly one wait window, because `FeSubStateTitleInformation` shows its own from `update`
//! rather than `enter` -- a per-class hook cannot see a step whose class it did not think to
//! name, and it reports success either way. An instrument that silently omits steps is worse than
//! no instrument, because its output looks complete.
//!
//! So both hooks here are on machinery every substate goes through, and neither names a substate:
//!
//! * **`FeStateFlow::update`** (RVA `0x00104540`) drives the resident substate. The detour samples
//!   the resident pointer at `+0x10` before and after the original and reports a change. Every
//!   transition passes through this function, so every arrival is seen.
//! * **`FeSubStateBase::v6`** (RVA `0x001043a0`), the "drop my transitions" slot the flow calls
//!   immediately before every `leave`. Checked against all 36 substate vtables: **not one
//!   overrides it**, so this single address is every departure in the game.
//!
//! # The two hooks are each other's check
//!
//! They are deliberately redundant. The sampler sees arrivals, the `v6` hook sees departures, and
//! in a complete log the two interleave exactly. A departure with no matching arrival means the
//! sampler missed a transition -- which is possible in principle, because `FeStateFlow::update`
//! writes the resident pointer in two places and a call that used both would look like one change
//! from outside. Nothing in the static trace says that happens on the boot path, but "nothing says
//! it happens" is not a measurement, so the log is built to show it if it does rather than to
//! assume it does not. `mismatch=` on a leave line is that signal.
//!
//! # What the ids mean
//!
//! Every substate carries its id as a `u32` at `+0x0c`, and `FeStateFlow`'s own transition search
//! matches on that field -- so the ids in this log are the game's, not this crate's invention. The
//! chain to look for on a cold boot is `0x00 0x01 0x13 0x14 0x15 0x17 0x05 0x20 0x37 0x38 0x39
//! 0x44 0x47`; see `docs/DS2-BOOT-WORK.md` for what each one does.
//!
//! # Off by default
//!
//! Every other feature in this repo defaults on, because removing boot screens is what the mod is
//! for. This one is a measuring instrument: it patches two more sites and writes lines nobody
//! playing the game wants. `[boot_timeline] enabled = true`, or `ds2-run.py --boot-timeline`,
//! turns it on for a measurement run.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, mark_origin, set_logger};

/// Prefix on every line this crate writes to the loader log, so a reader can tell which component
/// spoke and a filter can select these lines alone.
pub const LOG_PREFIX: &str = "ds2-boot-timeline:";
