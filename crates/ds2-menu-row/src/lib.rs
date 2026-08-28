//! Append a fourth row to the pause menu's quit tab, to find out whether the layout tolerates one.
//!
//! # What this answers
//!
//! DS2's pause menu is six tabs. Each tab's contents are a `DLFixedVector` of `(action, gate)`
//! entries, built by one function per tab and copied into the live `FeGroupInGameGroupSelect`. The
//! tab that carries the quit item -- `FeGroupInGameReturnTitleCheck`, action `9`, the dialog that
//! offers to save on the way to the title screen -- holds three of a possible five.
//!
//! Two things said a fourth entry should become a fourth ROW rather than dead data, and both were
//! read out of the game's own code:
//!
//! * the per-tab init ([`ds2_rva::FE_INGAME_MENU_TAB_INIT`]) sets a cell count FROM the item
//!   vector's count -- which turned out to be the count the CURSOR is bounded by, not the number of
//!   cells there are to draw;
//! * the copy into the live group (`0x1400a3ef0`) copies `count` entries and accepts up to five.
//!
//! One thing said it might not, and it could not be settled by reading the executable: nothing in
//! the image maps an action id to a caption.
//!
//! # It has been run, and the answer was worse than a missing caption
//!
//! 2026-08-28, one run, one character, the quit tab opened by hand: the vector took the fourth
//! entry (`count=3->4`, and the integrity check below passed, so this really was that tab), the
//! cursor reaches a fourth item and it responds -- **and nothing at all is drawn for it.**
//!
//! Not a blank caption. No cell. `FrontendEx::FexGridControl`'s layout bind
//! ([`ds2_rva::FEX_GRID_CONTROL_LAYOUT_BIND`]) takes no extent from anywhere: it probes the layout
//! for the element at each `(col, row)` and stops a row at the first null, so the grid's extent is
//! a count of AUTHORED ELEMENTS. `FUN_140021b30` then writes only the logical item count. Two
//! numbers, two sources, and appending moved just one of them -- which is exactly "selectable but
//! invisible".
//!
//! So the crate did its job and the result is negative in a useful way: **a new pause-menu row
//! needs layout data inside `GameDataEbl.bdt`, and the probe order means cell 3 has to exist before
//! cell 4 can ever be found.** The caption question was never reached. `docs/DS2-INGAME-MENU.md`
//! has the measurement and the disassembly.
//!
//! It is kept, and kept off, because it is how the next person re-establishes any of this in one
//! run after touching the archive.
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
