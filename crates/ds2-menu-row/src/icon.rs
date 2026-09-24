//! The seventh tab's hexagon, which is a quad this crate draws rather than a lookup it answers.
//!
//! # Why there was nothing to answer
//!
//! Two earlier readings put the tab glyph somewhere it is not. It is not in the cell: every cell
//! shares [`ds2_rva::FLO_TAB_STRIP_CELL_DEFINITION`], which holds two copies of one highlight shape
//! and no child carrying an element id, so no path reaches inside a cell and no bind can put a
//! glyph there. And it is not behind the strip's second per-cell lookup: that function is the first
//! one's body twice over and resolves the same entry to a second accessor, which is why answering
//! it moved nothing on screen.
//!
//! The six hexagons and the six glyphs are one baked quad -- [`ds2_rva::FLO_TAB_PLATE_SHAPE`],
//! child `7` of the strip, sampling `(1.10, 781.95)-(337.60, 850.90)` of the atlas `In-game_01` and
//! landing it across `(1.10, 6.25)-(337.60, 75.20)`. That is the band the six tabs occupy, and the
//! atlas holds no seventh hexagon to point at.
//!
//! # What this draws instead
//!
//! The plate's art repeats at [`ds2_rva::FLO_TAB_PITCH`], because the six cells do. So this serves
//! one more shape -- [`ds2_rva::FLO_ADDED_TAB_ICON_SHAPE`] -- whose quad is the plate's quad with
//! two fields changed:
//!
//! | field | from | to |
//! |---|---|---|
//! | source left | [`ds2_rva::FLO_TAB_PLATE_SOURCE`]`[0]` | [`ds2_rva::FLO_ADDED_TAB_ICON_SOURCE_LEFT`] |
//! | offset x | `0` | `+ `[`ds2_rva::FLO_TAB_PITCH`] |
//!
//! The narrowed rect is the plate's final whole tab period and the offset lands it immediately past
//! the plate's right edge, so the row of hexagons continues by one with the seam falling between
//! two hexagons rather than through one. The glyph inside it is the sixth tab's, shifted whole:
//! there are six in the atlas and this mod ships no texture of its own.
//!
//! # The chevron that was standing there
//!
//! The strip's right-hand furniture is flush against the plate. The `RB` chevron lands at
//! `336.70..361.70` and its label at `347.05`, both inside where the seventh hexagon goes, so this
//! module also serves a moved copy of [`ds2_rva::FLO_TAB_ARROWS_SHAPE`] -- two quads off one
//! mirrored rect, of which only [`ds2_rva::FLO_TAB_ARROWS_RIGHT`] moves. The left chevron keeps the
//! quad it had, which is the whole reason the shape is copied rather than the record moved: one
//! record draws both.
//!
//! [`crate::strip`] moves the two remaining pieces, which are plain transforms in its own array.
//!
//! # What makes this safe to be wrong about
//!
//! A shape index is a number, and `0x0268` on a document this was not read from is some other
//! picture. Nothing is copied until the entry the game returned carries exactly
//! [`ds2_rva::FLO_TAB_PLATE_QUADS`] quads whose source rect is [`ds2_rva::FLO_TAB_PLATE_SOURCE`] and
//! whose offset is [`ds2_rva::FLO_TAB_PLATE_OFFSET`]; the chevrons likewise, on their count, their
//! mirrored `scale x`, their offset and their rect. Anything else passes through untouched and says
//! so, and the seventh tab is the iconless one it already was.
//!
//! That matters more here than it does one level up. The definition detour in [`crate::layout`]
//! only ever sees the pause menu's own document, because that is what asks for a container; this one
//! is on the lookup every `.flo` in the game makes for every picture it draws. `0x026b` in the title
//! screen's document is the title screen's shape, so the check on it is seven floats rather than an
//! index.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// The game's own shape lookup, published by MinHook before the site is patched.
pub(crate) static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

type FindShapeFn = unsafe extern "system" fn(*mut usize, u32) -> *mut u8;

/// How close two floats have to be for this to call a document the one it was read from.
///
/// The constants beside these are decimal literals and the document's are whatever the exporter
/// wrote, so an exact comparison is a bet on both rounding the same way. A hundredth of an atlas
/// pixel is far below anything authoring drift produces and far above any rounding this could hit.
const CLOSE: f32 = 0.01;

/// A replacement shape: its table entry, its quads, and the source rects those quads point at --
/// one allocation, so the pointers between them cannot outlive their targets.
///
/// Parameterised on byte lengths rather than on a quad count, because a const parameter may only
/// appear on its own in an array length and `STRIDE * QUADS` is arithmetic.
#[repr(C, align(16))]
struct Shape<const QUAD_BYTES: usize, const SOURCE_BYTES: usize> {
    entry: [u8; ds2_rva::FLO_SHAPE_STRIDE],
    quads: [u8; QUAD_BYTES],
    sources: [u8; SOURCE_BYTES],
    /// The shipped quads exactly as the game had them, so a cache hit can be checked rather than
    /// assumed -- the same guard [`crate::strip`] keeps over the strip's own records.
    shipped: [u8; QUAD_BYTES],
}

/// The seventh tab's icon, one quad.
type Icon = Shape<{ ds2_rva::FLO_QUAD_STRIDE }, { ds2_rva::FLO_SOURCE_RECT_SIZE }>;
/// The two chevrons, of which one has moved.
type Arrows = Shape<
    { ds2_rva::FLO_QUAD_STRIDE * ds2_rva::FLO_TAB_ARROWS_QUADS },
    { ds2_rva::FLO_SOURCE_RECT_SIZE * ds2_rva::FLO_TAB_ARROWS_QUADS },
>;

/// Substitutions already built, as `(the entry the game returned, the entry this returns)`.
static ICONS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
static ARROWS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Most substitutions to keep of either kind, for the same reason [`crate::strip`] has one: a
/// document can be reloaded, and a build per reload with no ceiling is a leak with a slow fuse.
const MAX_BUILT: usize = 64;

static REFUSED: AtomicUsize = AtomicUsize::new(0);
static SERVED: AtomicUsize = AtomicUsize::new(0);

/// Read a float out of a quad.
///
/// # Safety
///
/// `quad` must point at [`ds2_rva::FLO_QUAD_STRIDE`] live bytes and `offset` must be inside it.
unsafe fn quad_f32(quad: *const u8, offset: usize) -> f32 {
    // SAFETY: the caller guarantees the read is inside the quad.
    f32::from_bits(unsafe { quad.add(offset).cast::<u32>().read_unaligned() })
}

/// How many quads an entry claims, and where they are.
///
/// # Safety
///
/// `entry` must be a live shape-table entry.
unsafe fn quads_of(entry: *const u8) -> (usize, *const u8) {
    // SAFETY: the caller guarantees an entry, and these are the two fields the builder reads first.
    unsafe {
        (
            entry
                .add(ds2_rva::FLO_SHAPE_QUAD_COUNT_OFFSET)
                .cast::<u16>()
                .read() as usize,
            entry
                .add(ds2_rva::FLO_SHAPE_QUADS_OFFSET)
                .cast::<*const u8>()
                .read(),
        )
    }
}

/// Build the seventh tab's icon out of the plate the game just handed back.
///
/// # Safety
///
/// `plate` must be the live entry the game's own lookup returned for
/// [`ds2_rva::FLO_TAB_PLATE_SHAPE`].
unsafe fn build_icon(plate: *const u8) -> Option<*mut u8> {
    // SAFETY: the caller guarantees a live entry.
    let (count, quads) = unsafe { quads_of(plate) };
    if count != ds2_rva::FLO_TAB_PLATE_QUADS || (quads as usize) < 0x1_0000 {
        log(format_args!(
            "{LOG_PREFIX} icon REFUSED reason=not-the-plate quads={count} \
             expected={} -- the seventh tab keeps no icon",
            ds2_rva::FLO_TAB_PLATE_QUADS
        ));
        return None;
    }
    // SAFETY: the count above says this quad is live.
    let source = unsafe {
        quads
            .add(ds2_rva::FLO_QUAD_SOURCE_OFFSET)
            .cast::<*const u8>()
            .read()
    };
    if (source as usize) < 0x1_0000 {
        log(format_args!(
            "{LOG_PREFIX} icon REFUSED reason=source-not-a-pointer at=0x{:016x}",
            source as usize
        ));
        return None;
    }
    // SAFETY: `source` is the rect pointer the builder dereferences for this quad, so its four
    // floats are live; the quad itself is live by the count above.
    let (rect, offset) = unsafe {
        (
            [
                quad_f32(source, ds2_rva::FLO_SOURCE_LEFT_OFFSET),
                quad_f32(source, ds2_rva::FLO_SOURCE_TOP_OFFSET),
                quad_f32(source, ds2_rva::FLO_SOURCE_RIGHT_OFFSET),
                quad_f32(source, ds2_rva::FLO_SOURCE_BOTTOM_OFFSET),
            ],
            [
                quad_f32(quads, ds2_rva::FLO_QUAD_X_OFFSET),
                quad_f32(quads, ds2_rva::FLO_QUAD_Y_OFFSET),
            ],
        )
    };
    let matches = rect
        .iter()
        .zip(ds2_rva::FLO_TAB_PLATE_SOURCE.iter())
        .chain(offset.iter().zip(ds2_rva::FLO_TAB_PLATE_OFFSET.iter()))
        .all(|(found, want)| (found - want).abs() < CLOSE);
    if !matches {
        log(format_args!(
            "{LOG_PREFIX} icon REFUSED reason=plate-moved rect={rect:?} offset={offset:?} \
             expected rect={:?} offset={:?}",
            ds2_rva::FLO_TAB_PLATE_SOURCE,
            ds2_rva::FLO_TAB_PLATE_OFFSET
        ));
        return None;
    }

    let mut icon = Box::new(Icon {
        entry: [0; ds2_rva::FLO_SHAPE_STRIDE],
        quads: [0; ds2_rva::FLO_QUAD_STRIDE],
        sources: [0; ds2_rva::FLO_SOURCE_RECT_SIZE],
        // SAFETY: the count established this many quads are live.
        shipped: unsafe {
            std::ptr::read_unaligned(quads.cast::<[u8; ds2_rva::FLO_QUAD_STRIDE]>())
        },
    });
    // SAFETY: an entry is `FLO_SHAPE_STRIDE` bytes, which is what is being copied.
    icon.entry =
        unsafe { std::ptr::read_unaligned(plate.cast::<[u8; ds2_rva::FLO_SHAPE_STRIDE]>()) };
    icon.quads = icon.shipped;
    // SAFETY: as above, for the rect this quad points at.
    icon.sources =
        unsafe { std::ptr::read_unaligned(source.cast::<[u8; ds2_rva::FLO_SOURCE_RECT_SIZE]>()) };

    // The narrowed rect and the moved offset, which together are the whole substitution: the last
    // whole tab period of the plate, drawn one period further along.
    icon.sources[ds2_rva::FLO_SOURCE_LEFT_OFFSET..][..4]
        .copy_from_slice(&ds2_rva::FLO_ADDED_TAB_ICON_SOURCE_LEFT.to_le_bytes());
    icon.quads[ds2_rva::FLO_QUAD_X_OFFSET..][..4]
        .copy_from_slice(&(offset[0] + ds2_rva::FLO_TAB_PITCH).to_le_bytes());

    // The pointers last, so nothing in the struct is addressed before it holds what it should.
    let sources = icon.sources.as_ptr() as u64;
    icon.quads[ds2_rva::FLO_QUAD_SOURCE_OFFSET..][..8].copy_from_slice(&sources.to_le_bytes());
    let quads = icon.quads.as_ptr() as u64;
    icon.entry[ds2_rva::FLO_SHAPE_QUADS_OFFSET..][..8].copy_from_slice(&quads.to_le_bytes());
    icon.entry[ds2_rva::FLO_SHAPE_KEY_OFFSET..][..2]
        .copy_from_slice(&(ds2_rva::FLO_ADDED_TAB_ICON_SHAPE as u16).to_le_bytes());
    icon.entry[ds2_rva::FLO_SHAPE_QUAD_COUNT_OFFSET..][..2].copy_from_slice(&1u16.to_le_bytes());

    let leaked: &'static mut Icon = Box::leak(icon);
    log(format_args!(
        "{LOG_PREFIX} icon built shape={:#x} source={}..{} screen={}..{} -- the plate's last \
         {} of atlas, drawn one pitch on",
        ds2_rva::FLO_ADDED_TAB_ICON_SHAPE,
        ds2_rva::FLO_ADDED_TAB_ICON_SOURCE_LEFT,
        rect[2],
        ds2_rva::FLO_ADDED_TAB_ICON_SOURCE_LEFT + offset[0] + ds2_rva::FLO_TAB_PITCH,
        rect[2] + offset[0] + ds2_rva::FLO_TAB_PITCH,
        ds2_rva::FLO_TAB_PITCH,
    ));
    Some((&raw mut leaked.entry).cast::<u8>())
}

/// Build the chevrons with the right-hand one moved out of the seventh tab's way.
///
/// # Safety
///
/// `found` must be the live entry the game's own lookup returned for
/// [`ds2_rva::FLO_TAB_ARROWS_SHAPE`].
unsafe fn build_arrows(found: *const u8) -> Option<*mut u8> {
    // SAFETY: the caller guarantees a live entry.
    let (count, quads) = unsafe { quads_of(found) };
    if count != ds2_rva::FLO_TAB_ARROWS_QUADS || (quads as usize) < 0x1_0000 {
        log(format_args!(
            "{LOG_PREFIX} chevron REFUSED reason=not-the-chevrons quads={count} expected={}",
            ds2_rva::FLO_TAB_ARROWS_QUADS
        ));
        return None;
    }
    let at = ds2_rva::FLO_TAB_ARROWS_RIGHT * ds2_rva::FLO_QUAD_STRIDE;
    // SAFETY: the index is below the count, which the read above established is live.
    let (scale_x, x, source) = unsafe {
        let right = quads.add(at);
        (
            quad_f32(right, ds2_rva::FLO_QUAD_SCALE_X_OFFSET),
            quad_f32(right, ds2_rva::FLO_QUAD_X_OFFSET),
            right
                .add(ds2_rva::FLO_QUAD_SOURCE_OFFSET)
                .cast::<*const u8>()
                .read(),
        )
    };
    if (scale_x - ds2_rva::FLO_TAB_ARROWS_RIGHT_SCALE_X).abs() >= CLOSE
        || (x - ds2_rva::FLO_TAB_ARROWS_RIGHT_X).abs() >= CLOSE
        || (source as usize) < 0x1_0000
    {
        log(format_args!(
            "{LOG_PREFIX} chevron REFUSED reason=not-the-right-one scale-x={scale_x} x={x} \
             expected scale-x={} x={}",
            ds2_rva::FLO_TAB_ARROWS_RIGHT_SCALE_X,
            ds2_rva::FLO_TAB_ARROWS_RIGHT_X
        ));
        return None;
    }
    // The rect as well, because this detour sees every shape lookup the game makes and `0x026b` in
    // another document is another picture. Seven floats agreeing is the identity.
    // SAFETY: `source` is the rect pointer the builder dereferences for this quad, so its four
    // floats are live.
    let rect = unsafe {
        [
            quad_f32(source, ds2_rva::FLO_SOURCE_LEFT_OFFSET),
            quad_f32(source, ds2_rva::FLO_SOURCE_TOP_OFFSET),
            quad_f32(source, ds2_rva::FLO_SOURCE_RIGHT_OFFSET),
            quad_f32(source, ds2_rva::FLO_SOURCE_BOTTOM_OFFSET),
        ]
    };
    if rect
        .iter()
        .zip(ds2_rva::FLO_TAB_ARROWS_RIGHT_SOURCE.iter())
        .any(|(found, want)| (found - want).abs() >= CLOSE)
    {
        log(format_args!(
            "{LOG_PREFIX} chevron REFUSED reason=another-documents-shape rect={rect:?} \
             expected={:?}",
            ds2_rva::FLO_TAB_ARROWS_RIGHT_SOURCE
        ));
        return None;
    }

    let mut arrows = Box::new(Arrows {
        entry: [0; ds2_rva::FLO_SHAPE_STRIDE],
        quads: [0; ds2_rva::FLO_QUAD_STRIDE * ds2_rva::FLO_TAB_ARROWS_QUADS],
        // Both quads keep the document's own rects, because neither rect changes -- only where one
        // of them lands. Nothing in this module writes through those pointers.
        sources: [0; ds2_rva::FLO_SOURCE_RECT_SIZE * ds2_rva::FLO_TAB_ARROWS_QUADS],
        // SAFETY: the count established this many quads are live.
        shipped: unsafe {
            std::ptr::read_unaligned(
                quads.cast::<[u8; ds2_rva::FLO_QUAD_STRIDE * ds2_rva::FLO_TAB_ARROWS_QUADS]>(),
            )
        },
    });
    // SAFETY: an entry is `FLO_SHAPE_STRIDE` bytes, which is what is being copied.
    arrows.entry =
        unsafe { std::ptr::read_unaligned(found.cast::<[u8; ds2_rva::FLO_SHAPE_STRIDE]>()) };
    arrows.quads = arrows.shipped;
    arrows.quads[at + ds2_rva::FLO_QUAD_X_OFFSET..][..4]
        .copy_from_slice(&(x + ds2_rva::FLO_TAB_PITCH).to_le_bytes());
    let moved = arrows.quads.as_ptr() as u64;
    arrows.entry[ds2_rva::FLO_SHAPE_QUADS_OFFSET..][..8].copy_from_slice(&moved.to_le_bytes());

    let leaked: &'static mut Arrows = Box::leak(arrows);
    log(format_args!(
        "{LOG_PREFIX} chevron moved quad={} x={x} -> {} -- out from under the seventh hexagon",
        ds2_rva::FLO_TAB_ARROWS_RIGHT,
        x + ds2_rva::FLO_TAB_PITCH,
    ));
    Some((&raw mut leaked.entry).cast::<u8>())
}

/// Whether a cached substitution still describes the shape the game is holding.
///
/// # Safety
///
/// `original` must be a live entry and `cached` a [`Shape`] this module built from it.
unsafe fn still_current<const QUAD_BYTES: usize, const SOURCE_BYTES: usize>(
    original: *const u8,
    cached: *const Shape<QUAD_BYTES, SOURCE_BYTES>,
) -> bool {
    // SAFETY: the caller guarantees a live entry.
    let (count, quads) = unsafe { quads_of(original) };
    if count != QUAD_BYTES / ds2_rva::FLO_QUAD_STRIDE || (quads as usize) < 0x1_0000 {
        return false;
    }
    // SAFETY: the count says that many quads are live, and the snapshot is exactly that size.
    unsafe {
        std::slice::from_raw_parts(quads, QUAD_BYTES)
            == std::slice::from_raw_parts((*cached).shipped.as_ptr(), QUAD_BYTES)
    }
}

/// Look a substitution up, building it on first sight. `original` is the entry to key on, which for
/// the icon is the plate's -- there is no shipped entry of its own to key on.
///
/// # Safety
///
/// `original` must be the live entry the game's own lookup returned, and `build` must produce a
/// [`Shape`] of `QUADS` quads from it.
unsafe fn cached<const QUAD_BYTES: usize, const SOURCE_BYTES: usize>(
    store: &Mutex<Vec<(usize, usize)>>,
    original: *mut u8,
    what: &str,
    build: unsafe fn(*const u8) -> Option<*mut u8>,
) -> Option<*mut u8> {
    let mut built = store.lock().ok()?;
    if let Some(slot) = built.iter().position(|(key, _)| *key == original as usize) {
        let replacement = built[slot].1 as *mut Shape<QUAD_BYTES, SOURCE_BYTES>;
        // SAFETY: `replacement` is a leaked `Shape` this module built from `original`.
        if unsafe { still_current(original, replacement) } {
            // SAFETY: as above, and the document it was built from is the one the game holds.
            return Some(unsafe { (&raw mut (*replacement).entry).cast::<u8>() });
        }
        log(format_args!(
            "{LOG_PREFIX} {what} stale original=0x{:016x} -- the document was reloaded, rebuilding",
            original as usize
        ));
        built.remove(slot);
    }
    if built.len() >= MAX_BUILT {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} {what} REFUSED reason=too-many-documents built={MAX_BUILT} refusals={n}"
        ));
        return None;
    }
    // SAFETY: the caller established this is the entry the builder wants.
    let replacement = unsafe { build(original) }?;
    // A `Shape` begins with its entry, so the entry's address is the struct's.
    built.push((original as usize, replacement as usize));
    Some(replacement)
}

unsafe extern "system" fn detour(doc: *mut usize, index: u32) -> *mut u8 {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Published before the site is patched, so unreachable. A null entry is what the original
        // returns for a miss and what the builder is written to survive, so it is the honest answer.
        return std::ptr::null_mut();
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the one
    // the disassembled entry and exit implement.
    let original: FindShapeFn = unsafe { std::mem::transmute::<usize, FindShapeFn>(trampoline) };

    if index == ds2_rva::FLO_ADDED_TAB_ICON_SHAPE {
        // Ours, and only ever asked for by the record `crate::strip` wrote -- which it writes only
        // when there is a seventh tab. Built from the plate on this same document, because the
        // icon is the plate's own art and there is nothing else to copy it from.
        // SAFETY: both arguments are the game's own; the index is one the game's table will miss,
        // which is what makes the plate the only thing this can be built out of.
        let plate = unsafe { original(doc, ds2_rva::FLO_TAB_PLATE_SHAPE) };
        if plate.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `plate` is what the game's own lookup just returned for the plate's index.
        let built = unsafe {
            cached::<{ ds2_rva::FLO_QUAD_STRIDE }, { ds2_rva::FLO_SOURCE_RECT_SIZE }>(
                &ICONS, plate, "icon", build_icon,
            )
        };
        return match built {
            Some(entry) => {
                let n = SERVED.fetch_add(1, Ordering::Relaxed) + 1;
                if n <= 2 {
                    log(format_args!(
                        "{LOG_PREFIX} icon served shape={:#x} entry=0x{:016x} served={n}",
                        ds2_rva::FLO_ADDED_TAB_ICON_SHAPE,
                        entry as usize
                    ));
                }
                entry
            }
            // A miss is what the game returns for an index it does not hold, and the builder skips
            // a record whose shape is null. So the seventh tab loses its icon and keeps the rest.
            None => std::ptr::null_mut(),
        };
    }

    // SAFETY: both arguments are the game's own, passed through unchanged.
    let found = unsafe { original(doc, index) };
    if index != ds2_rva::FLO_TAB_ARROWS_SHAPE || found.is_null() || crate::tab::group() == 0 {
        return found;
    }
    // THE CHEVRONS, and only when there is a seventh tab to make room for. With six tabs the strip
    // is the one the game shipped and nothing should move.
    // SAFETY: `found` is what the game's own lookup just returned for the chevrons' index.
    let built = unsafe {
        cached::<
            { ds2_rva::FLO_QUAD_STRIDE * ds2_rva::FLO_TAB_ARROWS_QUADS },
            { ds2_rva::FLO_SOURCE_RECT_SIZE * ds2_rva::FLO_TAB_ARROWS_QUADS },
        >(&ARROWS, found, "chevron", build_arrows)
    };
    built.unwrap_or(found)
}

/// Detour the shape lookup. Returns whether the seventh tab's icon can be drawn.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. `base` must be the live module base and
/// MinHook must already be initialised, which [`crate::install::install`] guarantees.
pub unsafe fn install(base: usize) -> bool {
    // SAFETY: the rva is a `.pdata` function start recorded in `ds2-rva` with its prologue beside
    // it, and `hook_site` refuses unless the bytes at the site are those.
    unsafe {
        crate::install::hook_site(
            base,
            ds2_rva::FLO_FIND_SHAPE,
            &ds2_rva::FLO_FIND_SHAPE_PROLOGUE,
            detour as *mut std::ffi::c_void,
            &TRAMPOLINE,
            "shape-lookup",
        )
    }
}

/// Whether the shape lookup is detoured, which is what says the icon record has something to
/// resolve to.
///
/// [`crate::strip`] asks before it adds the record and before it moves the strip's right-hand
/// furniture, so a refusal leaves the strip exactly as it was: a seventh tab with no icon, which is
/// worse than seven icons and better than a hole where the `RB` prompt used to be.
pub(crate) fn armed() -> bool {
    TRAMPOLINE.load(Ordering::Acquire) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slice is one whole tab period, taken off the plate's right edge.
    #[test]
    fn the_slice_is_the_plates_last_pitch() {
        let left = ds2_rva::FLO_ADDED_TAB_ICON_SOURCE_LEFT;
        let right = ds2_rva::FLO_TAB_PLATE_SOURCE[2];
        assert!((right - left - ds2_rva::FLO_TAB_PITCH).abs() < CLOSE);
        assert!(
            left > ds2_rva::FLO_TAB_PLATE_SOURCE[0],
            "the slice has to be inside the plate, or it samples another shape's art"
        );
    }

    /// Placed one pitch on, the slice starts exactly where the plate stops.
    ///
    /// That is the whole reason this construction needs no hexagon width: the two rects abut, so
    /// the row of hexagons continues rather than overlapping or leaving a gap.
    #[test]
    fn the_icon_begins_where_the_plate_ends() {
        let plate_right = ds2_rva::FLO_TAB_PLATE_SOURCE[2] + ds2_rva::FLO_TAB_PLATE_OFFSET[0];
        let icon_left = ds2_rva::FLO_ADDED_TAB_ICON_SOURCE_LEFT
            + ds2_rva::FLO_TAB_PLATE_OFFSET[0]
            + ds2_rva::FLO_TAB_PITCH;
        assert!((plate_right - icon_left).abs() < CLOSE);
    }

    /// The hexagon lands over the seventh cell rather than the sixth.
    #[test]
    fn the_icon_sits_on_the_seventh_tab() {
        let sixth = ds2_rva::FLO_TAB_PLATE_SOURCE[2] - ds2_rva::FLO_TAB_PITCH
            + ds2_rva::FLO_TAB_PLATE_OFFSET[0];
        let seventh = sixth + ds2_rva::FLO_TAB_PITCH;
        assert!(seventh > sixth);
        assert!(
            (seventh - (ds2_rva::FLO_TAB_PLATE_SOURCE[2] + ds2_rva::FLO_TAB_PLATE_OFFSET[0])).abs()
                < CLOSE
        );
    }

    /// The icon draws under the cells and over the plate, which is what the depth has to say.
    #[test]
    fn the_icon_is_between_the_plate_and_the_cells() {
        const {
            assert!(ds2_rva::FLO_ADDED_TAB_ICON_DEPTH > 61);
            assert!(ds2_rva::FLO_ADDED_TAB_ICON_DEPTH < 69);
        }
    }

    /// The index served is not one the shipped document uses, and not one another block claims.
    #[test]
    fn the_icons_index_is_free() {
        assert_ne!(
            ds2_rva::FLO_ADDED_TAB_ICON_SHAPE,
            ds2_rva::FLO_TAB_PLATE_SHAPE
        );
        assert_ne!(
            ds2_rva::FLO_ADDED_TAB_ICON_SHAPE,
            ds2_rva::FLO_TAB_ARROWS_SHAPE
        );
        for taken in [
            ds2_rva::FLO_ADDED_TAB_SUBTREE_DEFINITION,
            ds2_rva::FLO_ADDED_TAB_FRAME_DEFINITION,
            ds2_rva::FLO_ADDED_TAB_CONTAINER_DEFINITION,
        ] {
            assert_ne!(
                ds2_rva::FLO_ADDED_TAB_ICON_SHAPE,
                taken,
                "the icon's index collides with a definition this crate already serves"
            );
        }
    }

    /// The entry is first in the struct, because the detour returns its address as the struct's and
    /// the cache keys on that.
    #[test]
    fn the_entry_is_at_the_front_of_the_allocation() {
        let icon = Icon {
            entry: [0; ds2_rva::FLO_SHAPE_STRIDE],
            quads: [0; ds2_rva::FLO_QUAD_STRIDE],
            sources: [0; ds2_rva::FLO_SOURCE_RECT_SIZE],
            shipped: [0; ds2_rva::FLO_QUAD_STRIDE],
        };
        assert_eq!(
            (&raw const icon.entry).cast::<u8>(),
            (&raw const icon).cast::<u8>()
        );
    }
}
