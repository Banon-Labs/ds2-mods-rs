//! An infusion container for item cells that were authored without one.
//!
//! # Why the Attune Spell screen had nowhere to put the X
//!
//! The bonfire's Attune Spell picker and its attunement grid both draw their rows from
//! `l03_01_Bonfire.flo` def `0x00ad`: the icon group `0x5f5c3e0`, the highlight `0x5f5c800`, and a
//! frame. No infusion container. Measured on a running game (2026-09-27,
//! `scripts/frida/attune-badge-diag.js`): every picker row's `view + 0x2d0` accessor resolved to
//! nothing, and opening the screen built `0x00ad` 78 times through the container builder with no
//! nine-id container anywhere under it -- so [`crate::mark`] never saw one to add the badge to, and
//! the bind's `setVisible` on the badge id reached no element.
//!
//! The same document authors a second item cell, def `0x0078` (the bonfire's other item grid), whose
//! layout is the same -- icon group at `(-7.85, -5.5)` in both -- and which does carry the container,
//! as child `[2]`, id `0x5f5c3e2`, def `0x0070`. So the fix is the game's own record: when the
//! builder is about to walk a cell that has the icon group and the highlight but no container, and
//! the same document holds a cell with the icon group at the same place and a container, the
//! container record is copied onto the end of the first cell's children. The builder then builds the
//! container through [`crate::mark`]'s detour, which adds the badge, and the cell view's accessor at
//! `+0x2d0` -- path `0x5f5c3e2` from the cell -- finds it.
//!
//! The copied record is byte for byte the donor's: same definition index, same transform block,
//! same depth, same frame range. Nothing about where the container sits is invented here.

use std::sync::Mutex;

use crate::LOG_PREFIX;
use crate::install::log;
use crate::mark::{children_of, is_infusion_container};

/// Most children a cell can have for this to consider it. Both cells here have three and five.
const MAX_CHILDREN: usize = 8;

/// `[doc + 0x18]` is the definition table and `[doc + 0x4c]` its `u16` count, where `doc` is the
/// object the builder's second argument points at -- the scan `FLO_FIND_DEFINITION` performs.
const DOC_DEFINITIONS_OFFSET: usize = 0x18;
const DOC_DEFINITION_COUNT_OFFSET: usize = 0x4c;

/// Upper bound on a document's definition count, as a sanity check on what was read.
const MAX_DEFINITIONS: usize = 0x1000;

/// A replacement cell definition: its definition bytes, its records with the container appended,
/// and the records it was copied from, to check a cache hit against.
#[repr(C, align(16))]
struct Cell {
    definition: [u8; ds2_rva::FLO_DEFINITION_STRIDE],
    records: [u8; ds2_rva::FLO_RECORD_STRIDE * (MAX_CHILDREN + 1)],
    shipped: [u8; ds2_rva::FLO_RECORD_STRIDE * MAX_CHILDREN],
    count: usize,
}

/// Substitutions built, `(the definition the game passed, the Cell this passes on)`.
static BUILT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Most cell substitutions to keep, for the same reason [`crate::mark`] bounds its own.
const MAX_BUILT: usize = 64;

fn sane(at: usize) -> bool {
    at >= 0x1_0000
}

/// # Safety
///
/// `at` must be readable.
unsafe fn read_usize(at: usize) -> usize {
    // SAFETY: the caller guarantees the address is live.
    unsafe { (at as *const usize).read_unaligned() }
}

/// # Safety
///
/// `records` must hold at least `slot + 1` live records.
unsafe fn id_of(records: *const u8, slot: usize) -> u32 {
    // SAFETY: the record is live and the id is a `u32` inside it.
    unsafe {
        records
            .add(slot * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_ID_OFFSET)
            .cast::<u32>()
            .read_unaligned()
    }
}

/// The `(x, y)` a record's transform block places it at, or `None` for a missing block.
///
/// # Safety
///
/// `records` must hold at least `slot + 1` live records.
unsafe fn xy_of(records: *const u8, slot: usize) -> Option<(f32, f32)> {
    // SAFETY: the record is live and its transform pointer is a field inside it.
    let transform = unsafe {
        records
            .add(slot * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET)
            .cast::<usize>()
            .read_unaligned()
    };
    if !sane(transform) {
        return None;
    }
    // SAFETY: the pointer is the record's own transform block, whose first two floats are x, y.
    unsafe {
        Some((
            ((transform + ds2_rva::FLO_TRANSFORM_X_OFFSET) as *const f32).read_unaligned(),
            ((transform + ds2_rva::FLO_TRANSFORM_Y_OFFSET) as *const f32).read_unaligned(),
        ))
    }
}

/// Which slots of a record array carry the icon group, the highlight and the container.
///
/// # Safety
///
/// `records` must hold `count` live records.
unsafe fn roles(records: *const u8, count: usize) -> (Option<usize>, bool, Option<usize>) {
    let (mut icon, mut highlight, mut container) = (None, false, None);
    for slot in 0..count {
        // SAFETY: `slot < count`.
        match unsafe { id_of(records, slot) } {
            ds2_rva::FE_ITEM_CELL_INFUSION_ELEMENT_BASE => icon = icon.or(Some(slot)),
            ds2_rva::FE_ITEM_CELL_HIGHLIGHT_ELEMENT => highlight = true,
            ds2_rva::FE_EQUIP_SLOT_CONTAINER_ELEMENT => container = container.or(Some(slot)),
            _ => {}
        }
    }
    (icon, highlight, container)
}

/// The document's definition table, as `(first definition, count)`.
///
/// # Safety
///
/// `doc` must be the builder's own second argument.
unsafe fn definitions(doc: usize) -> Option<(usize, usize)> {
    if !sane(doc) {
        return None;
    }
    // SAFETY: the builder's argument points at the document object, whose table and count are the
    // two fields `FLO_FIND_DEFINITION` reads (`mov rax,[rcx]`, `[rax+0x18]`, `[rax+0x4c]`).
    unsafe {
        let object = read_usize(doc);
        if !sane(object) {
            return None;
        }
        let table = read_usize(object + DOC_DEFINITIONS_OFFSET);
        let count =
            ((object + DOC_DEFINITION_COUNT_OFFSET) as *const u16).read_unaligned() as usize;
        (sane(table) && count > 0 && count <= MAX_DEFINITIONS).then_some((table, count))
    }
}

/// The definition keyed `key` in the table, the scan `FLO_FIND_DEFINITION` performs.
///
/// # Safety
///
/// The table must be live for `count` definitions.
unsafe fn find(table: usize, count: usize, key: u16) -> Option<*const u8> {
    (0..count)
        .map(|i| (table + i * ds2_rva::FLO_DEFINITION_STRIDE) as *const u8)
        // SAFETY: inside the table; the key is the definition's leading `u16`.
        .find(|definition| unsafe { definition.cast::<u16>().read_unaligned() } == key)
}

/// A container record to copy: one from a cell in the same document whose icon group sits exactly
/// where this cell's does, naming a definition that is the nine-id infusion container.
///
/// # Safety
///
/// `doc` must be the builder's own second argument.
unsafe fn donor(doc: usize, icon_at: (f32, f32)) -> Option<(*const u8, u16)> {
    // SAFETY: as the caller guarantees.
    let (table, count) = unsafe { definitions(doc) }?;
    for i in 0..count {
        let definition = (table + i * ds2_rva::FLO_DEFINITION_STRIDE) as *const u8;
        // SAFETY: a definition inside the live table.
        let (children, records) = unsafe { children_of(definition) };
        if children == 0 || children > MAX_CHILDREN || !sane(records as usize) {
            continue;
        }
        // SAFETY: `children` live records.
        let (Some(icon), true, Some(container)) = (unsafe { roles(records, children) }) else {
            continue;
        };
        // SAFETY: `icon < children`.
        if unsafe { xy_of(records, icon) } != Some(icon_at) {
            continue;
        }
        // SAFETY: `container < children`; the definition index is the record's leading `u16`.
        let (record, key) = unsafe {
            let record = records.add(container * ds2_rva::FLO_RECORD_STRIDE);
            (
                record,
                record
                    .add(ds2_rva::FLO_RECORD_DEFINITION_OFFSET)
                    .cast::<u16>()
                    .read_unaligned(),
            )
        };
        // SAFETY: the table is live for `count` definitions.
        let Some(named) = (unsafe { find(table, count, key) }) else {
            continue;
        };
        // SAFETY: a definition inside the live table.
        let (nine, glyphs) = unsafe { children_of(named) };
        // SAFETY: `nine` live records when the pointer is sane.
        if sane(glyphs as usize) && unsafe { is_infusion_container(glyphs, nine) } {
            return Some((record, key));
        }
    }
    None
}

/// The replacement for a cell definition that lacks the container, or `None`.
///
/// # Safety
///
/// `doc` and `definition` must be the builder's own arguments.
pub(crate) unsafe fn substitution(doc: usize, definition: *mut u8) -> Option<*mut u8> {
    if !sane(definition as usize) {
        return None;
    }
    // SAFETY: the game passes a live definition to its own builder.
    let (count, records) = unsafe { children_of(definition) };
    if count == 0 || count > MAX_CHILDREN || !sane(records as usize) {
        return None;
    }
    // SAFETY: `count` live records.
    let (Some(icon), true, None) = (unsafe { roles(records, count) }) else {
        return None;
    };
    let bytes = count * ds2_rva::FLO_RECORD_STRIDE;

    let mut built = BUILT.lock().ok()?;
    if let Some(slot) = built
        .iter()
        .position(|(key, _)| *key == definition as usize)
    {
        let cached = built[slot].1 as *mut Cell;
        // SAFETY: `cached` is a leaked `Cell` built from this definition, never freed and only
        // written before it was published; `records` is live.
        let current = unsafe {
            let cell: &Cell = &*cached;
            cell.count == count
                && std::slice::from_raw_parts(records, bytes) == &cell.shipped[..bytes]
        };
        if current {
            // SAFETY: as above; a `Cell` begins with its definition.
            return Some(unsafe { (&raw mut (*cached).definition).cast::<u8>() });
        }
        built.remove(slot);
    }
    if built.len() >= MAX_BUILT {
        return None;
    }

    // SAFETY: `icon < count`.
    let icon_at = unsafe { xy_of(records, icon) }?;
    // SAFETY: `doc` is the builder's argument.
    let (container, key) = unsafe { donor(doc, icon_at) }?;

    let mut cell = Box::new(Cell {
        // SAFETY: a definition is this many bytes.
        definition: unsafe {
            std::ptr::read_unaligned(definition.cast::<[u8; ds2_rva::FLO_DEFINITION_STRIDE]>())
        },
        records: [0; ds2_rva::FLO_RECORD_STRIDE * (MAX_CHILDREN + 1)],
        shipped: [0; ds2_rva::FLO_RECORD_STRIDE * MAX_CHILDREN],
        count,
    });
    // SAFETY: `count` live records, and the donor record is one live record.
    unsafe {
        let shipped = std::slice::from_raw_parts(records, bytes);
        cell.shipped[..bytes].copy_from_slice(shipped);
        cell.records[..bytes].copy_from_slice(shipped);
        cell.records[bytes..bytes + ds2_rva::FLO_RECORD_STRIDE].copy_from_slice(
            std::slice::from_raw_parts(container, ds2_rva::FLO_RECORD_STRIDE),
        );
    }
    let records_at = cell.records.as_ptr() as u64;
    cell.definition[ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET..][..8]
        .copy_from_slice(&records_at.to_le_bytes());
    cell.definition[ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET..][..2]
        .copy_from_slice(&((count + 1) as u16).to_le_bytes());

    let leaked: &'static mut Cell = Box::leak(cell);
    // SAFETY: the definition's leading `u16` is its key.
    let own = unsafe { definition.cast::<u16>().read_unaligned() };
    log(format_args!(
        "{LOG_PREFIX} cell given container definition={own:#06x} children={count}->{} \
         container-definition={key:#06x} icon-at=({:.2},{:.2}) -- the record is the donor cell's, \
         copied whole",
        count + 1,
        icon_at.0,
        icon_at.1,
    ));
    built.push((definition as usize, leaked as *mut Cell as usize));
    Some((&raw mut leaked.definition).cast::<u8>())
}
