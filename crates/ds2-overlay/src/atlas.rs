//! The game's own menu atlases, decoded from the player's install for the panels to draw with.
//!
//! A `.flo` names its textures (`waku_03`, `In-game_01`); each lives as `<name>.tpf` inside a
//! `BND4` container in `GameDataEbl`. The archive stores no names, so the container is found by
//! the path hash `scripts/ds2-tpf.py index` recorded for it -- the same hash the game looks up.
//! Nothing is shipped: a missing or changed install is an error the caller falls back from.

use crate::ebl;
use crate::fefont::{self, Error, Page};

/// An atlas the panels know how to find.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atlas {
    /// The menu window frame: corners, crests and edge strips. 512x128.
    Waku,
    /// Bars, gauges, the item-warn X and the infusion glyphs. 1024x256.
    Waku03,
    /// The pause menu's tab glyphs and panels. 1024x1024.
    InGame01,
}

impl Atlas {
    /// Every atlas, in the order the panels upload them.
    pub const ALL: [Self; 3] = [Self::Waku, Self::Waku03, Self::InGame01];

    /// The `.tpf` member's name inside its container.
    const fn member(self) -> &'static str {
        match self {
            Self::Waku => "waku.tpf",
            Self::Waku03 => "waku_03.tpf",
            Self::InGame01 => "In-game_01.tpf",
        }
    }

    /// The `GameDataEbl` path hash of the container holding [`Self::member`], from
    /// `scripts/ds2-tpf.py index` on this game build. `waku` and `waku_03` share one.
    const fn container(self) -> u32 {
        match self {
            Self::Waku | Self::Waku03 => 0xb1fa_153f,
            Self::InGame01 => 0x903a_400d,
        }
    }
}

/// Read and decode one atlas out of `archive`.
///
/// # Errors
///
/// Whatever [`ebl::Archive::entry`] refuses, or [`Error::Format`] when the container does not hold
/// the member or the member is not a DXT3/DXT5 texture.
pub fn load(archive: &ebl::Archive, atlas: Atlas) -> Result<Page, Error> {
    let raw = archive.entry(atlas.container())?;
    let bnd = if raw.starts_with(b"DCX\0") {
        fefont::dcx(&raw)?
    } else {
        raw
    };
    let members = fefont::bnd4(&bnd)?;
    let (_, tpf) = members
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(atlas.member()))
        .ok_or(Error::Format("the container does not hold that atlas"))?;
    let (width, height, fourcc, blocks) = fefont::tpf(tpf)?;
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

    /// Both atlases decode at the sizes their `.flo` texture tables declare, and the game's own
    /// X sits where `docs/DS2-ITEM-REQUIREMENTS.md` measured it: opaque, and red.
    #[test]
    fn both_atlases_decode_and_the_x_is_where_it_was_measured() {
        let Some(game) = game() else {
            eprintln!("no DARK SOULS II install here; skipped");
            return;
        };
        let archive = ebl::Archive::open(&game).expect("archive");
        let waku = load(&archive, Atlas::Waku03).expect("waku_03");
        assert_eq!((waku.width, waku.height), (1024, 256));
        let ingame = load(&archive, Atlas::InGame01).expect("In-game_01");
        assert_eq!((ingame.width, ingame.height), (1024, 1024));
        let frame = load(&archive, Atlas::Waku).expect("waku");
        assert_eq!((frame.width, frame.height), (512, 128));
        // The frame's 6 px line crosses the top-left corner piece at rows 13-18, columns 5-10.
        let line = (15 * frame.width + 40) * 4;
        assert!(
            frame.rgba[line + 3] > 128,
            "{:?}",
            &frame.rgba[line..line + 4]
        );
        // The X's ink box is (745,169)-(767,191); its centre is solidly inked.
        let o = (180 * waku.width + 756) * 4;
        let px = &waku.rgba[o..o + 4];
        assert!(px[3] > 200 && px[0] > 120 && px[1] < 90, "{px:?}");
    }

    /// Every empty-slot silhouette `ds2_rva::FE_EQUIP_EMPTY_ART` names is art: one per sequence,
    /// its rect inside `In-game_01`, and at least two fifths of it inked. The ten measured 45% to
    /// 73% inked (alpha over 32), so a rect a few pixels off the art still passes and one pointing
    /// at empty atlas does not.
    #[test]
    fn every_empty_slot_silhouette_is_inked_art_in_in_game_01() {
        let mut sequences: Vec<u32> = ds2_rva::FE_EQUIP_EMPTY_ART
            .iter()
            .map(|(sequence, ..)| *sequence)
            .collect();
        sequences.sort_unstable();
        sequences.dedup();
        assert_eq!(sequences.len(), ds2_rva::FE_EQUIP_EMPTY_ART.len());
        let Some(game) = game() else {
            eprintln!("no DARK SOULS II install here; skipped");
            return;
        };
        let archive = ebl::Archive::open(&game).expect("archive");
        let page = load(&archive, Atlas::InGame01).expect("In-game_01");
        for (sequence, [x0, y0, x1, y1], scale) in ds2_rva::FE_EQUIP_EMPTY_ART {
            assert!(
                0.0 <= x0 && x0 < x1 && x1 <= page.width as f32,
                "sequence {sequence}"
            );
            assert!(
                0.0 <= y0 && y0 < y1 && y1 <= page.height as f32,
                "sequence {sequence}"
            );
            assert!(scale > 0.5 && scale <= 1.0, "sequence {sequence}: {scale}");
            let columns = x0 as usize..x1.ceil() as usize;
            let rows = y0 as usize..y1.ceil() as usize;
            let total = columns.len() * rows.len();
            let inked = rows
                .flat_map(|y| columns.clone().map(move |x| (x, y)))
                .filter(|&(x, y)| page.rgba[(y * page.width + x) * 4 + 3] > 32)
                .count();
            assert!(
                inked * 5 >= total * 2,
                "sequence {sequence}: {inked} of {total} pixels inked"
            );
        }
    }
}
