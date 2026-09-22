//! The pure maths: world -> screen, how bold a line is, and what colour it gets.
//!
//! Everything here is `f32` in, `f32` out, with no game and no `windows` crate, so it is proven
//! by `cargo test` on the host. That split is deliberate: a projection that is off by a factor of
//! `aspect` looks, in game, exactly like "the overlay is broken", and diagnosing it from a
//! screenshot costs a launch. Here it costs a test.
//!
//! # This projects through the game's OWN matrices
//!
//! `../er-mods-rs/crates/er-invasion-path` rebuilds the projection from a camera basis plus a
//! field of view, because what Elden Ring hands out is a camera-to-world transform and the
//! perspective terms have to be reconstructed. DARK SOULS II stores the finished pair instead --
//! a view matrix at `CameraOperator+0x10` and a projection matrix at `+0x50`, the second built by
//! `0x140001a90` -- so this module multiplies through them and never has to decide whether `fov`
//! was horizontal or vertical. One fewer convention to get backwards.
//!
//! **Row-vector, row-major, left-handed**, which is what `0x140001a90` emits:
//!
//! ```text
//! [ cot(fov/2)/aspect  0           0              0 ]
//! [ 0                  cot(fov/2)  0              0 ]
//! [ 0                  0           f/(f-n)        1 ]
//! [ 0                  0           -n*f/(f-n)     0 ]
//! ```
//!
//! so `clip = [x y z 1] * view * proj` and `clip.w` is the camera-space depth. That `m23 == 1.0`
//! and `m33 == 0.0` is not decoration: [`looks_like_a_projection`] uses the whole shape as a
//! signature to recognise the matrix in memory, which is how the camera is found at all.

// Every non-test consumer of this module is `cfg(windows)`, so on the host it is structurally
// dead. `dead_code` is still denied on the shipping target, where the windows modules are
// compiled and every item here has a caller -- so this allow cannot hide an unused item, it only
// stops the host test run from failing over cross-compiled callers.
#![cfg_attr(not(windows), allow(dead_code))]

/// A 4x4 matrix in the game's storage order: sixteen `f32`, row-major.
pub type Matrix = [f32; 16];

/// Clip-space `w` below which a point is behind (or on) the lens and cannot be projected.
///
/// Not `0.0`: a point exactly on the plane divides by zero, and a point a micrometre in front of
/// it projects to somewhere past the horizon, which draws as a line shooting off screen. The
/// game's own near plane is much larger than this; this is only the arithmetic floor.
pub const NEAR_EPSILON: f32 = 0.05;

/// `a - b`, componentwise.
#[must_use]
pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `a + b * scale`.
#[must_use]
pub fn add_scaled(a: [f32; 3], b: [f32; 3], scale: f32) -> [f32; 3] {
    [
        a[0] + b[0] * scale,
        a[1] + b[1] * scale,
        a[2] + b[2] * scale,
    ]
}

#[must_use]
pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[must_use]
pub fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// Unit vector, or `None` for a vector too short to have a direction.
#[must_use]
pub fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let len = length(a);
    if len <= f32::EPSILON {
        return None;
    }
    Some([a[0] / len, a[1] / len, a[2] / len])
}

#[must_use]
pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `a * b`, row-major, row-vector convention.
#[must_use]
pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [0.0f32; 16];
    for row in 0..4 {
        for column in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[row * 4 + k] * b[k * 4 + column];
            }
            out[row * 4 + column] = sum;
        }
    }
    out
}

/// Is every element a real number?
///
/// The camera matrices are read out of live game memory through a fault-safe reader, so "the read
/// succeeded" says nothing about whether the bytes were a matrix. A single NaN reaching the
/// projection produces NaN pixel coordinates, and a NaN vertex is not a visual glitch -- it is
/// whatever the renderer does with an unrepresentable triangle.
#[must_use]
pub fn is_finite(matrix: &Matrix) -> bool {
    matrix.iter().all(|value| value.is_finite())
}

/// Does this look like the matrix `0x140001a90` builds?
///
/// Every clause is read off that function rather than guessed, and together they are specific
/// enough to pick the projection out of a struct full of other float-shaped bytes:
///
/// | clause | why it holds |
/// | --- | --- |
/// | `m23 == 1`, `m33 == 0` | written from the constants at `0x1410ac458`/`0x1410ac45c` |
/// | the other nine off-diagonal terms are `0` | the builder writes zeroes there and nothing else |
/// | `m00 > 0`, `m11 > 0` | both are `cot(fov/2)`, positive for any `fov` in `(0, pi)` |
/// | `m00 <= m11` | `m00` is `m11 / aspect`, and a game window is at least as wide as it is tall |
/// | `0 < m22 <= 1` | `far / (far - near)`, which exceeds 1 only if `near` is negative |
/// | `m32 < 0` | `-near * far / (far - near)`, negated by the sign mask at `0x1410ac6c0` |
///
/// The tolerance is loose on purpose. This is a recogniser, not an assertion: the cost of
/// rejecting the real matrix is that the overlay never draws, and the cost of accepting a near
/// miss is nothing, because the projection is validated again by where it puts the player.
#[must_use]
pub fn looks_like_a_projection(matrix: &Matrix) -> bool {
    if !is_finite(matrix) {
        return false;
    }
    const TOLERANCE: f32 = 1.0e-3;
    let zero = |index: usize| matrix[index].abs() <= TOLERANCE;
    let zeros = [1, 2, 3, 4, 6, 7, 8, 9, 12, 13];
    if !zeros.into_iter().all(zero) {
        return false;
    }
    if (matrix[11] - 1.0).abs() > TOLERANCE || matrix[15].abs() > TOLERANCE {
        return false;
    }
    if matrix[0] <= 0.0 || matrix[5] <= 0.0 {
        return false;
    }
    // A hair of slack so a square viewport, where the two are equal, is not rejected by rounding.
    if matrix[0] > matrix[5] + TOLERANCE {
        return false;
    }
    if matrix[10] <= 0.0 || matrix[10] > 1.0 + TOLERANCE {
        return false;
    }
    matrix[14] < 0.0
}

/// A camera reduced to what drawing needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// World to camera, as the engine stores it at `CameraOperator+0x10`.
    ///
    /// Kept alongside the product below because the arrowheads need the camera's *up* axis in
    /// world space, and that cannot be recovered once the projection has been multiplied in.
    pub view: Matrix,
    /// `view * projection`, already multiplied so a frame's worth of points costs one matrix.
    pub view_projection: Matrix,
}

impl Camera {
    /// The camera's up axis, in world space.
    ///
    /// For a row-vector world-to-camera matrix the rotation's COLUMNS are the camera's axes
    /// expressed in world space -- the rows are the world's axes expressed in camera space, which
    /// is the transpose and is the easy mistake. Column 1 is up.
    ///
    /// Used only to decide which way an arrowhead opens, so that the head faces the viewer
    /// instead of collapsing to a line when the shaft points at the lens.
    #[must_use]
    pub fn up(&self) -> [f32; 3] {
        normalize([self.view[1], self.view[5], self.view[9]]).unwrap_or([0.0, 1.0, 0.0])
    }

    /// Where the camera is, in world space.
    ///
    /// The view matrix's last row is the translation in CAMERA space, not the eye: the transform
    /// is `v_camera = v_world * R + t`, so the eye -- the point that maps to the origin -- is
    /// `-t * R^T`. Reading the last row as a position is a plausible-looking answer that is
    /// wrong by the camera's own rotation.
    #[must_use]
    pub fn eye(&self) -> [f32; 3] {
        let t = [self.view[12], self.view[13], self.view[14]];
        [
            -(t[0] * self.view[0] + t[1] * self.view[1] + t[2] * self.view[2]),
            -(t[0] * self.view[4] + t[1] * self.view[5] + t[2] * self.view[6]),
            -(t[0] * self.view[8] + t[1] * self.view[9] + t[2] * self.view[10]),
        ]
    }

    /// World position -> clip space.
    #[must_use]
    pub fn to_clip(&self, world: [f32; 3]) -> [f32; 4] {
        let m = &self.view_projection;
        let [x, y, z] = world;
        [
            x * m[0] + y * m[4] + z * m[8] + m[12],
            x * m[1] + y * m[5] + z * m[9] + m[13],
            x * m[2] + y * m[6] + z * m[10] + m[14],
            x * m[3] + y * m[7] + z * m[11] + m[15],
        ]
    }

    /// Clip-space point -> pixels, or `None` when it is behind the lens.
    ///
    /// `screen` is the swap chain's back-buffer size, so the result is already in the coordinate
    /// space the overlay draws in: origin top-left, `y` down.
    #[must_use]
    pub fn clip_to_screen(&self, clip: [f32; 4], screen: [f32; 2]) -> Option<[f32; 2]> {
        // NaN is screened before the compare rather than by negating it, so the intent reads as
        // written: a NaN would pass a naive `<` and go on to produce NaN pixel coordinates.
        //
        // `< NEAR_EPSILON`, not `<=`: `project_segment` trims a crossing segment to exactly this
        // plane, and rejecting the point it just produced would drop the whole segment -- the
        // behaviour the trim exists to prevent.
        let w = clip[3];
        if !w.is_finite() || w < NEAR_EPSILON {
            return None;
        }
        let point = [
            (clip[0] / w * 0.5 + 0.5) * screen[0],
            (0.5 - clip[1] / w * 0.5) * screen[1],
        ];
        if !point[0].is_finite() || !point[1].is_finite() {
            return None;
        }
        Some(point)
    }

    /// World position -> pixels.
    #[must_use]
    pub fn project(&self, world: [f32; 3], screen: [f32; 2]) -> Option<[f32; 2]> {
        self.clip_to_screen(self.to_clip(world), screen)
    }

    /// Project a world-space segment, trimming it at the near plane.
    ///
    /// A polyline drawn by projecting endpoints independently and skipping the ones that fail is
    /// not merely incomplete -- when one end is behind the camera the surviving end connects to
    /// whatever the next visible point is, so the line visibly jumps across the screen as the
    /// player turns. Trimming instead keeps it ending where the world actually leaves view.
    #[must_use]
    pub fn project_segment(
        &self,
        from: [f32; 3],
        to: [f32; 3],
        screen: [f32; 2],
    ) -> Option<([f32; 2], [f32; 2])> {
        let (mut a, mut b) = (self.to_clip(from), self.to_clip(to));
        let (a_in, b_in) = (a[3] > NEAR_EPSILON, b[3] > NEAR_EPSILON);
        if !a_in && !b_in {
            return None;
        }
        if !a_in || !b_in {
            // Parameter along a->b where `w` crosses the near plane. The denominator cannot be
            // zero here: one endpoint is strictly above the plane and the other is not.
            let t = (NEAR_EPSILON - a[3]) / (b[3] - a[3]);
            let mut clipped = [0.0f32; 4];
            for index in 0..4 {
                clipped[index] = a[index] + (b[index] - a[index]) * t;
            }
            clipped[3] = NEAR_EPSILON;
            if a_in { b = clipped } else { a = clipped }
        }
        Some((
            self.clip_to_screen(a, screen)?,
            self.clip_to_screen(b, screen)?,
        ))
    }

    /// Is `point` somewhere a viewport of `screen` pixels could plausibly show it?
    ///
    /// Used only to confirm a candidate camera is the live one, and generous on purpose: a
    /// player at the very edge of view is still a pass, a matrix that is not a camera puts them
    /// thousands of pixels away or behind the lens and fails on any threshold at all.
    #[must_use]
    pub fn plausibly_on_screen(&self, world: [f32; 3], screen: [f32; 2]) -> bool {
        let Some(point) = self.project(world, screen) else {
            return false;
        };
        /// How far outside the viewport still counts, as a fraction of its size.
        const SLACK: f32 = 1.0;
        let (slack_x, slack_y) = (screen[0] * SLACK, screen[1] * SLACK);
        point[0] >= -slack_x
            && point[0] <= screen[0] + slack_x
            && point[1] >= -slack_y
            && point[1] <= screen[1] + slack_y
    }
}

/// Walk a polyline and return a point every `spacing` metres along it, start and end included.
///
/// Placing a marker at every route vertex instead would clump them: the engine emits vertices
/// where the mesh geometry demands a turn, so a stretch of open ground is a handful of points
/// tens of metres apart while a doorway is half a dozen inside two metres. Even spacing is what
/// reads as a trail rather than as debris.
///
/// The end point is always included even when it does not land on the spacing grid, because the
/// last marker is the one that says "here", and dropping it because the length was not a multiple
/// of the spacing is the one omission a player would notice.
///
/// Returns at most `max` points; a line longer than `max * spacing` is truncated rather than
/// thinned, so the markers nearest you -- the ones you are about to walk past -- stay put.
pub fn resample(points: &[[f32; 3]], spacing: f32, max: usize) -> Vec<[f32; 3]> {
    if points.len() < 2 || !spacing.is_finite() || spacing <= 0.0 || max == 0 {
        return points.iter().copied().take(max).collect();
    }
    let mut out = vec![points[0]];
    // Distance already walked past the last emitted marker.
    let mut carried = 0.0f32;
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let segment = sub(to, from);
        let span = length(segment);
        if !span.is_finite() || span <= f32::EPSILON {
            continue;
        }
        let Some(direction) = normalize(segment) else {
            continue;
        };
        let mut travelled = spacing - carried;
        while travelled <= span {
            if out.len() >= max {
                return out;
            }
            out.push(add_scaled(from, direction, travelled));
            travelled += spacing;
        }
        carried = span - (travelled - spacing);
    }
    let last = points[points.len() - 1];
    // Skipped only when a marker is already on the destination, which happens when the length is
    // a whole multiple of the spacing. The threshold is a few centimetres rather than a fraction
    // of the spacing: half a spacing looks like a sensible "close enough" and quietly deletes a
    // real destination marker that merely fell one metre short of the grid.
    const COINCIDENT_METERS: f32 = 0.05;
    let duplicate = out
        .last()
        .is_some_and(|previous| length(sub(last, *previous)) <= COINCIDENT_METERS);
    if !duplicate && out.len() < max {
        out.push(last);
    }
    out
}

/// How close a target has to be for its line to be drawn at full strength, in metres.
pub const DEFAULT_BOLD_AT_METERS: f32 = 20.0;
/// The distance at which a line has faded to [`MIN_ALPHA`], in metres.
pub const DEFAULT_FAINT_AT_METERS: f32 = 150.0;
/// A fully faded line is still visible. Fading to nothing would make a distant player and a
/// player the mod cannot place look identical -- both would show nothing at all.
pub const MIN_ALPHA: f32 = 0.22;
/// Stroke width of a fully faded line, in pixels.
pub const MIN_STROKE_PX: f32 = 1.6;
/// Stroke width of a line at [`DEFAULT_BOLD_AT_METERS`] or nearer, in pixels.
pub const MAX_STROKE_PX: f32 = 5.5;

/// `1.0` for a target at or inside `bold_at`, falling to `0.0` at `faint_at`.
///
/// Linear in distance rather than in squared distance: the eye reads a line's weight as a proxy
/// for "how far do I still have to run", and that is a distance, not its square.
#[must_use]
pub fn boldness(distance_meters: f32, bold_at: f32, faint_at: f32) -> f32 {
    if !distance_meters.is_finite() {
        return 0.0;
    }
    if distance_meters <= bold_at {
        return 1.0;
    }
    // A config that inverts or collapses the two thresholds must not produce NaN; treat anything
    // past the near threshold as fully faded.
    if !faint_at.is_finite() || !bold_at.is_finite() || faint_at <= bold_at {
        return 0.0;
    }
    (1.0 - (distance_meters - bold_at) / (faint_at - bold_at)).clamp(0.0, 1.0)
}

/// Stroke width in pixels for a given [`boldness`].
#[must_use]
pub fn stroke_px(boldness: f32) -> f32 {
    MIN_STROKE_PX + (MAX_STROKE_PX - MIN_STROKE_PX) * boldness.clamp(0.0, 1.0)
}

/// Alpha for a given [`boldness`], never fully transparent.
#[must_use]
pub fn alpha(boldness: f32) -> f32 {
    MIN_ALPHA + (1.0 - MIN_ALPHA) * boldness.clamp(0.0, 1.0)
}

/// Distinct RGB for line `index`.
///
/// Hues are spread by the golden angle rather than by `index / total`, so a line keeps its colour
/// when another player joins or dies. Dividing the circle by `total` would recolour every line
/// the instant the roster changed, and the player's mental "the red one is the invader" would
/// break mid-fight -- which is precisely when it is being relied on.
#[must_use]
pub fn path_color(index: usize) -> [f32; 3] {
    /// 360 / phi, the angle that fills the hue circle most evenly for any prefix length.
    const GOLDEN_ANGLE_DEGREES: f32 = 137.507_76;
    /// The first hue. 20 degrees is orange -- warm, and clear of the game's own pale UI.
    const FIRST_HUE_DEGREES: f32 = 20.0;
    let hue = (FIRST_HUE_DEGREES + GOLDEN_ANGLE_DEGREES * index as f32).rem_euclid(360.0);
    hsv_to_rgb(hue, 0.85, 1.0)
}

/// HSV (`h` in degrees, `s`/`v` in `0..=1`) -> RGB triple.
#[must_use]
pub fn hsv_to_rgb(hue_degrees: f32, saturation: f32, value: f32) -> [f32; 3] {
    let hue = hue_degrees.rem_euclid(360.0) / 60.0;
    let chroma = value * saturation;
    let second = chroma * (1.0 - ((hue % 2.0) - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let base = value - chroma;
    [r + base, g + base, b + base]
}

/// The world-space skeleton of the "no route" arrow: a shaft plus two barbs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrow {
    pub tail: [f32; 3],
    pub tip: [f32; 3],
    pub left_barb: [f32; 3],
    pub right_barb: [f32; 3],
}

/// Build the arrow that leaves `origin` pointing at `target`.
///
/// The direction is the full 3D direction, not its horizontal projection: a player directly above
/// you on a walkway and a player directly ahead of you are the difference between climbing and
/// running, and flattening the arrow would tell you they are the same. In DARK SOULS II that is
/// not a corner case -- it is Majula, Earthen Peak and most of Drangleic Castle.
///
/// Returns `None` when the two positions coincide and there is no direction to point.
#[must_use]
pub fn arrow(
    origin: [f32; 3],
    target: [f32; 3],
    length_meters: f32,
    up: [f32; 3],
) -> Option<Arrow> {
    /// Length of the head as a fraction of the shaft.
    const BARB_FRACTION: f32 = 0.28;
    /// How far each barb opens sideways, as a fraction of the head's length.
    const BARB_SPREAD: f32 = 0.6;
    let direction = normalize(sub(target, origin))?;
    let tip = add_scaled(origin, direction, length_meters);
    // A barb axis perpendicular to the shaft. `up` is the camera's up vector, so the head opens
    // towards the viewer and stays a visible arrowhead instead of collapsing to a line when the
    // shaft points straight at the camera.
    let side = normalize(cross(direction, up)).unwrap_or([0.0, 0.0, 0.0]);
    let back = add_scaled(tip, direction, -length_meters * BARB_FRACTION);
    let spread = length_meters * BARB_FRACTION * BARB_SPREAD;
    Some(Arrow {
        tail: origin,
        tip,
        left_barb: add_scaled(back, side, spread),
        right_barb: add_scaled(back, side, -spread),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The projection `0x140001a90` builds for a 90-degree vertical fov at 16:9, near 0.1,
    /// far 1000 -- computed here by the same arithmetic that function performs, so the test
    /// exercises the convention rather than a transcribed number.
    fn projection() -> Matrix {
        let (fov_y, aspect, near, far) = (std::f32::consts::FRAC_PI_2, 16.0 / 9.0, 0.1f32, 1000.0);
        let cot = (fov_y * 0.5).cos() / (fov_y * 0.5).sin();
        let depth = far / (far - near);
        let mut m = [0.0f32; 16];
        m[0] = cot / aspect;
        m[5] = cot;
        m[10] = depth;
        m[11] = 1.0;
        m[14] = -depth * near;
        m
    }

    fn identity() -> Matrix {
        let mut out = [0.0f32; 16];
        out[0] = 1.0;
        out[5] = 1.0;
        out[10] = 1.0;
        out[15] = 1.0;
        out
    }

    /// A camera at the origin looking down `+z` with `+y` up: the view matrix is the identity.
    fn camera() -> Camera {
        Camera {
            view: identity(),
            view_projection: multiply(&identity(), &projection()),
        }
    }

    const SCREEN: [f32; 2] = [1920.0, 1080.0];

    #[test]
    fn a_point_straight_ahead_lands_dead_centre() {
        let point = camera()
            .project([0.0, 0.0, 10.0], SCREEN)
            .expect("in front");
        assert!((point[0] - 960.0).abs() < 0.01, "x was {}", point[0]);
        assert!((point[1] - 540.0).abs() < 0.01, "y was {}", point[1]);
    }

    #[test]
    fn a_point_behind_the_camera_does_not_project() {
        assert!(camera().project([0.0, 0.0, -10.0], SCREEN).is_none());
        assert!(camera().project([0.0, 0.0, 0.0], SCREEN).is_none());
    }

    #[test]
    fn vertical_fov_reaches_the_screen_edge_before_horizontal() {
        // At 90 degrees vertical, y == z puts a point exactly on the top edge.
        let top = camera()
            .project([0.0, 10.0, 10.0], SCREEN)
            .expect("in front");
        assert!(top[1].abs() < 0.01, "top edge y was {}", top[1]);
        // The same offset horizontally must NOT reach the edge, because only X carries the
        // aspect divide. Applying it to the wrong axis stretches the overlay by 1.78 at 16:9 and
        // looks, in game, like the lines are simply in the wrong place.
        let side = camera()
            .project([10.0, 0.0, 10.0], SCREEN)
            .expect("in front");
        let expected = 960.0 + 960.0 / (16.0 / 9.0);
        assert!(
            (side[0] - expected).abs() < 0.01,
            "x was {}; aspect is being applied to the wrong axis",
            side[0]
        );
    }

    #[test]
    fn a_segment_crossing_the_near_plane_is_trimmed_not_dropped() {
        let (near, far) = camera()
            .project_segment([0.0, 0.0, -5.0], [0.0, 0.0, 20.0], SCREEN)
            .expect("partially visible");
        // Both survive, and the trimmed end sits at screen centre because the segment runs
        // straight down the view axis.
        assert!((near[0] - 960.0).abs() < 0.5, "near x was {}", near[0]);
        assert!((far[0] - 960.0).abs() < 0.5, "far x was {}", far[0]);
    }

    #[test]
    fn a_segment_entirely_behind_the_camera_is_dropped() {
        assert!(
            camera()
                .project_segment([0.0, 0.0, -5.0], [0.0, 0.0, -20.0], SCREEN)
                .is_none()
        );
    }

    #[test]
    fn the_real_projection_shape_is_recognised() {
        assert!(looks_like_a_projection(&projection()));
    }

    #[test]
    fn an_identity_matrix_is_not_mistaken_for_a_projection() {
        assert!(
            !looks_like_a_projection(&identity()),
            "identity passed the signature; every freshly constructed camera slot is one"
        );
    }

    #[test]
    fn the_up_axis_comes_from_the_column_and_not_the_row() {
        // A camera rolled 90 degrees about its own forward axis. As a world-to-camera matrix its
        // rotation is the TRANSPOSE of the camera's world basis, so reading a row instead of a
        // column gives the opposite roll -- a sign error that looks like working code.
        let mut view = identity();
        view[0] = 0.0;
        view[1] = -1.0;
        view[4] = 1.0;
        view[5] = 0.0;
        let camera = Camera {
            view,
            view_projection: multiply(&view, &projection()),
        };
        let up = camera.up();
        assert!(
            (up[0] - -1.0).abs() < 1.0e-4 && up[1].abs() < 1.0e-4,
            "up was {up:?}; the row was read instead of the column"
        );
    }

    #[test]
    fn the_eye_is_recovered_through_the_rotation() {
        // A camera at (10, 5, -3) with no rotation: the view matrix's translation row is the
        // negated eye, and with identity rotation the two agree.
        let mut view = identity();
        view[12] = -10.0;
        view[13] = -5.0;
        view[14] = 3.0;
        let camera = Camera {
            view,
            view_projection: multiply(&view, &projection()),
        };
        let eye = camera.eye();
        assert!((eye[0] - 10.0).abs() < 1.0e-4, "eye was {eye:?}");
        assert!((eye[1] - 5.0).abs() < 1.0e-4, "eye was {eye:?}");
        assert!((eye[2] - -3.0).abs() < 1.0e-4, "eye was {eye:?}");
    }

    #[test]
    fn a_view_matrix_is_not_mistaken_for_a_projection() {
        // A plausible world-to-camera transform: orthonormal basis, translation in the last row,
        // and `m33 == 1`. It shares the projection's zeros in the top-right and would pass a
        // check that only looked at those.
        let mut view = [0.0f32; 16];
        view[0] = 1.0;
        view[5] = 1.0;
        view[10] = 1.0;
        view[12] = -12.5;
        view[13] = -3.0;
        view[14] = 40.0;
        view[15] = 1.0;
        assert!(!looks_like_a_projection(&view));
    }

    #[test]
    fn a_matrix_full_of_nans_is_rejected() {
        assert!(!looks_like_a_projection(&[f32::NAN; 16]));
        let mut one_nan = projection();
        one_nan[5] = f32::NAN;
        assert!(!looks_like_a_projection(&one_nan));
    }

    #[test]
    fn resampling_keeps_the_destination_even_off_the_grid() {
        let line = [[0.0, 0.0, 0.0], [0.0, 0.0, 5.0]];
        let points = resample(&line, 2.0, 64);
        let last = points.last().copied().expect("non-empty");
        assert!(
            (last[2] - 5.0).abs() < 1.0e-4,
            "the destination marker was dropped: {last:?}"
        );
    }

    #[test]
    fn resampling_is_bounded_by_max() {
        let line = [[0.0, 0.0, 0.0], [0.0, 0.0, 1000.0]];
        assert_eq!(resample(&line, 1.0, 8).len(), 8);
    }

    #[test]
    fn an_arrow_points_at_the_target_in_three_dimensions() {
        // A target straight up. A horizontal-only arrow would have no direction at all here and
        // is exactly the case DARK SOULS II's vertical maps make common.
        let built = arrow([0.0, 0.0, 0.0], [0.0, 40.0, 0.0], 3.0, [0.0, 1.0, 0.0])
            .expect("a direction exists");
        assert!(
            (built.tip[1] - 3.0).abs() < 1.0e-4,
            "tip was {:?}",
            built.tip
        );
    }

    #[test]
    fn an_arrow_to_where_you_already_are_has_no_direction() {
        assert!(arrow([1.0, 2.0, 3.0], [1.0, 2.0, 3.0], 3.0, [0.0, 1.0, 0.0]).is_none());
    }

    #[test]
    fn a_nearer_line_is_bolder_than_a_distant_one() {
        assert!(boldness(5.0, 20.0, 150.0) > boldness(140.0, 20.0, 150.0));
        assert!(alpha(boldness(140.0, 20.0, 150.0)) >= MIN_ALPHA);
    }

    #[test]
    fn an_inverted_ramp_does_not_produce_nan() {
        // A config with `faint_at` below `bold_at` is a user error, not a crash.
        assert!(boldness(50.0, 150.0, 20.0).is_finite());
        assert!(boldness(f32::NAN, 20.0, 150.0).is_finite());
    }
}
