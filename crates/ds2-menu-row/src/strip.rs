//! The seventh tab's cell in the strip along the top, added to the layout document in memory.
//!
//! # The same substitution a row gets, one level up
//!
//! [`crate::layout`] hands back a copy of the quit tab's container definition with two more children
//! per added row. The tab strip is the same kind of object: definition
//! [`ds2_rva::FLO_TAB_STRIP_DEFINITION`] with [`ds2_rva::FLO_TAB_STRIP_CHILDREN`] children, the last
//! six of which are the tab cells, and a child count that is also the display-list capacity
//! (`FUN_140b6bd80` refuses to attach past it). So a seventh cell is one more record and a count of
//! nineteen.
//!
//! The record is a copy of the sixth cell's with three fields changed:
//!
//! | field | from | to |
//! |---|---|---|
//! | element id | `0x1eaba5` | [`ds2_rva::FLO_ADDED_TAB_ID`] |
//! | transform x | `265.05` | `+ `[`ds2_rva::FLO_TAB_PITCH`] |
//! | depth | `89` | `+ `[`ds2_rva::FLO_TAB_DEPTH_PITCH`] |
//!
//! Its DEFINITION is left as the shipped [`ds2_rva::FLO_TAB_STRIP_CELL_DEFINITION`], unchanged and
//! uncopied, because a tab cell authors only its selection highlight -- the glyph on a tab is bound
//! by the grid control at runtime, not by the layout. There is no icon here to get wrong.
//!
//! # Why the transform is copied rather than shared
//!
//! A record's `+0x08` is a pointer to a transform block in the document, and two records pointing at
//! one block are one x between them. The shipped sixth cell would move with ours. So the block is
//! copied into this crate's allocation and the new record points at the copy, exactly as
//! [`crate::layout`] does for an added row.
//!
//! # What makes this safe to be wrong about
//!
//! A definition index is a number, and `0x0271` on a document this was not read from is some other
//! container. The substitution happens only when the definition the game returned has exactly
//! [`ds2_rva::FLO_TAB_STRIP_CHILDREN`] children whose last six ids are
//! [`ds2_rva::FLO_TAB_STRIP_CELL_IDS`], in order. Anything else passes through untouched and says so.
//!
//! # Not established
//!
//! None of this has been in front of a running game. The cell may draw and be unreachable, or be
//! reachable and not draw: those are two numbers from two places -- the strip's item count, raised in
//! [`crate::tab`], and this record -- and the run has to show them agreeing.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// Children the replacement carries: the shipped eighteen and one more.
const CHILDREN: usize = ds2_rva::FLO_TAB_STRIP_CHILDREN + 1;

/// A replacement strip definition, its child records, and the one transform block the added record
/// points at -- one allocation, so the pointer between them cannot outlive its target.
#[repr(C, align(16))]
struct Strip {
    definition: [u8; ds2_rva::FLO_DEFINITION_STRIDE],
    records: [u8; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
    transform: [u8; ds2_rva::FLO_TRANSFORM_SIZE],
    /// The shipped records exactly as the game had them, so a cache hit can be checked rather than
    /// assumed. Not `records`, which has its added entry appended and would never match.
    shipped: [u8; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_TAB_STRIP_CHILDREN],
}

/// Substitutions already built, as `(definition the game returned, definition we return)`.
static BUILT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Most substitutions to keep, for the same reason [`crate::layout`] has one.
const MAX_BUILT: usize = 64;

static SUBSTITUTED: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);

/// Whether the definition the game returned is the strip this crate read.
///
/// # Safety
///
/// `definition` must be a live `.flo` definition.
unsafe fn is_the_strip(definition: *const u8) -> Option<*const u8> {
    // SAFETY: the caller guarantees a definition, and these are the two fields the game reads first.
    let (count, children) = unsafe {
        (
            definition
                .add(ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET)
                .cast::<u16>()
                .read() as usize,
            definition
                .add(ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET)
                .cast::<*const u8>()
                .read(),
        )
    };
    if count != ds2_rva::FLO_TAB_STRIP_CHILDREN || (children as usize) < 0x1_0000 {
        return None;
    }
    for (offset, id) in ds2_rva::FLO_TAB_STRIP_CELL_IDS.iter().enumerate() {
        let index = ds2_rva::FLO_TAB_STRIP_FIRST_CELL + offset;
        // SAFETY: `index < count`, and the caller guarantees that many records are live.
        let found = unsafe {
            children
                .add(index * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_ID_OFFSET)
                .cast::<u32>()
                .read()
        };
        if found != *id {
            return None;
        }
    }
    Some(children)
}

/// Build the replacement. `None` with a log line if the definition is not the one this read.
///
/// # Safety
///
/// `original` must be the live definition the game's own lookup just returned.
unsafe fn build(original: *mut u8) -> Option<*mut u8> {
    // SAFETY: the caller guarantees a live definition.
    let children = unsafe { is_the_strip(original) }?;
    let mut strip = Box::new(Strip {
        // SAFETY: a definition is `FLO_DEFINITION_STRIDE` bytes, which is what is being copied.
        definition: unsafe {
            std::ptr::read_unaligned(original.cast::<[u8; ds2_rva::FLO_DEFINITION_STRIDE]>())
        },
        records: [0; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
        transform: [0; ds2_rva::FLO_TRANSFORM_SIZE],
        // SAFETY: `is_the_strip` established this many records are live at `children`.
        shipped: unsafe {
            std::ptr::read_unaligned(
                children
                    .cast::<[u8; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_TAB_STRIP_CHILDREN]>(),
            )
        },
    });
    strip.records[..strip.shipped.len()].copy_from_slice(&strip.shipped);

    // THE LAST CELL IS THE TEMPLATE, copied whole and then edited. Copying the last rather than the
    // first means the added cell inherits whatever the sixth's authoring says about a tab at the end
    // of the strip, which is where ours is.
    let last = (ds2_rva::FLO_TAB_STRIP_CHILDREN - 1) * ds2_rva::FLO_RECORD_STRIDE;
    let added = ds2_rva::FLO_TAB_STRIP_CHILDREN * ds2_rva::FLO_RECORD_STRIDE;
    let (head, tail) = strip.records.split_at_mut(added);
    tail[..ds2_rva::FLO_RECORD_STRIDE]
        .copy_from_slice(&head[last..last + ds2_rva::FLO_RECORD_STRIDE]);

    // The template's transform, copied so moving ours does not move the sixth tab.
    let source = u64::from_le_bytes(
        strip.records[last + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET..][..8]
            .try_into()
            .ok()?,
    ) as usize;
    if source < 0x1_0000 {
        log(format_args!(
            "{LOG_PREFIX} strip REFUSED reason=transform-not-a-pointer at=0x{source:016x}"
        ));
        return None;
    }
    // SAFETY: a record's `+0x08` is a pointer to a `FLO_TRANSFORM_SIZE` block in the loaded
    // document, which `is_the_strip` established this record to be part of.
    strip.transform = unsafe {
        std::ptr::read_unaligned((source as *const u8).cast::<[u8; ds2_rva::FLO_TRANSFORM_SIZE]>())
    };
    let x = f32::from_le_bytes(
        strip.transform[ds2_rva::FLO_TRANSFORM_X_OFFSET..][..4]
            .try_into()
            .ok()?,
    );
    strip.transform[ds2_rva::FLO_TRANSFORM_X_OFFSET..][..4]
        .copy_from_slice(&(x + ds2_rva::FLO_TAB_PITCH).to_le_bytes());

    let transform = strip.transform.as_ptr() as u64;
    let record = &mut strip.records[added..added + ds2_rva::FLO_RECORD_STRIDE];
    record[ds2_rva::FLO_RECORD_TRANSFORM_OFFSET..][..8].copy_from_slice(&transform.to_le_bytes());
    record[ds2_rva::FLO_RECORD_ID_OFFSET..][..4]
        .copy_from_slice(&ds2_rva::FLO_ADDED_TAB_ID.to_le_bytes());
    let depth = u16::from_le_bytes(
        record[ds2_rva::FLO_RECORD_DEPTH_OFFSET..][..2]
            .try_into()
            .ok()?,
    );
    record[ds2_rva::FLO_RECORD_DEPTH_OFFSET..][..2].copy_from_slice(
        &depth
            .wrapping_add(ds2_rva::FLO_TAB_DEPTH_PITCH)
            .to_le_bytes(),
    );

    // The definition last, pointed at our records and carrying the raised count -- which is also the
    // capacity, so this one number is what gets the record walked AND lets it attach.
    let records = strip.records.as_ptr() as u64;
    strip.definition[ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET..][..8]
        .copy_from_slice(&records.to_le_bytes());
    strip.definition[ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET..][..2]
        .copy_from_slice(&(CHILDREN as u16).to_le_bytes());

    let leaked: &'static mut Strip = Box::leak(strip);
    let n = SUBSTITUTED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} strip cell added id={:#x} x={} depth={} children={}->{CHILDREN} \
         substitutions={n}",
        ds2_rva::FLO_ADDED_TAB_ID,
        x + ds2_rva::FLO_TAB_PITCH,
        depth.wrapping_add(ds2_rva::FLO_TAB_DEPTH_PITCH),
        ds2_rva::FLO_TAB_STRIP_CHILDREN
    ));
    Some((&raw mut leaked.definition).cast::<u8>())
}

/// Whether a cached substitution still describes the document the game is holding.
///
/// # Safety
///
/// `original` must be a live definition and `cached` a [`Strip`] this module built from it.
unsafe fn still_current(original: *const u8, cached: *const Strip) -> bool {
    // SAFETY: the caller guarantees a live definition.
    let Some(children) = (unsafe { is_the_strip(original) }) else {
        return false;
    };
    let shipped = ds2_rva::FLO_TAB_STRIP_CHILDREN * ds2_rva::FLO_RECORD_STRIDE;
    // SAFETY: `is_the_strip` established that many records are live, and the cached snapshot is
    // exactly that size.
    unsafe {
        std::slice::from_raw_parts(children, shipped)
            == std::slice::from_raw_parts((*cached).shipped.as_ptr(), shipped)
    }
}

/// The strip definition this crate returns for `original`, building it on first sight.
///
/// # Safety
///
/// `original` must be the definition the game's own lookup just returned for
/// [`ds2_rva::FLO_TAB_STRIP_DEFINITION`].
pub(crate) unsafe fn substitute(original: *mut u8) -> *mut u8 {
    let Ok(mut built) = BUILT.lock() else {
        return original;
    };
    if let Some(slot) = built.iter().position(|(key, _)| *key == original as usize) {
        let replacement = built[slot].1 as *mut Strip;
        // SAFETY: `replacement` is a leaked `Strip` this module built from `original`.
        if unsafe { still_current(original, replacement) } {
            // SAFETY: `replacement` is the leaked `Strip` this module built, and `still_current`
            // has just established the document it was built from is the one the game holds.
            return unsafe { (&raw mut (*replacement).definition).cast::<u8>() };
        }
        log(format_args!(
            "{LOG_PREFIX} strip stale original=0x{:016x} -- the document was reloaded, rebuilding",
            original as usize
        ));
        built.remove(slot);
    }
    if built.len() >= MAX_BUILT {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} strip REFUSED reason=too-many-documents built={MAX_BUILT} refusals={n}"
        ));
        return original;
    }
    // SAFETY: the caller established this is the strip's definition index.
    match unsafe { build(original) } {
        Some(replacement) => {
            // The `Strip` begins with its definition, so the definition's address is the struct's.
            built.push((original as usize, replacement as usize));
            replacement
        }
        None => original,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One more child than the game ships, and the added record is the last one.
    #[test]
    fn the_replacement_is_the_shipped_strip_plus_one() {
        assert_eq!(CHILDREN, ds2_rva::FLO_TAB_STRIP_CHILDREN + 1);
        assert_eq!(
            ds2_rva::FLO_TAB_STRIP_FIRST_CELL + ds2_rva::FLO_TAB_STRIP_CELL_IDS.len(),
            ds2_rva::FLO_TAB_STRIP_CHILDREN,
            "the six cells must be the LAST six children, or the template is the wrong record"
        );
    }

    /// The added tab's id is not one of the six, which is what keeps the identity check meaningful.
    #[test]
    fn the_added_id_is_not_one_the_strip_already_uses() {
        assert!(!ds2_rva::FLO_TAB_STRIP_CELL_IDS.contains(&ds2_rva::FLO_ADDED_TAB_ID));
    }

    /// The definition is first in the struct, because [`substitute`] returns its address as the
    /// struct's and the cache keys on that.
    #[test]
    fn the_definition_is_at_the_front_of_the_allocation() {
        let strip = Strip {
            definition: [0; ds2_rva::FLO_DEFINITION_STRIDE],
            records: [0; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
            transform: [0; ds2_rva::FLO_TRANSFORM_SIZE],
            shipped: [0; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_TAB_STRIP_CHILDREN],
        };
        assert_eq!(
            (&raw const strip.definition).cast::<u8>(),
            (&raw const strip).cast::<u8>()
        );
    }
}
