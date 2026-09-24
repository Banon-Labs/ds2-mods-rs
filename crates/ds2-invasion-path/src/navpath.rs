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

    fn u32(&self, at: usize) -> Option<u32> {
        let bytes = self.read(at, 4)?;
        Some(u32::from_le_bytes(bytes.try_into().ok()?))
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

/// One point of a decoded route, and whether the ground between it and the point before it is
/// known.
///
/// # Why a route is not one kind of point
///
/// `0x140bb4ac0` writes a segment's two portal midpoints at `+0x10` and `+0x20` and then calls
/// the polyline builder `0x140bbaa30` -- which is what fills `+0x40` and `+0x50` -- **exactly
/// once**, for the terminal segment. Every other segment of a fresh route therefore has no
/// expanded polyline at all, and `0x140bb3bd0`, the engine's own "position of route node N", falls
/// back to that segment's `+0x20` for them. The engine expands the rest as an agent walks into
/// them; it does not compute the whole path up front.
///
/// So a route is a short run of real, ground-following points near you, followed by a handful of
/// portal midpoints tens of metres apart. Both are genuine positions on the navmesh. What is not
/// genuine is the STRAIGHT LINE between two portals: it is an interpolation nothing computed, and
/// it cuts through whatever hill stands between them.
///
/// Measured live on 2026-09-24: a route to a character 63.9 m away decoded to twelve expanded
/// points spaced 0.18-1.95 m, then three lone portals with steps of 1.5 m, **33.9 m** and **28.8
/// m**. Stones spaced evenly along those last two hops are stones in mid-air, which is what the
/// player saw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoutePoint {
    /// Where it is, in world space.
    pub(crate) at: [f32; 3],
    /// Does the ground between the PREVIOUS point and this one follow the navmesh?
    ///
    /// True only inside one segment's expanded polyline. False for the first point of any segment
    /// and for every lone portal, because the span leading to it was never expanded by anything.
    pub(crate) ground: bool,
}

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
pub(crate) fn decode(memory: &dyn Memory, route: usize) -> Option<Vec<RoutePoint>> {
    let segments = memory.u64(route + ds2_rva::NV_ROUTE_SEGMENTS_OFFSET)? as usize;
    let count = memory.i32(route + ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET)?;
    if segments == 0 || count <= 0 || count > MAX_SEGMENTS {
        return None;
    }
    let mut out: Vec<RoutePoint> = Vec::new();
    for index in (0..count).rev() {
        let segment = segments.checked_add((index as usize).checked_mul(SEGMENT_STRIDE)?)?;
        let points = memory.i16(segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_COUNT_OFFSET)?;
        if points > MAX_POINTS_PER_SEGMENT {
            return None;
        }
        if points <= 0 {
            // The engine's own fallback: a segment with no expanded polyline is still one node,
            // its portal midpoint. `ground: false` because nothing computed the span leading to
            // it -- see [`RoutePoint`], and the 33.9 m hop that prompted this.
            out.push(RoutePoint {
                at: memory.point(segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_OFFSET)?,
                ground: false,
            });
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
            out.push(RoutePoint {
                at: memory.point(at)?,
                // The first point of a segment is reached from the previous segment's last
                // point, and nothing expanded THAT span. Only steps inside one expanded polyline
                // are known ground.
                ground: point != points - 1,
            });
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Just the positions, for the callers that only need where the line goes.
pub(crate) fn positions(points: &[RoutePoint]) -> Vec<[f32; 3]> {
    points.iter().map(|point| point.at).collect()
}

/// One end of one route segment: the packed navi id the engine wrote there, and the portal
/// midpoint written beside it.
///
/// The id is the point of this type. A position cannot be asked whether a character may stand on
/// it; an id can, because it indexes the attribute table the engine's own traversal test reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RouteNode {
    /// Packed navi id, as `0x140bb4ac0` stored it.
    pub(crate) id: u32,
    /// Which segment it came from, in engine order (`0` is the GOAL end -- see [`decode`]).
    pub(crate) segment: i32,
    /// `'A'` for [`ds2_rva::NV_ROUTE_SEGMENT_NODE_A_OFFSET`], `'B'` for the other.
    pub(crate) side: char,
    /// The portal midpoint stored alongside the id, or `None` if it did not read as three real
    /// numbers.
    pub(crate) at: Option<[f32; 3]>,
}

/// Every node id a finished route names, in engine order, deduplicated.
///
/// Segment order is NOT reversed here, unlike [`decode`]. That function exists to draw a line and
/// a line has to be in walking order; this one exists to answer "which places does this route
/// use", which is a set. Reversing it would imply an ordering the caller does not have and
/// cannot check.
///
/// Returns an empty vector rather than `None` when the segments read but carry no usable id --
/// "this route names no nodes" is a finding, and a caller that cannot tell it from "the route
/// would not read" will report the wrong one.
pub(crate) fn segment_nodes(memory: &dyn Memory, route: usize) -> Option<Vec<RouteNode>> {
    let segments = memory.u64(route + ds2_rva::NV_ROUTE_SEGMENTS_OFFSET)? as usize;
    let count = memory.i32(route + ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET)?;
    if segments == 0 || count <= 0 || count > MAX_SEGMENTS {
        return None;
    }
    let ends = [
        (
            ds2_rva::NV_ROUTE_SEGMENT_NODE_A_OFFSET,
            ds2_rva::NV_ROUTE_SEGMENT_POINT_A_OFFSET,
            'A',
        ),
        (
            ds2_rva::NV_ROUTE_SEGMENT_NODE_B_OFFSET,
            ds2_rva::NV_ROUTE_SEGMENT_POINT_OFFSET,
            'B',
        ),
    ];
    let mut out: Vec<RouteNode> = Vec::new();
    for index in 0..count {
        let segment = segments.checked_add((index as usize).checked_mul(SEGMENT_STRIDE)?)?;
        for (id_at, point_at, side) in ends {
            let Some(id) = memory.u32(segment + id_at) else {
                continue;
            };
            // `0xffffffff` is what the engine writes on the first segment's incoming end and the
            // last segment's outgoing one -- a terminator, not a node.
            if id == u32::MAX || id & ds2_rva::NAVI_ID_INDEX_MASK == ds2_rva::NAVI_ID_INDEX_NONE {
                continue;
            }
            if out.iter().any(|seen| seen.id == id) {
                continue;
            }
            out.push(RouteNode {
                id,
                segment: index,
                side,
                at: memory.point(segment + point_at),
            });
        }
    }
    Some(out)
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
        let xs: Vec<f32> = line.iter().map(|point| point.at[0]).collect();
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
        let xs: Vec<f32> = line.iter().map(|point| point.at[0]).collect();
        assert_eq!(xs, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_segment_with_no_polyline_contributes_its_own_point() {
        let (memory, route) = build(&[vec![], vec![[5.0, 0.0, 0.0]]]);
        let line = decode(&memory, route).expect("decodes");
        assert_eq!(line.len(), 2, "the empty segment was dropped: {line:?}");
        // Segment 1 is the start, segment 0 the goal, and `build` gives an empty segment the
        // x of its index.
        assert!((line[0].at[0] - 5.0).abs() < 1.0e-4);
        assert!((line[1].at[0] - 0.0).abs() < 1.0e-4);
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

    /// Write the id and midpoint fields `0x140bb4ac0` writes, on top of a route `build` laid out.
    fn stamp_ids(memory: &mut Fake, ids: &[(u32, u32)]) {
        let segment_array = 0x100usize;
        for (index, (a, b)) in ids.iter().enumerate() {
            let segment = segment_array + index * SEGMENT_STRIDE;
            let put_u32 = |bytes: &mut Vec<u8>, at: usize, value: u32| {
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            };
            put_u32(
                &mut memory.bytes,
                segment + ds2_rva::NV_ROUTE_SEGMENT_NODE_A_OFFSET,
                *a,
            );
            put_u32(
                &mut memory.bytes,
                segment + ds2_rva::NV_ROUTE_SEGMENT_NODE_B_OFFSET,
                *b,
            );
            // A distinct midpoint per end, so a swapped pairing shows up as a wrong position
            // rather than as a test that passes either way.
            let at_a = segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_A_OFFSET;
            let at_b = segment + ds2_rva::NV_ROUTE_SEGMENT_POINT_OFFSET;
            for (base, tag) in [(at_a, index as f32), (at_b, index as f32 + 0.5)] {
                for (slot, value) in [tag, 0.0, 0.0].iter().enumerate() {
                    memory.bytes[base + slot * 4..base + slot * 4 + 4]
                        .copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }

    #[test]
    fn every_node_id_on_a_route_comes_back_with_its_own_midpoint() {
        let (mut memory, route) = build(&[
            vec![[9.0, 0.0, 0.0], [8.0, 0.0, 0.0]],
            vec![[7.0, 0.0, 0.0], [6.0, 0.0, 0.0]],
        ]);
        stamp_ids(
            &mut memory,
            &[(0x0012_0133, 0x0012_4008), (0x0012_4055, 0x0012_481e)],
        );
        let nodes = segment_nodes(&memory, route).expect("the route reads");
        assert_eq!(
            nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            vec![0x0012_0133, 0x0012_4008, 0x0012_4055, 0x0012_481e],
            "engine order, not walking order -- this answers WHICH places, not in what sequence"
        );
        assert_eq!(nodes[0].side, 'A');
        assert_eq!(nodes[0].at, Some([0.0, 0.0, 0.0]));
        assert_eq!(nodes[1].side, 'B');
        assert_eq!(
            nodes[1].at,
            Some([0.5, 0.0, 0.0]),
            "side B must carry the +0x20 midpoint, not side A's"
        );
        assert_eq!(nodes[3].segment, 1);
    }

    #[test]
    fn the_terminators_the_engine_writes_are_not_nodes() {
        let (mut memory, route) = build(&[vec![[1.0, 0.0, 0.0]], vec![[2.0, 0.0, 0.0]]]);
        // `0x140bb4ac0` writes 0xffffffff on the ends of the route, and an index of 0x7fff is the
        // engine's own "no node" sentinel. Neither is somewhere a character can stand.
        stamp_ids(
            &mut memory,
            &[(u32::MAX, 0x0012_4008), (0x0012_7fff, u32::MAX)],
        );
        let nodes = segment_nodes(&memory, route).expect("the route reads");
        assert_eq!(
            nodes.iter().map(|node| node.id).collect::<Vec<_>>(),
            vec![0x0012_4008],
            "0xffffffff is a terminator and 0x7fff is the no-node index"
        );
    }

    #[test]
    fn one_node_named_twice_is_listed_once() {
        let (mut memory, route) = build(&[vec![[1.0, 0.0, 0.0]], vec![[2.0, 0.0, 0.0]]]);
        stamp_ids(
            &mut memory,
            &[(0x0012_0133, 0x0012_4008), (0x0012_4008, 0x0012_0133)],
        );
        let nodes = segment_nodes(&memory, route).expect("the route reads");
        assert_eq!(
            nodes.len(),
            2,
            "adjacent segments share their boundary node"
        );
    }

    #[test]
    fn a_route_that_will_not_read_is_none_not_empty() {
        let (memory, _) = build(&[vec![[1.0, 0.0, 0.0]]]);
        assert!(
            segment_nodes(&memory, BASE + 0x10_0000).is_none(),
            "`None` is 'the route would not read'; an empty vector is 'it named no nodes'"
        );
    }
}
