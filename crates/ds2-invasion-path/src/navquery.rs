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

/// Snap a world position to a navigation-graph id.
///
/// Returns `None` when there is no world, no loaded graph for the current area, or nothing within
/// [`ds2_rva::NAVI_GRAPH_SNAP_RADIUS_METERS`] -- all three of which are ordinary answers rather
/// than errors. A player in a lift shaft or mid-fall legitimately has no node under them.
///
/// # Safety
///
/// Game thread only. See the module header: the allocator underneath this is lazily built and
/// unguarded.
pub(crate) unsafe fn snap(position: [f32; 3]) -> Option<u32> {
    if !position.iter().all(|c| c.is_finite()) {
        return None;
    }
    let manager = game_manager()?;
    let map_manager = unsafe {
        let value = safe_read_usize(manager + ds2_rva::GAME_MANAGER_MAP_MANAGER_OFFSET)?;
        (value != 0).then_some(value)?
    };
    let area = unsafe { safe_read_u32(map_manager + ds2_rva::MAP_MANAGER_AREA_OFFSET)? };

    let key_from_area: KeyFromArea = unsafe { entry(ds2_rva::NAVI_GRAPH_KEY_FROM_AREA)? };
    let world_from_manager: GraphWorldFromGameManager =
        unsafe { entry(ds2_rva::NAVI_GRAPH_WORLD_FROM_GAME_MANAGER)? };
    let data_for_key: GraphDataForKey = unsafe { entry(ds2_rva::NAVI_GRAPH_DATA_FOR_KEY)? };
    let nearest: NearestGraphId = unsafe { entry(ds2_rva::NAVI_GRAPH_NEAREST_ID)? };

    // SAFETY: five instructions, no memory access, cannot fail. Recorded as a call rather than
    // reimplemented as `(area & 0x3f) << 24 | 0xffffff` so that a change to the engine's keying
    // is a changed behaviour here rather than a silent divergence.
    let key = unsafe { key_from_area(area) };
    // SAFETY: `manager` is non-null, which is the one precondition this function does not check
    // for itself.
    let world = unsafe { world_from_manager(manager) };
    if world == 0 {
        return None;
    }
    // SAFETY: `world` is the engine's own `NvNaviGraphWorld`; the scan is bounded by the count
    // the object carries.
    let data = unsafe { data_for_key(world, key) };
    if data == 0 {
        // The area is mid-transition and its graph is not resident. Not an error.
        return None;
    }
    let point = Aligned4::point(position);
    // SAFETY: `data` is a live graph, `point` is sixteen aligned bytes this frame owns, and the
    // out-distance pointer is null -- which the function explicitly tests for, and which
    // `0x14037bef8` itself passes.
    let id = unsafe {
        nearest(
            data,
            point.as_ptr(),
            ds2_rva::NAVI_GRAPH_SNAP_RADIUS_METERS,
            ds2_rva::NAVI_GRAPH_SNAP_FILTER,
            std::ptr::null_mut(),
        )
    };
    (id != ds2_rva::NAVI_GRAPH_ID_NONE).then_some(id)
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
