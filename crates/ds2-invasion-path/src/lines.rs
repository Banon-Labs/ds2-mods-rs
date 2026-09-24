//! Screen-space geometry: turning projected points into the triangles the overlay draws.
//!
//! Separate from `crate::render` on purpose. Everything here is arithmetic on pixels, which means
//! it can be -- and is -- proven by `cargo test` on the host, while `render` is COM calls that
//! only exist on Windows. Putting [`push_segment`] in `render` would have made its tests
//! unreachable from a Linux `cargo test`, which is the same as not writing them.

// Consumed by `crate::render`, which is `cfg(windows)`; the maths is host-tested.
#![cfg_attr(not(windows), allow(dead_code))]

/// One screen-space vertex: pixels and a straight (non-premultiplied) RGBA colour.
///
/// `repr(C)` because the layout is an ABI. `crate::render::INPUT_LAYOUT` describes these exact
/// byte offsets to Direct3D, and a Rust-ordered struct would hand the shader the colour as a
/// position -- which draws nothing recognisable and looks like a broken projection.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vertex {
    pub(crate) position: [f32; 2],
    pub(crate) color: [f32; 4],
}

/// Vertices per segment: two triangles.
pub(crate) const VERTICES_PER_SEGMENT: usize = 6;

/// The thinnest a line is allowed to be, in pixels.
///
/// A quad narrower than about a pixel can fall entirely between sample points and rasterise to
/// nothing at all -- so a distant player's line would not be faint, it would be absent, which is
/// the one thing `geometry::MIN_ALPHA` exists to prevent.
pub(crate) const MIN_WIDTH_PX: f32 = 1.0;

/// Expand a screen-space segment into two triangles and append them.
///
/// Direct3D's line primitives are one pixel wide with no way to ask for more, and the distance
/// ramp is entirely about width. So a segment is a quad, offset along the perpendicular. The
/// offset is taken in PIXELS rather than in world units, which is why a line stays the same
/// thickness whether the player it points at is two metres away or two hundred -- the fading is
/// the distance cue, not the foreshortening.
///
/// A degenerate segment appends nothing: its direction is `0/0`, and the NaN that follows would
/// reach the vertex buffer as four unrepresentable corners.
pub(crate) fn push_segment(
    out: &mut Vec<Vertex>,
    from: [f32; 2],
    to: [f32; 2],
    width_px: f32,
    color: [f32; 4],
) {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let length = (dx * dx + dy * dy).sqrt();
    if !length.is_finite() || length <= f32::EPSILON {
        return;
    }
    if !width_px.is_finite() || color.iter().any(|channel| !channel.is_finite()) {
        return;
    }
    let half = (width_px * 0.5).max(MIN_WIDTH_PX * 0.5);
    let (nx, ny) = (-dy / length * half, dx / length * half);
    let corners = [
        [from[0] + nx, from[1] + ny],
        [to[0] + nx, to[1] + ny],
        [to[0] - nx, to[1] - ny],
        [from[0] - nx, from[1] - ny],
    ];
    for index in [0usize, 1, 2, 0, 2, 3] {
        out.push(Vertex {
            position: corners[index],
            color,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    #[test]
    fn a_segment_becomes_two_triangles() {
        let mut out = Vec::new();
        push_segment(&mut out, [0.0, 0.0], [10.0, 0.0], 4.0, WHITE);
        assert_eq!(out.len(), VERTICES_PER_SEGMENT);
    }

    #[test]
    fn a_zero_length_segment_emits_nothing() {
        let mut out = Vec::new();
        push_segment(&mut out, [5.0, 5.0], [5.0, 5.0], 4.0, WHITE);
        assert!(out.is_empty(), "a degenerate quad would carry NaN corners");
    }

    #[test]
    fn a_horizontal_segment_is_expanded_vertically() {
        let mut out = Vec::new();
        push_segment(&mut out, [0.0, 10.0], [10.0, 10.0], 4.0, WHITE);
        let ys: Vec<f32> = out.iter().map(|vertex| vertex.position[1]).collect();
        assert!(ys.iter().any(|y| (*y - 12.0).abs() < 1.0e-4), "{ys:?}");
        assert!(ys.iter().any(|y| (*y - 8.0).abs() < 1.0e-4), "{ys:?}");
    }

    #[test]
    fn a_vertical_segment_is_expanded_horizontally() {
        let mut out = Vec::new();
        push_segment(&mut out, [10.0, 0.0], [10.0, 10.0], 6.0, WHITE);
        let xs: Vec<f32> = out.iter().map(|vertex| vertex.position[0]).collect();
        assert!(xs.iter().any(|x| (*x - 13.0).abs() < 1.0e-4), "{xs:?}");
        assert!(xs.iter().any(|x| (*x - 7.0).abs() < 1.0e-4), "{xs:?}");
    }

    #[test]
    fn a_hairline_still_has_width() {
        let mut out = Vec::new();
        push_segment(&mut out, [0.0, 0.0], [10.0, 0.0], 0.0, WHITE);
        let ys: Vec<f32> = out.iter().map(|vertex| vertex.position[1]).collect();
        let span = ys.iter().copied().fold(f32::MIN, f32::max)
            - ys.iter().copied().fold(f32::MAX, f32::min);
        assert!(
            span >= MIN_WIDTH_PX,
            "a zero-width quad rasterises to nothing, which is not what faint means"
        );
    }

    #[test]
    fn a_nan_endpoint_emits_nothing() {
        let mut out = Vec::new();
        push_segment(&mut out, [f32::NAN, 0.0], [10.0, 0.0], 4.0, WHITE);
        assert!(out.is_empty());
    }

    #[test]
    fn a_nan_colour_emits_nothing() {
        let mut out = Vec::new();
        push_segment(&mut out, [0.0, 0.0], [10.0, 0.0], 4.0, [f32::NAN; 4]);
        assert!(
            out.is_empty(),
            "a NaN channel is an unrepresentable vertex, not a dim one"
        );
    }

    #[test]
    fn the_colour_is_carried_to_every_vertex() {
        let mut out = Vec::new();
        let colour = [0.25, 0.5, 0.75, 0.5];
        push_segment(&mut out, [0.0, 0.0], [10.0, 0.0], 4.0, colour);
        assert!(out.iter().all(|vertex| vertex.color == colour));
    }
}
