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
//! **The tint only works because the cloned record names a definition.** `FUN_140b50bc0`
//! dispatches on the record's kind, and a record naming a shape (`kind & 1`) goes to
//! `FUN_140b70200`, which never sees the record or its transform -- a shape's colour lives in four
//! bytes inside each quad instead. The nine infusion children are all `kind = 0x4`, nested
//! definitions, so their transform colour is read; [`build`] checks that byte anyway rather than
//! trusting the file it was read from.
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

    // The kind is checked before anything is copied, because the colour is the whole badge and a
    // record naming a shape would take it nowhere. See this module's header.
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
    // The corner and the flip are NOT written here, and the `was` above is read only so the log
    // can say what the clone started from. Both were written into this block once and neither
    // reached the art: a calibration run set the corner to `-1000.0`, thirteen cell widths against
    // a grid whose pitch is `72.5`, and every badge stayed where it was. `FE_TEXTURE_SHAPE_INIT`
    // copies a shape's quad into the component at build time and never re-derives it from an
    // ancestor's transform, which is the same wall `ds2-menu-row` hit on the quit tab's banner and
    // why `FLO_PANEL_STRETCH_Y` is `1.0`. [`crate::place`] writes the destination rect instead.
    //
    // The colour below is a different field of the same block and does reach the art, which is
    // what makes the badge red and is why this block is copied at all.

    built.transform[ds2_rva::FLO_TRANSFORM_COLOUR_OFFSET..][..4]
        .copy_from_slice(&ds2_rva::FE_ITEM_WARN_TINT);
    // The colour is inert without these two bits, which cost `ds2-menu-row` a whole run to find: a
    // tint written into a transform block whose flag word is `0` produces the shipped colour and
    // no diagnostic anywhere. `0x10` is "the colour word is live" and `0x100` is "and its RGB is
    // not white"; the game's own greyed-out quit glyph carries exactly `0x110`.
    let flags = u32::from_le_bytes(
        built.transform[ds2_rva::FLO_TRANSFORM_FLAGS_OFFSET..][..4]
            .try_into()
            .ok()?,
    ) | ds2_rva::FLO_TRANSFORM_COLOUR_LIVE
        | ds2_rva::FLO_TRANSFORM_COLOUR_RGB;
    built.transform[ds2_rva::FLO_TRANSFORM_FLAGS_OFFSET..][..4]
        .copy_from_slice(&flags.to_le_bytes());

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
         cloned=child{} kind={kind:#x} depth={depth} clone-was-at=({:.2},{:.2}) tint={:02x?} \
         -- the corner is `place`, not this block",
        ds2_rva::FE_ITEM_WARN_ELEMENT,
        ds2_rva::FE_ITEM_WARN_CLONED_CHILD,
        was[0],
        was[1],
        ds2_rva::FE_ITEM_WARN_TINT,
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

    /// The offset really does land the badge in the icon's bottom-left corner.
    #[test]
    fn the_badge_lands_in_the_bottom_left_of_the_icon() {
        let x = ds2_rva::FE_ITEM_INFUSION_CONTAINER_AT[0] + ds2_rva::FE_ITEM_WARN_OFFSET[0];
        let y = ds2_rva::FE_ITEM_INFUSION_CONTAINER_AT[1] + ds2_rva::FE_ITEM_WARN_OFFSET[1];
        let [left, top, right, bottom] = ds2_rva::FE_ITEM_ICON_BOX;
        assert!(x >= left && x + ds2_rva::FE_ITEM_WARN_SIZE[0] <= right);
        assert!(y >= top && y + ds2_rva::FE_ITEM_WARN_SIZE[1] <= bottom);
        // Bottom-left: in the left half and the lower half of the icon, `+y` being downwards.
        assert!(x < (left + right) / 2.0);
        assert!(y > (top + bottom) / 2.0);
    }

    /// The tint is opaque, is not white, and reads red in the byte order the transform uses.
    #[test]
    fn the_tint_is_opaque_and_red() {
        assert_eq!(ds2_rva::FE_ITEM_WARN_TINT[ds2_rva::FLO_TINT_ALPHA], 0xff);
        assert_ne!(&ds2_rva::FE_ITEM_WARN_TINT[..3], &[0xff, 0xff, 0xff]);
        assert!(
            ds2_rva::FE_ITEM_WARN_TINT[0] > ds2_rva::FE_ITEM_WARN_TINT[1],
            "R is the first byte in memory order, and this badge is red"
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
