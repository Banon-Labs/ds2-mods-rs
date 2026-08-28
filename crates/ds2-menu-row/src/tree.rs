//! Read the quit tab's LIVE component tree, because the file did not identify the banner.
//!
//! # Why this exists
//!
//! Three stretch factors were applied to the quit tab's panel record — 8%, 17%, and finally a
//! deliberately unmissable 2.0 — and the third one proved the point the first two only hinted at.
//! The log showed the write landing (`before=[0 -103 1 1 …]`, `after=[0 -103 1 2 …]`) on the record
//! whose position identifies it beyond doubt, and nothing moved on screen.
//!
//! So `0x1eac81` / definition `0x0221` is not what draws the background. Reading the `.flo` harder
//! was not going to establish that, because the file says what elements EXIST and not which of them
//! is the one behind the rows. The tree says.
//!
//! # What it walks, and why it starts above the container
//!
//! It resolves each prefix of the quit tab's own path and dumps that component's children:
//!
//! ```text
//! 0x1eaba9 / 0x1eaccf / 0x1eace8 / 0x1eace6
//! ```
//!
//! The banner may well be a SIBLING of the rows' container rather than a child of it, which is
//! exactly the shape the failed stretch is evidence for — so stopping at the container would repeat
//! the mistake one level up.
//!
//! For each component it logs the element id, the definition index and kind from its record, and
//! the transform bytes. **The transform offset is not the same for every class**:
//! `FeComponentObject` keeps its identity at `+0x60` and `FeComponentScene` at `+0x50` — they are
//! siblings under `FeComponentBase`, not parent and child — so the dump covers both and the logged
//! vtable says which to read. `scripts/ds2-rtti-vtables.py --owner <vtable>` turns that into a
//! class name offline.
//!
//! # It only reads
//!
//! Nothing here writes to the game. A run with this armed and everything else disarmed would leave
//! the menu exactly as it shipped.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// `fn(scene, ids, count) -> component`, null when the path resolves to nothing.
type FindByIdPathFn = unsafe extern "system" fn(*mut u8, *const u32, u32) -> *mut u8;
/// `fn(sceneProxy) -> scene`, vtable slot 1 of whatever a `SceneObjProxy` holds at `+0x58`.
type GetSceneFn = unsafe extern "system" fn(*mut usize) -> *mut u8;

/// How many trees have been dumped. One is the measurement; more is noise in a log that is read by
/// eye, and the tab is rebuilt every time the pause menu opens.
static DUMPS: AtomicUsize = AtomicUsize::new(0);

/// Most components to log, in case a subtree is larger than expected. A cap that is hit is said so
/// in the log rather than silently truncating, because a truncated dump that looks complete is how
/// "the banner is not in the tree" gets concluded from a tree that was cut short.
const MAX_LINES: usize = 400;

/// Depth to descend. The rows are two levels below the container, so four covers the neighbourhood
/// without dumping the whole menu.
const MAX_DEPTH: usize = 4;

/// The scene the accessor's proxy is rooted at.
///
/// Transcribed from `SceneObjProxy::resolve`'s first three instructions rather than reasoned about:
/// `mov rcx,[rcx+0x58]; mov rax,[rcx]; call [rax+8]`.
///
/// # Safety
///
/// `accessor` must be an accessor filled by [`ds2_rva::FE_BIND_SCENE_OBJ_PROXY`].
unsafe fn scene_of(accessor: *const u8) -> *mut u8 {
    // SAFETY: the caller guarantees a filled accessor, and this is the field its own resolve reads.
    let proxy = unsafe {
        accessor
            .add(ds2_rva::FE_SCENE_OBJ_PROXY_SCENE_OFFSET)
            .cast::<*mut usize>()
            .read()
    };
    if proxy.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: a scene proxy is polymorphic, so its first qword is its vtable.
    let vtable = unsafe { proxy.read() } as *const u8;
    if vtable.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: slot 1 is the one the game's own resolve calls.
    let get_scene: GetSceneFn = unsafe {
        std::mem::transmute::<usize, GetSceneFn>(
            vtable
                .add(ds2_rva::FE_SCENE_PROXY_GET_SCENE_SLOT)
                .cast::<usize>()
                .read(),
        )
    };
    // SAFETY: the proxy is the `this` this slot expects.
    unsafe { get_scene(proxy) }
}

/// The component's transform range as floats, for reading back offline.
///
/// Only ever called on a pointer that has passed [`looks_like_component`], because
/// `FE_COMPONENT_TRANSFORM_DUMP_END` is `0xa0` and a class smaller than that would be overread.
fn transform(component: *const u8) -> String {
    let span =
        ds2_rva::FE_COMPONENT_TRANSFORM_DUMP_END - ds2_rva::FE_COMPONENT_TRANSFORM_DUMP_START;
    // SAFETY: a component is `0xa0` bytes at minimum -- `FeComponentObject`'s own allocation size.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            component.add(ds2_rva::FE_COMPONENT_TRANSFORM_DUMP_START),
            span,
        )
    };
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| format!("{}", f32::from_le_bytes(*c)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a pointer is plausibly a component: readable, aligned, and carrying a vtable that lands
/// in the loaded image.
///
/// The first version of this walk had no such test, followed a leaf's `+0x38` into unrelated
/// memory and killed the game. A pointer that came out of a struct is a claim, not a fact.
fn looks_like_component(candidate: *const u8, base: usize) -> Option<usize> {
    if candidate.is_null()
        || (candidate as usize) < 0x1_0000
        || !(candidate as usize).is_multiple_of(8)
    {
        return None;
    }
    // SAFETY: non-null and aligned; a wild pointer here is exactly what this is filtering, and the
    // range test below is what makes the value meaningful rather than the read safe. The read
    // itself is the risk this cannot remove -- which is why the caller only ever passes pointers
    // taken from a class whose layout is known.
    let vtable = unsafe { candidate.cast::<usize>().read() };
    // The image is ~0x1d76000 bytes; a vtable lives in its `.rdata`.
    if vtable > base && vtable < base + 0x1d7_6000 {
        Some(vtable)
    } else {
        None
    }
}

/// Log one component and, up to `MAX_DEPTH`, its descendants.
///
/// # Safety
///
/// `component` must be a live component from the layout tree, and `base` the live module base.
unsafe fn walk(component: *const u8, depth: usize, lines: &mut usize, label: &str, base: usize) {
    if component.is_null() || depth > MAX_DEPTH || *lines >= MAX_LINES {
        return;
    }
    let Some(_) = looks_like_component(component, base) else {
        return;
    };
    *lines += 1;
    // SAFETY: the caller guarantees a live component; every offset here is one the game's own
    // `findByIdPath` and child walk read.
    let (vtable, record) = unsafe {
        (
            component.cast::<usize>().read(),
            component
                .add(ds2_rva::FE_COMPONENT_RECORD_OFFSET)
                .cast::<*const u8>()
                .read(),
        )
    };
    // A record is a pointer into the loaded `.flo`, not into the image, so it gets its own test:
    // plausible address and 4-aligned, or it is not read at all.
    let record_ok =
        !record.is_null() && (record as usize) >= 0x1_0000 && (record as usize).is_multiple_of(4);
    let (id, definition, kind) = if !record_ok {
        (0, 0, 0)
    } else {
        // SAFETY: a record is the `.flo`'s own 0x28-byte child record, still mapped.
        unsafe {
            (
                record
                    .add(ds2_rva::FLO_RECORD_ID_OFFSET)
                    .cast::<u32>()
                    .read(),
                record
                    .add(ds2_rva::FLO_RECORD_DEFINITION_OFFSET)
                    .cast::<u16>()
                    .read(),
                record
                    .add(ds2_rva::FLO_RECORD_KIND_OFFSET)
                    .cast::<u16>()
                    .read(),
            )
        }
    };
    log(format_args!(
        "{LOG_PREFIX} node {:>depth$}{label} at=0x{:016x} vtable=0x{vtable:x} id={id:#08x} \
         def={definition:#06x} kind={kind:#06x} xf=[{}]",
        "",
        component as usize,
        transform(component),
        depth = depth * 2
    ));

    // HOW A CLASS HOLDS ITS CHILDREN IS PER-CLASS, and getting this wrong is what killed the first
    // version of this walk. Three shapes, all read off the `findByIdPath` overrides:
    //
    //   FeComponentObject / FeComponentScene   `[this+0x38]` then `[child+0x28]`  (FUN_140b77dc0)
    //   FeComponentSprite                      the display list at `[this+0x70]`  (0x140b6bec0)
    //   everything else                        `xor eax,eax; ret` -- a leaf       (0x140b6d2a0)
    let linked = vtable == base + ds2_rva::FE_COMPONENT_OBJECT_VTABLE as usize
        || vtable == base + ds2_rva::FE_COMPONENT_SCENE_VTABLE as usize;
    let sprite = vtable == base + ds2_rva::FE_COMPONENT_SPRITE_VTABLE as usize;

    if sprite {
        // SAFETY: the class has been established, so these are its own display list and count.
        let (list, count) = unsafe {
            (
                component
                    .add(ds2_rva::FE_COMPONENT_DISPLAY_LIST_OFFSET)
                    .cast::<*const u8>()
                    .read(),
                component
                    .add(ds2_rva::FE_COMPONENT_DISPLAY_COUNT_OFFSET)
                    .cast::<u16>()
                    .read() as usize,
            )
        };
        if list.is_null() || count > 64 {
            return;
        }
        for i in 0..count {
            if *lines >= MAX_LINES {
                return;
            }
            // SAFETY: `i < count`, and the game's own search reads exactly these two fields at
            // exactly this stride.
            let (child, key) = unsafe {
                let entry = list.add(i * ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_STRIDE);
                (
                    entry
                        .add(ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_CHILD_OFFSET)
                        .cast::<*const u8>()
                        .read(),
                    entry
                        .add(ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_KEY_OFFSET)
                        .cast::<u32>()
                        .read(),
                )
            };
            if looks_like_component(child, base).is_none() {
                continue;
            }
            // SAFETY: `child` passed the component test.
            unsafe { walk(child, depth + 1, lines, &format!("key={key:#x} "), base) };
        }
        return;
    }

    if !linked {
        return;
    }
    // SAFETY: the class has been established to be one whose `+0x38` is the child list
    // `FUN_140b77dc0` walks.
    let mut child = unsafe {
        component
            .add(ds2_rva::FE_COMPONENT_FIRST_CHILD_OFFSET)
            .cast::<*const u8>()
            .read()
    };
    let mut seen = 0usize;
    while looks_like_component(child, base).is_some() && *lines < MAX_LINES && seen < 64 {
        seen += 1;
        // SAFETY: `child` passed the component test above.
        unsafe { walk(child, depth + 1, lines, "", base) };
        // SAFETY: same link the game's own sibling walk follows, on a validated component.
        child = unsafe {
            child
                .add(ds2_rva::FE_COMPONENT_NEXT_SIBLING_OFFSET)
                .cast::<*const u8>()
                .read()
        };
    }
}

/// Dump every prefix of the quit tab's path, once per process.
///
/// # Safety
///
/// `accessor` must be an accessor filled for a path under the quit tab, and `ids`/`count` the path
/// it was filled from.
pub unsafe fn dump(accessor: *const u8, ids: &[u32]) {
    if DUMPS.load(Ordering::Relaxed) > 0 {
        return;
    }
    // SAFETY: the caller guarantees a filled accessor.
    let scene = unsafe { scene_of(accessor) };
    if scene.is_null() {
        log(format_args!(
            "{LOG_PREFIX} tree REFUSED reason=no-scene -- the accessor's proxy yielded nothing"
        ));
        return;
    }
    DUMPS.fetch_add(1, Ordering::Relaxed);
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(_) => return,
    };
    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`.
    let find: FindByIdPathFn =
        unsafe { std::mem::transmute(base + ds2_rva::FE_SCENE_FIND_BY_ID_PATH as usize) };

    log(format_args!(
        "{LOG_PREFIX} tree scene=0x{:016x} path=[{}] -- every PREFIX is dumped, because the banner \
         may be a sibling of the rows' container rather than a child of it",
        scene as usize,
        ids.iter()
            .map(|i| format!("{i:#x}"))
            .collect::<Vec<_>>()
            .join(" ")
    ));

    let mut lines = 0usize;
    for depth in 1..=ids.len() {
        // The lookup wants a zero-terminated id array, the way the game's own resolve builds one.
        let mut path = [0u32; 8];
        path[..depth].copy_from_slice(&ids[..depth]);
        // SAFETY: `scene` is live and `path` holds `depth` ids followed by a zero.
        let component = unsafe { find(scene, path.as_ptr(), depth as u32) };
        if component.is_null() {
            log(format_args!(
                "{LOG_PREFIX} tree prefix={depth} resolved to NOTHING"
            ));
            continue;
        }
        let label = format!("prefix{depth} ");
        // SAFETY: `component` came from the game's own lookup.
        unsafe { walk(component, 0, &mut lines, &label, base) };
    }
    if lines >= MAX_LINES {
        log(format_args!(
            "{LOG_PREFIX} tree TRUNCATED at {MAX_LINES} nodes -- the dump is not the whole tree"
        ));
    } else {
        log(format_args!("{LOG_PREFIX} tree done nodes={lines}"));
    }
}
