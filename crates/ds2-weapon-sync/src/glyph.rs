//! The on-screen sign that weapon sync is on: two crossed swords on a dark tile, top right.
//!
//! Drawn from code, not from a game texture: this module is only the geometry, lines and dots in
//! back-buffer pixels, so it is tested on the host. `hud.rs` hands it to imgui's draw list. While
//! the feature is off nothing is drawn at all -- the tile disappears, and `ds2-overlay` renders no
//! imgui frame for it.

/// A straight stroke from `from` to `to`, `width` pixels thick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    /// One end, in back-buffer pixels.
    pub from: [f32; 2],
    /// The other end.
    pub to: [f32; 2],
    /// Thickness in pixels.
    pub width: f32,
}

/// A filled circle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    /// Centre, in back-buffer pixels.
    pub centre: [f32; 2],
    /// Radius in pixels.
    pub radius: f32,
}

/// Everything one frame draws: the tile behind, then the strokes, then the dots.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    /// The tile's top-left corner.
    pub min: [f32; 2],
    /// The tile's bottom-right corner.
    pub max: [f32; 2],
    /// Corner rounding of the tile, in pixels.
    pub rounding: f32,
    /// Blades, guards and grips.
    pub strokes: Vec<Stroke>,
    /// Pommels.
    pub dots: Vec<Dot>,
}

/// The tile's side as a share of the back buffer's height.
const SIZE_OF_HEIGHT: f32 = 0.045;

/// The tile's side is never smaller or larger than this, in pixels.
const SIZE_MIN: f32 = 32.0;
const SIZE_MAX: f32 = 96.0;

/// One sword in unit coordinates (0..1 across the tile, `y` down), pointing up and right: pommel,
/// where the grip meets the guard, the tip, and the guard's half length.
const POMMEL: [f32; 2] = [0.18, 0.82];
const GUARD: [f32; 2] = [0.34, 0.66];
const TIP: [f32; 2] = [0.84, 0.16];
const GUARD_HALF: f32 = 0.09;

/// Stroke widths and the pommel's radius as shares of the tile's side.
const BLADE_WIDTH: f32 = 0.075;
const GUARD_WIDTH: f32 = 0.065;
const GRIP_WIDTH: f32 = 0.055;
const POMMEL_RADIUS: f32 = 0.05;

/// The tile's side, in pixels, for a back buffer this tall.
#[must_use]
pub fn size_for(display_height: f32) -> f32 {
    (display_height * SIZE_OF_HEIGHT).clamp(SIZE_MIN, SIZE_MAX)
}

/// The glyph for a back buffer of `display` pixels, in its top-right corner half a tile in from
/// both edges.
#[must_use]
pub fn layout(display: [f32; 2]) -> Glyph {
    let size = size_for(display[1]);
    let margin = size * 0.5;
    let min = [display[0] - margin - size, margin];
    let max = [min[0] + size, min[1] + size];
    let at = |unit: [f32; 2]| [min[0] + unit[0] * size, min[1] + unit[1] * size];
    let across = GUARD_HALF * core::f32::consts::FRAC_1_SQRT_2;
    // The guard lies across the blade: along (1, 1) for a blade along (1, -1).
    let guard_a = [GUARD[0] - across, GUARD[1] - across];
    let guard_b = [GUARD[0] + across, GUARD[1] + across];
    let mut strokes = Vec::with_capacity(6);
    let mut dots = Vec::with_capacity(2);
    // The second sword is the first mirrored left to right, so the two cross in the middle.
    for mirror in [false, true] {
        let flip = |unit: [f32; 2]| {
            if mirror {
                [1.0 - unit[0], unit[1]]
            } else {
                unit
            }
        };
        strokes.push(Stroke {
            from: at(flip(GUARD)),
            to: at(flip(TIP)),
            width: BLADE_WIDTH * size,
        });
        strokes.push(Stroke {
            from: at(flip(guard_a)),
            to: at(flip(guard_b)),
            width: GUARD_WIDTH * size,
        });
        strokes.push(Stroke {
            from: at(flip(POMMEL)),
            to: at(flip(GUARD)),
            width: GRIP_WIDTH * size,
        });
        dots.push(Dot {
            centre: at(flip(POMMEL)),
            radius: POMMEL_RADIUS * size,
        });
    }
    Glyph {
        min,
        max,
        rounding: size * 0.18,
        strokes,
        dots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(glyph: &Glyph, point: [f32; 2], pad: f32) -> bool {
        point[0] >= glyph.min[0] + pad
            && point[0] <= glyph.max[0] - pad
            && point[1] >= glyph.min[1] + pad
            && point[1] <= glyph.max[1] - pad
    }

    #[test]
    fn every_stroke_and_dot_stays_on_the_tile() {
        for display in [
            [1280.0, 720.0],
            [2260.0, 1272.0],
            [3840.0, 2160.0],
            [640.0, 480.0],
        ] {
            let glyph = layout(display);
            for stroke in &glyph.strokes {
                let pad = stroke.width / 2.0;
                assert!(inside(&glyph, stroke.from, pad), "{display:?} {stroke:?}");
                assert!(inside(&glyph, stroke.to, pad), "{display:?} {stroke:?}");
            }
            for dot in &glyph.dots {
                assert!(
                    inside(&glyph, dot.centre, dot.radius),
                    "{display:?} {dot:?}"
                );
            }
        }
    }

    #[test]
    fn the_tile_sits_in_the_top_right_corner_of_the_screen() {
        for display in [[1280.0, 720.0], [2260.0, 1272.0], [3840.0, 2160.0]] {
            let glyph = layout(display);
            assert!(glyph.min[0] > display[0] / 2.0 && glyph.max[0] < display[0]);
            assert!(glyph.min[1] > 0.0 && glyph.max[1] < display[1] / 4.0);
        }
    }

    #[test]
    fn the_size_follows_the_height_within_bounds() {
        assert_eq!(size_for(100.0), SIZE_MIN);
        assert_eq!(size_for(10_000.0), SIZE_MAX);
        assert!((size_for(1272.0) - 57.24).abs() < 0.01);
    }

    #[test]
    fn two_swords_mirrored_across_the_middle_so_they_cross() {
        let glyph = layout([1280.0, 720.0]);
        assert_eq!(glyph.strokes.len(), 6);
        assert_eq!(glyph.dots.len(), 2);
        let centre = (glyph.min[0] + glyph.max[0]) / 2.0;
        let (left, right) = (glyph.strokes[0], glyph.strokes[3]);
        assert!(((left.from[0] - centre) + (right.from[0] - centre)).abs() < 1e-3);
        assert!((left.to[1] - right.to[1]).abs() < 1e-3);
        // The first blade runs up and right, the second up and left.
        assert!(left.to[0] > left.from[0] && left.to[1] < left.from[1]);
        assert!(right.to[0] < right.from[0] && right.to[1] < right.from[1]);
    }

    #[test]
    fn each_guard_crosses_its_blade_at_a_right_angle() {
        let glyph = layout([2260.0, 1272.0]);
        for pair in glyph.strokes.chunks(3) {
            let (blade, guard) = (pair[0], pair[1]);
            let b = [blade.to[0] - blade.from[0], blade.to[1] - blade.from[1]];
            let g = [guard.to[0] - guard.from[0], guard.to[1] - guard.from[1]];
            assert!(
                (b[0] * g[0] + b[1] * g[1]).abs() < 1e-2,
                "{blade:?} {guard:?}"
            );
        }
    }
}
