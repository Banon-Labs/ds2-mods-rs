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

/// How tall a DARK SOULS II character is, from origin to head, in metres.
///
/// The character's own position is at their FEET, so this is what turns it into the point the
/// camera is actually pointed at. Used by [`Camera::head_offset_from_centre`] and by the capture's
/// scale test, which are the two places that need to know where a character LOOKS like they are
/// rather than where the game says they stand.
pub const HEAD_METERS: f32 = 1.8;

/// How far the head may land from the middle of the frame and still count as framed, as a
/// fraction of the half-extent.
///
/// **This was `0.30` and that was far too loose.** Thirty percent of a half-extent is +/-384
/// pixels on a 2560-wide frame: a camera that put the character most of the way to the edge
/// passed, and the overlay drew a confident arrow from a base that was visibly not on the player.
/// The report that found it: "I've seen the base off the player more times than I've seen it on
/// the player. The player's head is always in the center. The base of the arrow is not."
///
/// Five percent is +/-64 pixels horizontally on that frame -- tight enough that a camera which is
/// merely CLOSE is refused, and loose enough to survive the things that legitimately shift a
/// third-person character off centre: a lock-on, a wall pushing the camera in, and the one-frame
/// lag between the matrix the renderer uploaded and the position the overlay read.
///
/// It is deliberately not tighter than that. A tolerance of a fraction of a pixel would be exact
/// about the wrong thing -- it would reject the frame lag, which is real and harmless, and the
/// overlay would flicker off during every turn.
pub const FRAMING_TOLERANCE: f32 = 0.05;

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

/// Build a world-to-camera matrix from a camera's own rotation and position.
///
/// # Why this exists rather than reading one
///
/// The engine stores a finished view matrix somewhere, and a live scan of every sixteen-byte
/// boundary in the camera object failed to find it in either convention. What the engine
/// demonstrably DOES store is the pair it builds that matrix from: `0x140493245` writes two
/// `float4` to `this+0xd0` and `this+0xe0` immediately before handing them to `0x140002680`,
/// whose result is inverted by `0x140002380` and stored as the view matrix.
///
/// So this performs the same two steps. Inverting an orthonormal rotation is its transpose, and
/// the translation that goes with it is `-position` measured along the camera's own axes -- which
/// is what makes this an inverse rather than a rotation with the eye pasted into it, the mistake
/// that puts the camera in the right place facing the wrong way.
///
/// `quaternion` is taken in `(x, y, z, w)` order. Nothing in the image says which order this
/// field uses, and the wrong one is not a detectable error -- it is a perfectly valid rotation
/// that is simply not the camera's. So the caller tries both and lets the oracles decide, which
/// is why [`WXYZ`] exists.
#[must_use]
pub fn view_from_pose(quaternion: [f32; 4], position: [f32; 3]) -> Option<Matrix> {
    let [x, y, z, w] = quaternion;
    let norm = (x * x + y * y + z * z + w * w).sqrt();
    // A quaternion is a unit quaternion or it is not a quaternion. A field that happens to hold
    // four floats summing to something else is a field this has misidentified, and saying so is
    // cheaper than projecting through it.
    if !norm.is_finite() || (norm - 1.0).abs() > 0.05 {
        return None;
    }
    let (x, y, z, w) = (x / norm, y / norm, z / norm, w / norm);
    // Rows of the camera-to-world rotation: the camera's right, up and forward in world space.
    let right = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
    ];
    let up = [
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
    ];
    let forward = [
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
    ];
    // The inverse: the transpose in the rotation block, and the eye projected onto each axis,
    // negated, in the translation row.
    let mut out = [0.0f32; 16];
    out[0] = right[0];
    out[4] = right[1];
    out[8] = right[2];
    out[1] = up[0];
    out[5] = up[1];
    out[9] = up[2];
    out[2] = forward[0];
    out[6] = forward[1];
    out[10] = forward[2];
    out[12] = -dot(position, right);
    out[13] = -dot(position, up);
    out[14] = -dot(position, forward);
    out[15] = 1.0;
    is_finite(&out).then_some(out)
}

/// Re-order a `(w, x, y, z)` quaternion into the `(x, y, z, w)` [`view_from_pose`] expects.
///
/// Both orders are in wide use and a struct field does not say which it holds. Reading one as the
/// other is not a detectable error -- the result is a unit quaternion and a valid rotation, just
/// the wrong one -- so this exists to be tried alongside the identity reading rather than chosen.
#[must_use]
pub const fn wxyz(raw: [f32; 4]) -> [f32; 4] {
    [raw[1], raw[2], raw[3], raw[0]]
}

/// The transpose of `matrix`.
///
/// Needed because a 4x4 in memory does not say which convention wrote it. The projection's shape
/// pins ITS convention -- `m23 == 1` is in a different slot under the other one -- but a view
/// matrix is sixteen floats with no such landmark, and the same bytes are a valid
/// world-to-camera transform read either way. So both are tried and the world decides.
#[must_use]
pub fn transpose(matrix: &Matrix) -> Matrix {
    let mut out = [0.0f32; 16];
    for row in 0..4 {
        for column in 0..4 {
            out[column * 4 + row] = matrix[row * 4 + column];
        }
    }
    out
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

/// The aspect ratio this projection was built for, or `None` if it does not have one.
///
/// `0x140001a90` writes `m00 = cot(fov/2)/aspect` and `m11 = cot(fov/2)`, so the ratio of the two
/// IS the aspect and nothing else in the matrix is needed to recover it.
///
/// This is the check that tells the camera being rendered from a camera that merely exists. A
/// game this era renders several projections per frame -- shadow maps, reflections, cube faces --
/// and those are square or near it. Comparing against the back buffer's own ratio rejects them
/// without knowing anything about what they are for, and unlike a screen-position threshold it is
/// not a number anybody tuned.
#[must_use]
pub fn projection_aspect(matrix: &Matrix) -> Option<f32> {
    if matrix[0].abs() <= f32::EPSILON {
        return None;
    }
    let aspect = matrix[5] / matrix[0];
    (aspect.is_finite() && aspect > 0.0).then_some(aspect)
}

/// Does this projection's aspect ratio match the viewport it would be drawn into?
///
/// The tolerance is wide enough for a letterboxed or slightly-off back buffer and far too narrow
/// for the square projections a shadow or reflection pass uses.
#[must_use]
pub fn aspect_matches(matrix: &Matrix, screen: [f32; 2]) -> bool {
    const TOLERANCE: f32 = 0.12;
    if screen[1] <= 0.0 {
        return false;
    }
    let wanted = screen[0] / screen[1];
    projection_aspect(matrix).is_some_and(|aspect| ((aspect - wanted) / wanted).abs() <= TOLERANCE)
}

/// Trim a screen-space segment to the viewport, or `None` if none of it is inside.
///
/// # Why a near-plane trim is not enough on its own
///
/// `Camera::project_segment` trims a segment that crosses the camera plane to `w = NEAR_EPSILON`,
/// which is mathematically right and visually catastrophic: dividing by a number that small
/// throws the trimmed end thousands of pixels away, and the line drawn to it sweeps across the
/// whole frame. A live screenshot has an arrowhead whose two barbs reach the top-left corner from
/// a tip in the middle of the screen, drawn over the sky, for exactly this reason.
///
/// The direction of that line is correct; only its far end is nonsense. So the fix is not to drop
/// the segment -- a line leaving the frame towards a player behind you is the information wanted
/// -- but to end it where it leaves the viewport. Liang-Barsky, on the four edges.
///
/// The `1.0` slack keeps a line that runs exactly along an edge from being trimmed to nothing by
/// rounding.
#[must_use]
pub fn clip_to_viewport(
    from: [f32; 2],
    to: [f32; 2],
    screen: [f32; 2],
) -> Option<([f32; 2], [f32; 2])> {
    const SLACK: f32 = 1.0;
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let (mut enter, mut leave) = (0.0f32, 1.0f32);
    // Each edge as `direction * t <= distance`: negative direction is entering the half-plane,
    // positive is leaving it, and zero is parallel -- which only fails if it starts outside.
    for (direction, distance) in [
        (-dx, from[0] + SLACK),
        (dx, screen[0] + SLACK - from[0]),
        (-dy, from[1] + SLACK),
        (dy, screen[1] + SLACK - from[1]),
    ] {
        if direction == 0.0 {
            if distance < 0.0 {
                return None;
            }
            continue;
        }
        let t = distance / direction;
        if direction < 0.0 {
            if t > leave {
                return None;
            }
            enter = enter.max(t);
        } else {
            if t < enter {
                return None;
            }
            leave = leave.min(t);
        }
    }
    if !enter.is_finite() || !leave.is_finite() || enter > leave {
        return None;
    }
    Some((
        [from[0] + dx * enter, from[1] + dy * enter],
        [from[0] + dx * leave, from[1] + dy * leave],
    ))
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

    /// Where the camera is looking, in world space.
    ///
    /// Column 2 of the rotation, by exactly the argument [`Camera::up`] spells out: for a
    /// row-vector world-to-camera matrix the COLUMNS are the camera's axes in world space.
    #[must_use]
    pub fn forward(&self) -> [f32; 3] {
        // FROM THE COMBINED MATRIX, NOT THE VIEW HALF. The camera caught out of the renderer's
        // own upload has no view half at all -- `crate::capture` has only the product and stores
        // an identity in its place -- so reading `view` there returns the identity's third axis
        // and every heading is exactly zero. Live, that is what happened: the harness's probe
        // held all six pad axes in turn and reported `0.00 -> 0.00` for every one of them, which
        // reads as "the stick does nothing" and is really "the instrument is stuck".
        //
        // The product has the answer. Row-vector, with the projection's `m23 == 1`, the clip
        // `w` of a world point is its depth along the camera's own forward axis:
        //
        //     w = x*vp[3] + y*vp[7] + z*vp[11] + vp[15]
        //
        // so `(vp[3], vp[7], vp[11])` IS that axis in world space, whether or not the view half
        // was ever seen. It agrees with the view matrix for a camera assembled from both, since
        // that column of the product is the view's forward column scaled by one.
        let m = &self.view_projection;
        normalize([m[3], m[7], m[11]])
            .or_else(|| normalize([self.view[2], self.view[6], self.view[10]]))
            .unwrap_or([0.0, 0.0, 1.0])
    }

    /// The camera's heading, in degrees, in `-180.0..=180.0`.
    ///
    /// **Only differences between two readings of this are meaningful.** Where zero points
    /// depends on the engine's world axes, which this repo has not pinned down, and nothing here
    /// needs it to: the harness that consumes this drives a stick until the DIFFERENCE is what
    /// was asked for. Pitch is deliberately not folded in -- a heading that changed when the
    /// camera tilted would make a yaw controller chase its own tail.
    ///
    /// Degenerate straight up or straight down (where the forward vector has no horizontal
    /// component at all) is the one case with no answer, and `atan2(0, 0)` returning `0` there
    /// is as good as any -- a camera pointed at the sky has no heading to report.
    #[must_use]
    pub fn yaw_degrees(&self) -> f32 {
        let forward = self.forward();
        forward[0].atan2(forward[2]).to_degrees()
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
        clip_to_viewport(
            self.clip_to_screen(a, screen)?,
            self.clip_to_screen(b, screen)?,
            screen,
        )
    }

    /// Does this camera frame the character the way DARK SOULS II's camera frames a character?
    ///
    /// The strongest oracle available, and the last one added, because it is the only one that
    /// uses a fact about THIS GAME rather than about projection in general: the camera follows
    /// the player, so the player sits near the middle of the frame. Not exactly centred -- a
    /// lock-on or a wall shoves them off it -- but never in a corner.
    ///
    /// It is what separates a camera that is merely arithmetically valid from the one being
    /// rendered. A pose-built camera with the quaternion components in the wrong order is a
    /// perfectly good rotation about the wrong axis: it keeps the eye near the player, it keeps
    /// the world the right way up, and it puts the character at `376,213` on a `2560x1441`
    /// screen. Every other test here passes it.
    ///
    /// Used ONLY when choosing a camera, never per frame, so a cutscene that genuinely pushes the
    /// character to the edge cannot switch the overlay off mid-fight.
    #[must_use]
    pub fn frames_the_character(&self, world: [f32; 3], screen: [f32; 2]) -> bool {
        self.head_offset_from_centre(world, screen)
            .is_some_and(|(x, y)| x <= FRAMING_TOLERANCE && y <= FRAMING_TOLERANCE)
    }

    /// How far the character's HEAD lands from the middle of the frame, as a fraction of the
    /// half-extent in each axis. `None` when it cannot be projected at all.
    ///
    /// # Why the head and not the position the game gives you
    ///
    /// `world` is the character's origin, which is at their FEET. DARK SOULS II's camera aims at
    /// the upper body, so a perfectly correct camera puts the feet a character's height BELOW
    /// centre -- on a 1440-pixel frame that is a couple of hundred pixels, and measuring it
    /// against the centre makes a right answer look like a large error.
    ///
    /// The head is what the eye uses. The player reported it as "the player's head is always in
    /// the centre, the base of the arrow is not", and both halves of that are true at once: the
    /// head IS centred, and the arrow's base is at the feet where it belongs. Measuring the head
    /// is therefore the only way this test can be tightened without rejecting every real camera.
    #[must_use]
    pub fn head_offset_from_centre(&self, world: [f32; 3], screen: [f32; 2]) -> Option<(f32, f32)> {
        let head = [world[0], world[1] + HEAD_METERS, world[2]];
        let point = self.project(head, screen)?;
        Some((
            (point[0] - screen[0] * 0.5).abs() / (screen[0] * 0.5),
            (point[1] - screen[1] * 0.5).abs() / (screen[1] * 0.5),
        ))
    }

    /// Is `point` somewhere a viewport of `screen` pixels could plausibly show it?
    ///
    /// Generous on purpose: a player at the very edge of view is still a pass. On its own this
    /// is a WEAK test -- see [`Self::agrees_with_the_world`], which is the one that matters.
    #[must_use]
    pub fn plausibly_on_screen(&self, world: [f32; 3], screen: [f32; 2]) -> bool {
        let Some(point) = self.project(world, screen) else {
            return false;
        };
        /// How far outside the viewport still counts, as a fraction of its size.
        const SLACK: f32 = 0.5;
        let (slack_x, slack_y) = (screen[0] * SLACK, screen[1] * SLACK);
        point[0] >= -slack_x
            && point[0] <= screen[0] + slack_x
            && point[1] >= -slack_y
            && point[1] <= screen[1] + slack_y
    }

    /// Does this camera agree with the world about which way is up?
    ///
    /// # Why the on-screen test alone was not enough
    ///
    /// It accepted a matrix that was wrong, and the way it was wrong is instructive. A live run
    /// logged an arrow whose world delta was `(14.0, 4.1, 13.4)` -- almost horizontal -- and
    /// which projected to a near-vertical line up the screen. The local player still landed at
    /// screen centre, because in a third-person game the player is ALWAYS near screen centre;
    /// a matrix has to be badly wrong before that stops being true, and "is the player roughly
    /// where the player always is" is therefore almost no evidence at all.
    ///
    /// This is evidence. World `+Y` is up in this engine -- the navigation code treats component
    /// 1 as the height -- so a point directly above another must project ABOVE it, and by a
    /// sane number of pixels rather than a thousand. A transposed view matrix, a swapped axis
    /// pair or a column-vector convention all fail it; the correct matrix passes it from any
    /// camera angle short of looking straight down.
    ///
    /// `from` is the local player, whose position is read independently of anything here.
    /// How many pixels UP the screen a point ten metres above `from` projects.
    ///
    /// Negative means the camera thinks up is down; zero means it moved the point sideways or
    /// not at all, which is what a transposed or axis-swapped matrix does. Split out from
    /// [`Self::agrees_with_the_world`] so a candidate that FAILS can say by how much -- a
    /// rejection with no number is the shape of diagnostic that costs a launch.
    #[must_use]
    pub fn rise_px(&self, from: [f32; 3], screen: [f32; 2]) -> f32 {
        self.rise_along(from, 1, screen)
    }

    /// [`Self::rise_px`] along an arbitrary world axis, for finding out which one is up.
    ///
    /// The camera cannot say which component of a position is the height -- only the world can,
    /// and only by being asked. A live table of all three settles in one run what reasoning from
    /// the navigation code's field order did not.
    #[must_use]
    pub fn rise_along(&self, from: [f32; 3], axis: usize, screen: [f32; 2]) -> f32 {
        let mut probe = from;
        if let Some(component) = probe.get_mut(axis) {
            *component += UP_PROBE_METERS;
        }
        match (self.project(from, screen), self.project(probe, screen)) {
            (Some(here), Some(there)) => here[1] - there[1],
            _ => 0.0,
        }
    }

    #[must_use]
    pub fn agrees_with_the_world(&self, from: [f32; 3], screen: [f32; 2]) -> bool {
        /// The probe must move the projected point at least this many pixels. A matrix that
        /// moves it by nothing is not passing the test, it is abstaining from it.
        const MIN_RISE_PX: f32 = 4.0;
        if self.project(from, screen).is_none() {
            return false;
        }
        let rise = self.rise_px(from, screen);
        // Ten metres up must not move the point further than a screenful and a half either:
        // that is a scale error even when the direction is right.
        rise >= MIN_RISE_PX && rise <= screen[1] * 1.5
    }
}

/// How far up [`Camera::rise_px`] probes, in metres. Well above a character, well below a map.
const UP_PROBE_METERS: f32 = 10.0;

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

    /// A camera at the origin yawed `degrees` about `+y`.
    ///
    /// Written as the ROTATION of the world into camera space -- the transpose of the camera's
    /// own rotation -- because that is what a world-to-camera matrix holds, and building it the
    /// other way round would make these tests agree with a wrong `forward`.
    fn yawed(degrees: f32) -> Camera {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let mut view = identity();
        // Columns are the camera's axes in world space: right = (cos, 0, -sin),
        // forward = (sin, 0, cos).
        view[0] = cos;
        view[2] = sin;
        view[8] = -sin;
        view[10] = cos;
        Camera {
            view,
            view_projection: multiply(&view, &projection()),
        }
    }

    #[test]
    fn an_unrotated_camera_looks_down_positive_z() {
        let forward = camera().forward();
        assert!((forward[2] - 1.0).abs() < 1e-5, "forward was {forward:?}");
        assert!(camera().yaw_degrees().abs() < 1e-3);
    }

    #[test]
    fn yaw_tracks_the_rotation_it_was_built_from() {
        for degrees in [-170.0f32, -90.0, -1.0, 0.0, 30.0, 90.0, 179.0] {
            let reported = yawed(degrees).yaw_degrees();
            assert!(
                (reported - degrees).abs() < 1e-2,
                "built {degrees} degrees, read back {reported}"
            );
        }
    }

    #[test]
    fn yaw_ignores_pitch() {
        // The property the turn controller depends on: tilting the camera up or down must not
        // move the heading, or a yaw loop would chase its own tail every time the camera
        // bobbed.
        let flat = yawed(40.0).yaw_degrees();
        let mut pitched = yawed(40.0);
        // Tilt: mix some of the camera's own up axis into its forward column, then renormalise
        // happens inside `forward()`.
        pitched.view[6] = 0.5;
        let tilted = pitched.yaw_degrees();
        assert!(
            (tilted - flat).abs() < 1e-2,
            "pitching moved the heading from {flat} to {tilted}"
        );
    }

    #[test]
    fn a_camera_pointed_straight_up_reports_a_heading_rather_than_a_nan() {
        // No horizontal component at all: there is no heading, and the answer must still be a
        // number a controller can subtract.
        let straight_up = Camera {
            view: {
                let mut view = identity();
                view[2] = 0.0;
                view[6] = 1.0;
                view[10] = 0.0;
                view
            },
            view_projection: identity(),
        };
        assert!(straight_up.yaw_degrees().is_finite());
    }

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

#[cfg(test)]
mod viewport_clipping {
    use super::clip_to_viewport;

    const SCREEN: [f32; 2] = [1000.0, 800.0];

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1.5 && (a[1] - b[1]).abs() < 1.5
    }

    #[test]
    fn a_segment_wholly_inside_is_untouched() {
        let clipped = clip_to_viewport([100.0, 100.0], [900.0, 700.0], SCREEN).unwrap();
        assert!(near(clipped.0, [100.0, 100.0]) && near(clipped.1, [900.0, 700.0]));
    }

    #[test]
    fn the_near_plane_blowup_is_trimmed_to_the_edge() {
        // The live failure: a barb whose far end was thrown to the corner by a near-zero `w`.
        // The line must survive -- its direction is right -- and must end at the viewport.
        let clipped = clip_to_viewport([500.0, 400.0], [-40_000.0, -32_000.0], SCREEN).unwrap();
        assert!(near(clipped.0, [500.0, 400.0]));
        assert!(clipped.1[0] >= -1.5 && clipped.1[1] >= -1.5);
        // and it still points the same way
        assert!(clipped.1[0] < 500.0 && clipped.1[1] < 400.0);
    }

    #[test]
    fn a_segment_entirely_off_one_side_is_dropped() {
        assert!(clip_to_viewport([-500.0, 400.0], [-100.0, 400.0], SCREEN).is_none());
    }

    #[test]
    fn a_segment_crossing_the_whole_viewport_keeps_both_edges() {
        let clipped = clip_to_viewport([-500.0, 400.0], [1500.0, 400.0], SCREEN).unwrap();
        assert!(near(clipped.0, [-1.0, 400.0]) && near(clipped.1, [1001.0, 400.0]));
    }

    #[test]
    fn a_degenerate_point_inside_survives() {
        assert!(clip_to_viewport([500.0, 400.0], [500.0, 400.0], SCREEN).is_some());
    }
}

#[cfg(test)]
mod heading_from_the_product {
    use super::{Camera, multiply, view_from_pose};

    /// The projection `0x140001a90` emits, at 90 degrees and 16:9.
    fn projection() -> super::Matrix {
        let cot = 1.0f32;
        let (near, far) = (0.1f32, 1000.0f32);
        [
            cot / (16.0 / 9.0),
            0.0,
            0.0,
            0.0, //
            0.0,
            cot,
            0.0,
            0.0, //
            0.0,
            0.0,
            far / (far - near),
            1.0, //
            0.0,
            0.0,
            -near * far / (far - near),
            0.0,
        ]
    }

    /// A camera yawed `degrees` about the world up axis, built the way the engine builds one.
    fn yawed(degrees: f32) -> Camera {
        let half = degrees.to_radians() * 0.5;
        let view = view_from_pose([0.0, half.sin(), 0.0, half.cos()], [3.0, 5.0, -7.0]).unwrap();
        Camera {
            view,
            view_projection: multiply(&view, &projection()),
        }
    }

    #[test]
    fn a_captured_camera_with_no_view_half_still_has_a_heading() {
        // THE LIVE FAILURE. `crate::capture` stores an identity where the view matrix would be,
        // and the old reading of the view's third axis made every such camera report zero.
        let full = yawed(40.0);
        let captured = Camera {
            view: [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ],
            view_projection: full.view_projection,
        };
        assert!((captured.yaw_degrees() - full.yaw_degrees()).abs() < 0.01);
    }

    #[test]
    fn turning_the_camera_changes_the_heading_by_that_much() {
        let before = yawed(10.0).yaw_degrees();
        let after = yawed(55.0).yaw_degrees();
        let mut delta = after - before;
        while delta > 180.0 {
            delta -= 360.0;
        }
        while delta < -180.0 {
            delta += 360.0;
        }
        assert!((delta.abs() - 45.0).abs() < 0.5, "delta was {delta}");
    }

    #[test]
    fn the_heading_ignores_where_the_camera_is_standing() {
        let near = yawed(25.0);
        let half = 25.0f32.to_radians() * 0.5;
        let far_view =
            view_from_pose([0.0, half.sin(), 0.0, half.cos()], [400.0, -90.0, 250.0]).unwrap();
        let far = Camera {
            view: far_view,
            view_projection: multiply(&far_view, &projection()),
        };
        assert!((near.yaw_degrees() - far.yaw_degrees()).abs() < 0.01);
    }
}

#[cfg(test)]
mod framing_is_measured_at_the_head {
    use super::*;

    const SCREEN: [f32; 2] = [2560.0, 1440.0];

    /// A camera at `eye` looking at `at`, built the way the engine builds one.
    fn looking(eye: [f32; 3], at: [f32; 3]) -> Camera {
        let forward = normalize(sub(at, eye)).expect("a direction");
        let right = normalize(cross([0.0, 1.0, 0.0], forward)).expect("a right axis");
        let up = cross(forward, right);
        let mut view = [0.0f32; 16];
        view[0] = right[0];
        view[4] = right[1];
        view[8] = right[2];
        view[1] = up[0];
        view[5] = up[1];
        view[9] = up[2];
        view[2] = forward[0];
        view[6] = forward[1];
        view[10] = forward[2];
        view[12] = -dot(eye, right);
        view[13] = -dot(eye, up);
        view[14] = -dot(eye, forward);
        view[15] = 1.0;
        let cot = 1.0f32;
        let (near, far) = (0.1f32, 1000.0f32);
        let projection = [
            cot / (16.0 / 9.0),
            0.0,
            0.0,
            0.0, //
            0.0,
            cot,
            0.0,
            0.0, //
            0.0,
            0.0,
            far / (far - near),
            1.0, //
            0.0,
            0.0,
            -near * far / (far - near),
            0.0,
        ];
        Camera {
            view,
            view_projection: multiply(&view, &projection),
        }
    }

    /// The whole point. A camera aimed at the head is correct, and measuring the FEET against
    /// the centre calls it a large error -- which is the trap the old test fell into.
    #[test]
    fn a_camera_aimed_at_the_head_frames_the_character() {
        let feet = [10.0, 5.0, -16.0];
        let head = [feet[0], feet[1] + HEAD_METERS, feet[2]];
        let camera = looking([10.0, 6.8, -21.0], head);
        assert!(camera.frames_the_character(feet, SCREEN));

        let (_, head_y) = camera
            .head_offset_from_centre(feet, SCREEN)
            .expect("on screen");
        assert!(head_y < 0.01, "the head is centred, got {head_y}");

        // And the feet are far from centre while everything is right, which is why the feet
        // cannot be the thing measured.
        let at_feet = camera.project(feet, SCREEN).expect("on screen");
        assert!(
            (at_feet[1] - SCREEN[1] * 0.5).abs() > 100.0,
            "the feet should sit well below centre, got {}",
            at_feet[1]
        );
    }

    /// The failure the report described: a base visibly off the player. At the old 0.30 this
    /// passed; it must not now.
    #[test]
    fn a_camera_that_shoves_the_character_off_centre_is_refused() {
        let feet = [10.0, 5.0, -16.0];
        let head = [feet[0], feet[1] + HEAD_METERS, feet[2]];
        // Aimed a long way to one side of the character rather than at them.
        let camera = looking([10.0, 6.8, -21.0], [head[0] + 2.2, head[1], head[2] + 0.6]);
        let (x, _) = camera
            .head_offset_from_centre(feet, SCREEN)
            .expect("on screen");
        assert!(
            x > FRAMING_TOLERANCE,
            "this camera is visibly off and must fail, offset was {x}"
        );
        assert!(!camera.frames_the_character(feet, SCREEN));
        // It would have passed the tolerance this replaced.
        assert!(x <= 0.30, "the old 0.30 let exactly this through, {x}");
    }

    #[test]
    fn a_character_behind_the_lens_is_not_framed() {
        let feet = [10.0, 5.0, -16.0];
        let camera = looking(
            [10.0, 6.8, -21.0],
            [feet[0], feet[1] + HEAD_METERS, feet[2]],
        );
        let behind = [10.0, 5.0, -26.0];
        assert!(!camera.frames_the_character(behind, SCREEN));
        assert!(camera.head_offset_from_centre(behind, SCREEN).is_none());
    }
}
