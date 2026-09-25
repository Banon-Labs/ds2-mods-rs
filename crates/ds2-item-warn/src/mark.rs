//! The tenth child of the infusion container, which is the badge.
//!
//! # Why this hooks the builder and not the definition lookup
//!
//! `ds2-menu-row` owns [`ds2_rva::FLO_FIND_DEFINITION`] and MinHook binds one detour per address,
//! so a second raw hook on it would be silently dropped -- `MH_ERROR_ALREADY_CREATED` on whichever
//! crate installed second, which reports failure in a log nobody reads and looks on screen exactly
//! like a feature that does not work.
//!
//! [`ds2_rva::FLO_BUILD_CONTAINER`] is one level down and nothing else in this workspace touches
//! it. It takes the definition as an argument, walks `[def+0x02]` records out of `[def+0x08]`, and
//! hands the same pointer to the `FeComponentSprite` it allocates, which keeps it at `+0x48` as
//! the display list's capacity. That second half is what makes substituting the argument
//! equivalent to substituting the lookup's return rather than merely similar: it raises the walk
//! and the capacity together, for one container instead of for every consumer in the image.
//!
//! # What it refuses
//!
//! Everything that is not nine children carrying [`ds2_rva::FLO_INFUSION_CONTAINER_IDS`] in that
//! order. This detour sees every container every `.flo` in the game builds, so the check cannot be
//! a definition index: the same container is authored three times across the pause menu's three
//! documents under three different indices, and no other container in any of them carries these
//! nine ids. A refusal is silent by design -- it is the common case, thousands of times over.
//!
//! # Two things the badge depends on that are easy to get wrong
//!
//! **The badge wears the game's own ✕, and it can only do so because of what it was cloned from.**
//! The nine infusion glyphs sample `waku_03`, and so does the ✕ the game already draws on an
//! unusable quick-slot weapon. A `FeComponentTextureShape` resolves its texture at draw time out
//! of the shape-table entry it was built from, which is shared; its two rect arrays are
//! per-component copies, which are not. So [`crate::place`] can re-point one badge's source rect
//! at the ✕ and leave every other cell in the document alone -- and nothing here ships a texture
//! or tints anything. The cloned child is still checked for `kind & 4`, a nested definition,
//! because that is the subtree `place` walks to reach those arrays.
//!
//! **Draw order is attach order.** `FUN_140b6bd80` appends to the parent's display list at
//! `[parent+0x66]` with no sort, so a later record covers an earlier one. The badge is appended
//! after all nine, which is what puts it on top of an infusion glyph when a weapon has both.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// The game's own builder, published by MinHook before the site is patched.
pub(crate) static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

type BuildContainerFn = unsafe extern "system" fn(usize, usize, *mut u8, usize);

/// Whether a substitution may happen at all. Off until [`crate::install::install`] has hooked the
/// cell bind as well, because an element nothing can switch on is an element nobody sees.
static ARMED: AtomicBool = AtomicBool::new(false);

pub(crate) fn arm() {
    ARMED.store(true, Ordering::Release);
}

pub(crate) fn disarm() {
    ARMED.store(false, Ordering::Release);
}

/// Whether the badge element is being supplied, which is what [`crate::requirement`] asks before
/// it resolves anything.
pub(crate) fn armed() -> bool {
    ARMED.load(Ordering::Acquire)
}

/// How many children the container ships with, and how many it gets.
const SHIPPED_CHILDREN: usize = ds2_rva::FLO_INFUSION_CONTAINER_IDS.len();
const ADDED_CHILDREN: usize = SHIPPED_CHILDREN + 1;

/// The kind a nested-definition record carries, and the only kind whose transform colour is read.
const KIND_NESTED: u16 = 0x4;

/// A replacement container: the definition, its records, and the transform the added record points
/// at -- one allocation, so the pointers between them cannot outlive their targets.
#[repr(C, align(16))]
struct Container {
    definition: [u8; ds2_rva::FLO_DEFINITION_STRIDE],
    records: [u8; ds2_rva::FLO_RECORD_STRIDE * ADDED_CHILDREN],
    transform: [u8; ds2_rva::FLO_TRANSFORM_SIZE],
    /// The nine shipped records exactly as the game had them, so a cache hit can be checked rather
    /// than assumed. A document can be unloaded and another loaded at the same address.
    shipped: [u8; ds2_rva::FLO_RECORD_STRIDE * SHIPPED_CHILDREN],
}

/// Substitutions already built, as `(the definition the game passed, the one this passes on)`.
static BUILT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Most substitutions to keep. A document can be reloaded, and a build per reload with no ceiling
/// is a leak with a slow fuse -- the same bound `ds2-menu-row`'s shape substitution keeps.
const MAX_BUILT: usize = 64;

static SERVED: AtomicUsize = AtomicUsize::new(0);

/// Whether a record array is the infusion container's: nine records carrying nine known ids in the
/// order the file has them.
///
/// # Safety
///
/// `records` must be a live record array holding at least `count` records.
unsafe fn is_infusion_container(records: *const u8, count: usize) -> bool {
    if count != SHIPPED_CHILDREN {
        return false;
    }
    ds2_rva::FLO_INFUSION_CONTAINER_IDS
        .iter()
        .enumerate()
        .all(|(slot, want)| {
            // SAFETY: the count says this record is live, and the id is a `u32` inside it.
            let found = unsafe {
                records
                    .add(slot * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_ID_OFFSET)
                    .cast::<u32>()
                    .read_unaligned()
            };
            found == *want
        })
}

/// The child count and record array of a definition.
///
/// # Safety
///
/// `definition` must be a live definition.
unsafe fn children_of(definition: *const u8) -> (usize, *const u8) {
    // SAFETY: the caller guarantees a definition, and these are the two fields the builder reads.
    unsafe {
        (
            definition
                .add(ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET)
                .cast::<u16>()
                .read_unaligned() as usize,
            definition
                .add(ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET)
                .cast::<*const u8>()
                .read_unaligned(),
        )
    }
}

/// Read a `u16` out of a record in the local copy.
fn record_u16(records: &[u8], at: usize, offset: usize) -> u16 {
    u16::from_le_bytes([records[at + offset], records[at + offset + 1]])
}

/// Build the ten-child container out of the nine-child one the game is about to walk.
///
/// # Safety
///
/// `definition` must be the live definition the game passed, and `records` its record array with
/// [`SHIPPED_CHILDREN`] live records in it.
unsafe fn build(definition: *const u8, records: *const u8) -> Option<*mut u8> {
    let cloned = ds2_rva::FE_ITEM_WARN_CLONED_CHILD * ds2_rva::FLO_RECORD_STRIDE;
    // SAFETY: the caller established nine live records, and the clone index is inside them.
    let source_transform = unsafe {
        records
            .add(cloned + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET)
            .cast::<*const u8>()
            .read_unaligned()
    };
    if (source_transform as usize) < 0x1_0000 {
        log(format_args!(
            "{LOG_PREFIX} badge REFUSED reason=transform-not-a-pointer at=0x{:016x}",
            source_transform as usize
        ));
        return None;
    }

    let mut built = Box::new(Container {
        definition: [0; ds2_rva::FLO_DEFINITION_STRIDE],
        records: [0; ds2_rva::FLO_RECORD_STRIDE * ADDED_CHILDREN],
        transform: [0; ds2_rva::FLO_TRANSFORM_SIZE],
        // SAFETY: the caller established this many records are live.
        shipped: unsafe {
            std::ptr::read_unaligned(
                records.cast::<[u8; ds2_rva::FLO_RECORD_STRIDE * SHIPPED_CHILDREN]>(),
            )
        },
    });

    // The kind is checked before anything is copied. Nothing here writes a colour any more, but
    // the kind is still what says this record wraps a definition holding one texture shape --
    // which is the subtree `crate::place` walks to find the rect arrays. A `kind & 1` record has
    // no definition under it and that walk would have nothing to find. See this module's header.
    let kind = record_u16(&built.shipped, cloned, ds2_rva::FLO_RECORD_KIND_OFFSET);
    if kind & KIND_NESTED == 0 {
        log(format_args!(
            "{LOG_PREFIX} badge REFUSED reason=cloned-child-is-not-a-nested-definition \
             kind={kind:#x} -- its transform colour would never be read"
        ));
        return None;
    }

    // SAFETY: a definition is `FLO_DEFINITION_STRIDE` bytes, which is what is being copied.
    built.definition = unsafe {
        std::ptr::read_unaligned(definition.cast::<[u8; ds2_rva::FLO_DEFINITION_STRIDE]>())
    };
    built.records[..ds2_rva::FLO_RECORD_STRIDE * SHIPPED_CHILDREN].copy_from_slice(&built.shipped);
    // SAFETY: as above, for the transform block the cloned record points at.
    built.transform = unsafe {
        std::ptr::read_unaligned(source_transform.cast::<[u8; ds2_rva::FLO_TRANSFORM_SIZE]>())
    };

    // The added record is a copy, not a record assembled field by field. `ds2-menu-row` learned
    // that on the namer's entries: the bytes a record carries that nothing here has decoded are
    // still load-bearing, and the only way to keep them right is to not touch them.
    let added = SHIPPED_CHILDREN * ds2_rva::FLO_RECORD_STRIDE;
    built
        .records
        .copy_within(cloned..cloned + ds2_rva::FLO_RECORD_STRIDE, added);

    // Two fields change on the record itself, and nothing else.
    built.records[added + ds2_rva::FLO_RECORD_ID_OFFSET..][..4]
        .copy_from_slice(&ds2_rva::FE_ITEM_WARN_ELEMENT.to_le_bytes());
    let last = (SHIPPED_CHILDREN - 1) * ds2_rva::FLO_RECORD_STRIDE;
    let depth = record_u16(&built.shipped, last, ds2_rva::FLO_RECORD_DEPTH_OFFSET)
        .saturating_add(ds2_rva::FE_ITEM_WARN_DEPTH_STEP);
    built.records[added + ds2_rva::FLO_RECORD_DEPTH_OFFSET..][..2]
        .copy_from_slice(&depth.to_le_bytes());

    // The transform: the corner, and the colour that makes the badge mean "no".
    let was = [
        f32::from_le_bytes(
            built.transform[ds2_rva::FLO_TRANSFORM_X_OFFSET..][..4]
                .try_into()
                .ok()?,
        ),
        f32::from_le_bytes(
            built.transform[ds2_rva::FLO_TRANSFORM_Y_OFFSET..][..4]
                .try_into()
                .ok()?,
        ),
    ];
    // Nothing is written into the transform block, and the `was` above is read only so the log can
    // say what the clone started from. The block is still copied, because the added record must not
    // share one with the glyph it was cloned from -- a write to either would otherwise land on
    // both -- but there is no longer anything to write:
    //
    // * The corner and the flip went in here once and neither reached the art. A calibration run
    //   set the corner to `-1000.0`, thirteen cell widths against a grid whose pitch is `72.5`,
    //   and every badge stayed where it was. `FE_TEXTURE_SHAPE_INIT` copies a shape's quad into
    //   the component at build time and never re-derives it from an ancestor's transform -- the
    //   same wall `ds2-menu-row` hit on the quit tab's banner, and why `FLO_PANEL_STRETCH_Y` is
    //   `1.0`. `crate::place` writes the component's own destination rect instead.
    // * The tint did reach the art, and is gone because the art no longer needs it. The badge used
    //   to be an infusion arrow multiplied down to its red channel; it is now the game's own X,
    //   cropped out of the same atlas by `crate::place`, and that art is already `rgb(181, 44,
    //   16)`. Multiplying a red glyph by red would only darken it.

    // The pointers last, so nothing in the struct is addressed before it holds what it should.
    let transform = built.transform.as_ptr() as u64;
    built.records[added + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET..][..8]
        .copy_from_slice(&transform.to_le_bytes());
    let records_at = built.records.as_ptr() as u64;
    built.definition[ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET..][..8]
        .copy_from_slice(&records_at.to_le_bytes());
    built.definition[ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET..][..2]
        .copy_from_slice(&(ADDED_CHILDREN as u16).to_le_bytes());

    let leaked: &'static mut Container = Box::leak(built);
    log(format_args!(
        "{LOG_PREFIX} badge built id={:#x} children={SHIPPED_CHILDREN}->{ADDED_CHILDREN} \
         cloned=child{} kind={kind:#x} depth={depth} clone-was-at=({:.2},{:.2}) \
         -- the corner and the art are `place`, not this block",
        ds2_rva::FE_ITEM_WARN_ELEMENT,
        ds2_rva::FE_ITEM_WARN_CLONED_CHILD,
        was[0],
        was[1],
    ));
    Some((&raw mut leaked.definition).cast::<u8>())
}

/// Whether a cached substitution still describes the container the game is holding.
///
/// # Safety
///
/// `records` must be a live record array of [`SHIPPED_CHILDREN`] records and `cached` a
/// [`Container`] this module built from it.
unsafe fn still_current(records: *const u8, cached: *const Container) -> bool {
    const BYTES: usize = ds2_rva::FLO_RECORD_STRIDE * SHIPPED_CHILDREN;
    // SAFETY: the caller guarantees both are live and this many bytes long.
    unsafe {
        std::slice::from_raw_parts(records, BYTES)
            == std::slice::from_raw_parts((*cached).shipped.as_ptr(), BYTES)
    }
}

pub(crate) unsafe extern "system" fn detour(
    ctx: usize,
    doc: usize,
    definition: *mut u8,
    parent: usize,
) {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Published before the site is patched, so unreachable. Doing nothing is the only honest
        // option left -- there is no original to call.
        return;
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
    // one the decompiled entry implements: four integer arguments, no return.
    let original: BuildContainerFn =
        unsafe { std::mem::transmute::<usize, BuildContainerFn>(trampoline) };

    let substitute = substitution(definition);
    // SAFETY: every argument is the game's own; only `definition` may have been replaced, and then
    // only by a `Container` this module built by copying the one it replaces.
    // SAFETY: `original` is the trampoline MinHook produced for this target, so calling it runs the
    // bytes the detour displaced. The arguments are this detour's own, passed through untouched.
    unsafe { original(ctx, doc, substitute.unwrap_or(definition), parent) }
}

/// The replacement for `definition`, or `None` for the overwhelming majority of containers.
fn substitution(definition: *mut u8) -> Option<*mut u8> {
    if !armed() || (definition as usize) < 0x1_0000 {
        return None;
    }
    // SAFETY: the game passes a live definition to its own builder, and `children_of` reads the
    // two fields that builder reads first.
    let (count, records) = unsafe { children_of(definition) };
    if (records as usize) < 0x1_0000 {
        return None;
    }
    // SAFETY: `records` is the array the builder is about to walk `count` records of.
    if !unsafe { is_infusion_container(records, count) } {
        return None;
    }

    let mut built = BUILT.lock().ok()?;
    if let Some(slot) = built
        .iter()
        .position(|(key, _)| *key == definition as usize)
    {
        let cached = built[slot].1 as *mut Container;
        // SAFETY: `cached` is a leaked `Container` this module built from these records.
        if unsafe { still_current(records, cached) } {
            let n = SERVED.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 2 {
                log(format_args!(
                    "{LOG_PREFIX} badge served definition=0x{:016x} served={n}",
                    definition as usize
                ));
            }
            // SAFETY: as above; a `Container` begins with its definition.
            return Some(unsafe { (&raw mut (*cached).definition).cast::<u8>() });
        }
        log(format_args!(
            "{LOG_PREFIX} badge stale definition=0x{:016x} -- the document was reloaded, \
             rebuilding",
            definition as usize
        ));
        built.remove(slot);
    }
    if built.len() >= MAX_BUILT {
        log(format_args!(
            "{LOG_PREFIX} badge REFUSED reason=too-many-documents built={MAX_BUILT}"
        ));
        return None;
    }
    // SAFETY: the fingerprint above established nine live records carrying the container's ids.
    let replacement = unsafe { build(definition, records) }?;
    built.push((definition as usize, replacement as usize));
    Some(replacement)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The badge's slot is one the game's own loop drives and no element answers.
    #[test]
    fn the_badge_slot_is_driven_and_unauthored() {
        let base = ds2_rva::FE_ITEM_CELL_INFUSION_ELEMENT_BASE;
        let slot = ds2_rva::FE_ITEM_WARN_ELEMENT - base;
        assert!(
            slot < ds2_rva::FE_ITEM_CELL_INFUSION_SLOTS,
            "the bind loop never asks for this id, so nothing would ever hide it"
        );
        assert!(
            !ds2_rva::FLO_INFUSION_CONTAINER_IDS.contains(&ds2_rva::FE_ITEM_WARN_ELEMENT),
            "the badge would be an infusion glyph's id and would flicker with the infusion"
        );
        assert!(
            slot as usize >= ds2_rva::FLO_INFUSION_CONTAINER_IDS.len(),
            "the badge sits in a slot the shipped layout authors, so it would replace a glyph"
        );
        // The slot is inside the nibble's own range -- `0..=15` masked, `15` here -- so an entry
        // carrying a nibble no infusion produces would make the game's own loop show the badge.
        // That is why `crate::requirement`'s detour writes the element's visibility on EVERY bind
        // rather than only when it wants it on: its write runs after the loop and overrules it.
        // This assertion is the reminder, not a guard, and it is deliberately the weaker claim.
        assert!(slot as u8 <= ds2_rva::ITEM_ENTRY_INFUSION_MASK);
    }

    /// The offset really does land the badge in the cell's bottom-left corner.
    ///
    /// **Bounded by the tile and the bar, not by the icon's art box**, which is the correction the
    /// wine pass caught. This test asserted `x >= FE_ITEM_ICON_BOX[0]` while the badge's left edge
    /// is deliberately [`ds2_rva::FE_ITEM_CELL_BAR_LEFT`] -- `9.45` against a box starting at
    /// `15.25` -- so it contradicted the anchor its own crate documents and the sibling assertion
    /// `the_bar_margin_is_left_of_the_icon_box` exists to pin. It failed only under
    /// `scripts/check.sh --host-tests`, which is the pass that runs these under wine, so it sat
    /// red through a feature that was correct on screen.
    ///
    /// The bounds here are the ones `ds2-rva`'s `the_corner_is_inside_the_icon` uses: the cell's
    /// own parchment, and the durability bar as the real bottom.
    #[test]
    fn the_badge_lands_in_the_bottom_left_of_the_cell() {
        /// The cell background quad, offset plus rect, from `ds2-flo.py shape --shape 0x4d`.
        const TILE: [f32; 4] = [2.45, -5.85, 97.30, 85.00];
        let x = ds2_rva::FE_ITEM_INFUSION_CONTAINER_AT[0] + ds2_rva::FE_ITEM_WARN_OFFSET[0];
        let y = ds2_rva::FE_ITEM_INFUSION_CONTAINER_AT[1] + ds2_rva::FE_ITEM_WARN_OFFSET[1];
        let [left, top, right, _bottom] = TILE;
        assert!(x >= left && x + ds2_rva::FE_ITEM_WARN_SIZE[0] <= right);
        assert!(y >= top && y + ds2_rva::FE_ITEM_WARN_SIZE[1] <= ds2_rva::FE_ITEM_CELL_BAR_TOP);
        // Bottom-left: in the left half of the tile and the lower half of the portrait, `+y` being
        // downwards.
        assert!(x < (left + right) / 2.0);
        assert!(y > (top + ds2_rva::FE_ITEM_CELL_BAR_TOP) / 2.0);
        // And on the icon it is marking, which is the claim the old bound was reaching for: the
        // badge's right edge is inside the icon's art even though its left edge is not.
        assert!(
            x + ds2_rva::FE_ITEM_WARN_SIZE[0] > ds2_rva::FE_ITEM_ICON_BOX[0],
            "the mark has to overlap the portrait or it is marking the parchment"
        );
    }

    /// The badge is the game's ✕ and not the glyph it is cloned from, which is a rect apart.
    ///
    /// The clone exists for the texture the two share; if these ever became the same rect the
    /// badge would silently go back to being an infusion arrow, drawn in the wrong corner and
    /// meaning the wrong thing.
    #[test]
    fn the_badge_does_not_wear_the_glyph_it_was_cloned_from() {
        assert_ne!(
            ds2_rva::FE_ITEM_WARN_SOURCE,
            ds2_rva::FE_ITEM_WARN_SHIPPED_SOURCE
        );
    }

    /// The added record is appended, so the shipped nine keep the slots they had.
    #[test]
    fn the_shipped_children_keep_their_slots() {
        assert_eq!(ADDED_CHILDREN, SHIPPED_CHILDREN + 1);
        assert_eq!(SHIPPED_CHILDREN, ds2_rva::FLO_INFUSION_CONTAINER_IDS.len());
        const {
            assert!(
                ds2_rva::FE_ITEM_WARN_CLONED_CHILD < SHIPPED_CHILDREN,
                "the record cloned for the badge would be read past the end of the array"
            )
        };
    }

    /// A `Container` begins with its definition, because the detour passes its address on as one.
    #[test]
    fn the_definition_is_at_the_front_of_the_allocation() {
        let built = Container {
            definition: [0; ds2_rva::FLO_DEFINITION_STRIDE],
            records: [0; ds2_rva::FLO_RECORD_STRIDE * ADDED_CHILDREN],
            transform: [0; ds2_rva::FLO_TRANSFORM_SIZE],
            shipped: [0; ds2_rva::FLO_RECORD_STRIDE * SHIPPED_CHILDREN],
        };
        assert_eq!(
            (&raw const built.definition).cast::<u8>(),
            (&raw const built).cast::<u8>()
        );
    }
}
