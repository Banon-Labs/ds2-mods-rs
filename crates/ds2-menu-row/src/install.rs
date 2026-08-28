//! Installing the one detour, and the append itself.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything else
/// rather than opening one of its own. Stored as a `usize` because a `fn` pointer is not an
/// `Atomic` type; only ever set from [`set_logger`].
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// Trampoline back to the original builder, published before the site is patched so a detour that
/// fires immediately cannot read a zero.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// How many times the detour has fired, and how many of those appended.
///
/// Both, not one. "No fourth row appeared" has two very different causes -- the append was refused,
/// or the tab was never built because the pause menu was never opened -- and a single counter
/// cannot tell them apart. The whole point of this crate is that its negative result is readable.
static FIRED: AtomicUsize = AtomicUsize::new(0);
static APPENDED: AtomicUsize = AtomicUsize::new(0);

/// The builder: `descriptor* build(descriptor*)`, argument in RCX, returned in RAX.
///
/// Read off the disassembly rather than assumed: the entry is `rex push rbx` / `sub rsp,0x50` /
/// `mov rbx,rcx`, it touches no other argument register, and it ends `mov rax,rbx` / `ret`.
type BuildItemsFn = unsafe extern "system" fn(*mut u8) -> *mut u8;

/// The gate every appended entry carries.
///
/// Gate `0` deliberately, for every registered row: a gated row is greyed by the availability pass
/// and a run in which a row is greyed cannot be distinguished from a run in which it never
/// appeared. It is also what keeps a cloned disabled-state overlay off the icon -- see
/// [`ds2_rva::FLO_QUIT_ROW_ICON_GROUP`].
///
/// The actions are OURS, not borrowed from the game -- see
/// [`ds2_rva::FE_INGAME_MENU_ACTION_BASE`]. The shipped dispatch has no case for any of them, so
/// with the dispatch detour absent a registered row plays the ordinary confirm sound and does
/// nothing. An inert row is the right failure mode; a row that quietly opened Key Bindings would
/// not be.
const GATE: u32 = ds2_rva::FE_INGAME_MENU_GATE_ALWAYS;

/// Address of entry `index` inside a tab's item vector.
///
/// The odd-looking padding term is the game's own, transcribed rather than reasoned about: every
/// builder addresses its elements as `(-(int)descriptor & 3) + descriptor + n * 8`. On an aligned
/// descriptor it is zero, but "it is probably zero" is not a reason to compute a different address
/// from the code that will read it back.
///
/// # Safety
///
/// `descriptor` must point at a tab item vector, and `index` must be below
/// [`ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY`].
unsafe fn entry_at(descriptor: *mut u8, index: usize) -> *mut u8 {
    let padding = (0u32.wrapping_sub(descriptor as usize as u32) & 3) as usize;
    // SAFETY: the caller guarantees the descriptor and a within-capacity index, and the offset is
    // the one the game itself computes for the same element.
    unsafe { descriptor.add(padding + index * ds2_rva::FE_INGAME_MENU_ITEM_STRIDE) }
}

/// Read `count` entries as `(action, gate)` pairs.
///
/// # Safety
///
/// `descriptor` must point at a tab item vector holding at least `count` entries.
unsafe fn read_entries(descriptor: *mut u8, count: usize) -> Vec<(u32, u32)> {
    (0..count)
        .map(|index| {
            // SAFETY: `index < count`, and the caller guarantees that many entries are live.
            let entry = unsafe { entry_at(descriptor, index) };
            // SAFETY: an entry is two `u32`s, which is how both of the game's own readers split it.
            unsafe {
                (
                    entry.cast::<u32>().read(),
                    entry.add(4).cast::<u32>().read(),
                )
            }
        })
        .collect()
}

/// `(7,0) (8,0) (9,4)`, for a log line.
fn describe(entries: &[(u32, u32)]) -> String {
    let mut out = String::new();
    for (action, gate) in entries {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("({action:#x},{gate})"));
    }
    out
}

/// Run the original builder, then append one entry to what it produced.
///
/// # Safety
///
/// `descriptor` is the stack descriptor `FeGroupInGameTopSelect`'s constructor passed in; the
/// original has to run against it first, because everything below reads what the original wrote.
unsafe fn append(descriptor: *mut u8) -> *mut u8 {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    let returned = if trampoline == 0 {
        // Cannot happen -- the trampoline is published before the site is patched -- but returning
        // the argument keeps the ABI honest if it ever does, instead of returning uninitialised RAX.
        descriptor
    } else {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry and exit implement.
        let original: BuildItemsFn =
            unsafe { std::mem::transmute::<usize, BuildItemsFn>(trampoline) };
        unsafe { original(descriptor) }
    };

    let fired = FIRED.fetch_add(1, Ordering::Relaxed) + 1;
    if descriptor.is_null() {
        log(format_args!(
            "{LOG_PREFIX} REFUSED reason=null-descriptor fire={fired}"
        ));
        return returned;
    }

    // SAFETY: the original has just run against this pointer and wrote this very field, so the
    // descriptor is live and at least as large as the count the game itself writes at this offset.
    let count = unsafe {
        descriptor
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .read()
    } as usize;

    // WHAT THE ORIGINAL LEFT BEHIND, checked before anything is written. This is the integrity
    // check the whole experiment rests on: appending to the wrong tab produces a screenshot that
    // looks exactly like a result and is about nothing.
    if count > ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY {
        log(format_args!(
            "{LOG_PREFIX} REFUSED reason=count-over-capacity count={count} \
             capacity={} fire={fired}",
            ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY
        ));
        return returned;
    }
    // SAFETY: `count` is at or below capacity, so that many entries are within the vector.
    let entries = unsafe { read_entries(descriptor, count) };
    if entries != ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS {
        log(format_args!(
            "{LOG_PREFIX} REFUSED reason=unexpected-entries count={count} saw=[{}] \
             expected=[{}] fire={fired}",
            describe(&entries),
            describe(&ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS)
        ));
        return returned;
    }
    // ONE ENTRY PER REGISTERED ROW. `api::add_row` has already refused anything over the ceiling,
    // so this loop should never hit the bound -- but it checks anyway, because the registry is
    // this crate's arithmetic and the vector is the game's, and the game's is the one that panics.
    let rows = crate::api::rows_for(crate::api::Tab::Quit);
    let mut written = 0usize;
    for row in &rows {
        let at = count + written;
        if at >= ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY {
            log(format_args!(
                "{LOG_PREFIX} REFUSED reason=vector-full count={at} action={:#x} \
                 fire={fired} -- the registry allowed a row the vector cannot hold",
                row.action
            ));
            break;
        }
        // SAFETY: `at < capacity`, so this slot is inside the vector, and it is the same address
        // the builder's own next push would have written.
        unsafe {
            let slot = entry_at(descriptor, at);
            slot.cast::<u32>().write(row.action);
            slot.add(4).cast::<u32>().write(GATE);
        }
        written += 1;
    }
    if written == 0 {
        return returned;
    }
    // SAFETY: `written` slots were just filled, contiguously, from `count`.
    unsafe {
        descriptor
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .write((count + written) as u64);
    }

    let appended = APPENDED.fetch_add(1, Ordering::Relaxed) + 1;
    // Logged only for the first couple of opens. The pause menu is opened over and over in a
    // session and this sink calls `sync_all` per line, so a line that repeats forever is a stall
    // that repeats forever. The first two are the evidence; the rest are noise with a cost.
    if appended <= 2 {
        let added = rows
            .iter()
            .take(written)
            .map(|row| format!("({:#x},{GATE})", row.action))
            .collect::<Vec<_>>()
            .join(" ");
        log(format_args!(
            "{LOG_PREFIX} appended [{added}] was=[{}] count={}->{} fire={fired} \
             appends={appended}",
            describe(&entries),
            count,
            count + written
        ));
    }
    returned
}

unsafe extern "system" fn detour(descriptor: *mut u8) -> *mut u8 {
    unsafe { append(descriptor) }
}

/// Trampoline back to the item dispatch, and how many times we have asked the game to quit.
static DISPATCH_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static QUITS_REQUESTED: AtomicUsize = AtomicUsize::new(0);

/// The dispatch: `void dispatch(topSelect, action)`, `this` in RCX and the action in EDX.
///
/// Read off the disassembly: the entry is `48 89 5c 24 18` / `57` / `sub rsp,...`, and the body
/// switches on the 32-bit second argument.
type DispatchFn = unsafe extern "system" fn(*mut u8, u32);

/// Ask the game to shut down, by the only mechanism it has.
///
/// This is `FeSubStateTitleShutdown::v1` transcribed -- one pointer load and one byte -- and that
/// substate is what the TITLE screen's own exit row enters. The byte is polled every frame by
/// `GameManagerImp`'s master update, so the shutdown that follows is the game's own, on the game's
/// own schedule, and this function is not on the stack when it happens.
///
/// **It does not save, and it does not ask.** The quit-to-title flow offers to save because that
/// flow asks; this one is "without a confirmation" and the absence of a save is the same coin.
///
/// # Safety
///
/// Reads the singleton pointer and writes one byte inside it. Must run on the game thread with the
/// title/game systems constructed, which the menu dispatch guarantees -- there is no pause menu
/// before there is a game.
unsafe fn request_shutdown(base: usize) -> bool {
    // SAFETY: the RVA is a data global recorded in `ds2-rva`, resolved against the live base.
    let singleton = (base + ds2_rva::FE_SYSTEM_SINGLETON as usize) as *const usize;
    // SAFETY: the RVA is a data global recorded in `ds2-rva`, resolved against the live base.
    let system = unsafe { singleton.read() };
    if system == 0 {
        log(format_args!(
            "{LOG_PREFIX} quit REFUSED reason=singleton-null -- nothing was written"
        ));
        return false;
    }
    // SAFETY: non-null, and the game's own shutdown substate writes this exact byte at this exact
    // offset inside this exact object.
    unsafe {
        (system as *mut u8)
            .add(ds2_rva::FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET)
            .write(1);
    }
    let n = QUITS_REQUESTED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} quit-to-desktop requested system=0x{system:016x} \
         offset={:#x} value=1 requests={n} -- the game exits on its next frame",
        ds2_rva::FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET
    ));
    true
}

/// Quit to desktop, without saving and without asking.
///
/// The [`crate::RowSpec::on_confirm`] this crate ships, and the first consumer of its own API --
/// `ds2-loader` registers a row pointing here rather than this crate hardcoding one. Safe to call
/// from anywhere on the game thread: it writes one byte the master update polls, and does nothing
/// but log if the singleton is not up yet.
pub fn quit_to_desktop() {
    let base = ds2_game_base::mem::game_module_base().unwrap_or(0);
    if base == 0 {
        log(format_args!(
            "{LOG_PREFIX} quit REFUSED reason=no-module-base -- nothing was written"
        ));
        return;
    }
    // SAFETY: the base is the live game module and the pause menu cannot be open before the
    // systems this reads are constructed.
    unsafe { request_shutdown(base) };
}

unsafe extern "system" fn dispatch_detour(top_select: *mut u8, action: u32) {
    if let Some(row) = crate::api::row_for_action(action) {
        // Deliberately NOT calling the original for a registered action. The original would play a
        // sound and fall through its `default`, which is harmless, but the id is outside the range
        // its `switch` handles, so there is nothing there to run. Fewer moving parts.
        //
        // The callback belongs to whichever crate registered the row. It runs on the game thread,
        // inside the menu's own confirm path, which is the same place the shipped rows' handlers
        // run -- so it may do what they do.
        (row.on_confirm)();
        return;
    }
    let trampoline = DISPATCH_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry implements.
        let original: DispatchFn = unsafe { std::mem::transmute::<usize, DispatchFn>(trampoline) };
        unsafe { original(top_select, action) };
    }
}

/// Trampoline back to the per-tab init, and how many times the probe has reported.
static TAB_INIT_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static TAB_INIT_REPORTS: AtomicUsize = AtomicUsize::new(0);

/// The per-tab init: `void init(tab)`, `this` in RCX. Prologue `40 53 48 81 ec b0 00 00 00`.
type TabInitFn = unsafe extern "system" fn(*mut u8);

/// Read the three numbers that decide whether a row is drawn, AFTER the game has set them.
///
/// They come from three different places and this is the only way to see them disagree:
///
/// * the item count is what `FUN_140021b30` was handed -- the item vector's length, which the
///   append above moves;
/// * the column extent is a census of the cell elements the layout bind FOUND, which nothing in
///   code moves;
/// * the scroll object's visible count is a third number again, and `FUN_140021b30` compares the
///   total against it to decide whether to show a scrollbar.
///
/// If visible is 3 while the item count is 4, this grid is a VIRTUALISED list and the missing row
/// is a scroll that did not happen. If the extent is 3 and there is no scroll concept, the row has
/// no cell and only the layout can supply one. Those two futures cost very different amounts and
/// nothing short of this tells them apart.
///
/// # Safety
///
/// `tab` must be a constructed `FeGroupInGameGroupSelect` whose init has already run.
unsafe fn report_tab(tab: *mut u8) {
    if tab.is_null() {
        return;
    }
    // SAFETY: the original init has just run against this pointer and wrote every field below.
    let (items, cols, rows, scroll) = unsafe {
        (
            tab.add(ds2_rva::FEX_GRID_ITEM_COUNT_OFFSET)
                .cast::<u32>()
                .read(),
            tab.add(ds2_rva::FEX_GRID_COL_EXTENT_OFFSET)
                .cast::<u32>()
                .read(),
            tab.add(ds2_rva::FEX_GRID_ROW_EXTENT_OFFSET)
                .cast::<u32>()
                .read(),
            tab.add(ds2_rva::FEX_GRID_SCROLL_OFFSET)
                .cast::<usize>()
                .read(),
        )
    };
    // The scroll object is a pointer the grid may legitimately not have; a null one is reported as
    // such rather than dereferenced, because "this grid does not scroll" is itself the answer to
    // half the question.
    let (visible, total) = if scroll == 0 {
        (None, None)
    } else {
        // SAFETY: non-null, and `FUN_140021b30` reads and writes these two fields on every call.
        unsafe {
            (
                Some(
                    (scroll as *const u8)
                        .add(ds2_rva::FEX_GRID_SCROLL_VISIBLE_OFFSET)
                        .cast::<u32>()
                        .read(),
                ),
                Some(
                    (scroll as *const u8)
                        .add(ds2_rva::FEX_GRID_SCROLL_TOTAL_OFFSET)
                        .cast::<u32>()
                        .read(),
                ),
            )
        }
    };
    // THE NAMER'S IDENTITY. `FUN_140022160` asks the object at `grid+0xf0` for a cell's element,
    // and which CLASS that is decides how the id for an absent cell is generated. A vtable address
    // walks back to an RTTI type descriptor offline with `scripts/ds2-rtti.py`, so one logged
    // pointer turns an open question into a class name.
    // SAFETY: the original init has run, so the grid's namer is live.
    let namer = unsafe { tab.add(0xf0).cast::<usize>().read() };
    // SAFETY: a namer is polymorphic, so its first qword is its vtable.
    let namer_vtable = if namer == 0 {
        0
    } else {
        unsafe { (namer as *const usize).read() }
    };
    let n = TAB_INIT_REPORTS.fetch_add(1, Ordering::Relaxed) + 1;
    // THE AXIS MATTERS AND THIS GOT IT WRONG ONCE. These tabs measure one COLUMN by N ROWS -- the
    // five tabs this crate does not touch all report `col-extent=1` and `row-extent == items` --
    // so the number of authored cells is the ROW extent. Comparing against the column extent said
    // "NO CELL" for every tab including the four that are perfectly fine.
    let cells = rows;
    let verdict = if items <= cells {
        "ok: every item has a cell"
    } else if visible.is_some_and(|v| v < items) {
        "VIRTUALISED: visible<items, the row needs a SCROLL"
    } else {
        "NO CELL: items>cells and there is no scroll to cover it"
    };
    log(format_args!(
        "{LOG_PREFIX} tab tab=0x{:016x} items={items} col-extent={cols} row-extent={rows} \
         cells={cells} scroll=0x{scroll:x} visible={visible:?} total={total:?} \
         namer=0x{namer:x} namer-vtable=0x{namer_vtable:x} report={n} -- {verdict}",
        tab as usize
    ));
}

/// `FexGridControl::indexToCell(grid, out_cell, index)` and `cellToElement(grid, out, cell)`.
///
/// Called rather than reimplemented, and that is the point: the id an appended cell WOULD have is
/// whatever the game's own namer produces for it, and asking the namer is the only way to learn it
/// that cannot be wrong. Reimplementing the naming would be inventing the answer we came for.
type IndexToCellFn = unsafe extern "system" fn(*mut u8, *mut u8, i32) -> *mut u8;
type CellToElementFn = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8) -> *mut u8;

/// Hex, because the accessor's shape is not known and a decoded field would be a guess.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 8 == 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Ask the grid for the element accessor of each cell, INCLUDING one past the last.
///
/// The one past the last is the whole reason this exists. Cells 0..n-1 are the records that already
/// live in the `.flo`; cell n is the one that does not, and its accessor still carries the id the
/// namer would have looked for. That id is what a new record has to be keyed by, and it cannot be
/// derived from the file -- only from the namer.
///
/// # Safety
///
/// `tab` must be a bound grid, and `base` the live module base. Both callees are pure accessor
/// builders that write only into the caller's buffers.
unsafe fn report_cell_ids(base: usize, tab: *mut u8, cells: u32) {
    // SAFETY: both RVAs are `.pdata` function starts recorded in `ds2-rva`.
    let index_to_cell: IndexToCellFn =
        unsafe { std::mem::transmute(base + ds2_rva::FEX_GRID_INDEX_TO_CELL as usize) };
    // SAFETY: same.
    let cell_to_element: CellToElementFn =
        unsafe { std::mem::transmute(base + ds2_rva::FEX_GRID_CELL_TO_ELEMENT as usize) };
    // 160 bytes because the game's own callers of these give them 144-byte stack buffers; the
    // extra 16 is slack, not a guess about the type.
    for index in 0..(cells + 1).min(8) {
        let mut coords = [0u8; 160];
        let mut accessor = [0u8; 160];
        // SAFETY: the buffers are at least as large as the ones the game gives these functions.
        unsafe {
            index_to_cell(tab, coords.as_mut_ptr(), index as i32);
            cell_to_element(tab, accessor.as_mut_ptr(), coords.as_mut_ptr());
        }
        let past = if index >= cells {
            " ONE-PAST-THE-END"
        } else {
            ""
        };
        log(format_args!(
            "{LOG_PREFIX} cell tab=0x{:016x} index={index} coords={} accessor={}{past}",
            tab as usize,
            hex(&coords[..8]),
            hex(&accessor[..48])
        ));
    }
}

/// The push a cell namer's list takes: `fn(&namer[0x18], src_entry)`.
type NamerPushFn = unsafe extern "system" fn(*mut u8, *mut u8);

/// The namer constructor: `fn(out_ref, allocator) -> out_ref`, leaving the namer in `*out_ref`.
///
/// **IT RETURNS ITS FIRST ARGUMENT, AND THE DECOMPILER SAYS IT DOES NOT.** Ghidra types it `void`;
/// the disassembly ends `mov rax, r14` with `r14` holding the `rcx` saved in the prologue, and the
/// TopSelect constructor passes that return straight into `FeGroupInGameGroupSelect`'s constructor,
/// whose first instruction dereferences it.
///
/// A detour declared `-> ()` therefore hands the game whatever Rust left in RAX. Measured, that was
/// `1`, and the game died reading address `1` inside `0x1400a40f5` -- a crash with our DLL nowhere
/// in the stack, on the very next call. Read the RET, not the decompiler's signature.
type NamerCtorFn = unsafe extern "system" fn(*mut usize, *mut u8) -> *mut usize;

static NAMER_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static CELLS_ADDED: AtomicUsize = AtomicUsize::new(0);

/// Name the fourth cell, so the grid asks the layout for the record `layout.rs` just added.
///
/// The namer holds the cell path of every row it will draw: four shared components and then one id
/// per row. The grid walks that list, resolves each path against the scene, and its extent is a
/// census of the ones that came back non-null -- which is why the appended item had no row.
///
/// This appends a fourth entry naming [`ds2_rva::FLO_ADDED_ROW_ID`]. **It is one half of a pair.**
/// On its own it names an element that does not exist and the extent stays at three -- which is
/// exactly what an earlier run measured, and which is now the control for this one: if the extent
/// reads four, the container substitution worked, because nothing else changed.
///
/// The clone is a copy of the last shipped entry with one field rewritten, not an entry assembled
/// from scratch. The slack between the ids and the length is uninitialised stack and differs
/// between two entries the game built back to back, so only the named fields mean anything.
///
/// # Safety
///
/// `namer` is the object the original constructor just returned, and `base` the live module base.
/// The push takes the source entry by reference the same way the original loop does.
unsafe fn name_added_cell(base: usize, namer: usize) -> bool {
    if namer == 0 {
        return false;
    }
    let refuse = |why: core::fmt::Arguments<'_>| {
        log(format_args!(
            "{LOG_PREFIX} cell REFUSED {why} -- the tab keeps its three rows"
        ));
        false
    };
    let list = namer + ds2_rva::FE_SCENE_NAMER_LIST_OFFSET;
    let count_at = (list + ds2_rva::FE_SCENE_NAMER_COUNT_OFFSET) as *mut u64;
    // SAFETY: the original constructor has just filled this vector.
    let count = unsafe { count_at.read() } as usize;
    if count != ds2_rva::FE_QUIT_TAB_CELL_IDS.len() {
        return refuse(format_args!(
            "count={count}, expected {}",
            ds2_rva::FE_QUIT_TAB_CELL_IDS.len()
        ));
    }
    let rows = crate::api::rows_for(crate::api::Tab::Quit);
    if rows.is_empty() {
        return false;
    }
    if count + rows.len() > ds2_rva::FE_SCENE_NAMER_LIST_CAPACITY {
        return refuse(format_args!(
            "list holds {count} and {} rows want {} of {}",
            rows.len(),
            count + rows.len(),
            ds2_rva::FE_SCENE_NAMER_LIST_CAPACITY
        ));
    }
    let stride = ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE;
    let padding = (0u32.wrapping_sub(list as u32) & 7) as usize;
    let entry = |i: usize| (list + padding + i * stride) as *mut u8;
    let dword = |e: *mut u8, off: usize| {
        // SAFETY: `off` is inside an entry the caller has established is live.
        unsafe { e.add(off).cast::<u32>().read() }
    };

    // VERIFY THE ENTRY THE CLONE IS TAKEN FROM, field by field, against what the namer's own
    // constructor put there. Appending to some other tab's namer would produce a run that looks
    // exactly like a result and is about nothing.
    let last = entry(count - 1);
    for (i, want) in ds2_rva::FE_QUIT_TAB_BASE_PATH.iter().enumerate() {
        let got = dword(last, i * 4);
        if got != *want {
            return refuse(format_args!(
                "entry[{}] component {i} is {got:#x}, expected {want:#x}",
                count - 1
            ));
        }
    }
    let want_id = ds2_rva::FE_QUIT_TAB_CELL_IDS[count - 1];
    let got_id = dword(last, ds2_rva::FE_SCENE_NAMER_ENTRY_ID_OFFSET);
    if got_id != want_id {
        return refuse(format_args!(
            "entry[{}] id is {got_id:#x}, expected {want_id:#x}",
            count - 1
        ));
    }
    let got_len = dword(last, ds2_rva::FE_SCENE_NAMER_ENTRY_LEN_OFFSET);
    if got_len != ds2_rva::FE_SCENE_NAMER_ENTRY_LEN {
        return refuse(format_args!(
            "entry[{}] length is {got_len}, expected {}",
            count - 1,
            ds2_rva::FE_SCENE_NAMER_ENTRY_LEN
        ));
    }

    // SAFETY: the RVA is a `.pdata` function start recorded in `ds2-rva`, and the source is a live
    // element of the vector it is pushed back into.
    let push = unsafe {
        std::mem::transmute::<usize, NamerPushFn>(base + ds2_rva::FE_SCENE_NAMER_PUSH as usize)
    };
    // ONE PUSHED ENTRY PER REGISTERED ROW, each a clone of the last SHIPPED entry with its id
    // rewritten. Cloned rather than assembled: the slack between the named fields is uninitialised
    // stack and differs between two entries the game built back to back, so only the named fields
    // mean anything and inventing the rest would be inventing the answer.
    let mut named = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let at = count + index;
        // SAFETY: `at < capacity`, checked above, which is the same bound the push enforces itself.
        unsafe { push(list as *mut u8, last) };
        // SAFETY: the push just made element `at` live, and the offset is inside it.
        unsafe {
            entry(at)
                .add(ds2_rva::FE_SCENE_NAMER_ENTRY_ID_OFFSET)
                .cast::<u32>()
                .write(row.row_id);
        }
        named.push(format!("{}={:#x}", at, row.row_id));
    }
    let n = CELLS_ADDED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} cells named [{}] container={:#x} count={count}->{} additions={n} \
         -- row-extent {} on the tab line below means the layout answered",
        named.join(" "),
        ds2_rva::FE_QUIT_TAB_BASE_PATH[3],
        count + rows.len(),
        count + rows.len()
    ));
    true
}

unsafe extern "system" fn namer_detour(out: *mut usize, allocator: *mut u8) -> *mut usize {
    let trampoline = NAMER_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        // The caller dereferences what comes back, so returning the out pointer is the only safe
        // answer even on a path that cannot happen.
        return out;
    }
    // SAFETY: MinHook published this trampoline for exactly this site.
    let original: NamerCtorFn = unsafe { std::mem::transmute::<usize, NamerCtorFn>(trampoline) };
    // RETURNED, not discarded. See the note on `NamerCtorFn`.
    let returned = unsafe { original(out, allocator) };
    if out.is_null() {
        return returned;
    }
    // SAFETY: the original writes the constructed namer here.
    let namer = unsafe { out.read() };
    let base = ds2_game_base::mem::game_module_base().unwrap_or(0);
    if base != 0 {
        unsafe { name_added_cell(base, namer) };
    }
    returned
}

/// The three scene-path calls the quit tab's namer makes, in the order it makes them.
unsafe extern "system" fn tab_init_detour(tab: *mut u8) {
    let trampoline = TAB_INIT_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry implements.
        let original: TabInitFn = unsafe { std::mem::transmute::<usize, TabInitFn>(trampoline) };
        unsafe { original(tab) };
    }
    unsafe { report_tab(tab) };
    let base = ds2_game_base::mem::game_module_base().unwrap_or(0);
    if base != 0 && !tab.is_null() {
        // SAFETY: the original init has run, so the grid is bound and its namer is live.
        let cells = unsafe {
            tab.add(ds2_rva::FEX_GRID_ROW_EXTENT_OFFSET)
                .cast::<u32>()
                .read()
        };
        unsafe { report_cell_ids(base, tab, cells) };
    }
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The builder is now detoured.
    pub installed: bool,
}

/// Detour the quit tab's item builder. Call from the post-Arxan callback, never `DllMain`.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`), which in practice means the loader's Arxan callback. It does not have
/// to be early: the site is only reached when `FeGroupInGameTopSelect` is constructed, which is
/// long after the entry point.
pub unsafe fn install() -> Outcome {
    // SEALED FIRST, so a row registered from another thread while the hooks are going in cannot be
    // half-applied -- named by the namer but absent from the item vector, or the reverse.
    crate::api::seal();
    if !crate::api::any() {
        log(format_args!(
            "{LOG_PREFIX} nothing registered -- no rows, no hooks, the shipped menu is untouched"
        ));
        return Outcome { installed: false };
    }
    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome { installed: false };
        }
    };

    let rva = ds2_rva::FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS;
    let site = base + rva as usize;

    // THE BYTES BEFORE THE PATCH, because an RVA is just a number. On a build this table was not
    // read from, `site` points into the middle of something else and MinHook would happily detour
    // it. Refusing here costs one comparison and is the difference between "the mod did nothing"
    // and "the mod corrupted an unrelated function".
    let expected = ds2_rva::FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS_PROLOGUE;
    // SAFETY: `site` is inside the loaded image's `.text` -- it is a `.pdata` function start
    // recorded in `ds2-rva`, resolved against the live base -- so `expected.len()` bytes are
    // readable there.
    let found = unsafe { std::slice::from_raw_parts(site as *const u8, expected.len()) };
    if found != expected.as_slice() {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=prologue va=0x{site:016x} expected={expected:02x?} \
             found={found:02x?}"
        ));
        return Outcome { installed: false };
    }

    // MinHook is statically linked into this DLL, so nothing else shares this instance and
    // ALREADY_INITIALIZED can only mean this ran twice. Treat it as success.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome { installed: false };
    }

    let hook = match unsafe { MhHook::new(site as *mut c_void, detour as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=MH_CreateHook va=0x{site:016x} status={status:?}"
            ));
            return Outcome { installed: false };
        }
    };
    // Published BEFORE the site is patched, so a detour cannot observe a zero and skip the
    // original -- which here would mean handing the game a tab with no items at all.
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_EnableHook va=0x{site:016x} status={status:?}"
        ));
        return Outcome { installed: false };
    }
    // The handle falls out of scope here. `MhHook` has no `Drop`, so that does NOT remove the hook
    // -- the patch stays for the life of the process, which is what is wanted.

    log(format_args!(
        "{LOG_PREFIX} hooked rva=0x{rva:08x} va=0x{site:016x} rows={} gate={GATE} \
         open the pause menu's last tab to read the result",
        crate::api::rows_for(crate::api::Tab::Quit).len()
    ));

    // THE DISPATCH DETOUR, which is what makes the appended row DO something. Installed before the
    // probe because it is the one that matters: without it the row is inert.
    let dispatch_rva = ds2_rva::FE_INGAME_MENU_DISPATCH;
    let dispatch_site = base + dispatch_rva as usize;
    match unsafe { MhHook::new(dispatch_site as *mut c_void, dispatch_detour as *mut c_void) } {
        Ok(hook) => {
            DISPATCH_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(dispatch_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} dispatch hooked rva=0x{dispatch_rva:08x} \
                     va=0x{dispatch_site:016x} actions={:#x}..{:#x}",
                    ds2_rva::FE_INGAME_MENU_ACTION_BASE,
                    ds2_rva::FE_INGAME_MENU_ACTION_BASE
                        + crate::api::rows_for(crate::api::Tab::Quit).len() as u32
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} dispatch NOT installed stage=MH_EnableHook status={status:?} \
                     -- the appended row will be INERT"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} dispatch NOT installed stage=MH_CreateHook status={status:?} \
             -- the appended row will be INERT"
        )),
    }

    // THE ROW'S CELL, in two halves that only work together. The container substitution adds the
    // layout record; the namer entry is what makes the grid ask for it. Installed in that order so
    // that if the first refuses, the log says so before the second claims a cell that is not there.
    let cell = unsafe { crate::layout::install(base) };
    let _captions = unsafe { crate::caption::install(base) };

    let namer_rva = ds2_rva::FE_INGAME_MENU_QUIT_TAB_NAMER;
    let namer_site = base + namer_rva as usize;
    match unsafe { MhHook::new(namer_site as *mut c_void, namer_detour as *mut c_void) } {
        Ok(hook) => {
            NAMER_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(namer_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} namer hooked rva=0x{namer_rva:08x} va=0x{namer_site:016x} \
                     cells=[{}] container-substitution={cell}",
                    crate::api::rows_for(crate::api::Tab::Quit)
                        .iter()
                        .map(|row| format!("{:#x}", row.row_id))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} namer NOT installed stage=MH_EnableHook status={status:?} \
                     -- the appended row will have no cell and stay invisible"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} namer NOT installed stage=MH_CreateHook status={status:?} \
             -- the appended row will have no cell and stay invisible"
        )),
    }

    // THE PROBE IS NOT INSTALLED, AND LEAVING IT INSTALLED IS WHAT FROZE THE PAUSE MENU.
    //
    // It reports six tabs and every cell of each: twenty-six lines per open, each one followed by a
    // `sync_all` in the loader's log sink, plus twenty-odd calls into the grid's own cell accessors
    // just to produce them. That ran on every single pause-menu open, long after the question it
    // was written to answer -- `row-extent=4` -- had been settled.
    //
    // The tree dump in `tree.rs` was disarmed for exactly this reason and the freeze SURVIVED,
    // because this one was still going. Two instruments, one mistake, and the second run was wasted
    // by fixing one without looking for the other. An instrument left in the shipping path is a
    // feature nobody asked for.
    //
    // Re-arm by restoring the `MhHook::new` on `ds2_rva::FE_INGAME_MENU_TAB_INIT` here; the detour
    // and its reporters are untouched.
    let _ = (
        report_tab as *const (),
        tab_init_detour as *const (),
        ds2_rva::FE_INGAME_MENU_TAB_INIT,
    );

    Outcome { installed: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action a slot can be handed must be outside the game's own id space -- the
    /// dispatch's cases are `0..=9`, `0xb`, `0xc`, `0xd`. An id inside that range would make the
    /// row do whatever the game does for it on any run where the detour failed to install, and it
    /// must never be the quit action: a second Return-to-Title row would be a perfectly visible
    /// row that quietly doubles the one item in this menu that discards progress.
    #[test]
    fn every_action_is_ours_and_outside_the_games_range() {
        for slot in 0..crate::api::MAX_ADDED_ROWS {
            let action = ds2_rva::FE_INGAME_MENU_ACTION_BASE + slot as u32;
            assert!(action > 0xd, "{action:#x} is inside the shipped switch");
            assert_ne!(action, ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE);
        }
        assert_eq!(GATE, ds2_rva::FE_INGAME_MENU_GATE_ALWAYS);
    }

    /// The shipped tab must have room, or the detour can only ever refuse.
    #[test]
    fn the_tab_has_room_for_one_more() {
        assert!(
            ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS.len()
                < ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY
        );
    }

    /// The expected contents have to include the quit item, or this is not the tab this crate says
    /// it is.
    #[test]
    fn the_expected_tab_carries_the_quit_item() {
        assert!(
            ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS
                .iter()
                .any(|(action, _)| *action == ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE)
        );
    }

    /// The padding term is the game's, so it is worth pinning: on any 4-aligned descriptor it must
    /// vanish, and entries must be [`ds2_rva::FE_INGAME_MENU_ITEM_STRIDE`] apart.
    #[test]
    fn entries_are_stride_apart_on_an_aligned_descriptor() {
        let mut buffer = [0u8; 64];
        let base = buffer.as_mut_ptr();
        assert_eq!(base as usize % 4, 0, "test buffer is not 4-aligned");
        // SAFETY: indices 0 and 1 are within a 64-byte buffer at stride 8.
        let (first, second) = unsafe { (entry_at(base, 0), entry_at(base, 1)) };
        assert_eq!(first, base);
        assert_eq!(
            second as usize - first as usize,
            ds2_rva::FE_INGAME_MENU_ITEM_STRIDE
        );
    }

    /// The log's entry rendering is read by a human comparing it against this repo's docs, so its
    /// shape is part of the instrument.
    #[test]
    fn entries_render_as_hex_action_and_decimal_gate() {
        assert_eq!(
            describe(&ds2_rva::FE_INGAME_MENU_SYSTEM_TAB_ITEMS),
            "(0x7,0) (0x8,0) (0x9,4)"
        );
    }
}
