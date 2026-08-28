//! The fourth row's CELL: added to the layout document in memory, not to the archive on disk.
//!
//! # Why this is a pointer edit and not a repack
//!
//! The pause menu's rows are records in `menu/02.febnd.dcx`'s `l02_01_In-Game.flo`, and the drawn
//! row count is a census of those records -- which is why appending a fourth item to the tab's
//! vector produced a row that could be selected and could not be seen.
//!
//! The obvious fix is to edit the file: decompress the DCX, rebuild the BND4, serve it from a
//! loose path. That was the plan (`docs/DS2-INGAME-MENU.md`, option 2) until the loader was read
//! properly. It loads the `.flo` IN PLACE -- the file header *is* the document object, and the
//! `u64` offsets inside it are absolute pointers once the fixup has run -- and every consumer of a
//! container's child list goes through one lookup, [`ds2_rva::FLO_FIND_DEFINITION`].
//!
//! So this detours that lookup, and when the quit tab's container is asked for, hands back a copy
//! of the definition with two more children and a child array of our own. No zlib, no container
//! writer, no file on disk, and nothing to keep in sync with a game update beyond the ids checked
//! below.
//!
//! # What "two more children" buys, and why the count is also the capacity
//!
//! `FUN_140b6bd80` -- the attach -- refuses once the parent's live child count reaches
//! `[parent->definition + 0x02]`. That is the same field the builder reads as "how many child
//! records to walk". One number, both meanings: raising it grows the display list *and* gets the
//! extra records walked. Nothing else has to be resized.
//!
//! Two, not one. Every shipped row is a pair -- the row itself and a mark at x `60.2` -- and a row
//! missing its mark is visibly not the same kind of thing as its neighbours.
//!
//! # The row is the icon and its highlight; the mark is the caption
//!
//! A row's definition holds exactly two children and nothing else: the icon, and a shape at
//! `(6.9, -3.45)` whose colour is `00ffffff`. The text is not in there -- that is the `0x022c`
//! mark beside it, bound by [`crate::caption`].
//!
//! **The second child is the SELECTION HIGHLIGHT, and reading it as decoration cost a run.** It is
//! transparent at rest and carries a 69-frame range, which in the file looks like a flourish; on
//! screen it is what lights up under the cursor. Pointing the container's row record straight at
//! [`ds2_rva::FLO_QUIT_ICON_DEFINITION`] put the right glyph on the row and took the highlight
//! away in the same stroke, with nothing in the log to say so.
//!
//! So the row gets its OWN DEFINITION -- a copy of Quit Game's with only the icon child changed,
//! served under [`ds2_rva::FLO_ADDED_ROW_DEFINITION`] the way the caret's panel is. The icon
//! becomes the glyph alone, tinted red, because the row above it is the other Quit Game and two
//! identical icons would be a puzzle rather than a menu.
//!
//! # What makes this safe to be wrong about
//!
//! A definition index is a number, and `0x263` on a document this was not read from is some other
//! container. So the substitution happens only when the definition the game returned has exactly
//! seven children carrying exactly [`ds2_rva::FLO_QUIT_TAB_CHILD_IDS`], in order. Anything else is
//! logged and passed through untouched, and the game gets the menu it shipped with.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// `fn(&doc, index) -> *definition`, null on a miss.
///
/// The first argument is a HOLDER, not the document: `FUN_140b54740` opens `mov rax,[rcx]` and
/// works from that. It is passed straight through, so its shape does not matter here.
type FindDefinitionFn = unsafe extern "system" fn(*mut usize, u32) -> *mut u8;

static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// How many times the container was substituted, and how many times it was refused.
///
/// Both, because "no fourth row" has two readings -- the check said no, or the menu was never
/// built -- and one counter cannot separate them.
static SUBSTITUTED: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);

/// Children each added row costs: the row and its mark.
const PER_ROW: usize = 2;

/// Rows this can carry. The struct below is sized for it, and the registry refuses past it.
const MAX_ROWS: usize = crate::api::MAX_ADDED_ROWS;

/// Children the substituted definition can declare at most. What it DOES declare is
/// `shipped + PER_ROW * rows.len()`, which is smaller whenever fewer rows are registered.
const CHILDREN: usize = ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len() + PER_ROW * MAX_ROWS;

/// Index of slot `n`'s row and mark inside the new child array.
///
/// The shipped records keep indices `0..7` untouched; ours are appended in pairs, so slot 0 is
/// `(7, 8)` and slot 1 is `(9, 10)`. Pairs rather than two blocks because the transform pointers
/// are written back per index and interleaving keeps the arithmetic in one place.
const fn row_at(slot: usize) -> usize {
    ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len() + PER_ROW * slot
}
const fn mark_at(slot: usize) -> usize {
    row_at(slot) + 1
}

/// Depths given to the added records.
///
/// The shipped rows carry `14, 10, 4` top to bottom and their marks `22, 20, 18`, so a fourth of
/// each continues both series into a value nothing else uses. It is very likely cosmetic --
/// `FUN_140b50bc0` passes this field on only for the LEAF kinds, and both of these are nested
/// definitions, so the game never reads it back. Continuing the series costs nothing and leaves
/// the array readable next to the shipped one.
///
/// Slot `n` steps down from each base, into the gaps `2, 3` and `15, 16` -- values no shipped
/// record uses, which is asserted rather than eyeballed.
const ROW_DEPTH: u16 = 3;
const MARK_DEPTH: u16 = 16;

const fn row_depth(slot: usize) -> u16 {
    ROW_DEPTH - slot as u16
}
const fn mark_depth(slot: usize) -> u16 {
    MARK_DEPTH - slot as u16
}

/// A replacement container definition, its child records, and the two transform blocks the added
/// records point at -- one allocation so the pointers between them cannot outlive each other.
///
/// `align(16)` because the fields are byte arrays, which would otherwise let the allocator hand
/// back an odd address for a block the game reads `u64` pointers and `u16` counts out of.
#[repr(C, align(16))]
struct Container {
    definition: [u8; ds2_rva::FLO_DEFINITION_STRIDE],
    records: [u8; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
    /// One transform block per added row, and one per added mark. Fixed-size rather than a `Vec`
    /// because the game's own item vector caps a tab at five rows -- there is no allocation to
    /// grow, only two slots to leave empty when one row is registered.
    row_transform: [[u8; ds2_rva::FLO_TRANSFORM_SIZE]; MAX_ROWS],
    mark_transform: [[u8; ds2_rva::FLO_TRANSFORM_SIZE]; MAX_ROWS],
    panel_transform: [u8; ds2_rva::FLO_TRANSFORM_SIZE],
    /// The shipped records exactly as the game had them, kept only so [`still_current`] has
    /// something to compare against.
    ///
    /// **`records` cannot be that thing, and assuming it could made the cache miss every single
    /// time.** The copy in `records` has its panel transform repointed at our own block, so it
    /// differs from the game's array by eight bytes and always will. Every lookup therefore
    /// rebuilt, leaked another container, and logged two fsynced lines -- which is most of the
    /// freeze the user saw on opening the menu.
    shipped: [u8; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len()],
    /// The quit tab's OWN copy of panel definition `0x0221`, so its caret can move without moving
    /// the other two tabs'.
    panel_definition: [u8; ds2_rva::FLO_DEFINITION_STRIDE],
    panel_records: [u8; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_PANEL_CHILDREN],
    caret_transform: [u8; ds2_rva::FLO_TRANSFORM_SIZE],
    /// The added row's OWN copy of row definition `0x0258` -- Quit Game's -- with its icon child
    /// repointed at the glyph alone and tinted.
    ///
    /// **Both children are copied, and the second one is why this exists at all.** A row
    /// definition holds an icon and a shape at `(6.9, -3.45)` whose colour is `00ffffff`, and that
    /// second, invisible-at-rest child is the SELECTION HIGHLIGHT. Pointing the container's row
    /// record straight at the icon put the right glyph on screen and silently took the highlight
    /// with it.
    row_definition: [[u8; ds2_rva::FLO_DEFINITION_STRIDE]; MAX_ROWS],
    row_records: [[u8; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_QUIT_ROW_CHILDREN]; MAX_ROWS],
    icon_transform: [[u8; ds2_rva::FLO_TRANSFORM_SIZE]; MAX_ROWS],
}

/// Substitutions already built, as `(definition the game returned, definition we return)`.
///
/// Built once per DOCUMENT rather than once per process, because the records this copies hold
/// pointers into the document's buffer and a document can be unloaded and reloaded. In practice
/// the pause menu's `.flo` stays loaded for a session and this is one entry, one allocation.
///
/// **The address is not enough of a key.** A reload can land on the same address with new
/// contents, and the cached copy would then hand the game records pointing at freed transform
/// blocks -- a crash whose stack contains nothing of ours. So a hit is only a hit if the seven
/// copied records still byte-match the seven the game is holding -- compared against the pristine
/// `shipped` snapshot, NOT against `records`, which is edited by construction. See
/// [`still_current`].
static BUILT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// The quit tab's own panel definition, answered for [`ds2_rva::FLO_ADDED_PANEL_DEFINITION`].
///
/// A plain pointer rather than a cache entry: the index is one nothing in the file uses, so a
/// lookup for it can only have come from the record this crate wrote, and the newest container is
/// always the right answer.
static PANEL: AtomicUsize = AtomicUsize::new(0);

/// The added rows' own definitions, answered for [`ds2_rva::FLO_ADDED_ROW_DEFINITION`]` + slot`.
/// As [`PANEL`], and for the same reason.
static ROW_DEFINITIONS: [AtomicUsize; MAX_ROWS] = [const { AtomicUsize::new(0) }; MAX_ROWS];

/// Most substitutions to keep. Past this the detour passes through and says so. The bound exists
/// so that a misunderstanding shows up as a logged refusal rather than as unbounded growth; at
/// ~0x210 bytes each it is not the memory that matters.
const MAX_BUILT: usize = 64;

/// Whether a cached substitution still describes the document the game is holding.
///
/// # Safety
///
/// `original` must be a live definition and `cached` a [`Container`] this module built from it.
unsafe fn still_current(original: *const u8, cached: *const Container) -> bool {
    // SAFETY: both are definitions, and this is the field the game itself reads first.
    let (count, children) = unsafe {
        (
            original
                .add(ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET)
                .cast::<u16>()
                .read() as usize,
            original
                .add(ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET)
                .cast::<*const u8>()
                .read(),
        )
    };
    if count != ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len() || (children as usize) < 0x1_0000 {
        return false;
    }
    let shipped = count * ds2_rva::FLO_RECORD_STRIDE;
    // SAFETY: `count` records are live at `children`, and the cached copy is at least that large --
    // it holds `CHILDREN` records, of which the first `count` are the copies being compared.
    unsafe {
        std::slice::from_raw_parts(children, shipped)
            == std::slice::from_raw_parts((*cached).shipped.as_ptr(), shipped)
    }
}

/// Read `count` child ids out of a record array.
///
/// # Safety
///
/// `children` must point at `count` records of [`ds2_rva::FLO_RECORD_STRIDE`] bytes.
unsafe fn child_ids(children: *const u8, count: usize) -> Vec<u32> {
    (0..count)
        .map(|i| {
            // SAFETY: `i < count`, and the caller guarantees that many records are live.
            unsafe {
                children
                    .add(i * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_ID_OFFSET)
                    .cast::<u32>()
                    .read()
            }
        })
        .collect()
}

/// The twelve `f32` of a transform block, for a log line.
fn floats(block: &[u8]) -> String {
    block
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| format!("{}", f32::from_le_bytes(*c)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `0x1eac81 0x1eace9 ...`, for a log line.
fn describe(ids: &[u32]) -> String {
    ids.iter()
        .map(|id| format!("{id:#x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the replacement container, or explain in the log why not.
///
/// # Safety
///
/// `original` must be the definition [`ds2_rva::FLO_FIND_DEFINITION`] just returned for
/// [`ds2_rva::FLO_QUIT_TAB_CONTAINER_DEFINITION`], in a loaded document.
unsafe fn build(original: *mut u8, panel: *mut u8, row: *mut u8) -> Option<*mut u8> {
    let refuse = |why: std::fmt::Arguments<'_>| -> Option<*mut u8> {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} container REFUSED {why} refusals={n} -- the menu is the shipped one"
        ));
        None
    };

    // SAFETY: the game just returned this pointer from its own table and reads both fields itself.
    let (count, children) = unsafe {
        (
            original
                .add(ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET)
                .cast::<u16>()
                .read() as usize,
            original
                .add(ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET)
                .cast::<*mut u8>()
                .read(),
        )
    };
    if count != ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len() {
        return refuse(format_args!(
            "children={count}, expected {}",
            ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len()
        ));
    }
    // A record array is a pointer once the document is loaded; a small integer means the fixup has
    // not run and everything below would be reading a file offset as an address.
    if (children as usize) < 0x1_0000 {
        return refuse(format_args!(
            "child array is {children:p}, which is a file offset rather than a pointer"
        ));
    }
    // SAFETY: `count` records are live -- the game's own builder is about to walk exactly this
    // many from exactly this pointer.
    let ids = unsafe { child_ids(children, count) };
    if ids != ds2_rva::FLO_QUIT_TAB_CHILD_IDS {
        return refuse(format_args!(
            "ids=[{}] expected=[{}]",
            describe(&ids),
            describe(&ds2_rva::FLO_QUIT_TAB_CHILD_IDS)
        ));
    }

    // THE ROWS THIS CONTAINER WILL CARRY, read once. `install` seals the registry before it
    // patches anything, so this cannot change between here and the log line at the bottom.
    let rows = crate::api::rows_for(crate::api::Tab::Quit);
    if rows.is_empty() {
        return refuse(format_args!("no rows are registered for this tab"));
    }
    if rows.len() > MAX_ROWS {
        return refuse(format_args!(
            "{} rows are registered and this container is sized for {MAX_ROWS}",
            rows.len()
        ));
    }
    let declared = count + PER_ROW * rows.len();

    let mut container = Box::new(Container {
        definition: [0; ds2_rva::FLO_DEFINITION_STRIDE],
        records: [0; ds2_rva::FLO_RECORD_STRIDE * CHILDREN],
        row_transform: [[0; ds2_rva::FLO_TRANSFORM_SIZE]; MAX_ROWS],
        mark_transform: [[0; ds2_rva::FLO_TRANSFORM_SIZE]; MAX_ROWS],
        panel_transform: [0; ds2_rva::FLO_TRANSFORM_SIZE],
        shipped: [0; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len()],
        panel_definition: [0; ds2_rva::FLO_DEFINITION_STRIDE],
        panel_records: [0; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_PANEL_CHILDREN],
        caret_transform: [0; ds2_rva::FLO_TRANSFORM_SIZE],
        row_definition: [[0; ds2_rva::FLO_DEFINITION_STRIDE]; MAX_ROWS],
        row_records: [[0; ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_QUIT_ROW_CHILDREN]; MAX_ROWS],
        icon_transform: [[0; ds2_rva::FLO_TRANSFORM_SIZE]; MAX_ROWS],
    });

    // COPIED, never assembled field by field. The definition and the records carry fields this
    // crate has not decoded, and a zero in one of them is a guess about the format wearing the
    // shape of a value.
    // SAFETY: `original` is a definition and `children` a `count`-record array, both established
    // above; the destinations are exactly as large.
    unsafe {
        std::ptr::copy_nonoverlapping(
            original,
            container.definition.as_mut_ptr(),
            ds2_rva::FLO_DEFINITION_STRIDE,
        );
        std::ptr::copy_nonoverlapping(
            children,
            container.records.as_mut_ptr(),
            count * ds2_rva::FLO_RECORD_STRIDE,
        );
        std::ptr::copy_nonoverlapping(
            children,
            container.shipped.as_mut_ptr(),
            count * ds2_rva::FLO_RECORD_STRIDE,
        );
    }

    // The added records are clones of shipped ones, so everything about them -- definition index,
    // kind, frame range, the leaf contents further down -- is the game's own.
    let clone_into = |records: &mut [u8], to: usize, from: usize| {
        let (source, destination) = (
            from * ds2_rva::FLO_RECORD_STRIDE,
            to * ds2_rva::FLO_RECORD_STRIDE,
        );
        records.copy_within(source..source + ds2_rva::FLO_RECORD_STRIDE, destination);
    };
    for slot in 0..rows.len() {
        clone_into(
            &mut container.records,
            row_at(slot),
            ds2_rva::FLO_QUIT_TAB_ROW_TEMPLATE,
        );
        clone_into(
            &mut container.records,
            mark_at(slot),
            ds2_rva::FLO_QUIT_TAB_MARK_TEMPLATE,
        );
    }

    // Same for the transform blocks: copied from the record being cloned, then moved down one step
    // per slot. `template_transform` reads the pointer out of a record the game itself
    // dereferences; a small integer there means the document's fixup has not run and everything
    // below would be treating a file offset as an address.
    let template_transform = |records: &[u8], template: usize| -> *const u8 {
        // SAFETY: `template` indexes a record copied verbatim from the game's own array.
        unsafe {
            records
                .as_ptr()
                .add(template * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET)
                .cast::<*const u8>()
                .read()
        }
    };
    let mut sources = [std::ptr::null::<u8>(); 3];
    for (index, template) in [
        ds2_rva::FLO_QUIT_TAB_ROW_TEMPLATE,
        ds2_rva::FLO_QUIT_TAB_MARK_TEMPLATE,
        ds2_rva::FLO_QUIT_TAB_PANEL,
    ]
    .into_iter()
    .enumerate()
    {
        let source = template_transform(&container.records, template);
        if (source as usize) < 0x1_0000 {
            return refuse(format_args!(
                "record {template}'s transform is {source:p}, which is a file offset"
            ));
        }
        sources[index] = source;
    }
    let source_panel = sources[2];
    // SAFETY: every source is a live transform block and every destination is exactly that size.
    unsafe {
        std::ptr::copy_nonoverlapping(
            source_panel,
            container.panel_transform.as_mut_ptr(),
            ds2_rva::FLO_TRANSFORM_SIZE,
        );
        for slot in 0..rows.len() {
            std::ptr::copy_nonoverlapping(
                sources[0],
                container.row_transform[slot].as_mut_ptr(),
                ds2_rva::FLO_TRANSFORM_SIZE,
            );
            std::ptr::copy_nonoverlapping(
                sources[1],
                container.mark_transform[slot].as_mut_ptr(),
                ds2_rva::FLO_TRANSFORM_SIZE,
            );
        }
    }
    for (slot, row) in rows.iter().enumerate() {
        let (row_x, row_y) = row.row_xy();
        let (mark_x, mark_y) = row.mark_xy();
        container.row_transform[slot][ds2_rva::FLO_TRANSFORM_X_OFFSET..][..4]
            .copy_from_slice(&row_x.to_le_bytes());
        container.row_transform[slot][ds2_rva::FLO_TRANSFORM_Y_OFFSET..][..4]
            .copy_from_slice(&row_y.to_le_bytes());
        container.mark_transform[slot][ds2_rva::FLO_TRANSFORM_X_OFFSET..][..4]
            .copy_from_slice(&mark_x.to_le_bytes());
        container.mark_transform[slot][ds2_rva::FLO_TRANSFORM_Y_OFFSET..][..4]
            .copy_from_slice(&mark_y.to_le_bytes());
    }

    // THE SCROLL. The panel is one fixed graphic with three row slots and about half a row of
    // slack under the last one, so a fourth row hangs off the bottom of it. Its record is copied
    // like the others and only its vertical scale changes -- see `FLO_PANEL_STRETCH_Y`, which is
    // arithmetic on two measurements rather than a number that looked about right.
    let scale_at = ds2_rva::FLO_TRANSFORM_SCALE_Y_OFFSET;
    let scale = f32::from_le_bytes(
        container.panel_transform[scale_at..][..4]
            .try_into()
            .expect("four bytes"),
    );
    if !(0.5..=2.0).contains(&scale) {
        return refuse(format_args!(
            "the panel's scale-y is {scale}, which is not the 1.0 every tab ships with"
        ));
    }
    container.panel_transform[scale_at..][..4]
        .copy_from_slice(&(scale * ds2_rva::FLO_PANEL_STRETCH_Y).to_le_bytes());
    // THE WHOLE BLOCK, BEFORE AND AFTER. Two runs reported no visible change from a 17% stretch,
    // which is 58 units on a 341-unit scroll -- so the question stopped being "what factor" and
    // became "did the write land, and on which field". Printing the block makes a null result
    // diagnosable instead of another thing to guess about.
    // SAFETY: `source_panel` is the live transform this block was copied from, and both are
    // `FLO_TRANSFORM_SIZE` bytes.
    let before = unsafe { std::slice::from_raw_parts(source_panel, ds2_rva::FLO_TRANSFORM_SIZE) };
    log(format_args!(
        "{LOG_PREFIX} panel transform at=0x{:016x} before=[{}] after=[{}] scaled-offset={scale_at:#x}",
        source_panel as usize,
        floats(before),
        floats(&container.panel_transform),
    ));

    for (slot, row) in rows.iter().enumerate() {
        for (index, id, depth) in [
            (row_at(slot), row.row_id, row_depth(slot)),
            (mark_at(slot), row.label_id, mark_depth(slot)),
        ] {
            let at = index * ds2_rva::FLO_RECORD_STRIDE;
            container.records[at + ds2_rva::FLO_RECORD_ID_OFFSET..][..4]
                .copy_from_slice(&id.to_le_bytes());
            container.records[at + ds2_rva::FLO_RECORD_DEPTH_OFFSET..][..2]
                .copy_from_slice(&depth.to_le_bytes());
        }
    }

    // THE ROWS' OWN DEFINITIONS. A cloned record names row 0's definition, which is the Game
    // Options glyph; what each row should name is a glyph of its caller's choosing, tinted. That
    // is not a field on the record -- a row definition holds an icon AND the selection highlight,
    // and repointing the record at a bare glyph loses the second one -- so every row gets a copy
    // of row 2's definition with only its icon child changed. Same substitution as the panel, one
    // level down, once per slot.
    //
    // The SOURCE is validated once: it is the same definition for every row. If any check says no,
    // every record keeps naming row 0's definition -- an untinted icon with a working highlight,
    // which is what shipped before this block existed.
    let mut own_definitions = 0usize;
    if !row.is_null() {
        // SAFETY: `row` is a definition the game's own table just yielded.
        let (row_count, row_children) = unsafe {
            (
                row.add(ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET)
                    .cast::<u16>()
                    .read() as usize,
                row.add(ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET)
                    .cast::<*const u8>()
                    .read(),
            )
        };
        if row_count == ds2_rva::FLO_QUIT_ROW_CHILDREN && (row_children as usize) >= 0x1_0000 {
            let icon = ds2_rva::FLO_QUIT_ROW_ICON * ds2_rva::FLO_RECORD_STRIDE;
            let highlight = ds2_rva::FLO_QUIT_ROW_HIGHLIGHT * ds2_rva::FLO_RECORD_STRIDE;
            let names = |at: usize| -> u32 {
                // SAFETY: `at` is inside the `row_count` records established live above.
                unsafe {
                    row_children
                        .add(at + ds2_rva::FLO_RECORD_DEFINITION_OFFSET)
                        .cast::<u16>()
                        .read() as u32
                }
            };
            // Both children are checked, not just the one being replaced. `0x0258` on a document
            // this was not read from is some other row, and copying its highlight into ours would
            // be the same class of mistake as substituting the wrong container.
            let (found_icon, found_highlight) = (names(icon), names(highlight));
            if found_icon != ds2_rva::FLO_QUIT_ROW_ICON_GROUP
                || found_highlight != ds2_rva::FLO_QUIT_ROW_HIGHLIGHT_DEFINITION
            {
                log(format_args!(
                    "{LOG_PREFIX} row definition REFUSED icon={found_icon:#x} \
                     highlight={found_highlight:#x}, expected {:#x} and {:#x} -- the added rows \
                     keep row 0's icon",
                    ds2_rva::FLO_QUIT_ROW_ICON_GROUP,
                    ds2_rva::FLO_QUIT_ROW_HIGHLIGHT_DEFINITION
                ));
            } else {
                // SAFETY: the record is one the game itself dereferences the transform of.
                let icon_source = unsafe {
                    row_children
                        .add(icon + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET)
                        .cast::<*const u8>()
                        .read()
                };
                if (icon_source as usize) < 0x1_0000 {
                    log(format_args!(
                        "{LOG_PREFIX} row definition REFUSED icon transform is {icon_source:p}, \
                         which is a file offset -- the added rows keep row 0's icon"
                    ));
                } else {
                    for (slot, spec) in rows.iter().enumerate() {
                        // SAFETY: `row_count` records are live at `row_children`, the source
                        // transform is live, and every destination is exactly as large.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                row,
                                container.row_definition[slot].as_mut_ptr(),
                                ds2_rva::FLO_DEFINITION_STRIDE,
                            );
                            std::ptr::copy_nonoverlapping(
                                row_children,
                                container.row_records[slot].as_mut_ptr(),
                                row_count * ds2_rva::FLO_RECORD_STRIDE,
                            );
                            std::ptr::copy_nonoverlapping(
                                icon_source,
                                container.icon_transform[slot].as_mut_ptr(),
                                ds2_rva::FLO_TRANSFORM_SIZE,
                            );
                        }
                        // The caller's glyph, and no id: `0x0255` pairs the quit glyph with a
                        // greyed-out twin under `0x1eacd0`, and the availability pass only takes
                        // that twin off a row whose GATE is nonzero. Every registered row is
                        // ungated, so a cloned twin would draw `ff808080` for the life of the
                        // process.
                        container.row_records[slot][icon + ds2_rva::FLO_RECORD_DEFINITION_OFFSET..]
                            [..2]
                            .copy_from_slice(&(spec.icon as u16).to_le_bytes());
                        container.row_records[slot][icon + ds2_rva::FLO_RECORD_ID_OFFSET..][..4]
                            .copy_from_slice(&0u32.to_le_bytes());
                        // THE TINT, AND THE LICENCE TO USE IT. A colour with no flag bits is what
                        // one run wrote, and it changed nothing at all -- see
                        // `FLO_TRANSFORM_FLAGS_OFFSET`. `Tint::flags` returns zero for a mix that
                        // came out white, which needs no licence because it is not a tint.
                        if let Some(tint) = spec.tint {
                            let flags_at = ds2_rva::FLO_TRANSFORM_FLAGS_OFFSET;
                            let flags = u32::from_le_bytes(
                                container.icon_transform[slot][flags_at..][..4]
                                    .try_into()
                                    .expect("four bytes"),
                            ) | tint.flags();
                            container.icon_transform[slot][flags_at..][..4]
                                .copy_from_slice(&flags.to_le_bytes());
                            container.icon_transform[slot][ds2_rva::FLO_TRANSFORM_COLOUR_OFFSET..]
                                [..4]
                                .copy_from_slice(&tint.bytes());
                        }
                        // Our record names our definition instead of row 0's.
                        let at = row_at(slot) * ds2_rva::FLO_RECORD_STRIDE
                            + ds2_rva::FLO_RECORD_DEFINITION_OFFSET;
                        let index = ds2_rva::FLO_ADDED_ROW_DEFINITION + slot as u32;
                        container.records[at..][..2].copy_from_slice(&(index as u16).to_le_bytes());
                        own_definitions += 1;
                    }
                }
            }
        }
    }

    // THE CARET. `0x0221` is shared by all three tabs, so the quit tab gets its own copy of it and
    // our panel record is repointed at that copy -- the same substitution as the container, one
    // level down, and unambiguous because nothing else ever asks for the index it is filed under.
    let mut caret_ok = false;
    if !panel.is_null() {
        // SAFETY: `panel` is a definition the game's own table just yielded.
        let (panel_count, panel_children) = unsafe {
            (
                panel
                    .add(ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET)
                    .cast::<u16>()
                    .read() as usize,
                panel
                    .add(ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET)
                    .cast::<*const u8>()
                    .read(),
            )
        };
        if panel_count == ds2_rva::FLO_PANEL_CHILDREN && (panel_children as usize) >= 0x1_0000 {
            // SAFETY: `panel_count` records are live at `panel_children`, and both destinations are
            // exactly as large.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    panel,
                    container.panel_definition.as_mut_ptr(),
                    ds2_rva::FLO_DEFINITION_STRIDE,
                );
                std::ptr::copy_nonoverlapping(
                    panel_children,
                    container.panel_records.as_mut_ptr(),
                    panel_count * ds2_rva::FLO_RECORD_STRIDE,
                );
            }
            let at = ds2_rva::FLO_PANEL_CARET * ds2_rva::FLO_RECORD_STRIDE;
            // SAFETY: the record was copied from a live one whose transform the game dereferences.
            let source = unsafe {
                container
                    .panel_records
                    .as_ptr()
                    .add(at + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET)
                    .cast::<*const u8>()
                    .read()
            };
            if (source as usize) >= 0x1_0000 {
                // SAFETY: a live transform block of exactly this size.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        source,
                        container.caret_transform.as_mut_ptr(),
                        ds2_rva::FLO_TRANSFORM_SIZE,
                    );
                }
                let y = f32::from_le_bytes(
                    container.caret_transform[ds2_rva::FLO_TRANSFORM_Y_OFFSET..][..4]
                        .try_into()
                        .expect("four bytes"),
                );
                if (y - ds2_rva::FLO_CARET_SHIPPED_Y).abs() < 0.5 {
                    container.caret_transform[ds2_rva::FLO_TRANSFORM_Y_OFFSET..][..4]
                        .copy_from_slice(&ds2_rva::FLO_CARET_Y.to_le_bytes());
                    caret_ok = true;
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} caret REFUSED y={y}, expected {} -- the tab keeps the shipped \
                         caret and the added row has none",
                        ds2_rva::FLO_CARET_SHIPPED_Y
                    ));
                }
            }
        }
    }
    if caret_ok {
        // Our panel record names our panel definition instead of the shared one.
        let at = ds2_rva::FLO_QUIT_TAB_PANEL * ds2_rva::FLO_RECORD_STRIDE;
        container.records[at + ds2_rva::FLO_RECORD_DEFINITION_OFFSET..][..2]
            .copy_from_slice(&(ds2_rva::FLO_ADDED_PANEL_DEFINITION as u16).to_le_bytes());
    }

    // LEAKED ON PURPOSE. The game keeps this definition at the built container's `+0x48` for the
    // container's whole life and re-reads the capacity from it on every attach, so it has to
    // outlive anything this crate can observe. It is ~0x210 bytes, once per document load.
    let container: &'static mut Container = Box::leak(container);

    // The self-pointers, written last because they need the final address.
    let records = container.records.as_ptr() as usize;
    container.definition[ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET..][..8]
        .copy_from_slice(&records.to_le_bytes());
    // DECLARED, not `CHILDREN`. The struct is sized for the ceiling; the definition must claim
    // only the records that were actually filled, or the game walks slots holding zeroes.
    container.definition[ds2_rva::FLO_DEFINITION_CHILD_COUNT_OFFSET..][..2]
        .copy_from_slice(&(declared as u16).to_le_bytes());
    {
        let at = ds2_rva::FLO_QUIT_TAB_PANEL * ds2_rva::FLO_RECORD_STRIDE
            + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET;
        let transform = container.panel_transform.as_ptr() as usize;
        container.records[at..][..8].copy_from_slice(&transform.to_le_bytes());
    }
    for slot in 0..rows.len() {
        for (index, transform) in [
            (
                row_at(slot),
                container.row_transform[slot].as_ptr() as usize,
            ),
            (
                mark_at(slot),
                container.mark_transform[slot].as_ptr() as usize,
            ),
        ] {
            let at = index * ds2_rva::FLO_RECORD_STRIDE + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET;
            container.records[at..][..8].copy_from_slice(&transform.to_le_bytes());
        }
    }

    for slot in 0..own_definitions {
        let records = container.row_records[slot].as_ptr() as usize;
        container.row_definition[slot][ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET..][..8]
            .copy_from_slice(&records.to_le_bytes());
        let index = ds2_rva::FLO_ADDED_ROW_DEFINITION + slot as u32;
        container.row_definition[slot][..2].copy_from_slice(&(index as u16).to_le_bytes());
        let at = ds2_rva::FLO_QUIT_ROW_ICON * ds2_rva::FLO_RECORD_STRIDE
            + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET;
        let transform = container.icon_transform[slot].as_ptr() as usize;
        container.row_records[slot][at..][..8].copy_from_slice(&transform.to_le_bytes());
        ROW_DEFINITIONS[slot].store(
            container.row_definition[slot].as_ptr() as usize,
            Ordering::Release,
        );
        let spec = rows[slot];
        // The tint is printed as r/g/b/a rather than as a packed word, because a packed word is
        // exactly the notation that let it be written backwards.
        let tint = match spec.tint {
            Some(tint) => {
                let [r, g, b, a] = tint.bytes();
                format!(
                    "r{r:02x}/g{g:02x}/b{b:02x}/a{a:02x} strength={}/255 flags+={:#x}",
                    tint.strength,
                    tint.flags()
                )
            }
            None => "none".to_string(),
        };
        log(format_args!(
            "{LOG_PREFIX} row definition slot={slot} index={index:#x} from={:#x} \
             icon={:#x} tint={tint} highlight={:#x} kept",
            ds2_rva::FLO_QUIT_ROW_DEFINITION,
            spec.icon,
            ds2_rva::FLO_QUIT_ROW_HIGHLIGHT_DEFINITION
        ));
    }

    if caret_ok {
        let records = container.panel_records.as_ptr() as usize;
        container.panel_definition[ds2_rva::FLO_DEFINITION_CHILDREN_OFFSET..][..8]
            .copy_from_slice(&records.to_le_bytes());
        container.panel_definition[..2]
            .copy_from_slice(&(ds2_rva::FLO_ADDED_PANEL_DEFINITION as u16).to_le_bytes());
        let at = ds2_rva::FLO_PANEL_CARET * ds2_rva::FLO_RECORD_STRIDE
            + ds2_rva::FLO_RECORD_TRANSFORM_OFFSET;
        let transform = container.caret_transform.as_ptr() as usize;
        container.panel_records[at..][..8].copy_from_slice(&transform.to_le_bytes());
        PANEL.store(
            container.panel_definition.as_ptr() as usize,
            Ordering::Release,
        );
        log(format_args!(
            "{LOG_PREFIX} caret moved definition={:#x} y={}->{}",
            ds2_rva::FLO_ADDED_PANEL_DEFINITION,
            ds2_rva::FLO_CARET_SHIPPED_Y,
            ds2_rva::FLO_CARET_Y
        ));
    }

    let n = SUBSTITUTED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} container substituted original=0x{:016x} replacement=0x{:016x} \
         children={count}->{declared} rows=[{}] own-definitions={own_definitions}/{} \
         panel-scale-y=x{} substitutions={n}",
        original as usize,
        container as *const Container as usize,
        rows.iter()
            .enumerate()
            .map(|(slot, row)| format!(
                "{slot}:{:#x}@({:?})+label {:#x}@({:?})",
                row.row_id,
                row.row_xy(),
                row.label_id,
                row.mark_xy()
            ))
            .collect::<Vec<_>>()
            .join(" "),
        rows.len(),
        ds2_rva::FLO_PANEL_STRETCH_Y,
    ));
    Some(container as *mut Container as *mut u8)
}

/// Return the substitution for `original`, building it the first time it is seen.
///
/// # Safety
///
/// As [`build`].
unsafe fn substitute(original: *mut u8, panel: *mut u8, row: *mut u8) -> *mut u8 {
    let Ok(mut built) = BUILT.lock() else {
        // A poisoned mutex means a previous call panicked inside the lock. Handing back the
        // original is the one answer that cannot make that worse.
        return original;
    };
    if let Some(slot) = built.iter().position(|(key, _)| *key == original as usize) {
        let replacement = built[slot].1 as *mut Container;
        // SAFETY: `replacement` is a leaked `Container` this module built, and `original` is the
        // definition the game just returned.
        if unsafe { still_current(original, replacement) } {
            return replacement as *mut u8;
        }
        // The document was reloaded onto the same address. Drop the entry -- not the allocation,
        // which something may still be reading -- and fall through to build a fresh one.
        log(format_args!(
            "{LOG_PREFIX} container stale original=0x{:016x} -- the document was reloaded, rebuilding",
            original as usize
        ));
        built.remove(slot);
    }
    if built.len() >= MAX_BUILT {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} container REFUSED reason=too-many-documents built={MAX_BUILT} \
             refusals={n} -- something is reloading the layout and this crate did not expect it"
        ));
        return original;
    }
    // SAFETY: the caller established this is the quit tab's container definition.
    match unsafe { build(original, panel, row) } {
        Some(replacement) => {
            built.push((original as usize, replacement as usize));
            replacement
        }
        None => original,
    }
}

unsafe extern "system" fn detour(doc: *mut usize, index: u32) -> *mut u8 {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // Published before the site is patched, so this cannot happen; a null definition is what
        // the original returns for a miss, so it is the honest answer if it ever does.
        return std::ptr::null_mut();
    }
    // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
    // one the disassembled entry and exit implement.
    let original: FindDefinitionFn =
        unsafe { std::mem::transmute::<usize, FindDefinitionFn>(trampoline) };
    // SAFETY: both arguments are the game's own, passed through unchanged.
    let found = unsafe { original(doc, index) };
    if index == ds2_rva::FLO_ADDED_PANEL_DEFINITION {
        // Ours, and only ever asked for by our own record.
        return PANEL.load(Ordering::Acquire) as *mut u8;
    }
    if index >= ds2_rva::FLO_ADDED_ROW_DEFINITION
        && index < ds2_rva::FLO_ADDED_ROW_DEFINITION + MAX_ROWS as u32
    {
        // Likewise, one index per slot.
        let slot = (index - ds2_rva::FLO_ADDED_ROW_DEFINITION) as usize;
        return ROW_DEFINITIONS[slot].load(Ordering::Acquire) as *mut u8;
    }
    if index != ds2_rva::FLO_QUIT_TAB_CONTAINER_DEFINITION || found.is_null() {
        return found;
    }
    // The shared panel definition and Quit Game's row definition, both fetched through the
    // original so the copies are the game's own.
    // SAFETY: the trampoline is the game's own lookup, called with its own arguments.
    let panel = unsafe { original(doc, ds2_rva::FLO_PANEL_DEFINITION) };
    // SAFETY: as above.
    let row = unsafe { original(doc, ds2_rva::FLO_QUIT_ROW_DEFINITION) };
    // SAFETY: `found` is a definition the game's own table just yielded.
    unsafe { substitute(found, panel, row) }
}

/// Detour the definition lookup. Returns whether the row's cell will exist.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan`, and before
/// the pause menu's layout document is built -- which the loader's Arxan callback satisfies by a
/// wide margin, since no menu exists until a game is loaded.
pub unsafe fn install(base: usize) -> bool {
    let rva = ds2_rva::FLO_FIND_DEFINITION;
    let site = base + rva as usize;
    // SAFETY: `site` is a `.pdata` function start recorded in `ds2-rva`, resolved against the live
    // base, so its prologue is readable.
    let found = unsafe { std::slice::from_raw_parts(site as *const u8, PROLOGUE.len()) };
    if found != PROLOGUE.as_slice() {
        log(format_args!(
            "{LOG_PREFIX} container NOT installed stage=prologue va=0x{site:016x} \
             expected={PROLOGUE:02x?} found={found:02x?} -- the row will have no cell"
        ));
        return false;
    }
    let hook = match unsafe { MhHook::new(site as *mut c_void, detour as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} container NOT installed stage=MH_CreateHook status={status:?} \
                 -- the row will have no cell"
            ));
            return false;
        }
    };
    // Published BEFORE the site is patched: a detour that observed a zero here would return null
    // for every definition in the game, which is a black menu rather than a missing row.
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} container NOT installed stage=MH_EnableHook status={status:?} \
             -- the row will have no cell"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} container hooked rva=0x{rva:08x} va=0x{site:016x} \
         definition={:#x} children={}->{CHILDREN}",
        ds2_rva::FLO_QUIT_TAB_CONTAINER_DEFINITION,
        ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len()
    ));
    true
}

/// `mov rax,[rcx]` / `mov r9d,edx` / `test rax,rax`, the entry of `FUN_140b54740`.
///
/// Nine bytes, so MinHook's five-byte patch lands inside instructions this has actually seen.
const PROLOGUE: [u8; 9] = [0x48, 0x8b, 0x01, 0x44, 0x8b, 0xca, 0x48, 0x85, 0xc0];

#[cfg(test)]
mod tests {
    use super::*;

    /// The added ids must be ids the file does not already carry, or the new records shadow
    /// existing ones and the path resolves to whichever comes first.
    #[test]
    fn the_added_ids_are_not_shipped_ids() {
        for slot in 0..MAX_ROWS {
            for id in [
                ds2_rva::FLO_ADDED_ROW_IDS[slot],
                ds2_rva::FLO_ADDED_LABEL_IDS[slot],
            ] {
                assert!(!ds2_rva::FLO_QUIT_TAB_CHILD_IDS.contains(&id));
                assert!(!ds2_rva::FE_QUIT_TAB_CELL_IDS.contains(&id));
                assert!(!ds2_rva::FE_QUIT_TAB_BASE_PATH.contains(&id));
            }
            assert_ne!(
                ds2_rva::FLO_ADDED_ROW_IDS[slot],
                ds2_rva::FLO_ADDED_LABEL_IDS[slot]
            );
        }
    }

    /// The templates must be the rows this crate says they are, or the clone inherits the wrong
    /// undecoded fields.
    #[test]
    fn the_templates_are_row_zero_and_its_mark() {
        assert_eq!(
            ds2_rva::FLO_QUIT_TAB_CHILD_IDS[ds2_rva::FLO_QUIT_TAB_ROW_TEMPLATE],
            ds2_rva::FE_QUIT_TAB_CELL_IDS[0]
        );
        assert!(ds2_rva::FLO_QUIT_TAB_MARK_TEMPLATE < ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len());
    }

    /// The icon is the glyph ALONE, never the pair that carries its greyed-out twin -- the
    /// availability pass skips an ungated row, so a cloned twin would draw `ff808080` forever.
    /// See `FLO_QUIT_ICON_DEFINITION`.
    #[test]
    fn the_icon_is_the_glyph_without_the_disabled_twin() {
        assert_ne!(
            ds2_rva::FLO_QUIT_ICON_DEFINITION,
            ds2_rva::FLO_QUIT_ROW_ICON_GROUP
        );
        assert_ne!(
            ds2_rva::FLO_QUIT_ICON_DEFINITION,
            ds2_rva::FLO_QUIT_ROW_DEFINITION
        );
        // A real file index, not one of ours -- the game's own lookup has to find it.
        const { assert!(ds2_rva::FLO_QUIT_ICON_DEFINITION < ds2_rva::FLO_ADDED_PANEL_DEFINITION) };
        // The gate is what makes the twin permanent, so the pairing is asserted from both sides.
        assert_eq!(ds2_rva::FE_INGAME_MENU_GATE_ALWAYS, 0);
    }

    /// The added row keeps BOTH of a row's children. Losing the second one is a highlight that
    /// stops appearing, which no log line reports and only a run shows.
    #[test]
    fn the_added_row_keeps_the_selection_highlight() {
        assert_eq!(ds2_rva::FLO_QUIT_ROW_CHILDREN, 2);
        assert_ne!(ds2_rva::FLO_QUIT_ROW_ICON, ds2_rva::FLO_QUIT_ROW_HIGHLIGHT);
        const { assert!(ds2_rva::FLO_QUIT_ROW_ICON < ds2_rva::FLO_QUIT_ROW_CHILDREN) };
        const { assert!(ds2_rva::FLO_QUIT_ROW_HIGHLIGHT < ds2_rva::FLO_QUIT_ROW_CHILDREN) };
        assert_ne!(
            ds2_rva::FLO_QUIT_ROW_HIGHLIGHT_DEFINITION,
            ds2_rva::FLO_QUIT_ROW_ICON_GROUP
        );
        // Our index must be outside the file's own range, like the panel's, or a real definition
        // is shadowed.
        const { assert!(ds2_rva::FLO_ADDED_ROW_DEFINITION > 0x0272) };
        assert_ne!(
            ds2_rva::FLO_ADDED_ROW_DEFINITION,
            ds2_rva::FLO_ADDED_PANEL_DEFINITION
        );
    }

    /// The tint has to be opaque, non-white, and carry both flag bits -- a colour without them is
    /// what the first run wrote, and it changed nothing at all.
    #[test]
    fn the_tint_is_opaque_not_white_and_licensed() {
        // The tint is R, G, B, A in the order it is written, and the LAST byte is the alpha --
        // `+0x1b`, the byte the builder's inline test reads. Asserting the index rather than
        // trusting a packed literal is the whole point of the array: the first version of this
        // constant was a `u32` whose documented order was backwards, and it shipped as blue.
        assert_eq!(
            ds2_rva::FLO_TINT_ALPHA,
            ds2_rva::FLO_ADDED_ROW_TINT.len() - 1
        );
        assert_eq!(
            ds2_rva::FLO_TRANSFORM_COLOUR_OFFSET + ds2_rva::FLO_TINT_ALPHA,
            0x1b
        );
        assert_eq!(
            ds2_rva::FLO_ADDED_ROW_TINT[ds2_rva::FLO_TINT_ALPHA],
            0xff,
            "a non-opaque tint would also make the record inline-eligible"
        );
        // Red, and red means the FIRST byte is the large one. A tint whose red channel is not the
        // dominant one is the same mistake wearing a different literal.
        let [r, g, b, _] = ds2_rva::FLO_ADDED_ROW_TINT;
        assert!(r > g && r > b, "tint is r{r:02x} g{g:02x} b{b:02x}");
        // Non-white RGB needs the second bit as well as the first; 108 records in the file agree
        // and none disagrees. Strength zero would land exactly here.
        assert_ne!(
            [r, g, b],
            [0xff, 0xff, 0xff],
            "strength {} mixes to white, which needs no colour at all",
            ds2_rva::FLO_ADDED_ROW_TINT_STRENGTH
        );
        // The mix is a mix: white at one end, the hue itself at the other, and this is neither.
        assert_ne!(
            [r, g, b],
            ds2_rva::FLO_ADDED_ROW_HUE,
            "strength {} is full strength, which measured as a re-skin rather than a mark",
            ds2_rva::FLO_ADDED_ROW_TINT_STRENGTH
        );
        assert_eq!(
            ds2_rva::FLO_TRANSFORM_COLOUR_LIVE | ds2_rva::FLO_TRANSFORM_COLOUR_RGB,
            0x110
        );
        // Both bits sit inside the flag word, not past it.
        const { assert!(ds2_rva::FLO_TRANSFORM_FLAGS_OFFSET + 4 <= ds2_rva::FLO_TRANSFORM_SIZE) };
    }

    /// The added row goes BELOW the last shipped one. Above it would overlap a row that exists.
    #[test]
    fn the_added_row_is_below_the_last_shipped_row() {
        // Quit Game, the bottom row, sits at y 103.9 and its mark at 114.35.
        assert!(ds2_rva::FLO_ADDED_ROW_XY.1 > 103.9);
        assert!(ds2_rva::FLO_ADDED_MARK_XY.1 > 114.35);
        // And the mark stays below its own row, the way all three shipped pairs do.
        assert!(ds2_rva::FLO_ADDED_MARK_XY.1 > ds2_rva::FLO_ADDED_ROW_XY.1);
        // One row's pitch below row 2, the same step row 2 sits below row 1.
        let step = ds2_rva::FLO_ADDED_ROW_XY.1 - 103.9;
        assert!((step - 48.0).abs() < 0.01, "row step is {step}");
    }

    /// Every slot adds a row and a mark, in pairs, after the shipped records and never over them.
    #[test]
    fn each_slot_adds_a_row_and_its_mark() {
        assert_eq!(
            CHILDREN,
            ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len() + PER_ROW * MAX_ROWS
        );
        assert_eq!(row_at(0), ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len());
        for slot in 0..MAX_ROWS {
            assert_eq!(mark_at(slot), row_at(slot) + 1);
            assert!(row_at(slot) >= ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len());
            assert!(mark_at(slot) < CHILDREN);
            if slot > 0 {
                assert_eq!(row_at(slot), mark_at(slot - 1) + 1);
            }
        }
    }

    /// The struct is what its pointers assume: the child array immediately follows the definition,
    /// and every piece is the size the game reads.
    #[test]
    fn the_container_is_laid_out_the_way_the_game_reads_it() {
        // The fields, then whatever `align(16)` adds on the end. Asserting the bare sum would fail
        // on the padding rather than on a field, which is the opposite of what this is for.
        // Per container: the container definition, the panel's copy, and one row definition per
        // slot. Per container transforms: the panel's and the caret's. Per slot: a row transform,
        // a mark transform and an icon transform.
        let fields = ds2_rva::FLO_DEFINITION_STRIDE * (2 + MAX_ROWS)
            + ds2_rva::FLO_RECORD_STRIDE * CHILDREN
            + ds2_rva::FLO_TRANSFORM_SIZE * (2 + 3 * MAX_ROWS)
            + ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_QUIT_TAB_CHILD_IDS.len()
            + ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_PANEL_CHILDREN
            + ds2_rva::FLO_RECORD_STRIDE * ds2_rva::FLO_QUIT_ROW_CHILDREN * MAX_ROWS;
        assert_eq!(
            std::mem::size_of::<Container>(),
            fields.next_multiple_of(16)
        );
        assert_eq!(std::mem::align_of::<Container>(), 16);
    }

    /// The synthetic panel index must be outside the file's own range, or a real definition would
    /// be shadowed and some other screen would get our caret.
    #[test]
    fn the_added_panel_index_is_not_a_file_index() {
        const { assert!(ds2_rva::FLO_ADDED_PANEL_DEFINITION > 0x0272) };
        assert_ne!(
            ds2_rva::FLO_ADDED_PANEL_DEFINITION,
            ds2_rva::FLO_PANEL_DEFINITION
        );
        assert_ne!(
            ds2_rva::FLO_ADDED_PANEL_DEFINITION,
            ds2_rva::FLO_QUIT_TAB_CONTAINER_DEFINITION
        );
    }

    /// The caret goes DOWN, by the same pitch the added row is spaced at.
    #[test]
    fn the_caret_moves_one_row_down() {
        let moved = ds2_rva::FLO_CARET_Y - ds2_rva::FLO_CARET_SHIPPED_Y;
        assert!((moved - 48.0).abs() < 0.5, "moved by {moved}");
    }

    /// The depths must not collide with a shipped one, since the whole point of choosing them was
    /// to continue the series into free values.
    #[test]
    fn the_added_depths_are_free() {
        const SHIPPED: [u16; 7] = [1, 4, 10, 14, 18, 20, 22];
        for slot in 0..MAX_ROWS {
            assert!(!SHIPPED.contains(&row_depth(slot)));
            assert!(!SHIPPED.contains(&mark_depth(slot)));
            assert_ne!(row_depth(slot), mark_depth(slot));
        }
    }
}
