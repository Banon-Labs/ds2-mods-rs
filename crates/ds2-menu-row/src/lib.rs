//! Extra rows on the pause menu's tabs, registered by whoever wants one.
//!
//! # The API
//!
//! A caller fills in a [`RowSpec`] and calls [`add_row`] before [`install`] runs. The four hooks
//! below then loop over whatever is registered instead of over one hardcoded row:
//!
//! ```no_run
//! ds2_menu_row::add_row(ds2_menu_row::RowSpec {
//!     tab: ds2_menu_row::Tab::Quit,
//!     caption: "Quit Game",
//!     icon: ds2_rva::FLO_QUIT_ICON_DEFINITION,
//!     tint: Some(ds2_menu_row::Tint { rgb: [0xff, 0x64, 0x50], strength: 120 }),
//!     on_confirm: ds2_menu_row::quit_to_desktop,
//! })?;
//! # Ok::<(), ds2_menu_row::AddRowError>(())
//! ```
//!
//! **The quit-to-desktop row goes through that same call**, made by `ds2-loader`. Nothing in this
//! crate is privileged, which is the only way to know the API is usable.
//!
//! **The ceiling is the game's and it is small.** A tab's item vector holds five entries and
//! panics above it; the quit tab ships three, so [`MAX_ADDED_ROWS`] is two. [`add_row`] refuses
//! the third with the numbers in the error rather than letting the game's allocator find out. See
//! [`crate::api`] for the other two bounds and which one binds.
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
//! Action ids start at `0x1000` and step by slot, deliberately outside the dispatch's own space
//! (`0..=9`, `0xb`, `0xc`, `0xd`). If the dispatch detour ever fails to install, a registered row
//! plays the confirm sound and does nothing -- an inert row is the right failure mode, where a row
//! that quietly opened Key Bindings would not be.
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
//! # The icon
//!
//! A row's definition IS its icon -- an icon and a transparent flash overlay, with the caption
//! living in the separate mark beside it -- so a cloned row wears the icon of whatever row it was
//! cloned from, which was Game Options. The record instead names the Quit Game glyph on its own
//! ([`ds2_rva::FLO_QUIT_ICON_DEFINITION`]), tinted red by the colour in its own transform, because
//! the row above it is the other Quit Game.
//!
//! Not row 2's definition, which pairs that glyph with a grey twin under id `0x1eacd0`. The
//! availability pass only resolves that id for a row with a nonzero GATE, and this row's gate is
//! `0` -- so a cloned twin would draw `ff808080` for the life of the process.
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

mod api;

#[cfg(windows)]
mod install;

#[cfg(windows)]
mod layout;

#[cfg(windows)]
mod caption;

#[cfg(windows)]
mod tree;

#[cfg(windows)]
mod banner;

pub use api::{AddRowError, MAX_ADDED_ROWS, RowId, RowSpec, Tab, Tint, add_row};

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, quit_to_desktop, set_logger};

/// Prefix on every line this crate writes to the loader log, so a reader can tell which component
/// spoke and a filter can select it alone.
pub const LOG_PREFIX: &str = "ds2-menu-row:";
