//! UP on the Inventory tab's category strip enters the item list at its bottom row.
//!
//! The shipped strip answers one direction. Its input tail (`0x1400bc160`) hands the event to the
//! category tab grid and then tests `byte [event] & 2`: DOWN runs vtable `+0x168`
//! ([`ds2_rva::FE_INVENTORY_ENTER_LIST_SLOT`]), which enters the list. UP has no branch and does
//! nothing. This adds the mirror, and it adds it by calling the same slot DOWN calls -- so an empty
//! category is refused with the game's own cue, the strip is unfocused and the menu sound plays
//! exactly as they do for DOWN -- and then moving the list's cursor to the bottom row, in its
//! column or on the last item when that column stops short, through the game's own cursor setter
//! ([`ds2_rva::FEX_GRID_SET_CURSOR`]).
//!
//! It rides on the inventory update detour this crate already owns, because that update
//! (`0x1400bbea0`) is where the event arrives and MinHook cannot patch one site twice. The press is
//! read from the event itself (`+0` = pressed this frame), not from the device, so it is the same
//! press the game acted on, keyboard or pad, and a held UP fires once.
//!
//! Two states are checked before and after the original runs, and both must hold: the strip had
//! focus when the event arrived (empty focus stack), and still has it afterwards (nothing else
//! consumed the press). The `+0x2040 == 1` mode, in which the shipped code does not enter the list
//! on DOWN either, is left alone.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log_line;

/// `FEX_GRID_CURRENT_INDEX`, resolved and prologue-checked at install. `0` disables the feature.
static CURRENT_INDEX: AtomicUsize = AtomicUsize::new(0);
/// `FEX_GRID_SET_CURSOR`, resolved and prologue-checked at install. `0` disables the feature.
static SET_CURSOR: AtomicUsize = AtomicUsize::new(0);
/// How many entries have been logged. Each UP entry is a deliberate press, so the cap is generous.
static LOGGED: AtomicU32 = AtomicU32::new(0);
const LOG_CAP: u32 = 24;

type CurrentIndexFn = unsafe extern "system" fn(*mut u8) -> i32;
type SetCursorFn = unsafe extern "system" fn(*mut u8, i32) -> i32;
type EnterListFn = unsafe extern "system" fn(*mut u8);

/// Resolve and check the two grid functions. Returns whether the feature is armed.
pub(crate) fn install(base: usize) -> bool {
    let checks = [
        (
            ds2_rva::FEX_GRID_CURRENT_INDEX,
            ds2_rva::FEX_GRID_CURRENT_INDEX_PROLOGUE,
            &CURRENT_INDEX,
            "grid-current-index",
        ),
        (
            ds2_rva::FEX_GRID_SET_CURSOR,
            ds2_rva::FEX_GRID_SET_CURSOR_PROLOGUE,
            &SET_CURSOR,
            "grid-set-cursor",
        ),
    ];
    for (rva, want, slot, what) in checks {
        let site = base + rva as usize;
        let mut seen = [0u8; 5];
        // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
        let read = unsafe { ds2_game_base::mem::read_bytes(site, &mut seen) };
        if !read || seen != want {
            log_line(format_args!(
                "{LOG_PREFIX} up-enter disarmed stage=prologue what={what} va=0x{site:016x} \
                 read={read} saw={seen:02x?} want={want:02x?}"
            ));
            CURRENT_INDEX.store(0, Ordering::Release);
            SET_CURSOR.store(0, Ordering::Release);
            return false;
        }
        slot.store(site, Ordering::Release);
    }
    log_line(format_args!(
        "{LOG_PREFIX} up-enter armed -- UP on the category strip enters the list at its bottom row"
    ));
    true
}

fn read_i32(address: usize) -> Option<i32> {
    let mut bytes = [0u8; 4];
    // SAFETY: `read_bytes` probes rather than trusting; an unmapped address is `false`.
    unsafe { ds2_game_base::mem::read_bytes(address, &mut bytes) }.then(|| i32::from_le_bytes(bytes))
}

fn read_usize(address: usize) -> Option<usize> {
    let mut bytes = [0u8; size_of::<usize>()];
    // SAFETY: as above.
    unsafe { ds2_game_base::mem::read_bytes(address, &mut bytes) }
        .then(|| usize::from_le_bytes(bytes))
}

/// The focused child on top of the group's focus stack, `0` when the strip has focus.
///
/// Mirrors `0x1400bc167..0x1400bc1a6`: storage at `this + 0x70` rounded up to 8, count at `+0xb8`,
/// top entry at `storage + (count - 1) * 8`.
fn focused_child(this: usize) -> Option<usize> {
    let count = read_usize(this + ds2_rva::FE_INVENTORY_FOCUS_STACK_COUNT_OFFSET)?;
    if count == 0 {
        return Some(0);
    }
    let storage = (this + ds2_rva::FE_INVENTORY_FOCUS_STACK_OFFSET + 7) & !7;
    read_usize(storage + (count - 1) * size_of::<usize>())
}

/// The state this module may act in, sampled before the original update runs.
pub(crate) struct Before {
    armed: bool,
}

/// Read, before the original update, whether this event is a fresh UP on a focused strip.
pub(crate) fn before(this: *mut u8, vtable: usize, event: usize) -> Before {
    let this = this as usize;
    let armed = SET_CURSOR.load(Ordering::Acquire) != 0
        && event != 0
        && read_usize(this) == Some(vtable)
        && read_i32(event + ds2_rva::FE_MENU_EVENT_PRESSED_OFFSET)
            .is_some_and(|pressed| pressed as u32 & ds2_rva::FE_MENU_EVENT_UP != 0)
        && focused_child(this) == Some(0)
        && read_i32(this + ds2_rva::FE_INVENTORY_STRIP_ALT_MODE_OFFSET) != Some(1)
        && read_usize(this + ds2_rva::FE_INVENTORY_GROUP_BUSY_OFFSET) == Some(0);
    Before { armed }
}

/// After the original update: enter the list the way DOWN does, then put the cursor on the bottom
/// row of its column.
///
/// # Safety
///
/// Game thread, from inside the inventory group's own update, with the `this` that update was
/// given. `before` must be the value [`before`] returned for this same call.
pub(crate) unsafe fn after(this: *mut u8, before: Before) {
    if !before.armed || focused_child(this as usize) != Some(0) {
        return;
    }
    let Some(vtable) = read_usize(this as usize) else { return };
    let Some(enter) = read_usize(vtable + ds2_rva::FE_INVENTORY_ENTER_LIST_SLOT) else {
        return;
    };

    // The cursor moves before the list takes focus, the same order DOWN sees: entering highlights
    // whatever cell the list's cursor already holds. Moving it after the entry left the entry's
    // highlight on the retained cell and put a second one on the bottom row -- the first in-game
    // run, 2026-09-29, showed two highlighted cells. An empty category has no bottom row and is
    // left for the entry to refuse with its own cue.
    let grid = this as usize + ds2_rva::FE_INVENTORY_ITEM_GRID_OFFSET;
    let moved = bottom_of_column(grid).map(|target| {
        let set = SET_CURSOR.load(Ordering::Acquire);
        // SAFETY: the prologue-checked cursor setter, called on the item grid embedded in the live
        // group, on the game thread -- the same call the D-pad step makes.
        let took =
            unsafe { std::mem::transmute::<usize, SetCursorFn>(set)(grid as *mut u8, target) };
        (target, took)
    });

    // SAFETY: `enter` is the live group's own vtable slot +0x168 (the vtable identity was checked in
    // `before`), called with that group on the game thread -- exactly the call DOWN makes at
    // 0x1400bc246.
    unsafe { std::mem::transmute::<usize, EnterListFn>(enter)(this) };

    let entered = focused_child(this as usize) == Some(grid);
    if LOGGED.fetch_add(1, Ordering::Relaxed) < LOG_CAP {
        match moved {
            Some((target, took)) => log_line(format_args!(
                "{LOG_PREFIX} up-enter cursor set to index {target} before entry (setter returned \
                 {took}); entered={entered}"
            )),
            None => log_line(format_args!(
                "{LOG_PREFIX} up-enter empty list, cursor untouched; entered={entered}"
            )),
        }
    }
}

/// The bottom-row index for the cursor's column, see [`crate::bottom_row_index`]. `None` for an
/// empty grid.
///
/// Columns come from the same rule the D-pad step uses (`0x140022c9d..0x140022cfc`): with
/// `+0xd8 == 1` the grid is one row of `max(1, +0xd4, count)` cells, otherwise rows are
/// `max(1, +0xd4)` wide.
fn bottom_of_column(grid: usize) -> Option<i32> {
    let count = read_i32(grid + ds2_rva::FEX_GRID_ITEM_COUNT_OFFSET)?;
    if count <= 0 {
        return None;
    }
    let extent = read_i32(grid + ds2_rva::FEX_GRID_COL_EXTENT_OFFSET)?;
    let rows_mode = read_i32(grid + ds2_rva::FEX_GRID_ROW_EXTENT_OFFSET)?;
    let cols = if rows_mode == 1 { extent.max(count).max(1) } else { extent.max(1) };
    let current_index = CURRENT_INDEX.load(Ordering::Acquire);
    // SAFETY: the prologue-checked pure getter, on the live item grid.
    let current =
        unsafe { std::mem::transmute::<usize, CurrentIndexFn>(current_index)(grid as *mut u8) };
    Some(crate::bottom_row_index(current, count, cols))
}
