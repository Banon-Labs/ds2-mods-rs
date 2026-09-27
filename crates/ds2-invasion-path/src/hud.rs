//! What the toggle key shows and says: a small glyph in the top-left corner while the overlay is
//! on, and a spoken "Invasion path, on." / "Invasion path, off." on each press.
//!
//! Both are this crate's own. The glyph is drawn from segments with [`push_segment`], through the
//! same vertex list and the same pipeline as the route lines -- no game texture, no second
//! `Present` hook, no imgui panel slot. It is a trail of three dashes rising to an arrowhead:
//! amber over a dark outline, so it reads on snow and in a dark crypt alike.
//!
//! **Off is hidden, not dimmed.** While the overlay is off this crate hands the renderer no
//! vertices at all and the frame is untouched, and a dimmed glyph would end that for every player
//! who has the feature installed and switched off. The spoken clip is what says "off".
//!
//! **Top-left**, because weapon sync's crossed swords sit in the top-right corner.
//!
//! Pixels and bytes only, so `cargo test` on the host proves the placement, the scale and the
//! clips' format.

use crate::lines::{Vertex, push_segment};

/// The glyph's square edge as a fraction of the back buffer's height: about 50 px at 1440p.
pub(crate) const GLYPH_SIZE_FRACTION: f32 = 0.035;
/// Its distance from the top and left edges, the same way.
pub(crate) const GLYPH_MARGIN_FRACTION: f32 = 0.03;

/// The stroke, in glyph units (one unit is the square's edge).
const STROKE: f32 = 0.12;
/// The dark border around each stroke, in glyph units per side.
const OUTLINE: f32 = 0.05;

const AMBER: [f32; 4] = [1.0, 0.76, 0.22, 0.95];
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.75];

/// The strokes in glyph units, origin top-left and `y` down: three dashes of a trail climbing
/// from the bottom-left, a shaft, and the two barbs of an arrowhead pointing up and to the right.
const STROKES: [([f32; 2], [f32; 2]); 6] = [
    ([0.06, 0.94], [0.16, 0.84]),
    ([0.26, 0.74], [0.36, 0.64]),
    ([0.46, 0.54], [0.56, 0.44]),
    ([0.66, 0.34], [0.93, 0.07]),
    ([0.93, 0.07], [0.55, 0.07]),
    ([0.93, 0.07], [0.93, 0.45]),
];

/// Vertices the glyph adds: every stroke twice (outline, then colour), six vertices each.
pub(crate) const GLYPH_VERTICES: usize = STROKES.len() * 2 * crate::lines::VERTICES_PER_SEGMENT;

/// Append the on-state glyph for a back buffer of `screen` pixels.
///
/// A screen that is not a positive, finite size appends nothing.
pub(crate) fn push_glyph(out: &mut Vec<Vertex>, screen: [f32; 2]) {
    let height = screen[1];
    if !(height.is_finite() && height > 0.0 && screen[0].is_finite() && screen[0] > 0.0) {
        return;
    }
    let size = height * GLYPH_SIZE_FRACTION;
    let margin = height * GLYPH_MARGIN_FRACTION;
    let at = |p: [f32; 2]| [margin + p[0] * size, margin + p[1] * size];
    // Outlines first, then every coloured stroke, so no outline is painted over a stroke where the
    // arrowhead's three strokes meet.
    for (from, to) in STROKES {
        let (a, b) = extend(at(from), at(to), OUTLINE * size);
        push_segment(out, a, b, (STROKE + 2.0 * OUTLINE) * size, SHADOW);
    }
    for (from, to) in STROKES {
        push_segment(out, at(from), at(to), STROKE * size, AMBER);
    }
}

/// Lengthen a segment by `by` pixels at each end, so the outline caps the stroke's ends as well
/// as its sides.
fn extend(from: [f32; 2], to: [f32; 2], by: f32) -> ([f32; 2], [f32; 2]) {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f32::EPSILON {
        return (from, to);
    }
    let (ux, uy) = (dx / length * by, dy / length * by);
    ([from[0] - ux, from[1] - uy], [to[0] + ux, to[1] + uy])
}

/// The clip a press plays: Piper `en_US-lessac-medium`, 16 kHz 16-bit mono, the voice and format
/// of voice chat's English clips.
pub(crate) const fn clip(on: bool) -> &'static [u8] {
    if on {
        include_bytes!("../assets/en-on.wav")
    } else {
        include_bytes!("../assets/en-off.wav")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: [f32; 2] = [2560.0, 1440.0];

    fn bounds(vertices: &[Vertex]) -> [f32; 4] {
        vertices.iter().fold(
            [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
            |[x0, y0, x1, y1], v| {
                [
                    x0.min(v.position[0]),
                    y0.min(v.position[1]),
                    x1.max(v.position[0]),
                    y1.max(v.position[1]),
                ]
            },
        )
    }

    #[test]
    fn every_stroke_is_drawn_with_its_outline() {
        let mut out = Vec::new();
        push_glyph(&mut out, SCREEN);
        assert_eq!(out.len(), GLYPH_VERTICES);
        let amber = out.iter().filter(|v| v.color == AMBER).count();
        assert_eq!(amber, GLYPH_VERTICES / 2);
    }

    #[test]
    fn the_outline_is_drawn_before_the_colour() {
        let mut out = Vec::new();
        push_glyph(&mut out, SCREEN);
        let first_amber = out.iter().position(|v| v.color == AMBER).unwrap();
        assert!(out[..first_amber].iter().all(|v| v.color == SHADOW));
        assert!(out[first_amber..].iter().all(|v| v.color == AMBER));
    }

    #[test]
    fn it_sits_in_the_top_left_corner_clear_of_the_top_right() {
        let mut out = Vec::new();
        push_glyph(&mut out, SCREEN);
        let [x0, y0, x1, y1] = bounds(&out);
        assert!(x0 > 0.0 && y0 > 0.0, "clipped by the edge: {x0} {y0}");
        assert!(x1 < SCREEN[0] * 0.1 && y1 < SCREEN[1] * 0.1, "{x1} {y1}");
    }

    #[test]
    fn it_is_about_fifty_pixels_at_1440p_and_scales_with_height() {
        let mut big = Vec::new();
        push_glyph(&mut big, SCREEN);
        let [x0, _, x1, _] = bounds(&big);
        let width = x1 - x0;
        assert!((45.0..65.0).contains(&width), "{width}");

        let mut small = Vec::new();
        push_glyph(&mut small, [1280.0, 720.0]);
        let [a, _, b, _] = bounds(&small);
        assert!(((b - a) * 2.0 - width).abs() < 0.5, "{} vs {width}", b - a);
    }

    #[test]
    fn a_degenerate_screen_draws_nothing() {
        for screen in [
            [0.0, 0.0],
            [2560.0, 0.0],
            [f32::NAN, 1440.0],
            [2560.0, f32::INFINITY],
        ] {
            let mut out = Vec::new();
            push_glyph(&mut out, screen);
            assert!(out.is_empty(), "{screen:?}");
        }
    }

    /// The clips are whole RIFF WAVE files in voice chat's format: PCM, one channel, 16 kHz,
    /// 16 bits. `PlaySoundW` with `SND_MEMORY` reads the header, so a raw or mis-resampled file
    /// is silence in game rather than a test failure unless this reads it here.
    #[test]
    fn both_clips_are_distinct_16k_mono_16bit_pcm_wavs() {
        assert_ne!(clip(true), clip(false));
        for on in [true, false] {
            let wav = clip(on);
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            let u16_at = |i: usize| u16::from_le_bytes([wav[i], wav[i + 1]]);
            let u32_at =
                |i: usize| u32::from_le_bytes([wav[i], wav[i + 1], wav[i + 2], wav[i + 3]]);
            assert_eq!(u16_at(20), 1, "PCM");
            assert_eq!(u16_at(22), 1, "mono");
            assert_eq!(u32_at(24), 16_000, "16 kHz");
            assert_eq!(u16_at(34), 16, "16-bit");
            assert!(wav.len() > 16_000, "under half a second of audio");
        }
    }
}
