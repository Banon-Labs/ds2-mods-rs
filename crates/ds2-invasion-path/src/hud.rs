//! What the toggle key shows and says: a glyph on its tile in the status strip while the overlay
//! is on, and a spoken "Invasion path, on." / "Invasion path, off." on each press.
//!
//! Both are this crate's own. The glyph is drawn from segments with [`push_segment`], through the
//! same vertex list and the same pipeline as the route lines -- no game texture, no second
//! `Present` hook, no imgui panel slot. It is a trail of three dashes rising to an arrowhead, in
//! the strip's bronze over a dark outline, on the strip's square tile.
//!
//! **Off is hidden, not dimmed.** While the overlay is off this crate hands the renderer no
//! vertices at all and the frame is untouched, and a dimmed glyph would end that for every player
//! who has the feature installed and switched off. The spoken clip is what says "off".
//!
//! **In the strip, top right** ([`Slot::InvasionPath`], leftmost of the four signs). It used to
//! sit top-left, which is where the game draws health and stamina. This pipeline draws no text,
//! so unlike the other signs it does not show its name under the tile; the spoken clip says it.
//!
//! Pixels and bytes only, so `cargo test` on the host proves the placement, the scale and the
//! clips' format.

use ds2_overlay::status_strip::{self, Slot};

use crate::lines::{Vertex, push_segment};

/// The glyph's square inside its tile, as a share of the tile's side: the rest is margin.
const GLYPH_OF_TILE: f32 = 0.72;

/// The stroke, in glyph units (one unit is the square's edge).
const STROKE: f32 = 0.12;
/// The dark border around each stroke, in glyph units per side.
const OUTLINE: f32 = 0.05;

const INK: [f32; 4] = status_strip::INK;
const SHADOW: [f32; 4] = status_strip::SHADOW;
const TILE: [f32; 4] = status_strip::TILE;

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

/// Segments the tile takes: one wide segment for the fill, four for the frame.
const TILE_SEGMENTS: usize = 5;

/// Vertices the glyph adds: the tile, then every stroke twice (outline, then colour), six vertices
/// a segment.
pub(crate) const GLYPH_VERTICES: usize =
    (TILE_SEGMENTS + STROKES.len() * 2) * crate::lines::VERTICES_PER_SEGMENT;

/// Append the on-state glyph for a back buffer of `screen` pixels.
///
/// A screen that is not a positive, finite size appends nothing.
pub(crate) fn push_glyph(out: &mut Vec<Vertex>, screen: [f32; 2]) {
    let height = screen[1];
    if !(height.is_finite() && height > 0.0 && screen[0].is_finite() && screen[0] > 0.0) {
        return;
    }
    let tile = status_strip::tile(screen, Slot::InvasionPath);
    push_tile(out, &tile);
    let size = tile.size * GLYPH_OF_TILE;
    let inset = (tile.size - size) / 2.0;
    let origin = [tile.min[0] + inset, tile.min[1] + inset];
    let at = |p: [f32; 2]| [origin[0] + p[0] * size, origin[1] + p[1] * size];
    // Outlines first, then every coloured stroke, so no outline is painted over a stroke where the
    // arrowhead's three strokes meet.
    for (from, to) in STROKES {
        let (a, b) = extend(at(from), at(to), OUTLINE * size);
        push_segment(out, a, b, (STROKE + 2.0 * OUTLINE) * size, SHADOW);
    }
    for (from, to) in STROKES {
        push_segment(out, at(from), at(to), STROKE * size, INK);
    }
}

/// The strip's tile from segments, the only primitive this pipeline has: one segment as wide as
/// the tile is tall fills it, and four hairlines frame it.
fn push_tile(out: &mut Vec<Vertex>, tile: &status_strip::Tile) {
    let [x0, y0] = tile.min;
    let [x1, y1] = tile.max;
    let mid = (y0 + y1) / 2.0;
    push_segment(out, [x0, mid], [x1, mid], tile.size, TILE);
    let half = FRAME / 2.0;
    push_segment(out, [x0, y0 + half], [x1, y0 + half], FRAME, INK);
    push_segment(out, [x0, y1 - half], [x1, y1 - half], FRAME, INK);
    push_segment(out, [x0 + half, y0], [x0 + half, y1], FRAME, INK);
    push_segment(out, [x1 - half, y0], [x1 - half, y1], FRAME, INK);
}

/// The frame's hairline, in pixels.
const FRAME: f32 = ds2_overlay::style::FRAME_PX;

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

    /// The tile's vertices come first, then the outlines, then the coloured strokes.
    fn strokes(out: &[Vertex]) -> &[Vertex] {
        &out[TILE_SEGMENTS * crate::lines::VERTICES_PER_SEGMENT..]
    }

    #[test]
    fn every_stroke_is_drawn_with_its_outline() {
        let mut out = Vec::new();
        push_glyph(&mut out, SCREEN);
        assert_eq!(out.len(), GLYPH_VERTICES);
        let ink = strokes(&out).iter().filter(|v| v.color == INK).count();
        assert_eq!(ink, STROKES.len() * crate::lines::VERTICES_PER_SEGMENT);
    }

    #[test]
    fn the_outline_is_drawn_before_the_colour() {
        let mut out = Vec::new();
        push_glyph(&mut out, SCREEN);
        let s = strokes(&out);
        let first_ink = s.iter().position(|v| v.color == INK).unwrap();
        assert!(s[..first_ink].iter().all(|v| v.color == SHADOW));
        assert!(s[first_ink..].iter().all(|v| v.color == INK));
    }

    /// It is the leftmost sign of the strip, top right, off the game's bars in the top left.
    #[test]
    fn it_sits_on_its_strip_tile() {
        for screen in [SCREEN, [1920.0, 1080.0], [1280.0, 720.0]] {
            let mut out = Vec::new();
            push_glyph(&mut out, screen);
            let tile = status_strip::tile(screen, Slot::InvasionPath);
            let [x0, y0, x1, y1] = bounds(&out);
            let slack = 0.5;
            assert!(
                x0 >= tile.min[0] - slack && y0 >= tile.min[1] - slack,
                "{screen:?}"
            );
            assert!(
                x1 <= tile.max[0] + slack && y1 <= tile.max[1] + slack,
                "{screen:?}"
            );
            assert!(tile.min[0] > screen[0] * 0.5, "{screen:?}");
        }
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
