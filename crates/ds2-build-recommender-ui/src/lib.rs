//! The `PvP` build recommender's in-game panel, opened from the pause menu's Build Recommender row.
//!
//! Nine typed stats and the soul level they make, a searchable weapon and its infusion, four modes
//! (weapons for these stats, optimize for a weapon, the minimum for a weapon, what similar builds
//! carry), a results table, and Generate Build -- whose build can then be put on the live character
//! with Apply, behind a confirm that says soul memory is raised for good.
//!
//! Every decision is `ds2-build-recommender-core`'s and is tested on the host; the answers come from
//! its `CorpusBackend`, over `ds2-build-recommender.dat` beside the game, or from its `StubBackend`
//! when that file is missing or unreadable, which the log and the panel both say. This crate draws,
//! reads the keyboard and pad, and routes presses, the way `ds2-save-file`'s picker does:
//!
//! | who | when | what it does here |
//! |---|---|---|
//! | the row's press | the pause menu's confirm | [`open`]: fresh state, take the input hold |
//! | `on_frame` | `ds2-overlay`'s clock, every `Present` | keys and pad into the focused field, release the hold after closing |
//! | `draw` | `ds2-overlay`'s panel table, the same `Present` | draw, and turn mouse clicks into actions |
//!
//! Apply does not touch the game from here. It hands the build to `ds2_build_import::queue_generated`,
//! and the pause menu's own tick applies it on the next frame through the same path a soulsplanner
//! link takes.
//!
//! # The mouse is `ds2-overlay`'s, not imgui's
//!
//! Every control is drawn on the foreground draw list and hit-tested against
//! `ds2_overlay::panels::mouse`, which undoes a stretched fullscreen window; imgui's own
//! `mouse_pos` is short by screen over window there, which is why the picker does the same.

// DEBT: ds2-mods-rs-24r -- the crate ships as part of a Windows DLL; on the host only `LOG_PREFIX`
// and `LogFn` compile, and this keeps that half quiet.
#![cfg_attr(not(windows), allow(unused))]

/// What every line this crate writes begins with.
pub const LOG_PREFIX: &str = "ds2-build-recommender:";

/// The loader's log sink. Same shape as every other crate's.
pub type LogFn = fn(std::fmt::Arguments<'_>);

#[cfg(windows)]
mod input;
#[cfg(windows)]
mod panel;

#[cfg(windows)]
pub use panel::{install, open};
