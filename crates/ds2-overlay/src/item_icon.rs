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

/// A weapon icon's texture, width then height: every weapon's but one small one is this size
/// (`scripts/ds2-item-icons.py measure`).
pub const WEAPON_TEXTURE: [f32; 2] = [128.0, 256.0];

/// The part of a [`WEAPON_TEXTURE`] a weapon's art is drawn in, `x0, y0, x1, y1`.
///
/// Every weapon icon's ink lies in rows 29 to 219 (`scripts/ds2-item-icons.py measure`), and this
/// keeps a row either side. The rest of the texture is empty, so a picker that shows only this
/// band shows the art at the texture's own size in the smallest box that holds any of it.
pub const WEAPON_ART: [f32; 4] = [0.0, 28.0, 128.0, 220.0];

/// [`WEAPON_ART`] as fractions of [`WEAPON_TEXTURE`], which is how [`crate::panels::item_icon`]
/// takes a crop: the one weapon icon smaller than 128x256 shows the same band of its art.
pub const WEAPON_ART_FRACTION: [f32; 4] = [
    WEAPON_ART[0] / WEAPON_TEXTURE[0],
    WEAPON_ART[1] / WEAPON_TEXTURE[1],
    WEAPON_ART[2] / WEAPON_TEXTURE[0],
    WEAPON_ART[3] / WEAPON_TEXTURE[1],
];

/// The band of ANY item's icon its art is drawn in, as fractions of the texture: what a slot that
/// can hold a weapon, a ring, a piece of armour or a quick item shows.
///
/// The game draws every icon into one 64x128 box, whatever its own size, so every kind's art lands
/// in the same place. `scripts/ds2-item-icons.py measure --kind ...` puts all of it between rows
/// 24/256 (armour) and 118/128 (items), and this keeps a row either side of that.
pub const SLOT_ART: [f32; 4] = [0.0, 23.0 / 256.0, 1.0, 119.0 / 128.0];

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

    /// The Longsword's ink lies inside the band a picker shows, which lies inside its texture.
    #[test]
    fn a_weapons_art_is_inside_the_band() {
        let [x0, y0, x1, y1] = WEAPON_ART;
        assert!(x0 >= 0.0 && y0 >= 0.0 && x1 <= WEAPON_TEXTURE[0] && y1 <= WEAPON_TEXTURE[1]);
        let Some(game) = game() else {
            eprintln!("no DARK SOULS II install here; skipped");
            return;
        };
        let archive = Archive::open(&game).expect("archive");
        let icon = load(&archive, 1_220_000).expect("the Longsword has an icon");
        assert_eq!([icon.width as f32, icon.height as f32], WEAPON_TEXTURE);
        let inked = |x: usize, y: usize| icon.rgba[(y * icon.width + x) * 4 + 3] >= 32;
        for y in (0..icon.height).filter(|&y| (y as f32) < y0 || (y as f32) >= y1) {
            assert!(
                (0..icon.width).all(|x| !inked(x, y)),
                "the Longsword has ink in row {y}, outside the band"
            );
        }
    }

    /// The band a slot shows holds the art of every kind of item, at both texture sizes: a weapon,
    /// a ring, armour at 128x256 and at 64x128, and a quick item.
    #[test]
    fn every_kinds_art_is_inside_the_slot_band() {
        let [x0, y0, x1, y1] = SLOT_ART;
        assert!(x0 >= 0.0 && y0 >= 0.0 && x1 <= 1.0 && y1 <= 1.0 && y0 < y1);
        let Some(game) = game() else {
            eprintln!("no DARK SOULS II install here; skipped");
            return;
        };
        let archive = Archive::open(&game).expect("archive");
        for (item, what) in [
            (1_220_000, "the Longsword"),
            (40_040_002, "ring 40040002"),
            (21_500_101, "armour 21500101"),
            (23_050_102, "the Smelter Demon Gauntlets"),
            (60_010_000, "the Lifegem"),
            (60_540_000, "the Throwing Knife"),
        ] {
            let icon = load(&archive, item).unwrap_or_else(|why| panic!("{what}: {why}"));
            let (top, bottom) = (y0 * icon.height as f32, y1 * icon.height as f32);
            let inked = |x: usize, y: usize| icon.rgba[(y * icon.width + x) * 4 + 3] >= 32;
            for y in (0..icon.height).filter(|&y| (y as f32) < top || (y as f32) >= bottom) {
                assert!(
                    (0..icon.width).all(|x| !inked(x, y)),
                    "{what} ({}x{}) has ink in row {y}, outside the band",
                    icon.width,
                    icon.height
                );
            }
        }
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
