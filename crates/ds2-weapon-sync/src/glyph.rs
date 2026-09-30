//! The on-screen sign that weapon sync is on: two crossed swords on a tile in the status strip.
//!
//! Drawn from code, not from a game texture: this module is only the geometry, lines and dots in
//! back-buffer pixels, so it is tested on the host. `hud.rs` hands it to imgui's draw list. While
//! the feature is off nothing is drawn at all -- the tile disappears, and `ds2-overlay` renders no
//! imgui frame for it.

use ds2_overlay::status_strip::{self, Slot};
use ds2_overlay::style;

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

/// The glyph for a back buffer of `display` pixels, in its top-right corner half a tile in from
/// both edges.
#[must_use]
pub fn layout(display: [f32; 2]) -> Glyph {
    let (min, max, size) = tile(display, Slot::WeaponSync);
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
        rounding: style::ROUNDING,
        strokes,
        dots,
    }
}

/// `slot`'s tile in the shared status strip (`ds2_overlay::status_strip`), top right. Answers
/// `(min, max, side)`.
fn tile(display: [f32; 2], slot: Slot) -> ([f32; 2], [f32; 2], f32) {
    let t = status_strip::tile(display, slot);
    (t.min, t.max, t.size)
}

/// The helm, in unit coordinates: a dome from the left cheek over the top to the right cheek, a
/// brim across, a nose guard down the middle, cheek plates, and a crest rivet.
const HELM_CENTRE: [f32; 2] = [0.5, 0.50];
const HELM_RADIUS: f32 = 0.30;
/// How many straight strokes approximate the dome.
const HELM_DOME_SEGMENTS: u8 = 10;
const HELM_CHEEK_BOTTOM: f32 = 0.82;
const HELM_CHEEK_IN: f32 = 0.06;
const HELM_NOSE_BOTTOM: f32 = 0.74;
const HELM_STROKE: f32 = 0.07;
const HELM_NOSE: f32 = 0.06;
const HELM_RIVET: f32 = 0.045;

/// The armour sync glyph: a helm on its own tile, one tile left of the swords, so each feature's
/// sign is shown and hidden on its own.
#[must_use]
pub fn helm_layout(display: [f32; 2]) -> Glyph {
    let (min, max, size) = tile(display, Slot::ArmorSync);
    let at = |unit: [f32; 2]| [min[0] + unit[0] * size, min[1] + unit[1] * size];
    let width = HELM_STROKE * size;
    let mut strokes = Vec::with_capacity(usize::from(HELM_DOME_SEGMENTS) + 4);
    let point = |step: u8| {
        let angle = core::f32::consts::PI * (1.0 + f32::from(step) / f32::from(HELM_DOME_SEGMENTS));
        [
            HELM_CENTRE[0] + HELM_RADIUS * angle.cos(),
            HELM_CENTRE[1] + HELM_RADIUS * angle.sin(),
        ]
    };
    for step in 0..HELM_DOME_SEGMENTS {
        strokes.push(Stroke {
            from: at(point(step)),
            to: at(point(step + 1)),
            width,
        });
    }
    let left = HELM_CENTRE[0] - HELM_RADIUS;
    let right = HELM_CENTRE[0] + HELM_RADIUS;
    // The brim, across the bottom of the dome.
    strokes.push(Stroke {
        from: at([left, HELM_CENTRE[1]]),
        to: at([right, HELM_CENTRE[1]]),
        width,
    });
    // The cheek plates, down and slightly in.
    strokes.push(Stroke {
        from: at([left, HELM_CENTRE[1]]),
        to: at([left + HELM_CHEEK_IN, HELM_CHEEK_BOTTOM]),
        width,
    });
    strokes.push(Stroke {
        from: at([right, HELM_CENTRE[1]]),
        to: at([right - HELM_CHEEK_IN, HELM_CHEEK_BOTTOM]),
        width,
    });
    // The nose guard.
    strokes.push(Stroke {
        from: at(HELM_CENTRE),
        to: at([HELM_CENTRE[0], HELM_NOSE_BOTTOM]),
        width: HELM_NOSE * size,
    });
    let dots = vec![Dot {
        centre: at([HELM_CENTRE[0], HELM_CENTRE[1] - HELM_RADIUS]),
        radius: HELM_RIVET * size,
    }];
    Glyph {
        min,
        max,
        rounding: style::ROUNDING,
        strokes,
        dots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helm_stays_on_its_own_tile_left_of_the_swords_and_is_symmetric() {
        for display in [
            [1280.0, 720.0],
            [2260.0, 1272.0],
            [3840.0, 2160.0],
            [640.0, 480.0],
        ] {
            let swords = layout(display);
            let helm = helm_layout(display);
            assert!(
                helm.max[0] < swords.min[0],
                "{display:?}: the tiles overlap"
            );
            assert!(helm.min[0] > display[0] / 2.0, "{display:?}");
            assert!((helm.min[1] - swords.min[1]).abs() < 1e-3, "same row");
            for stroke in &helm.strokes {
                let pad = stroke.width / 2.0;
                assert!(inside(&helm, stroke.from, pad), "{display:?} {stroke:?}");
                assert!(inside(&helm, stroke.to, pad), "{display:?} {stroke:?}");
            }
            for dot in &helm.dots {
                assert!(inside(&helm, dot.centre, dot.radius), "{display:?} {dot:?}");
            }
            // Mirror every stroke end across the tile's middle: it lands on another stroke end.
            let centre = (helm.min[0] + helm.max[0]) / 2.0;
            let ends: Vec<[f32; 2]> = helm.strokes.iter().flat_map(|s| [s.from, s.to]).collect();
            for end in &ends {
                let mirrored = [2.0 * centre - end[0], end[1]];
                assert!(
                    ends.iter()
                        .any(|e| (e[0] - mirrored[0]).abs() < 1e-2
                            && (e[1] - mirrored[1]).abs() < 1e-2),
                    "{display:?}: {end:?} has no mirror"
                );
            }
        }
    }

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
