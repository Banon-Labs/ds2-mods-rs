//! While another player is in our world, or we are in theirs, lower every weapon we carry that is
//! above the highest weapon level any of them has equipped.
//!
//! The host is at +0 and their phantom at +9: our +10 becomes +9, our +3 stays +3. The cap only
//! lowers. When the last other player is gone, every weapon goes back to its real level.
//!
//! # Where the level is lowered, and where it is not
//!
//! A weapon's level exists four times (docs/DS2-WEAPON-LEVEL-SYNC.md, Q1):
//!
//! | copy | written by this crate | saved |
//! |---|---|---|
//! | the inventory entry, `ItemEntry +0x25` | lowered while capped, every weapon | no |
//! | the save block record, `[ItemInventory2 + 0x10] + 0x30 + index * 0x10`, `+0x0C` | only ever given the real level | yes |
//! | the equipment record table, `ChrAsmCtrl +0x20` | through the game's weapon update | no |
//! | the live weapon state, `ChrAsmEquip +0x70` | through the game's weapon update | no |
//!
//! While a cap is on, every weapon and shield in the inventory (pack and box: one entry array)
//! above it is lowered in its entry, so the pause menu shows the capped level and anything
//! equipped mid-encounter is capped before it is equipped. A [`policy::Ledger`] keeps each one's
//! real level. The save does not read the entries: [`ds2_rva::SAVE_DATA_ITEM_INVENTORY_WRITE`]
//! streams the separate save block, and its detour gives every record the ledger lowered its real
//! level just before it is streamed, so a save made while capped keeps the real levels.
//!
//! The game copies inventory -> record table -> live state in one function,
//! [`ds2_rva::CHR_WEAPON_UPDATE`], and the same function sends P2P packet 61, which is how every
//! peer learns our level. This crate detours that function and lowers the level in the request it
//! is handed, which with the inventory already lowered is a backstop. Any change to the equipped
//! slots while capped, whatever caused it, is a resweep of all six ([`policy::Watch`]).
//!
//! # When the cap changes
//!
//! A tick after [`ds2_rva::NET_SESSION_UPDATE`] (game thread, every frame, measured at 60/s in the
//! world with `scripts/frida/tick-count.js`) reads the roster every few frames: every remote
//! `PlayerCtrl` that is a person (`NetworkPlayer_` name, not a bloodstain replay), and each one's
//! six weapon records. [`policy::cap`] turns those into the cap, and [`policy::Tracker`] decides
//! whether our weapons have to be pushed through the weapon update again. The push happens after
//! the original update has returned, from native code: the game's own weapon update, called with
//! a request built from our inventory entry exactly the way `0x1401b66a0` builds it.
//!
//! The same check sweeps the whole inventory against the cap first ([`policy::Ledger::sweep`]):
//! a weapon picked up mid-encounter is lowered within a quarter second, and when the cap goes
//! every lowered entry, and its save record, gets its real level back.
//!
//! # The key
//!
//! [`DEFAULT_KEY`] (or `[weapon_sync] key`) turns the feature off and on in game. Off restores real
//! levels in the same frame; on caps in the same frame if another player is in the world. The net
//! session update is shared with `ds2-voice-chat` through `ds2-net-tick`, which owns its one
//! detour, so both features run together.
//!
//! Every press says which way it went, out loud ([`clip`]), and while the feature is on a pair of
//! crossed swords sits in the top-right corner of the screen ([`glyph`]); off, they are gone. The
//! swords are one of `ds2-overlay`'s imgui panels, drawn through its one `Present` detour on the
//! game's own swap chain.
//!
//! # Every exit path
//!
//! * The encounter ends in the same world (a phantom leaves while we host): the cap goes to
//!   `None` and the push restores real levels, and sends them.
//! * We die, the host dies, we are sent home, we quit to the title, we disconnect: the world is
//!   torn down and rebuilt from the inventory. The tracker sees the character change and pushes
//!   real levels in case the rebuild met the clamp.
//!   Leaving the world is a check with no character, whose cap is `None`, so the whole inventory
//!   is restored at the first check of the load screen.
//! * Any save while capped: the save writer's detour puts the real levels in the save block.
//! * The process dies: nothing capped was ever in the save.
//!
//! # What has not been proven
//!
//! Which copy the damage code reads, whether the server-side status upload reads the inventory or
//! the equipment copy, and when a joining player's records are first filled. See the doc.

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-weapon-sync:";

/// The key that turns the feature on and off in game, unless `[weapon_sync] key` says otherwise.
///
/// F7 is inventory sort, F8 voice chat and F9 net effects; nothing in this repo binds F6.
pub const DEFAULT_KEY: &str = "F6";

/// The spoken line a toggle plays: "Weapon sync, on." or "Weapon sync, off.", 16 kHz 16-bit mono
/// WAV, rendered with Piper's `en_US-lessac-medium`, the voice `ds2-voice-chat`'s English clips use.
#[must_use]
pub const fn clip(on: bool) -> &'static [u8] {
    if on {
        include_bytes!("../assets/en-on.wav")
    } else {
        include_bytes!("../assets/en-off.wav")
    }
}

pub mod glyph;
pub mod policy;

#[cfg(windows)]
mod hud;
#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_key, set_logger, set_test_cap};

#[cfg(test)]
mod tests {
    use super::clip;

    /// The `fmt ` chunk's format tag, channels, sample rate and bits per sample.
    fn wav_format(wav: &[u8]) -> Option<(u16, u16, u32, u16)> {
        if wav.get(0..4)? != b"RIFF" || wav.get(8..12)? != b"WAVE" {
            return None;
        }
        let mut at = 12;
        while at + 8 <= wav.len() {
            let id = &wav[at..at + 4];
            let len = u32::from_le_bytes(wav[at + 4..at + 8].try_into().ok()?) as usize;
            let body = wav.get(at + 8..at + 8 + len)?;
            if id == b"fmt " {
                let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                let rate = u32::from_le_bytes(body.get(4..8)?.try_into().ok()?);
                return Some((u16_at(0), u16_at(2), rate, u16_at(14)));
            }
            at += 8 + len + (len & 1);
        }
        None
    }

    #[test]
    fn both_clips_are_16_khz_16_bit_mono_pcm_like_voice_chats() {
        for on in [true, false] {
            assert_eq!(wav_format(clip(on)), Some((1, 1, 16_000, 16)), "on={on}");
        }
        assert_ne!(clip(true), clip(false));
    }
}
