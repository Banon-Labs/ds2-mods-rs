//! While another player is in our world, or we are in theirs, lower every weapon we carry that is
//! above the highest weapon level any of them has equipped.
//!
//! The host is at +0 and their phantom at +9: our +10 becomes +9, our +3 stays +3. The cap only
//! lowers. When the last other player is gone, every weapon goes back to its real level.
//!
//! # Where the level is lowered, and where it is not
//!
//! A weapon's level exists three times (docs/DS2-WEAPON-LEVEL-SYNC.md, Q1):
//!
//! | copy | written by this crate | saved |
//! |---|---|---|
//! | the inventory entry, `ItemEntry +0x25` | never, only read | yes |
//! | the equipment record table, `ChrAsmCtrl +0x20` | through the game's weapon update | no |
//! | the live weapon state, `ChrAsmEquip +0x70` | through the game's weapon update | no |
//!
//! The game copies inventory -> record table -> live state in one function,
//! [`ds2_rva::CHR_WEAPON_UPDATE`], and the same function sends P2P packet 61, which is how every
//! peer learns our level. This crate detours that function and lowers the level in the request it
//! is handed. Nothing else is written. So the character and every peer carry the capped level,
//! while the inventory entry, the thing the save serialises (`SaveDataItemInventory2` writes the
//! inventory manager's own block, `0x1402e53f0`), keeps the real one. There is no restore-before-save
//! step because there is nothing to restore in what is saved.
//!
//! # When the cap changes
//!
//! A detour on [`ds2_rva::NET_SESSION_UPDATE`] (game thread, every frame, measured at 60/s in the
//! world with `scripts/frida/tick-count.js`) reads the roster every few frames: every remote
//! `PlayerCtrl` that is a person (`NetworkPlayer_` name, not a bloodstain replay), and each one's
//! six weapon records. [`policy::cap`] turns those into the cap, and [`policy::Tracker`] decides
//! whether our weapons have to be pushed through the weapon update again. The push happens after
//! the original update has returned, from native code: the game's own weapon update, called with
//! a request built from our inventory entry exactly the way `0x1401b66a0` builds it.
//!
//! Inventory weapons are covered by the same detour: a weapon equipped mid-encounter goes through
//! the weapon update and comes out capped. The pause menu still reads the inventory entry and
//! shows real levels.
//!
//! # Every exit path
//!
//! * The encounter ends in the same world (a phantom leaves while we host): the cap goes to
//!   `None` and the push restores real levels, and sends them.
//! * We die, the host dies, we are sent home, we quit to the title, we disconnect: the world is
//!   torn down and rebuilt from the inventory. The tracker sees the character change and pushes
//!   real levels in case the rebuild met the clamp.
//! * The process dies: nothing capped was ever in the save.
//!
//! # What has not been proven
//!
//! Which copy the damage code reads, whether the server-side status upload reads the inventory or
//! the equipment copy, and when a joining player's records are first filled. See the doc.

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-weapon-sync:";

pub mod policy;

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};
