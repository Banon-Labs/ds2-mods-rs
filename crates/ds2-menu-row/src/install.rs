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

fn log(args: std::fmt::Arguments<'_>) {
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

/// The entry this crate appends, as `(action, gate)`.
///
/// Gate `0` deliberately: the row must be selectable unconditionally, or a run in which it is
/// greyed cannot be distinguished from a run in which it never appeared.
///
/// The action is OURS, not one borrowed from the game -- see
/// [`ds2_rva::FE_INGAME_MENU_ACTION_QUIT_TO_DESKTOP`]. The shipped dispatch has no case for it, so
/// with the detour below absent the row plays the ordinary confirm sound and does nothing. An
/// inert row is the right failure mode; a row that quietly opened Key Bindings would not be.
const PAYLOAD: (u32, u32) = (
    ds2_rva::FE_INGAME_MENU_ACTION_QUIT_TO_DESKTOP,
    ds2_rva::FE_INGAME_MENU_GATE_ALWAYS,
);

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
    if count >= ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_CAPACITY {
        log(format_args!(
            "{LOG_PREFIX} REFUSED reason=vector-full count={count} fire={fired}"
        ));
        return returned;
    }

    let (action, gate) = PAYLOAD;
    // SAFETY: `count < capacity`, so this slot is inside the vector, and it is the same address the
    // builder's own next push would have written.
    unsafe {
        let slot = entry_at(descriptor, count);
        slot.cast::<u32>().write(action);
        slot.add(4).cast::<u32>().write(gate);
        descriptor
            .add(ds2_rva::FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET)
            .cast::<u64>()
            .write(count as u64 + 1);
    }

    let appended = APPENDED.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} appended action={action:#x} gate={gate} was=[{}] count={}->{} \
         fire={fired} appends={appended}",
        describe(&entries),
        count,
        count + 1
    ));
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

unsafe extern "system" fn dispatch_detour(top_select: *mut u8, action: u32) {
    if action == ds2_rva::FE_INGAME_MENU_ACTION_QUIT_TO_DESKTOP {
        // Deliberately NOT calling the original for our own action. The original would play a
        // sound and fall through its `default`, which is harmless, but every frame between here
        // and the shutdown is a frame in which the game is still running a menu that is about to
        // disappear. Fewer moving parts.
        let base = ds2_game_base::mem::game_module_base().unwrap_or(0);
        if base != 0 {
            unsafe { request_shutdown(base) };
        }
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
        "{LOG_PREFIX} hooked rva=0x{rva:08x} va=0x{site:016x} payload=({:#x},{}) \
         open the pause menu's last tab to read the result",
        PAYLOAD.0, PAYLOAD.1
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
                     va=0x{dispatch_site:016x} action={:#x}=quit-to-desktop",
                    ds2_rva::FE_INGAME_MENU_ACTION_QUIT_TO_DESKTOP
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

    // THE PROBE, installed third and never fatal. It only reads, and it reports for all SIX tabs
    // -- five of which this crate does not touch, which is what makes the sixth's numbers mean
    // something instead of being a lone reading with nothing to compare against.
    let probe_rva = ds2_rva::FE_INGAME_MENU_TAB_INIT;
    let probe_site = base + probe_rva as usize;
    match unsafe { MhHook::new(probe_site as *mut c_void, tab_init_detour as *mut c_void) } {
        Ok(hook) => {
            TAB_INIT_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(probe_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                log(format_args!(
                    "{LOG_PREFIX} probe hooked rva=0x{probe_rva:08x} va=0x{probe_site:016x} \
                     (per-tab init; reports all six tabs)"
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} probe NOT installed stage=MH_EnableHook status={status:?} \
                     -- the row is still appended, only the measurement is missing"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} probe NOT installed stage=MH_CreateHook status={status:?} \
             -- the row is still appended, only the measurement is missing"
        )),
    }

    Outcome { installed: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The payload must be OUR action, and it must be outside the game's own id space -- the
    /// dispatch's cases are `0..=9`, `0xb`, `0xc`, `0xd`. An id inside that range would make the
    /// row do whatever the game does for it on any run where the detour failed to install.
    #[test]
    fn payload_action_is_ours_and_outside_the_games_range() {
        assert_eq!(PAYLOAD.0, ds2_rva::FE_INGAME_MENU_ACTION_QUIT_TO_DESKTOP);
        assert!(PAYLOAD.0 > 0xd);
        assert_eq!(PAYLOAD.1, ds2_rva::FE_INGAME_MENU_GATE_ALWAYS);
    }

    /// The payload must NOT be the quit action. Appending a second Return-to-Title row would be a
    /// perfectly visible fourth row that answers the layout question and quietly doubles the one
    /// item in this menu that discards progress.
    #[test]
    fn payload_is_not_the_quit_action() {
        assert_ne!(PAYLOAD.0, ds2_rva::FE_INGAME_MENU_ACTION_RETURN_TITLE);
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
