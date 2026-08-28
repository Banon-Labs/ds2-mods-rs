//! Append a fourth row to the pause menu's quit tab, to find out whether the layout tolerates one.
//!
//! # What this answers
//!
//! DS2's pause menu is six tabs. Each tab's contents are a `DLFixedVector` of `(action, gate)`
//! entries, built by one function per tab and copied into the live `FeGroupInGameGroupSelect`. The
//! tab that carries the quit item -- `FeGroupInGameReturnTitleCheck`, action `9`, the dialog that
//! offers to save on the way to the title screen -- holds three of a possible five.
//!
//! Two things say a fourth entry should become a fourth ROW rather than dead data, and both were
//! read out of the game's own code:
//!
//! * the per-tab init ([`ds2_rva::FE_INGAME_MENU_TAB_INIT`]) sets the grid's visible-cell count
//!   FROM the item vector's count, so the row count is code-driven, not baked;
//! * the copy into the live group (`0x1400a3ef0`) copies `count` entries and accepts up to five.
//!
//! One thing says it might not, and it could not be settled by reading the executable: **nothing
//! in the image maps an action id to a caption.** The tab captions the exe does set resolve layout
//! elements by hard-coded name hash, which points at the row labels being authored in the
//! frontend layout inside `GameDataEbl.bdt` -- an archive this repo does not open. So a fourth row
//! may come up captionless even if it comes up at all.
//!
//! That is the whole question, and it is one screenshot wide. This crate is the instrument that
//! takes it.
//!
//! # Why the payload is action `0xd` and not something new
//!
//! `0xd` already has a case in the dispatch switch, and its factory branch shares a `case` label
//! with kind 4 -- `case 4: case 6:` allocate the same `0xc68` bytes and call the same constructor,
//! `FeGroupInGameSystemSettingKeyboard`. The job carries its kind only to select that branch. So
//! confirming the new row runs code the shipped Key Bindings row runs every time it is pressed.
//!
//! **This crate adds no code path to the game.** It adds one entry to one vector. Anything it can
//! be blamed for is either the extra row or nothing, which is exactly what makes the result
//! readable.
//!
//! # Why it refuses more often than it writes
//!
//! The detour checks what the original left behind -- three entries, `(7,0) (8,0) (9,4)` -- and
//! declines to touch the vector if that is not what it finds. An RVA is a number: on a build whose
//! tabs are ordered differently this would append to some other tab, and the resulting screenshot
//! would be evidence about nothing while looking exactly like evidence. A run that refuses says so
//! in the log, with the entries it actually saw.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};

/// Prefix on every line this crate writes to the loader log, so a reader can tell which component
/// spoke and a filter can select it alone.
pub const LOG_PREFIX: &str = "ds2-menu-row:";
