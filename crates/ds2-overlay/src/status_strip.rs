//! One strip of feature signs in the top-right corner, instead of one per corner.
//!
//! Weapon sync, armour sync, net effects and the invasion path each put a sign on screen while
//! they are on. They used to pick their own corners -- the invasion path's sat top-left, on the
//! game's HP and stamina bars -- with their own sizes and golds. Every sign now asks this module
//! for its tile, so they line up right to left in [`Slot`] order, all the same size, and each
//! shows its name under its tile for [`NAME_SECONDS`] after it is switched on
//! (docs/DS2-UI-DESIGN.md, "Per element").
//!
//! Geometry only, so it is tested on the host; each feature draws its own glyph inside the tile.

use crate::style;

/// Where a sign sits in the strip, counted from the right-hand edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// `ds2-weapon-sync`'s crossed swords, rightmost.
    WeaponSync = 0,
    /// `ds2-armor-sync`'s helm.
    ArmorSync = 1,
    /// `ds2-net-effects`'s sparkle.
    NetEffects = 2,
    /// `ds2-invasion-path`'s trail and arrowhead, leftmost.
    InvasionPath = 3,
}

/// The tile's side as a share of the back buffer's height.
const SIZE_OF_HEIGHT: f32 = 0.045;
/// The tile's side is never smaller or larger than this, in pixels.
const SIZE_MIN: f32 = 32.0;
const SIZE_MAX: f32 = 96.0;
/// Gap between tiles, as a share of the tile's side.
const GAP_OF_SIZE: f32 = 0.25;

/// How long a sign's name shows under it after the sign is switched on.
pub const NAME_SECONDS: f32 = 2.0;

/// Tile fill: the panel ink, so the scene shows through a little.
pub const TILE: [f32; 4] = style::with_alpha(style::INK_0, 0.8);
/// Tile frame and glyph strokes.
pub const INK: [f32; 4] = style::BRONZE;
/// Under the glyph strokes, so they read over a bright scene.
pub const SHADOW: [f32; 4] = style::rgba(0x00_00_00, 0.85);

/// A tile's corners and side, in back-buffer pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    /// Top-left corner.
    pub min: [f32; 2],
    /// Bottom-right corner.
    pub max: [f32; 2],
    /// Side length.
    pub size: f32,
}

impl Tile {
    /// A point given in unit coordinates across the tile (0..1, `y` down).
    #[must_use]
    pub fn at(&self, unit: [f32; 2]) -> [f32; 2] {
        [
            self.min[0] + unit[0] * self.size,
            self.min[1] + unit[1] * self.size,
        ]
    }

    /// The tile's centre.
    #[must_use]
    pub fn centre(&self) -> [f32; 2] {
        self.at([0.5, 0.5])
    }
}

/// The tile side for a back buffer this tall.
#[must_use]
pub fn size_for(height: f32) -> f32 {
    (height * SIZE_OF_HEIGHT).clamp(SIZE_MIN, SIZE_MAX)
}

/// Where `slot`'s tile goes on a back buffer of `display` pixels.
#[must_use]
pub fn tile(display: [f32; 2], slot: Slot) -> Tile {
    let size = size_for(display[1]);
    let margin = size * 0.5;
    let shift = f32::from(slot as u8) * size * (1.0 + GAP_OF_SIZE);
    let min = [display[0] - margin - size - shift, margin];
    Tile {
        min,
        max: [min[0] + size, min[1] + size],
        size,
    }
}

/// Where a name of `text_width` pixels goes under `tile`: right-aligned to it, a quarter tile below.
#[must_use]
pub fn name_origin(tile: &Tile, text_width: f32) -> [f32; 2] {
    [tile.max[0] - text_width, tile.max[1] + tile.size * 0.25]
}

/// Whether a sign switched on `seconds_on` seconds ago still shows its name.
#[must_use]
pub fn shows_name(seconds_on: f32) -> bool {
    (0.0..NAME_SECONDS).contains(&seconds_on)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOTS: [Slot; 4] = [
        Slot::WeaponSync,
        Slot::ArmorSync,
        Slot::NetEffects,
        Slot::InvasionPath,
    ];

    #[test]
    fn tiles_line_up_right_to_left_without_touching() {
        for display in [
            [1280.0, 720.0],
            [1920.0, 1080.0],
            [2560.0, 1440.0],
            [640.0, 360.0],
        ] {
            let tiles = SLOTS.map(|s| tile(display, s));
            for pair in tiles.windows(2) {
                assert!(pair[1].max[0] < pair[0].min[0], "{display:?}: {pair:?}");
                assert!((pair[1].min[1] - pair[0].min[1]).abs() < f32::EPSILON);
            }
        }
    }

    /// The game's HP and stamina bars are top-left; the whole strip stays in the right half.
    #[test]
    fn the_strip_stays_off_the_left_half() {
        for display in [
            [1280.0, 720.0],
            [1920.0, 1080.0],
            [2560.0, 1440.0],
            [3440.0, 1440.0],
        ] {
            let last = tile(display, Slot::InvasionPath);
            assert!(last.min[0] > display[0] * 0.5, "{display:?}: {last:?}");
            assert!(last.min[1] > 0.0);
        }
    }

    #[test]
    fn size_follows_the_height_within_its_bounds() {
        assert!((size_for(1080.0) - 48.6).abs() < 0.01);
        assert!((size_for(200.0) - SIZE_MIN).abs() < f32::EPSILON);
        assert!((size_for(4000.0) - SIZE_MAX).abs() < f32::EPSILON);
    }

    #[test]
    fn the_name_shows_for_two_seconds() {
        assert!(shows_name(0.0));
        assert!(shows_name(1.99));
        assert!(!shows_name(2.0));
        assert!(!shows_name(-0.1));
    }

    #[test]
    fn the_name_sits_under_its_tile() {
        let t = tile([1920.0, 1080.0], Slot::NetEffects);
        let o = name_origin(&t, 80.0);
        assert!((o[0] + 80.0 - t.max[0]).abs() < f32::EPSILON);
        assert!(o[1] > t.max[1]);
    }
}
