//! Asking DARK SOULS II's own navigation stack for a route, using DARK SOULS II's own functions.
//!
//! # What changed, and why this module can exist at all
//!
//! `crate::navpath` -- the module next door -- says a route cannot be *asked for* because turning
//! a world position into a navigation-graph id is an asynchronous engine job. **That was wrong.**
//! The snap is an ordinary synchronous function returning the id in `eax`, and `0x14037be30`
//! contains the whole of it in twenty-eight instructions and does nothing else:
//!
//! ```text
//! key   = 0x140bab1f0(MapManager->area)      ; (area & 0x3f) << 24 | 0xffffff
//! world = 0x14039a9f0(GameManagerImp)        ; [[gm + 0xBC0] + 0x10]
//! data  = 0x140badb90(world, key)            ; the loaded area's graph
//! id    = 0x140babf90(data, &pos, 20.0, 0x28, NULL)
//! ```
//!
//! Every offset and every address is in `ds2-rva` with the disassembly it came from. bd
//! `ds2-mods-rs-4yd` was filed on the opposite premise and has been corrected.
//!
//! # Nothing here fabricates an engine context
//!
//! bd `ds2-call-the-games-own-functions` forbids hand-building the update context a planner's
//! `Update` needs. This does not build one: `0x140bae8d0` links a fresh planner onto
//! `NvNavigationSystem`'s own intrusive list, and from that moment `NvNavigationSystem::Update`
//! steps it every frame with the context the engine already owns. The planner this crate creates
//! is driven by the game, not by this crate.
//!
//! # Every function in here must be called from the game thread
//!
//! Not as a convention -- as a property of what they touch. `0x140bac070`, underneath the snap,
//! lazy-initialises a `DLKR` allocator through a process-global and then aborts the process with
//! `"Tried to create container with incompatible heap."` if the container it builds does not
//! match. The planner is on a list `NvNavigationSystem::Update` is walking. Neither takes a lock.
//!
//! So the only caller is `crate::gametick`, which is a detour on `NvNavigationSystem::Update`
//! itself. `Present` may ask for a route; it may not fetch one.

use ds2_game_base::mem::{game_rva, read_bytes, safe_read_u8, safe_read_u32, safe_read_usize};

/// A position handed to the engine.
///
/// **Sixteen bytes, sixteen-byte aligned, and both halves of that matter.** The engine's own
/// callers pass a buffer filled by `CharacterCtrl`'s position accessor (vtable slot `+0x148`),
/// which is a `movaps` of a whole `__m128`; the snap's inner loop reads it back the same way. A
/// `[f32; 3]` on an arbitrary stack slot is three-quarters of that and misaligned, which on the
/// aligned move is a fault rather than a wrong answer.
///
/// `w` is padding the navigation code never reads. It is zero here because an uninitialised
/// fourth lane that happens to be a signalling NaN is the kind of thing that only shows up on
/// someone else's machine.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Aligned4([f32; 4]);

impl Aligned4 {
    /// A world point, with the fourth lane zeroed.
    pub(crate) fn point(value: [f32; 3]) -> Self {
        Self([value[0], value[1], value[2], 0.0])
    }

    fn as_ptr(&self) -> *const f32 {
        self.0.as_ptr()
    }
}

/// `u32 area -> u32 key`. See [`ds2_rva::NAVI_GRAPH_KEY_FROM_AREA`].
type KeyFromArea = unsafe extern "system" fn(u32) -> u32;
/// `GameManagerImp* -> NvNaviGraphWorld*`. See [`ds2_rva::NAVI_GRAPH_WORLD_FROM_GAME_MANAGER`].
type GraphWorldFromGameManager = unsafe extern "system" fn(usize) -> usize;
/// `(NvNaviGraphWorld*, u32 key) -> graph data*`. See [`ds2_rva::NAVI_GRAPH_DATA_FOR_KEY`].
type GraphDataForKey = unsafe extern "system" fn(usize, u32) -> usize;
/// `(graph data*, const f32* pos, f32 radius, u32 filter, f32* out_dist) -> u32`.
///
/// The third parameter lands in `xmm2` and the fourth in `r9d` because the Windows x64 ABI
/// assigns argument registers by POSITION rather than by type -- which is exactly what
/// `0x14037bee d` does at the call site.
type NearestGraphId = unsafe extern "system" fn(usize, *const f32, f32, u32, *mut f32) -> u32;
/// `NvNavigationSystem* -> NvRoutePlanner*`, linked into the system's update list.
///
/// Declared with the three trailing arguments the function really takes even though they are
/// used only on a branch `0x140bb4080` makes unreachable -- passing zeros is free and keeps
/// "uninitialised register" off the list of things that could be wrong.
type CreateRoutePlanner = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
/// `(NvRoutePlanner*, u32 start, u32 goal, u32 capability, f32 max_cost)`.
///
/// The fifth argument is on the stack; there are only four register slots.
type RequestRoute = unsafe extern "system" fn(usize, u32, u32, u32, f32);
/// `(NvNavigationSystem*, object*)` -- sets the retire byte and returns.
type RetireNavObject = unsafe extern "system" fn(usize, usize);

/// Resolve an RVA and transmute it to `T`.
///
/// # Safety
///
/// `rva` must name a function in the loaded game image whose real signature is `T`. Every caller
/// below passes a constant from `ds2-rva` whose documentation records the disassembly the
/// signature was read from.
unsafe fn entry<T: Copy>(rva: u32) -> Option<T> {
    const {
        assert!(size_of::<T>() == size_of::<usize>());
    }
    let address = game_rva(rva).ok()?;
    // SAFETY: `T` is a pointer-sized function pointer (asserted above) and `address` is a
    // resolved RVA inside the loaded image, so this reinterprets an address as the function that
    // lives at it -- the same thing `GetProcAddress` plus a cast does, with the size checked at
    // compile time instead of by convention.
    Some(unsafe { std::mem::transmute_copy::<usize, T>(&address) })
}

/// The live `GameManagerImp`, or `None` before there is one.
pub(crate) fn game_manager() -> Option<usize> {
    let at = game_rva(ds2_rva::GAME_MANAGER_IMP).ok()?;
    // SAFETY: a resolved RVA inside the loaded image; the reader reports an unmapped page rather
    // than faulting, which is the state at the very start of boot.
    let manager = unsafe { safe_read_usize(at)? };
    (manager != 0).then_some(manager)
}

// NO BARE `snap()` WRAPPER HERE. There was one -- `snap_reporting(pos).id` -- and it had no
// caller, because the only thing that snaps is the route request and the route request is the
// place that most needs to be able to say WHY a snap missed. A convenience function that
// discards the diagnosis would be available for someone to reach for on the day the diagnosis
// matters again.

/// Everything one snap found out, for a log line that can name which suspect was guilty.
///
/// A live run returned NO ROUTE for two characters 16.8 m apart, and the three candidate causes
/// -- a bad key, a bad capability mask, a small cost budget -- produce the same silence. These
/// fields exist to separate the first from the other two, and to turn the key itself from a
/// guess into a measurement.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SnapReport {
    /// The navigation-graph id, or `None` if nothing was in range of any graph.
    pub(crate) id: Option<u32>,
    /// How many graphs the world says it is holding.
    pub(crate) graphs: i32,
    /// The key computed from [`ds2_rva::MAP_MANAGER_AREA_OFFSET`], and whether it found anything.
    pub(crate) key: u32,
    /// Did the engine's own keyed lookup return a graph for that key?
    pub(crate) keyed_hit: bool,
    /// Index of the graph the sweep chose, if any.
    pub(crate) chosen: Option<usize>,
    /// **The measurement.** The key that graph actually carries. When this differs from `key`,
    /// the area lookup is wrong and this is the value it should have produced.
    pub(crate) chosen_key: Option<u32>,
    /// Squared distance from `position` to the node that won.
    pub(crate) distance_squared: f32,
}

/// [`snap`], reporting how it got there.
///
/// # Why this sweeps instead of trusting the key
///
/// The engine's own path is `0x140badb90(world, key)` -- one linear scan that returns the graph
/// whose [`ds2_rva::NV_NAVI_GRAPH_HEADER_KEY_OFFSET`] matches. That is one lookup and it is
/// cheaper than what this does. It is also only as good as the `area` fed to
/// [`ds2_rva::NAVI_GRAPH_KEY_FROM_AREA`], which keeps six bits of a number whose provenance the
/// engine itself is inconsistent about -- `0x14037be30` reads `MapManager + 0x170`,
/// `0x14042c9a0` reads a byte off the character instead. Twenty-eight `.ngp` meshes cannot be
/// told apart by six bits of a map number, so at least one of those is a slot index, and nobody
/// has established which.
///
/// So: ask EVERY graph the world holds, and let the distance decide. That is sound for a reason
/// beyond stubbornness -- `0x140bac070` rejects a sub-graph on an AABB test before it allocates
/// or searches anything, so graphs for other areas cost a handful of float comparisons and
/// return [`ds2_rva::NAVI_GRAPH_ID_NONE`] immediately. The sweep is nearly free exactly where it
/// is redundant, and it is correct exactly where the key is not.
///
/// The keyed lookup is still performed, and its result recorded, so the log can say whether the
/// two agreed. The day they always agree, this can go back to being one call.
///
/// # Safety
///
/// Game thread only. See the module header: the allocator underneath this is lazily built and
/// unguarded.
pub(crate) unsafe fn snap_reporting(position: [f32; 3]) -> SnapReport {
    let mut report = SnapReport::default();
    if !position.iter().all(|c| c.is_finite()) {
        return report;
    }
    let Some(manager) = game_manager() else {
        return report;
    };
    let key_from_area: Option<KeyFromArea> = unsafe { entry(ds2_rva::NAVI_GRAPH_KEY_FROM_AREA) };
    let world_from_manager: Option<GraphWorldFromGameManager> =
        unsafe { entry(ds2_rva::NAVI_GRAPH_WORLD_FROM_GAME_MANAGER) };
    let data_for_key: Option<GraphDataForKey> = unsafe { entry(ds2_rva::NAVI_GRAPH_DATA_FOR_KEY) };
    let nearest: Option<NearestGraphId> = unsafe { entry(ds2_rva::NAVI_GRAPH_NEAREST_ID) };
    let (Some(key_from_area), Some(world_from_manager), Some(data_for_key), Some(nearest)) =
        (key_from_area, world_from_manager, data_for_key, nearest)
    else {
        return report;
    };

    // SAFETY: `manager` is non-null, which is the one precondition this function does not check
    // for itself.
    let world = unsafe { world_from_manager(manager) };
    if world == 0 {
        return report;
    }

    // The keyed lookup, kept for the log rather than for the answer.
    if let Some(map_manager) =
        // SAFETY: a live `GameManagerImp`; the reader refuses an unmapped page.
        unsafe { safe_read_usize(manager + ds2_rva::GAME_MANAGER_MAP_MANAGER_OFFSET) }
                .filter(|value| *value != 0)
        && let Some(area) =
            // SAFETY: as above.
            unsafe { safe_read_u32(map_manager + ds2_rva::MAP_MANAGER_AREA_OFFSET) }
    {
        // SAFETY: five instructions, no memory access, cannot fail.
        report.key = unsafe { key_from_area(area) };
        // SAFETY: `world` is the engine's own object; the scan is bounded by its own count.
        report.keyed_hit = unsafe { data_for_key(world, report.key) } != 0;
    }

    // SAFETY: a live `NvNaviGraphWorld`; the reader refuses an unmapped page.
    let Some(count) =
        (unsafe { safe_read_u32(world + ds2_rva::NV_NAVI_GRAPH_WORLD_GRAPH_COUNT_OFFSET) })
    else {
        return report;
    };
    report.graphs = count as i32;
    // A COUNT READ FROM LIVE MEMORY IS A NUMBER UNTIL SOMETHING BOUNDS IT. The array is inline
    // between `+0x28` and the count at `+0x68`, so a ninth entry would overwrite the count that
    // describes it -- eight is structural, not a preference.
    let count = (count as usize).min(ds2_rva::NV_NAVI_GRAPH_WORLD_MAX_GRAPHS);

    let point = Aligned4::point(position);
    let mut best = f32::INFINITY;
    for index in 0..count {
        let at = world + ds2_rva::NV_NAVI_GRAPH_WORLD_GRAPHS_OFFSET + index * 8;
        // SAFETY: inside the inline array the count above bounds.
        let Some(graph) = (unsafe { safe_read_usize(at) }).filter(|graph| *graph != 0) else {
            continue;
        };
        let mut distance = f32::INFINITY;
        // SAFETY: `graph` is one of the engine's own resident meshes and `point` is sixteen
        // aligned bytes this call owns. The out-distance pointer is written only on a hit; the
        // function tests it for null, so a non-null one is equally fine.
        let id = unsafe {
            nearest(
                graph,
                point.as_ptr(),
                ds2_rva::NAVI_GRAPH_SNAP_RADIUS_METERS,
                ds2_rva::NAVI_GRAPH_SNAP_FILTER,
                &raw mut distance,
            )
        };
        // SQUARED distances, both of them -- `0x140babf90` writes the square and takes the root
        // only to narrow its own search radius. Comparing them is therefore comparing like with
        // like, and taking a root here would be arithmetic for nobody.
        if id != ds2_rva::NAVI_GRAPH_ID_NONE && distance < best {
            best = distance;
            report.id = Some(id);
            report.chosen = Some(index);
            report.chosen_key = graph_key(graph);
            report.distance_squared = distance;
        }
    }
    report
}

/// The map key a loaded graph carries, straight off the object.
///
/// This is the number `0x140badb90` compares against, so reading it from a graph that was found
/// another way says what the key for this map IS -- rather than what some expression hopes it is.
fn graph_key(graph: usize) -> Option<u32> {
    // SAFETY: a live `NvNaviGraphData`; both reads refuse an unmapped page.
    unsafe {
        let header = safe_read_usize(graph + ds2_rva::NV_NAVI_GRAPH_HEADER_OFFSET)?;
        (header != 0).then_some(())?;
        safe_read_u32(header + ds2_rva::NV_NAVI_GRAPH_HEADER_KEY_OFFSET)
    }
}

// NO `nav_system()` HELPER HERE, deliberately. Reading `GameManagerImp + 0xBC0` would produce a
// second `NvNavigationSystem` pointer alongside the one `crate::gametick`'s detour is handed in
// `rcx`, and only the argument is guaranteed to be the object the engine is stepping this tick.
// Two sources of truth for one pointer is how a planner ends up linked onto a list nobody walks.

/// Create an `NvRoutePlanner` and link it onto the navigation system's update list.
///
/// From here the engine steps it every frame. Nothing in this crate calls its `Update`.
///
/// # Safety
///
/// Game thread only, and NOT from inside `NvNavigationSystem::Update`'s own list walk -- this
/// pushes onto the head of the list that walk is traversing. `crate::gametick` calls it after
/// the original has returned, which is why that ordering is not an implementation detail.
pub(crate) unsafe fn create_planner(nav_system: usize) -> Option<usize> {
    let create: CreateRoutePlanner =
        unsafe { entry(ds2_rva::NV_NAVIGATION_SYSTEM_CREATE_ROUTE_PLANNER)? };
    // SAFETY: `nav_system` is the engine's own singleton; the factory allocates from the
    // allocator that object carries and returns 0 if it cannot.
    let planner = unsafe { create(nav_system, 0, 0, 0) };
    (planner != 0).then_some(planner)
}

/// Ask for a route between two graph ids.
///
/// Clears both result bits and sets pending, so calling it over a search still in flight is a
/// re-request rather than a leak.
///
/// # Safety
///
/// Game thread only. `planner` must be one [`create_planner`] returned and which has not been
/// retired.
pub(crate) unsafe fn request(planner: usize, start: u32, goal: u32, max_cost: f32) -> bool {
    let found: Option<RequestRoute> = unsafe { entry(ds2_rva::NV_ROUTE_PLANNER_REQUEST) };
    let Some(request) = found else {
        return false;
    };
    // SAFETY: eight instructions, four stores and a read-modify-write of one byte, all within
    // the planner.
    unsafe {
        request(
            planner,
            start,
            goal,
            ds2_rva::NV_ROUTE_PLANNER_CAPABILITY_DEFAULT,
            max_cost,
        );
    }
    true
}

/// Tell the navigation system to drop a planner on its next tick.
///
/// # Safety
///
/// `planner` must be one [`create_planner`] returned. Safe to call from anywhere the tick is not
/// currently running, because it only sets a byte -- the unlink and the destructor happen inside
/// `NvNavigationSystem::Update`.
pub(crate) unsafe fn retire(nav_system: usize, planner: usize) {
    let found: Option<RetireNavObject> = unsafe { entry(ds2_rva::NV_NAVIGATION_SYSTEM_RETIRE) };
    if let Some(retire) = found {
        // SAFETY: four instructions, one byte store into the object.
        unsafe { retire(nav_system, planner) };
    }
}

/// What one poll of a planner found.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Poll {
    /// The search has not finished. Ask again next tick.
    Pending,
    /// The search finished and there is no way to walk there. Draw the arrow.
    Failed,
    /// A route, in walking order, start first.
    Ready {
        /// The decoded polyline.
        points: Vec<[f32; 3]>,
        /// How many segments the engine's own route held, read straight off
        /// [`ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET`].
        ///
        /// Carried because the two numbers fail differently and a log with only one of them
        /// cannot say which happened. A route of 40 segments that decodes to 2 points means the
        /// decoder is wrong; 2 segments and 2 points means the map really is that short a walk.
        segments: i32,
    },
    /// The planner could not be read at all -- an unmapped page, which means the object is gone.
    Lost,
}

/// Read a planner's flags and, if a route is ready, decode it.
///
/// # The order of the two bit tests is the whole function
///
/// All three of the engine's give-up paths end with `or byte [planner+0x30], 6`, which sets
/// READY and FAILED together. Testing READY first therefore believes every failure and goes on
/// to dereference `planner + 0x48`, which those paths left holding whatever was there before.
/// See [`ds2_rva::NV_ROUTE_PLANNER_FLAG_FAILED`].
///
/// # Safety
///
/// Game thread only: the planner is on a list the tick walks, and the route behind `+0x48` is
/// freed by the next request.
pub(crate) unsafe fn poll(planner: usize) -> Poll {
    // SAFETY: a resolved engine object; the reader reports an unmapped page rather than faulting,
    // which is how a planner that has already been destroyed is detected instead of crashed on.
    let Some(flags) = (unsafe { safe_read_u8(planner + ds2_rva::NV_ROUTE_PLANNER_FLAGS_OFFSET) })
    else {
        return Poll::Lost;
    };
    if flags & ds2_rva::NV_ROUTE_PLANNER_FLAG_FAILED != 0 {
        return Poll::Failed;
    }
    if flags & ds2_rva::NV_ROUTE_PLANNER_FLAG_READY == 0 {
        return Poll::Pending;
    }
    // SAFETY: as above.
    let Some(route) =
        (unsafe { safe_read_usize(planner + ds2_rva::NV_ROUTE_PLANNER_ROUTE_OFFSET) })
    else {
        return Poll::Lost;
    };
    if route == 0 {
        // READY without a route is not a shape the engine produces, but it is cheaper to treat it
        // as a failed search than to explain a null dereference afterwards.
        return Poll::Failed;
    }
    // Read before decoding: the decoder walks the same structure, and a count that disagrees
    // with the number of points it produced is the single most useful thing the log can say
    // about a route that came back wrong.
    let segments =
        crate::navpath::Memory::i32(&GameMemory, route + ds2_rva::NV_ROUTE_SEGMENT_COUNT_OFFSET)
            .unwrap_or(-1);
    crate::navpath::decode(&GameMemory, route)
        .map_or(Poll::Failed, |points| Poll::Ready { points, segments })
}

/// `crate::navpath::Memory` over the live process, through the fault-safe reader.
///
/// The decoder is proved against a synthetic buffer in `navpath`'s own tests; this is the only
/// thing that changes between that proof and the game, and it is four lines.
struct GameMemory;

impl crate::navpath::Memory for GameMemory {
    fn read(&self, at: usize, len: usize) -> Option<Vec<u8>> {
        // A length this large means the structure was not a route; refusing here keeps the
        // allocation bounded before the read is even attempted.
        if at == 0 || len == 0 || len > 4096 {
            return None;
        }
        let mut out = vec![0u8; len];
        // SAFETY: `read_bytes` probes and reports an unmapped page rather than faulting, which is
        // the whole reason a route half-freed by another subsystem is a `None` here instead of an
        // access violation in someone's invasion.
        unsafe { read_bytes(at, &mut out) }.then_some(out)
    }
}

/// How many objects the navigation system currently has on its update list.
///
/// Read for one purpose: a log line that distinguishes "the planner was created" from "the
/// planner was created AND linked", which are different failures and look identical otherwise.
pub(crate) fn listed_objects(nav_system: usize) -> Option<i32> {
    // SAFETY: a live `NvNavigationSystem`; the reader refuses an unmapped page.
    unsafe {
        safe_read_u32(nav_system + ds2_rva::NV_NAVIGATION_SYSTEM_LIST_COUNT_OFFSET)
            .map(|count| count as i32)
    }
}
