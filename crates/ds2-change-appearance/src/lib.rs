//! A "Change Appearance" row in the bonfire menu, below Item box, that opens the game's own
//! character creator and commits only the appearance: face, sex and body. And a "Rename Character"
//! row below it that opens the game's own name entry ([`ds2_rva::OPEN_NAME_WINDOW`]); either row
//! can be left out ([`Rows`]).
//!
//! Class, stats, souls, level, gift and inventory stay as they are. The prototype this ports,
//! `scripts/frida/appearance-only-creator.js`, was proven end to end in game on 2026-10-03; the
//! trace behind every step is `docs/DS2-FACE-ONLY-CREATOR.md`.
//!
//! # What happens, in order
//!
//! 1. **The row.** The bonfire menu builder ([`ds2_rva::BONFIRE_MENU_BUILD`]) adds its rows with
//!    [`ds2_rva::FEX_COMMAND_DIALOG_ADD_ROW`]. While the builder runs, the row after the one
//!    labelled with the game's own "Item box" text is ours. The label is matched by pointer, the
//!    one [`ds2_rva::FMG_TEXT_LOOKUP`] returns, so it holds in every language.
//! 2. **Choosing it** returns no job, which closes the bonfire menu, and asks for the creator.
//! 3. **The open** waits for the game tick on which the HUD is no longer suspended, i.e. the menu
//!    has let go of the frontend. Opening inside the row's own handler crashed, and opening one
//!    frame later was torn down by the menu's close.
//! 4. **The creator's enter** ([`ds2_rva::CHARA_MAKER_ENTER`]) strips the visible equipment; the
//!    appearance block and the equipment are copied first. Class and gift are pre-filled after,
//!    so Finish creation is available.
//! 5. **Finish creation** runs the face commit, whose block is kept, and then the class/gift
//!    commit, which is skipped. After the face commit the block is imported into the player's face
//!    part, which also writes sex, and the equipment is put back.
//!
//! 6. **The tab bar** while ours is up has no Class & gift, and Body, Face and Advanced settings
//!    each take a third of it ([`tabs`]). The first tab spec is not appended, and the creator's
//!    cached layout is rewritten for the open and put back after.
//!
//! The creator's menu bundle is created on the resource manager's default heap
//! ([`ds2_rva::CHARA_MAKER_BUNDLE_HEAP_ARG`]): after a bonfire the frontend heap could not take its
//! layout, and the creator stayed blank for the rest of the process.

/// Prefix on every log line this crate writes.
pub const LOG_PREFIX: &str = "ds2-change-appearance:";

/// The row's text.
pub const ROW_LABEL: &str = "Change Appearance";

/// [`ROW_LABEL`] as the NUL-terminated UTF-16 the menu draws. The game keeps the pointer, so this
/// lives in a `static`.
pub static ROW_LABEL_UTF16: [u16; ROW_LABEL.len() + 1] = utf16_nul(ROW_LABEL);

const fn utf16_nul<const N: usize>(text: &str) -> [u16; N] {
    let bytes = text.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    while i < bytes.len() {
        // ASCII only: one byte is one UTF-16 unit. `utf16_nul_is_the_label` checks that.
        out[i] = bytes[i] as u16;
        i += 1;
    }
    out
}

/// The second row's text: opens the game's own name entry ([`ds2_rva::OPEN_NAME_WINDOW`]).
pub const RENAME_ROW_LABEL: &str = "Rename Character";

/// [`RENAME_ROW_LABEL`] as NUL-terminated UTF-16, in a `static` for the same reason as
/// [`ROW_LABEL_UTF16`].
pub static RENAME_ROW_LABEL_UTF16: [u16; RENAME_ROW_LABEL.len() + 1] = utf16_nul(RENAME_ROW_LABEL);

/// Which rows to add below Item box, in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    /// "Change Appearance": the creator, committing only face, sex and body.
    pub change_appearance: bool,
    /// "Rename Character": name entry.
    pub rename: bool,
}

impl Rows {
    /// Whether any row is wanted, i.e. whether to install at all.
    pub const fn any(self) -> bool {
        self.change_appearance || self.rename
    }
}

/// How many game ticks after the class/gift commit was skipped the result is logged, so the face
/// rebuild and the equipment refresh have landed. The tick is 60 a second.
pub const REPORT_AFTER_TICKS: u32 = 120;

/// Whether this tick is the one to open the creator on.
///
/// `suspended` is the HUD operator's suspended byte, `None` when there is no HUD (the title, a
/// load). Only a clear `0` opens: the bonfire menu sets it while it owns the frontend.
pub const fn open_now(requested: bool, suspended: Option<u8>) -> bool {
    requested && matches!(suspended, Some(0))
}

/// How many visible-equipment records differ between `before` and `after`.
pub fn records_differing<R: PartialEq>(before: &[R], after: &[R]) -> usize {
    before.iter().zip(after).filter(|(b, a)| b != a).count() + before.len().abs_diff(after.len())
}

pub mod tabs;

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{Outcome, install, set_logger};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_nul_is_the_label() {
        for (text, wide) in [
            (ROW_LABEL, &ROW_LABEL_UTF16[..]),
            (RENAME_ROW_LABEL, &RENAME_ROW_LABEL_UTF16[..]),
        ] {
            let decoded = String::from_utf16(&wide[..text.len()]).unwrap();
            assert_eq!(decoded, text);
            assert_eq!(wide[text.len()], 0);
            assert!(text.is_ascii());
        }
    }

    #[test]
    fn opens_only_once_asked_and_the_hud_is_back() {
        assert!(open_now(true, Some(0)));
        assert!(
            !open_now(true, Some(1)),
            "the bonfire menu still owns the frontend"
        );
        assert!(!open_now(true, None), "no HUD: title or load");
        assert!(!open_now(false, Some(0)));
    }

    #[test]
    fn counts_differing_records() {
        let before = [[1u8; 4], [2; 4], [3; 4]];
        assert_eq!(records_differing(&before, &before), 0);
        assert_eq!(records_differing(&before, &[[1; 4], [9; 4], [3; 4]]), 1);
        assert_eq!(records_differing(&before, &[[1; 4]]), 2);
    }
}
