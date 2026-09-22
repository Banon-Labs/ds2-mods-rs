//! Reading a finished `NvRoute` out of the engine, and the honest account of what is missing.
//!
//! # The state of this, plainly
//!
//! `docs/PORTING.md` filed `er-invasion-path` under "no DS2 analogue (Havok-AI navmesh)". Half of
//! that is right: there is no Havok AI here. The conclusion is wrong. DARK SOULS II ships its own
//! navigation stack, RTTI-named throughout -- `NvNavigationSystem`, `NvNaviGraphWorld`,
//! `NvRoutePlanner`, `NvRouteNavigator`, `NvNaviPolyNearestSearchTask`, `ChrAiNavimeshCtrl` -- and
//! it has the same request/poll shape Elden Ring's `CSHkAiWorld` has.
//!
//! What this module implements is the **reader**: given a finished route, produce the polyline.
//! Every offset in it was read out of the engine's own accessors, and the decode is exercised
//! below against a synthetic buffer laid out byte for byte the way the engine lays one out.
//!
//! # The half that used to be missing, and is not
//!
//! This module used to say -- at length -- that a route could not be *asked for*, because
//! `0x140bb4090` takes navigation-graph ids rather than world positions and turning a position
//! into one was an asynchronous job whose context would have to be fabricated. bd
//! `ds2-mods-rs-4yd` was filed on that premise.
//!
//! **The premise was false.** The snap is a plain synchronous function returning the id in
//! `eax`, and `0x14037be30` does the whole chain in twenty-eight instructions. `crate::navquery`
//! calls it; `crate::gametick` requests the route and polls it; this module decodes what comes
//! back. Nothing fabricates a context: `0x140bae8d0` links a planner onto the navigation
//! system's own list and the engine steps it from then on, which is the opposite of the thing
//! bd `ds2-call-the-games-own-functions` warns against.
//!
//! The arrow is still there, and still matters: it is what the overlay falls back to when the
//! planner reports [`ds2_rva::NV_ROUTE_PLANNER_FLAG_FAILED`], which is the same degraded mode the
//! Elden Ring crate uses when its navmesh answers "there is no way to walk there".

// The bounds below are a contract with the engine rather than a set of call sites, and two of
// them (`MAX_POINTS_PER_SEGMENT`, `MAX_POINTS`) are enforced inside `decode` while
// `MAX_SEGMENTS` is read by its caller's tests. Scoped to this module so the rest of the crate
// keeps its unused items visible.
#![allow(dead_code)]

/// Somewhere bytes can be read from, fallibly.
///
/// The live implementation is the fault-safe reader in `ds2-game-base`, which reports an unmapped
/// page rather than faulting. The test implementation is a `Vec<u8>`. Having both behind one
/// trait is what lets the indexing -- which is where the mistakes live -- be proven without a
/// game.
pub(crate) trait Memory {
    /// Read `len` bytes at `at`, or `None` if any of them is unreadable.
    fn read(&self, at: usize, len: usize) -> Option<Vec<u8>>;

    fn u64(&self, at: usize) -> Option<u64> {
        let bytes = self.read(at, 8)?;
        Some(u64::from_le_bytes(bytes.try_into().ok()?))
    }

    fn i32(&self, at: usize) -> Option<i32> {
        let bytes = self.read(at, 4)?;
        Some(i32::from_le_bytes(bytes.try_into().ok()?))
    }

    fn i16(&self, at: usize) -> Option<i16> {
        let bytes = self.read(at, 2)?;
        Some(i16::from_le_bytes(bytes.try_into().ok()?))
    }

    /// Read three `f32` at `at`, refusing anything that is not a real number.
    ///
    /// The fourth component of the engine's 16-byte points is padding this never reads.
    fn point(&self, at: usize) -> Option<[f32; 3]> {
        let bytes = self.read(at, 12)?;
        let mut out = [0.0f32; 3];
        for (index, slot) in out.iter_mut().enumerate() {
            let word: [u8; 4] = bytes[index * 4..index * 4 + 4].try_into().ok()?;
            *slot = f32::from_le_bytes(word);
            if !slot.is_finite() {
                return None;
            }
        }
        Some(out)
    }
}

/// Most segments a route may have before this refuses to walk it.
///
/// The count is an `i32` read out of live memory, so it is a number rather than a promise. A
/// route with more segments than this is not truncated quietly -- [`decode`] returns `None` and
/// the caller falls back to the arrow, because a count that large means the pointer was not a
/// route and the points behind it are not points.
pub(crate) const MAX_SEGMENTS: i32 = 4096;

/// Most points one segment may hold. The engine's own field is an `i16`, so this is a sanity
/// bound rather than a format limit.
pub(crate) const MAX_POINTS_PER_SEGMENT: i16 = 1024;

/// Total points [`decode`] will produce, after which it stops.
pub(crate) const MAX_POINTS: usize = 8192;

/// Decode a finished `NvRoute` into a world-space polyline, **start first**.
///
/// # The two things that are not obvious
///
/// Both are read out of `0x140bb3bd0`, the engine's own "position of route node N":
///
/// 1. **A route is stored goal-first.** `0x140bb5cd0` sets the navigator's cursor to
///    `(segment_count - 1) << 16` and walks it down, so segment `count - 1` is where the
///    character is standing and segment `0` is the destination. This returns them in walking
///    order, which means iterating segments in reverse.
/// 2. **Points inside a segment are stored in reverse too.** Node `p` of a segment is
///    `points[count - 1 - p]`. That is a second reversal, not the same one, and applying only
///    one of the two produces a polyline that is plausible, connected, and inside out.
///
/// A segment whose point count is not positive still contributes one node -- its own
/// [`ds2_rva::NV_ROUTE_SEGMENT_POINT_OFFSET`] -- which is the `0 < count` branch in the engine's
/// accessor, not an edge case this invented.
///
/// Returns `None` rather than a partial line if anything about the structure fails to validate.
/// A route half-read is worse than no route: it draws a confident line to somewhere nobody is.
pub(crate) fn decode(memory: &dyn Memory, route: usize) -> Option<Vec<[f32; 3]>> {
    let segments = memory.u64(route + ds2_rva::NV_ROUTE_SEGMENTS_OFFSET)? as usize;
    let count = memory.i32(route + ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET)?;
    if segments == 0 || count <= 0 || count > MAX_SEGMENTS {
        return None;
    }
    let mut out: Vec<[f32; 3]> = Vec::new();
    for index in (0..count).rev() {
        let segment = segments.checked_add((index as usize).checked_mul(SEGMENT_STRIDE)?)?;
        let points = memory.i16(segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_COUNT_OFFSET)?;
        if points > MAX_POINTS_PER_SEGMENT {
            return None;
        }
        if points <= 0 {
            // The engine's own fallback: a segment with no expanded polyline is still one node.
            out.push(memory.point(segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_OFFSET)?);
            continue;
        }
        let array = memory.u64(segment + ds2_rva::NV_ROUTE_SEGMENT_POINTS_OFFSET)? as usize;
        if array == 0 {
            return None;
        }
        for point in (0..points).rev() {
            if out.len() >= MAX_POINTS {
                return None;
            }
            let at = array.checked_add((point as usize).checked_mul(POINT_STRIDE)?)?;
            out.push(memory.point(at)?);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Bytes per route segment, from [`ds2_rva::NV_ROUTE_SEGMENT_STRIDE`].
const SEGMENT_STRIDE: usize = ds2_rva::NV_ROUTE_SEGMENT_STRIDE;

/// Bytes per point. Sixteen: the engine reads them with a pair of 8-byte moves.
const POINT_STRIDE: usize = 16;

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat buffer addressed from a chosen base, so tests can use engine-looking addresses.
    struct Fake {
        base: usize,
        bytes: Vec<u8>,
    }

    impl Memory for Fake {
        fn read(&self, at: usize, len: usize) -> Option<Vec<u8>> {
            let start = at.checked_sub(self.base)?;
            let end = start.checked_add(len)?;
            self.bytes.get(start..end).map(<[u8]>::to_vec)
        }
    }

    const BASE: usize = 0x1000;

    /// Lay out a route exactly the way the engine does, and hand back the buffer plus the route's
    /// address.
    ///
    /// `segments` is given **in engine order**, which is goal-first, and each segment's points
    /// are given in engine order too, which is reversed. So the caller writes what memory holds
    /// and the test asserts what walking order should come back -- rather than writing the
    /// answer and checking it is echoed.
    fn build(segments: &[Vec<[f32; 3]>]) -> (Fake, usize) {
        let mut bytes = vec![0u8; 0x2000];
        let route_at = 0x0usize;
        let segment_array = 0x100usize;
        let mut point_cursor = 0x100 + segments.len() * SEGMENT_STRIDE;
        // Round the point area up so it never overlaps the segment records.
        point_cursor = point_cursor.div_ceil(16) * 16;

        let put_u64 = |bytes: &mut Vec<u8>, at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        let put_i32 = |bytes: &mut Vec<u8>, at: usize, value: i32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        let put_i16 = |bytes: &mut Vec<u8>, at: usize, value: i16| {
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        };
        let put_point = |bytes: &mut Vec<u8>, at: usize, value: [f32; 3]| {
            for (index, component) in value.iter().enumerate() {
                bytes[at + index * 4..at + index * 4 + 4].copy_from_slice(&component.to_le_bytes());
            }
        };

        put_u64(
            &mut bytes,
            route_at + ds2_rva::NV_ROUTE_SEGMENTS_OFFSET,
            (BASE + segment_array) as u64,
        );
        put_i32(
            &mut bytes,
            route_at + ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET,
            segments.len() as i32,
        );
        for (index, points) in segments.iter().enumerate() {
            let segment = segment_array + index * SEGMENT_STRIDE;
            put_i16(
                &mut bytes,
                segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_COUNT_OFFSET,
                points.len() as i16,
            );
            if points.is_empty() {
                // The no-polyline case carries its position in the segment itself.
                put_point(
                    &mut bytes,
                    segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_OFFSET,
                    [index as f32, 0.0, 0.0],
                );
                continue;
            }
            put_u64(
                &mut bytes,
                segment + ds2_rva::NV_ROUTE_SEGMENT_POINTS_OFFSET,
                (BASE + point_cursor) as u64,
            );
            for point in points {
                put_point(&mut bytes, point_cursor, *point);
                point_cursor += POINT_STRIDE;
            }
        }
        (Fake { base: BASE, bytes }, BASE + route_at)
    }

    #[test]
    fn a_route_comes_back_in_walking_order() {
        // Memory holds: segment 0 is the GOAL, and within it points are reversed. So the walking
        // order is the exact reverse of the flattened literal below.
        let (memory, route) = build(&[
            vec![[9.0, 0.0, 0.0], [8.0, 0.0, 0.0]],
            vec![[7.0, 0.0, 0.0], [6.0, 0.0, 0.0]],
        ]);
        let line = decode(&memory, route).expect("a well-formed route decodes");
        let xs: Vec<f32> = line.iter().map(|point| point[0]).collect();
        assert_eq!(
            xs,
            vec![6.0, 7.0, 8.0, 9.0],
            "start must come first and both reversals must be applied"
        );
    }

    #[test]
    fn a_single_segment_route_is_still_reversed_inside() {
        let (memory, route) = build(&[vec![[3.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 0.0, 0.0]]]);
        let line = decode(&memory, route).expect("decodes");
        let xs: Vec<f32> = line.iter().map(|point| point[0]).collect();
        assert_eq!(xs, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_segment_with_no_polyline_contributes_its_own_point() {
        let (memory, route) = build(&[vec![], vec![[5.0, 0.0, 0.0]]]);
        let line = decode(&memory, route).expect("decodes");
        assert_eq!(line.len(), 2, "the empty segment was dropped: {line:?}");
        // Segment 1 is the start, segment 0 the goal, and `build` gives an empty segment the
        // x of its index.
        assert!((line[0][0] - 5.0).abs() < 1.0e-4);
        assert!((line[1][0] - 0.0).abs() < 1.0e-4);
    }

    #[test]
    fn a_null_segment_array_is_refused() {
        let (mut memory, route) = build(&[vec![[1.0, 0.0, 0.0]]]);
        let at = ds2_rva::NV_ROUTE_SEGMENTS_OFFSET;
        memory.bytes[at..at + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(decode(&memory, route).is_none());
    }

    #[test]
    fn an_absurd_segment_count_is_refused_rather_than_walked() {
        let (mut memory, route) = build(&[vec![[1.0, 0.0, 0.0]]]);
        let at = ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET;
        memory.bytes[at..at + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(
            decode(&memory, route).is_none(),
            "a count this large means the pointer was not a route"
        );
    }

    #[test]
    fn a_negative_segment_count_is_refused() {
        let (mut memory, route) = build(&[vec![[1.0, 0.0, 0.0]]]);
        let at = ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET;
        memory.bytes[at..at + 4].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(decode(&memory, route).is_none());
    }

    #[test]
    fn a_point_that_is_not_a_number_refuses_the_whole_route() {
        let (memory, route) = build(&[vec![[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]]]);
        assert!(
            decode(&memory, route).is_none(),
            "half a route drawn confidently is worse than none"
        );
    }

    #[test]
    fn an_unreadable_page_refuses_rather_than_panicking() {
        let (memory, _) = build(&[vec![[1.0, 0.0, 0.0]]]);
        // An address outside the fake buffer stands in for an unmapped page.
        assert!(decode(&memory, BASE + 0x10_0000).is_none());
    }
}
