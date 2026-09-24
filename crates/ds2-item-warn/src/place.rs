//! Where the badge is drawn, which is not where its record says.
//!
//! # A record's `xy` does not reach this art, and a run proved it
//!
//! [`crate::mark`] writes the corner into the cloned record's transform block, and that block is
//! live: the tint written four bytes further into it is on screen, and a read-back off the built
//! component's own record returns `xy` exactly as written. It still does nothing. A calibration run
//! set the offset to `-1000.0` -- thirteen cell widths, against a grid whose pitch is `72.5`
//! (`def 0x0079`) -- and every badge stayed in the corner it was already in.
//!
//! That is the same wall `ds2-menu-row` hit on the quit tab's banner, and its note says why:
//! `FE_TEXTURE_SHAPE_INIT` copies a shape's quad into the `FeComponentTextureShape` at build time
//! and never re-derives it from an ancestor's transform. [`ds2_rva::FLO_PANEL_STRETCH_Y`] is `1.0`
//! for that reason -- a scale written one level up changed nothing there either.
//!
//! So the quad is the field, and the copy is per component. That is what makes this possible at
//! all: the badge and the infusion glyph it was cloned from share one shape-table entry, so moving
//! the entry would move both, while moving the badge's own copy moves only the badge.
//!
//! # Why the write is absolute and not a nudge
//!
//! This runs on every cell bind, and a relative shift applied on every bind walks the badge off the
//! screen in about a second. The target is computed from the SOURCE rect instead -- which nothing
//! here ever writes -- so re-applying it writes the same four floats and the badge cannot drift.
//!
//! # The flip is the rect, not the scale
//!
//! [`ds2_rva::FE_ITEM_WARN_SCALE_Y`] is written into the transform for the same reason the position
//! was, and reaches the art for the same reason -- it does not. A destination rect whose bottom
//! edge is above its top edge mirrors the art it samples, so the upside-down arrow is `y1 < y0`
//! here rather than a negative scale one level up.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// How many placements to log before going quiet. The bind runs per visible row per refresh.
const LOGGED: usize = 4;

static PLACED: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);

/// A pointer this crate is willing to follow.
fn sane(at: usize) -> bool {
    at >= 0x1_0000 && at.is_multiple_of(8)
}

/// The `FeComponentTextureShape` drawing one component's art.
///
/// The `+0x38` child is one of two things, and which one depends on the definition rather than on
/// anything this crate controls -- so both are handled and neither is assumed. Either the child is
/// the `FeComponentTextureShape` itself, which is the badge, or it is the `FeComponentSprite` that
/// owns a display list holding the shape under
/// [`ds2_rva::FE_TEXTURE_SHAPE_DISPLAY_KEY`], which is the quit tab's panel that `ds2-menu-row`
/// walks. Every vtable is checked before anything is followed, because `FUN_140b77dc0` taking
/// `+0x38` on a class where it is not a child link cost that crate a crash.
///
/// # Safety
///
/// `component` must be a live component and `base` the module base.
unsafe fn texture_shape_of(component: usize, base: usize) -> usize {
    if !sane(component) {
        return 0;
    }
    // SAFETY: a validated component; `+0x38` is its child link.
    let child = unsafe { read_usize(component + ds2_rva::FE_COMPONENT_FIRST_CHILD_OFFSET) };
    if !sane(child) {
        return 0;
    }
    // SAFETY: validated pointer; its first qword is its vtable.
    let class = unsafe { read_usize(child) };
    // The badge's shape hangs off it DIRECTLY, with no `FeComponentSprite` in between, and a walk
    // that insisted on one refused four binds in a row. The reason is the builder's fast path:
    // `def 0x005a` has a single child whose record is trivial by `FUN_140b50f20`'s own test -- id
    // `0`, `xy` `(0,0)`, unit scale, opaque white -- so the walk takes the inline branch, and
    // `FUN_140b50950` hands the shape straight to `FUN_140b51270`, which allocates
    // `FeComponentTextureShape` (`FUN_140b6ef80` writes that vftable) and attaches it to the badge
    // through vtable `+0x158`. A definition whose children are NOT all trivial gets the sprite
    // instead, which is the shape `ds2-menu-row` met on the quit tab's panel -- so both are
    // accepted here rather than one being called the right one.
    if class == base + ds2_rva::FE_COMPONENT_TEXTURE_SHAPE_VTABLE as usize {
        return child;
    }
    let sprite = child;
    if class != base + ds2_rva::FE_COMPONENT_SPRITE_VTABLE as usize {
        // SAFETY: as above -- this only re-reads the qword the check just rejected.
        unsafe { describe(component, sprite, base) };
        return 0;
    }
    // SAFETY: the class is established, so these are its own display list and live count.
    let (list, count) = unsafe {
        (
            read_usize(sprite + ds2_rva::FE_COMPONENT_DISPLAY_LIST_OFFSET),
            ((sprite + ds2_rva::FE_COMPONENT_DISPLAY_COUNT_OFFSET) as *const u16).read_unaligned()
                as usize,
        )
    };
    if !sane(list) || count > 64 {
        return 0;
    }
    for slot in 0..count {
        // SAFETY: `slot < count`, at the stride the game's own search uses.
        let (child, key) = unsafe {
            let entry = list + slot * ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_STRIDE;
            (
                read_usize(entry + ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_CHILD_OFFSET),
                ((entry + ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_KEY_OFFSET) as *const u32)
                    .read_unaligned(),
            )
        };
        if key != ds2_rva::FE_TEXTURE_SHAPE_DISPLAY_KEY || !sane(child) {
            continue;
        }
        // SAFETY: validated pointer.
        if unsafe { read_usize(child) }
            == base + ds2_rva::FE_COMPONENT_TEXTURE_SHAPE_VTABLE as usize
        {
            return child;
        }
    }
    0
}

/// How many times the class report has been written.
static DESCRIBED: AtomicUsize = AtomicUsize::new(0);

/// Name the classes the walk actually found, as vtable RVAs.
///
/// The walk was `ds2-menu-row`'s, written for the quit tab's panel, and the badge's subtree is not
/// shaped the same way -- its `+0x38` child exists and is not a `FeComponentSprite`. Guessing which
/// class it is instead is how the last three runs were spent, so this prints the RVA: it goes
/// straight into `scripts/ds2-rtti-vtables.py 'FeComponent'` and comes back as a name.
///
/// # Safety
///
/// `component` and `child` must be live, and `base` the module base.
unsafe fn describe(component: usize, child: usize, base: usize) {
    if DESCRIBED.fetch_add(1, Ordering::Relaxed) >= LOGGED {
        return;
    }
    let rva = |at: usize| -> usize {
        if !sane(at) {
            return 0;
        }
        // SAFETY: a validated pointer's first qword is its vtable.
        let vtable = unsafe { read_usize(at) };
        vtable.wrapping_sub(base)
    };
    // SAFETY: the caller guarantees both are live; every hop is checked by `sane` first.
    let (grandchild, sibling) = unsafe {
        (
            read_usize(child + ds2_rva::FE_COMPONENT_FIRST_CHILD_OFFSET),
            read_usize(child + ds2_rva::FE_COMPONENT_NEXT_SIBLING_OFFSET),
        )
    };
    log(format_args!(
        "{LOG_PREFIX} badge CLASSES badge=0x{:08x} child=0x{:08x} grandchild=0x{:08x} \
         sibling=0x{:08x} -- want sprite=0x{:08x} shape=0x{:08x}",
        rva(component),
        rva(child),
        rva(grandchild),
        rva(sibling),
        ds2_rva::FE_COMPONENT_SPRITE_VTABLE,
        ds2_rva::FE_COMPONENT_TEXTURE_SHAPE_VTABLE,
    ));
}

/// Read a pointer-sized field.
///
/// # Safety
///
/// `at` must be a live, readable address.
unsafe fn read_usize(at: usize) -> usize {
    // SAFETY: the caller guarantees the address is live.
    unsafe { (at as *const usize).read_unaligned() }
}

/// Put the badge in the icon's bottom-left corner, upside down.
///
/// Silent on success after the first few, and silent on refusal in the same way every other check
/// in this crate is: the badge stays where the shipped art put it, which is the corner it was
/// already in rather than anything broken.
///
/// # Safety
///
/// `component` must be the live component the badge's element id resolved to, and `base` the
/// module base.
pub(crate) unsafe fn place(component: usize, base: usize) {
    let refuse = |why: std::fmt::Arguments<'_>| {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= LOGGED {
            log(format_args!(
                "{LOG_PREFIX} badge NOT PLACED {why} refusals={n} -- it keeps the corner the \
                 shipped art gave it"
            ));
        }
    };
    // SAFETY: the caller guarantees a live component.
    let shape = unsafe { texture_shape_of(component, base) };
    if shape == 0 {
        refuse(format_args!("no texture shape under the badge"));
        return;
    }
    // SAFETY: `shape` is a validated `FeComponentTextureShape`; this is the field its own
    // initialiser reads its quad count from.
    let entry = unsafe { read_usize(shape + ds2_rva::FE_TEXTURE_SHAPE_ENTRY_OFFSET) };
    if !sane(entry) {
        refuse(format_args!("the shape has no table entry"));
        return;
    }
    // THE CHECK. The arrow the badge is cloned from is one quad. A shape with a different count is
    // a different picture, and writing into it would be a guess wearing a measurement's clothes.
    // SAFETY: a shape table entry, still mapped in the loaded document.
    let quads =
        unsafe { ((entry + ds2_rva::FE_SHAPE_ENTRY_COUNT_OFFSET) as *const u16).read_unaligned() };
    if quads != 1 {
        refuse(format_args!("quads={quads}, expected 1"));
        return;
    }
    // SAFETY: the count is 1, so element 0 of each array is inside it.
    let (destination, source) = unsafe {
        (
            read_usize(shape + ds2_rva::FE_TEXTURE_SHAPE_DEST_RECT_OFFSET),
            read_usize(shape + ds2_rva::FE_TEXTURE_SHAPE_SOURCE_RECT_OFFSET),
        )
    };
    if !sane(destination) || !sane(source) {
        refuse(format_args!("a rect array is null"));
        return;
    }

    // The target is derived from the SOURCE rect, which nothing here ever writes, so running this
    // on every bind writes the same four floats instead of nudging the badge a little further each
    // time. `FUN_140b70200` fills both arrays from the same quad field (`quad+0x30`), so an
    // untouched shape has `destination == source` -- and the art still lands at the record's origin
    // because the quad's own offset, `(-934.70, -52.50)` against a rect starting at
    // `(934.70, 52.50)`, cancels it in the per-quad matrix at `+0x48`. Shifting the destination by
    // `FE_ITEM_WARN_OFFSET` therefore moves the art by exactly that, in the container's own space,
    // which is the space that constant is measured in.
    //
    // The two `y` edges are swapped, which is what mirrors the arrow. That is the flip
    // `FE_ITEM_WARN_SCALE_Y` was written into the transform for and never delivered.
    let [x, y] = ds2_rva::FE_ITEM_WARN_OFFSET;
    // SAFETY: a live four-float rect, as the quad count established.
    let from = unsafe { std::slice::from_raw_parts(source as *const f32, 4) };
    let want = [from[0] + x, from[3] + y, from[2] + x, from[1] + y];
    // SAFETY: as above.
    let before = unsafe { std::slice::from_raw_parts(destination as *const f32, 4) };
    if before == want {
        return;
    }
    let was = [before[0], before[1], before[2], before[3]];
    // SAFETY: as above, and this is the array the shape's own draw reads its destination from.
    unsafe { std::slice::from_raw_parts_mut(destination as *mut f32, 4).copy_from_slice(&want) };
    let n = PLACED.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= LOGGED {
        log(format_args!(
            "{LOG_PREFIX} badge placed shape=0x{shape:016x} rect={was:.2?} -> {want:.2?} \
             placements={n}"
        ));
    }
}

// The geometry this writes is asserted in `ds2-rva`'s own tests rather than here. This module is
// `#[cfg(windows)]`, so a test in it compiles nowhere on the host this is developed on and reads
// like coverage that does not exist.
