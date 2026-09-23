//! The seventh tab's cell in the strip along the top, added to the layout document in memory.
//!
//! # The same substitution a row gets, one level up
//!
//! [`crate::layout`] hands back a copy of the quit tab's container definition with two more children
//! per added row. The tab strip is the same kind of object: definition
//! [`ds2_rva::FLO_TAB_STRIP_DEFINITION`] with [`ds2_rva::FLO_TAB_STRIP_CHILDREN`] children, the last
//! six of which are the tab cells, and a child count that is also the display-list capacity
//! (`FUN_140b6bd80` refuses to attach past it). So a seventh tab is two more records -- a cell and
//! the panel that cell selects -- and a count of twenty.
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
//! # The second record, which is the tab itself
//!
//! A cell is the icon in the strip. What the tab shows when it is selected is a separate child of
//! the same container -- [`ds2_rva::FLO_TAB_STRIP_PANEL`], the System tab's subtree -- and the
//! seventh tab gets one of those too, cloned from it with two fields changed:
//!
//! | field | from | to |
//! |---|---|---|
//! | element id | [`ds2_rva::FLO_TAB_STRIP_PANEL_ID`] | [`ds2_rva::FLO_ADDED_TAB_SUBTREE_ID`] |
//! | definition | [`ds2_rva::FLO_TAB_SUBTREE_DEFINITION`] | [`ds2_rva::FLO_ADDED_TAB_SUBTREE_DEFINITION`] |
//!
//! That definition is [`crate::layout`]'s copy, and it is what the seventh tab's rows hang under
//! instead of the System tab's. Sharing the System tab's subtree is what the first run did, and it
//! drew the System tab's three captions on top of the seventh tab's first three rows: a row record
//! is a grid cell and a namer can decline to name it, a caption is a plain child and nothing can
//! decline to draw it.
//!
//! The clone is inserted beside its template rather than appended after the cells. Depth is not
//! read back for a nested-definition record, so two subtrees draw in array order, and a tab's panel
//! placed after the strip's own cells would draw over them.
//!
//! # Why one transform is copied and the other shared
//!
//! A record's `+0x08` is a pointer to a transform block in the document, and two records pointing at
//! one block are one x between them. The added cell moves -- one [`ds2_rva::FLO_TAB_PITCH`] along
//! the strip -- so its block is copied first, exactly as [`crate::layout`] does for an added row.
//! The added subtree does not move: it wants the position the System tab's panel already has,
//! because it is the same panel with different rows in it. So it shares that pointer, and nothing
//! in this module writes through it.
//!
//! # What makes this safe to be wrong about
//!
//! A definition index is a number, and `0x0271` on a document this was not read from is some other
//! container. The substitution happens only when the definition the game returned has exactly
//! [`ds2_rva::FLO_TAB_STRIP_CHILDREN`] children whose last six ids are
//! [`ds2_rva::FLO_TAB_STRIP_CELL_IDS`], in order, and whose [`ds2_rva::FLO_TAB_STRIP_PANEL`] is a
//! record naming [`ds2_rva::FLO_TAB_SUBTREE_DEFINITION`] under [`ds2_rva::FLO_TAB_STRIP_PANEL_ID`].
//! Anything else passes through untouched and says so.
//!
//! # What a run has shown, and what it has not
//!
//! The cell is established. A run logged `strip cell added id=0x1eaba8 x=319.05 children=18->19`
//! and `strip count raised tabs=6 -> items=7` with no mismatch, and the seventh tab drew its four
//! rows. The subtree record is new and has not been in front of a running game.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// Children the replacement carries: the shipped eighteen, the seventh tab's subtree, and its cell.
const CHILDREN: usize = ds2_rva::FLO_TAB_STRIP_CHILDREN + 2;

/// Where the added subtree sits in the replacement: directly after the template it is cloned from,
/// so the two tabs' panels are adjacent and both precede the strip's cells.
const PANEL_AT: usize = ds2_rva::FLO_TAB_STRIP_PANEL + 1;

/// Where the added cell sits: last, after the six the game ships.
const CELL_AT: usize = CHILDREN - 1;

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
    // The subtree template, checked on both of the fields the clone changes. A record at this index
    // naming something else is a document this was not read from, and cloning it would hang the
    // seventh tab's rows off whatever it happens to be.
    let at = ds2_rva::FLO_TAB_STRIP_PANEL * ds2_rva::FLO_RECORD_STRIDE;
    // SAFETY: the index is below `count`, which the caller guarantees is live at `children`.
    let (id, definition) = unsafe {
        (
            children
                .add(at + ds2_rva::FLO_RECORD_ID_OFFSET)
                .cast::<u32>()
                .read(),
            children
                .add(at + ds2_rva::FLO_RECORD_DEFINITION_OFFSET)
                .cast::<u16>()
                .read() as u32,
        )
    };
    if id != ds2_rva::FLO_TAB_STRIP_PANEL_ID || definition != ds2_rva::FLO_TAB_SUBTREE_DEFINITION {
        return None;
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
    // THE SHIPPED RECORDS, IN TWO RUNS WITH A GAP. Everything up to and including the subtree
    // template keeps its index; everything after it moves one along to leave [`PANEL_AT`] free.
    // Written from the pristine snapshot rather than from the document, so a template read below
    // cannot pick up a record this loop has already moved.
    let stride = ds2_rva::FLO_RECORD_STRIDE;
    let head = PANEL_AT * stride;
    strip.records[..head].copy_from_slice(&strip.shipped[..head]);
    strip.records[head + stride..][..strip.shipped.len() - head]
        .copy_from_slice(&strip.shipped[head..]);

    // THE LAST CELL IS THE TEMPLATE, copied whole and then edited. Copying the last rather than the
    // first means the added cell inherits whatever the sixth's authoring says about a tab at the end
    // of the strip, which is where ours is.
    let last = (ds2_rva::FLO_TAB_STRIP_CHILDREN - 1) * stride;
    let added = CELL_AT * stride;
    strip.records[added..added + stride].copy_from_slice(&strip.shipped[last..last + stride]);

    // The subtree, cloned from the record beside it, with its definition pointed at this crate's
    // copy and a new element id so a path can tell the two tabs apart. Its transform pointer is the
    // template's and stays that way: the seventh tab's panel wants the position the System tab's
    // panel already has.
    {
        let template = ds2_rva::FLO_TAB_STRIP_PANEL * stride;
        let at = PANEL_AT * stride;
        strip.records[at..at + stride].copy_from_slice(&strip.shipped[template..template + stride]);
        strip.records[at + ds2_rva::FLO_RECORD_DEFINITION_OFFSET..][..2]
            .copy_from_slice(&(ds2_rva::FLO_ADDED_TAB_SUBTREE_DEFINITION as u16).to_le_bytes());
        strip.records[at + ds2_rva::FLO_RECORD_ID_OFFSET..][..4]
            .copy_from_slice(&ds2_rva::FLO_ADDED_TAB_SUBTREE_ID.to_le_bytes());
    }

    // The cell template's transform, copied so moving ours does not move the sixth tab.
    let source = u64::from_le_bytes(
        strip.records[added + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET..][..8]
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
         subtree={:#x}@{PANEL_AT} definition={:#x} substitutions={n}",
        ds2_rva::FLO_ADDED_TAB_ID,
        x + ds2_rva::FLO_TAB_PITCH,
        depth.wrapping_add(ds2_rva::FLO_TAB_DEPTH_PITCH),
        ds2_rva::FLO_TAB_STRIP_CHILDREN,
        ds2_rva::FLO_ADDED_TAB_SUBTREE_ID,
        ds2_rva::FLO_ADDED_TAB_SUBTREE_DEFINITION,
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

    /// Two more children than the game ships, and the added cell is the last one.
    #[test]
    fn the_replacement_is_the_shipped_strip_plus_two() {
        assert_eq!(CHILDREN, ds2_rva::FLO_TAB_STRIP_CHILDREN + 2);
        assert_eq!(
            ds2_rva::FLO_TAB_STRIP_FIRST_CELL + ds2_rva::FLO_TAB_STRIP_CELL_IDS.len(),
            ds2_rva::FLO_TAB_STRIP_CHILDREN,
            "the six cells must be the last six children, or the template is the wrong record"
        );
    }

    /// The record this module adds and the entry the strip's namer stand-in carries are the same
    /// cell.
    ///
    /// Two halves of one tab, written in two files: a record here, and an entry in
    /// [`crate::install::adopt_strip_namer`]. A run had the first without the second, and the tab
    /// was reachable by cursor and invisible -- the grid binds a column by resolving its namer
    /// entry, so a record nobody names is never asked for. The two id lists agreeing is what says
    /// the stand-in is naming a cell that exists.
    #[test]
    fn the_layouts_cells_and_the_namers_cells_are_the_same_cells() {
        assert_eq!(
            ds2_rva::FLO_TAB_STRIP_CELL_IDS,
            ds2_rva::FE_INGAME_TOP_SELECT_NAMER_CELL_IDS,
            "the strip's namer and its layout disagree about which cells exist"
        );
        // The stand-in clones the namer's LAST entry and rewrites its id, so the cell it names must
        // be the one this module's added record carries.
        assert!(!ds2_rva::FE_INGAME_TOP_SELECT_NAMER_CELL_IDS.contains(&ds2_rva::FLO_ADDED_TAB_ID));
    }

    /// The added tab's id is not one of the six, which is what keeps the identity check meaningful.
    #[test]
    fn the_added_id_is_not_one_the_strip_already_uses() {
        assert!(!ds2_rva::FLO_TAB_STRIP_CELL_IDS.contains(&ds2_rva::FLO_ADDED_TAB_ID));
        assert!(!ds2_rva::FLO_TAB_STRIP_CELL_IDS.contains(&ds2_rva::FLO_ADDED_TAB_SUBTREE_ID));
        assert_ne!(
            ds2_rva::FLO_ADDED_TAB_SUBTREE_ID,
            ds2_rva::FLO_TAB_STRIP_PANEL_ID
        );
    }

    /// The added subtree goes in beside its template and before the cells, and the added cell goes
    /// after them. Both halves matter: two panels drawn in the wrong order is the defect this
    /// record exists to avoid, one level up from the one it fixes.
    #[test]
    fn the_added_subtree_precedes_the_cells_and_the_added_cell_follows_them() {
        const {
            assert!(PANEL_AT == ds2_rva::FLO_TAB_STRIP_PANEL + 1);
            // A tab's panel inserted among the cells would draw over them.
            assert!(PANEL_AT <= ds2_rva::FLO_TAB_STRIP_FIRST_CELL);
            assert!(CELL_AT == CHILDREN - 1);
            assert!(
                CELL_AT > ds2_rva::FLO_TAB_STRIP_FIRST_CELL + ds2_rva::FLO_TAB_STRIP_CELL_IDS.len()
            );
        }
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
