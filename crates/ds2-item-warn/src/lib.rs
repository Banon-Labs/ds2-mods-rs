//! A red mark on the icon of any weapon the player's stats cannot meet.
//!
//! Off unless `<Game>/ds2-mods.toml` says `[item_warn] enabled = true`.
//!
//! **Nothing in this crate has been in front of a running game.** Every address, offset and
//! coordinate below was read statically out of `darksoulsii-deobf.bin` and the shipped
//! `/menu/02.febnd.dcx`; the hooks refuse rather than write when what they find is not what was
//! recorded, and a refusal is the expected outcome on any build these numbers were not read from.
//! `docs/DS2-ITEM-REQUIREMENTS.md` has the disassembly.
//!
//! # What it draws, and the honest description of it
//!
//! A red badge in the bottom-left corner of the item icon. **It is not an X**, and the reason is
//! recorded rather than glossed: this mod ships no texture, the inventory layout's 88 shapes hold
//! nothing that can be shown to be an X or a plain fill without rendering the atlas, and picking
//! an unclaimed atlas rect blind is picking a picture nobody has seen. So the badge is one of the
//! nine infusion glyphs the game already draws in that icon's corner, cloned and re-skinned opaque
//! red. See [`ds2_rva::FE_ITEM_WARN_CLONED_CHILD`] for what was checked, including the transform
//! skew terms that would make a genuine crossed X expressible if a bar existed to rotate.
//!
//! # The two hooks
//!
//! | site | RVA | what it does |
//! |---|---|---|
//! | the `.flo` container builder | `0x00b50f20` | gives the infusion container a tenth child, which is the badge |
//! | the item-cell bind | `0x000bc850` | after the original, shows that child when the requirements are unmet |
//!
//! They go in together or not at all. A badge element with nothing to switch it on is invisible
//! and harmless; a switch with no element resolves to nothing and writes nothing -- but a run
//! where only one landed would look exactly like a run where the requirement check was wrong, so
//! [`install`] refuses the pair.
//!
//! # Where the badge element comes from
//!
//! The item cell already contains an INFUSION CONTAINER -- accessor
//! [`ds2_rva::FE_ITEM_CELL_INFUSION_ACCESSOR_OFFSET`], element `0x5f5c3e2` -- holding nine glyphs
//! with ids [`ds2_rva::FLO_INFUSION_CONTAINER_IDS`], of which the bind shows at most one. That
//! bind loops over SIXTEEN ids and nine are authored, so seven of its sixteen `setVisible` calls
//! address nothing. This crate authors one of the seven: [`ds2_rva::FE_ITEM_WARN_ELEMENT`], slot
//! `15`, which the game's own loop hides on every single bind because no item's infusion nibble is
//! ever `15`. The badge is therefore off by default in the most literal sense available -- the
//! game turns it off, and only this crate's second detour turns it back on.
//!
//! # Where the requirement check comes from
//!
//! Not from a parallel implementation. `FUN_1400bcde0` ([`ds2_rva::FE_STAT_ROW_COLOUR`]) is the
//! function that decides a stat row in the item detail pane is red, and it is four reads and a
//! `comiss`: take the column's player-stat index out of [`ds2_rva::FE_STAT_ROW_TABLE`], read that
//! stat out of the table the game hangs off `GameManagerImp`, and compare it against the column's
//! value. This crate performs the same three lookups through the same game functions for the four
//! weapon requirement columns. The stat indices are read from the game's table at runtime rather
//! than written down here, so the mapping stays the game's.
//!
//! **The game has a second, different check** -- `FUN_14034d3c0`, which feeds the damage penalty
//! and is continuous rather than boolean, reads `WeaponParam` directly, and halves the Strength
//! requirement for a two-handed grip. The badge follows the presentation check on purpose: an item
//! in a list is not being held, so it has no grip, and the pane is what a player opens to find out
//! why a number is red. The two disagree for a weapon in two hands.
//!
//! **Two things it does not know:**
//!
//! * whether the frontend's stat table holds base or modified stats -- every cross-reference to it
//!   is a reader and the writer was not found, so whether rings and spEffects are in these numbers
//!   is open;
//! * whether an item in a shop list (rather than the bag) reaches the same rows -- the descriptor
//!   covers both, but only the bag path has been traced.
//!
//! # Armour and rings are not marked
//!
//! The same table gives `0x11..0x14` for armour and `0x42`/`0x43` for rings, so the check would
//! extend. The gate this crate uses is the game's own infusion gate --
//! [`ds2_rva::ITEM_ENTRY_TYPE_MAX_INFUSABLE`], "item type 0 or 1" -- because the badge lives
//! inside the infusion container and only weapons and shields have one. Marking armour would need
//! a second element in a second container and a second fingerprint.

// DEBT: ds2-mods-rs-24r -- not debt to be paid: this crate ships as a Windows DLL and the
// attribute is what keeps its Rust half parseable on the host, so the game-free tests below it
// can run at all. The issue is the standing record of that decision.
#![cfg_attr(not(windows), allow(unused))]

#[cfg(windows)]
mod install;

#[cfg(windows)]
mod mark;

#[cfg(windows)]
mod place;

#[cfg(windows)]
mod requirement;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};

/// Prefix on every line this crate writes to the loader log, so a reader can tell which component
/// spoke and a filter can select it alone.
pub const LOG_PREFIX: &str = "ds2-item-warn:";
