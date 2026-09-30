//! An item's inventory icon, decoded from the player's install for a panel to draw.
//!
//! The executable builds an item icon's path as `icon:/tex/Icon/IC_%010d.tpf`, one texture per
//! icon id and nothing for an infusion (docs/DS2-ITEM-REQUIREMENTS.md). In `GameDataEbl` that is
//! `/menu/tex/icon/ic_<id>.tpf`: `scripts/ds2-item-icons.py locate` finds the Longsword's
//! (1220000) and the Dagger's (1000000) there and under none of the other spellings it tries. Each
//! is a bare `TPF`, not wrapped in `DCX`, holding one DXT5 texture: 128x256 for the Longsword.
//! Nothing is shipped; a missing install or item is an error the caller draws without.
//!
//! The icon id is the item's own id except where `ItemParam` names another item's, which
//! `data/item-icons.tsv` lists: the Old Mirrah Greatsword wears the Mirrah Greatsword's icon, and
//! most keys share one.

use std::sync::OnceLock;

use crate::ebl::{self, Archive};
use crate::fefont::{self, Error, Page};

/// `(item, icon)`: every item whose icon is filed under another item's id, from `ItemParam` field
/// 0 (`scripts/ds2-item-icons.py table`).
const REMAPS: &str = include_str!("../data/item-icons.tsv");

/// The id item `item`'s icon is filed under.
#[must_use]
pub fn icon_id(item: u32) -> u32 {
    static TABLE: OnceLock<Vec<(u32, u32)>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        REMAPS
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let (item, icon) = line.split_once('\t')?;
                Some((item.parse().ok()?, icon.parse().ok()?))
            })
            .collect()
    });
    table
        .iter()
        .find(|(from, _)| *from == item)
        .map_or(item, |&(_, icon)| icon)
}

/// Where the icon filed under `icon` is in `GameDataEbl`.
#[must_use]
pub fn path(icon: u32) -> String {
    format!("/menu/tex/icon/ic_{icon:010}.tpf")
}

/// Read and decode item `item`'s icon out of `archive`.
///
/// # Errors
///
/// Whatever [`Archive::entry`] refuses, which includes an item with no icon, or [`Error::Format`]
/// when the entry is not a DXT3/DXT5 `TPF`.
pub fn load(archive: &Archive, item: u32) -> Result<Page, Error> {
    let raw = archive.entry(ebl::path_hash(&path(icon_id(item))))?;
    let tpf = if raw.starts_with(b"DCX\0") {
        fefont::dcx(&raw)?
    } else {
        raw
    };
    let (width, height, fourcc, blocks) = fefont::tpf(&tpf)?;
    fefont::decode(width, height, fourcc, blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn game() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        let dir = PathBuf::from(home).join(
            ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game",
        );
        dir.join("GameDataEbl.bhd").is_file().then_some(dir)
    }

    #[test]
    fn the_path_is_ten_digits() {
        assert_eq!(path(1_220_000), "/menu/tex/icon/ic_0001220000.tpf");
    }

    /// The table parses whole, and an item it does not list keeps its own id.
    #[test]
    fn an_item_borrows_the_icon_itemparam_names() {
        let rows = REMAPS.lines().filter(|line| !line.starts_with('#')).count();
        let parsed = REMAPS
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter(|line| {
                line.split_once('\t')
                    .and_then(|(item, icon)| {
                        Some((item.parse::<u32>().ok()?, icon.parse::<u32>().ok()?))
                    })
                    .is_some()
            })
            .count();
        assert_eq!(parsed, rows);
        assert_eq!(icon_id(1_911_000), 1_910_000, "the Old Mirrah Greatsword");
        assert_eq!(icon_id(1_220_000), 1_220_000, "the Longsword");
    }

    /// The Longsword's icon decodes at the size its DDS header gives, with both ink and a clear
    /// margin in it; an id with no icon is an error, not a blank texture.
    #[test]
    fn the_longswords_icon_decodes_and_a_made_up_id_has_none() {
        let Some(game) = game() else {
            eprintln!("no DARK SOULS II install here; skipped");
            return;
        };
        let archive = Archive::open(&game).expect("archive");
        let icon = load(&archive, 1_220_000).expect("the Longsword has an icon");
        assert_eq!((icon.width, icon.height), (128, 256));
        let alpha = icon.rgba.as_chunks::<4>().0.iter().map(|px| px[3]);
        let inked = alpha.clone().filter(|&a| a > 200).count();
        let clear = alpha.filter(|&a| a == 0).count();
        assert!(inked > 1000 && clear > 1000, "inked {inked} clear {clear}");
        assert!(load(&archive, 1_220_001).is_err());
        // Filed under the Mirrah Greatsword's id, which is the only one with a file.
        assert!(load(&archive, 1_911_000).is_ok());
    }
}
