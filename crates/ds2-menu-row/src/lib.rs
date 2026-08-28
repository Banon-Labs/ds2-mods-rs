//! A fourth row on the pause menu's quit tab, that quits to desktop without asking.
//!
//! # The four hooks, and what each one is for
//!
//! | site | RVA | what it does |
//! |---|---|---|
//! | `FeGroupInGameTopSelect`'s quit-tab item builder | `0x000a5900` | appends the item |
//! | the tab's item dispatch | `0x000a6090` | turns the item into a shutdown |
//! | the quit tab's cell namer | `0x000a5b50` | names the fourth cell |
//! | `FeLayoutDocument::findDefinition` | `0x00b54740` | supplies the cell |
//!
//! The first two are the ACTION and were finished first. The last two are the ROW, and they only
//! work as a pair: the namer asks the scene for an element by id, and the layout is what has one.
//!
//! # The action
//!
//! `FeSubStateTitleShutdown::v1` is three instructions -- load the title singleton, write `1` to
//! `+0x13a`, return -- and `GameManagerImp`'s per-frame master update polls that byte. So the
//! dispatch detour writes it directly and does not call the original. The shutdown that follows is
//! the game's own, on the game's own schedule, with this crate nowhere on the stack.
//!
//! **It does not save and it does not ask.** The quit-to-title row offers to save because that
//! flow asks; this one is "without a confirmation" and the absence of a save is the same coin.
//!
//! The action id is `0x1000`, deliberately outside the dispatch's own space (`0..=9`, `0xb`,
//! `0xc`, `0xd`). If the dispatch detour ever fails to install, the row plays the confirm sound and
//! does nothing -- an inert row is the right failure mode, where a row that quietly opened Key
//! Bindings would not be.
//!
//! # The row, and the two measurements that cost the most
//!
//! Appending the item was easy and produced a row that could be selected and could not be seen.
//! Two beliefs had to die before that made sense:
//!
//! * **"the row count is code-driven."** `FrontendEx::FexGridControl`'s layout bind probes the
//!   layout for the element at each `(col, row)` and stops at the first null, so the drawn count is
//!   a census of AUTHORED ELEMENTS. `FUN_140021b30` writes only the logical item count. Two
//!   numbers, two sources, and appending moved one of them.
//! * **"the layout authors five cells."** It does not. `FeSceneInGameMenu`'s cache asks for five
//!   ids and stores whatever comes back, including the zeros; four controlled runs, and later the
//!   file itself, agree that two of those five were never authored.
//!
//! What actually works is in [`crate::layout`]: the `.flo` is loaded in place, one function hands
//! out the table entry that says how many children a container has, and that same `u16` is the
//! display list's capacity. Substituting a copy that says two more -- a row and its mark -- is the
//! whole edit. Nothing is written to disk and no archive is repacked.
//!
//! # Why it refuses more often than it writes
//!
//! Every one of the four hooks checks what the game left behind before it touches anything: the
//! item builder demands `(7,0) (8,0) (9,4)`, the namer demands the quit tab's own base path, and
//! the definition lookup demands seven children carrying seven known ids. An RVA is a number, and
//! on a build these tables were not read from, each of those sites points at something else that
//! would accept the write and produce a screenshot that looks exactly like a result. A run that
//! refuses says so in the log, with what it actually saw.
//!
//! `docs/DS2-INGAME-MENU.md` has the disassembly, the measurements, and the corrections.

#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
mod layout;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};

/// Prefix on every line this crate writes to the loader log, so a reader can tell which component
/// spoke and a filter can select it alone.
pub const LOG_PREFIX: &str = "ds2-menu-row:";
