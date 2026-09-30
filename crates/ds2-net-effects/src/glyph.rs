//! The HUD glyph that says the F9 toggle is on, and the spoken clip each press plays.
//!
//! # The glyph
//!
//! A sparkle, drawn from imgui vector primitives and nothing of the game's: a dot in the middle
//! and eight rays around it, on its tile in the status strip (`ds2_overlay::status_strip`,
//! [`Slot::NetEffects`]) top right, beside `ds2-weapon-sync`'s swords and helm. It used to sit
//! alone at the top centre on a round disc. It is drawn while the toggle is on and not at all
//! while it is off.
//!
//! Everything here is geometry and bytes, so the host tests reach it; the drawing is in
//! `overlay.rs`.

use ds2_overlay::status_strip::{self, Slot, Tile};

/// One ray of the sparkle: from `inner` to `outer`, in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// The end nearer the centre.
    pub inner: [f32; 2],
    /// The end nearer the rim.
    pub outer: [f32; 2],
}

/// Where and how large the glyph is on a screen of a given size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// The strip tile the sparkle is drawn on.
    pub tile: Tile,
    /// Centre of the tile.
    pub center: [f32; 2],
    /// How far the longest ray may reach from the centre.
    pub radius: f32,
    /// Radius of the dot in the middle.
    pub core: f32,
    /// Line width of the rim and the rays.
    pub thickness: f32,
    /// The eight rays, starting straight up and going clockwise.
    pub rays: [Ray; RAYS],
}

/// How many rays the sparkle has.
pub const RAYS: usize = 8;

/// The sparkle's reach as a share of the tile's side.
pub const RADIUS_OF_TILE: f32 = 0.45;

/// Lay the glyph out on a `display` of `[width, height]` pixels.
pub fn layout(display: [f32; 2]) -> Glyph {
    let tile = status_strip::tile(display, Slot::NetEffects);
    let radius = tile.size * RADIUS_OF_TILE;
    let center = tile.centre();
    let rays = core::array::from_fn(|i| {
        let angle = i as f32 * core::f32::consts::TAU / RAYS as f32;
        // Clockwise from straight up, in screen coordinates (y grows downwards).
        let (dx, dy) = (angle.sin(), -angle.cos());
        // The diagonal rays are shorter, so the sparkle reads as a star rather than a wheel.
        let reach = if i % 2 == 0 { 0.80 } else { 0.62 };
        let point = |r: f32| [center[0] + dx * radius * r, center[1] + dy * radius * r];
        Ray {
            inner: point(0.40),
            outer: point(reach),
        }
    });
    Glyph {
        tile,
        center,
        radius,
        core: radius * 0.24,
        thickness: (radius * 0.1).max(2.0),
        rays,
    }
}

/// The clip a press plays: "Net effects on." or "Net effects off.", Piper's `en_US-lessac-medium`
/// voice -- the one `ds2-voice-chat`'s English pair uses -- as 16 kHz 16-bit mono PCM WAV.
pub const fn clip(on: bool) -> &'static [u8] {
    if on {
        include_bytes!("../assets/en-on.wav")
    } else {
        include_bytes!("../assets/en-off.wav")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    #[test]
    fn the_glyph_sits_in_its_strip_tile_off_the_game_bars() {
        for display in [[2260.0, 1272.0], [1920.0, 1080.0], [1280.0, 720.0]] {
            let g = layout(display);
            let t = g.tile;
            assert_eq!(t, status_strip::tile(display, Slot::NetEffects));
            assert!(g.center[0] - g.radius >= t.min[0] && g.center[0] + g.radius <= t.max[0]);
            assert!(g.center[1] - g.radius >= t.min[1] && g.center[1] + g.radius <= t.max[1]);
            // The top-left is the game's health and stamina.
            assert!(t.min[0] > display[0] * 0.5, "{display:?}");
        }
    }

    #[test]
    fn every_ray_is_inside_the_reach_and_outside_the_dot() {
        let g = layout([1920.0, 1080.0]);
        for ray in g.rays {
            let inner = distance(ray.inner, g.center);
            let outer = distance(ray.outer, g.center);
            assert!(inner > g.core, "{inner} clears the dot {}", g.core);
            assert!(outer > inner);
            assert!(
                outer + g.thickness / 2.0 < g.radius,
                "{outer} stays inside the rim"
            );
        }
        // The first ray points straight up.
        assert!((g.rays[0].outer[0] - g.center[0]).abs() < 0.01);
        assert!(g.rays[0].outer[1] < g.center[1]);
    }

    /// The `fmt ` chunk of a RIFF/WAVE file: (format tag, channels, rate, bits).
    fn wave_format(bytes: &[u8]) -> (u16, u16, u32, u16) {
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let id = &bytes[at..at + 4];
            let len =
                u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
                    as usize;
            if id == b"fmt " {
                let f = &bytes[at + 8..];
                return (
                    u16::from_le_bytes([f[0], f[1]]),
                    u16::from_le_bytes([f[2], f[3]]),
                    u32::from_le_bytes([f[4], f[5], f[6], f[7]]),
                    u16::from_le_bytes([f[14], f[15]]),
                );
            }
            at += 8 + len + (len & 1);
        }
        panic!("no fmt chunk");
    }

    #[test]
    fn both_clips_are_distinct_16khz_16bit_mono_pcm_waves() {
        for on in [true, false] {
            assert_eq!(wave_format(clip(on)), (1, 1, 16_000, 16), "clip({on})");
        }
        assert_ne!(clip(true), clip(false));
    }
}
