//! Draw the title menu's unavailable rows instead of hiding them.
//!
//! # What the game does
//!
//! The top menu is a fixed vector of **six rows, always**. `0x1400f4250` appends the same six
//! descriptors on every path -- there is no branch that skips an append, and the count is 6 unless
//! the fixed vector overflows and panics. **Nothing is ever inserted or removed.** The only
//! per-row variable is one byte, [`ds2_rva::FE_TOP_MENU_ROW_ENABLED_OFFSET`], computed from three
//! facts: a save exists, online is available, and its inverse.
//!
//! [`ds2_rva::FE_TOP_MENU_APPLY_STATES`] turns that byte into **two independent writes**:
//!
//! | | available | unavailable |
//! | --- | --- | --- |
//! | cell state at `+0x8` | `3`, or `4` under the cursor | `2` |
//! | sequence played | `0x67` | `0x7a` |
//!
//! The state is what removes a row from cursor navigation: `FeObjectButtonEx::v16` at
//! `0x14004c5c0` is `vtable[3]() == 1 && [rcx+8] == 3`, and the navigation search consults it on
//! every candidate. The sequence is the entire visual difference -- **nothing in the image reads
//! `cell[+8] == 2` to decide how to draw**, so the appearance of an unavailable row is decided
//! solely by `0x7a`.
//!
//! On this layout `0x7a` does not grey a row, it removes it from the screen. That is the shipped
//! behaviour the player sees as "LOAD GAME is not there until you have a save", and it is what this
//! module changes.
//!
//! # What this does instead
//!
//! It swaps which of the two writes carries the "unavailable" meaning:
//!
//! * the row is styled as if available, so it is **drawn**;
//! * the cell state is put back to [`ds2_rva::FE_BUTTON_STATE_UNAVAILABLE`], so it is **still not
//!   selectable**.
//!
//! Both halves are the game's own values written to the game's own fields. The mod invents no
//! state and adds no notion of "available" of its own -- it reads the byte the game computed and
//! re-applies the same verdict through the other one of the two mechanisms the game already has.
//!
//! # The honest limitation
//!
//! **The row is drawn in its normal style, not a greyed one.** The sequence ids are indices into a
//! layout resource inside `GameDataEbl.bdt`, so what any given id looks like cannot be read out of
//! the executable; `0x67` is used because it is the only id this exact element is *proven* to
//! render, by the available branch of the very function being replaced. Finding a genuine
//! greyed-out sequence means decrypting the layout archive with `GameDataKeyCode.pem` and reading
//! its animation table, which is a separate piece of work and is not guessed at here. What ships is
//! "visible and inert", which is the half that can be established.
//!
//! # Why the state is re-asserted twice
//!
//! Once synchronously, inside the styling detour, so the corrected state is in place before
//! `FexGridControl`'s refresh runs and picks a cursor position -- the activate handler
//! `0x1400f4a60` does **no** enable check of its own, so a cursor that could rest on an unavailable
//! row would be able to fire its action. And once per frame from
//! [`ds2_rva::FE_TOP_MENU_UPDATE`], which is the only per-frame function specific to this menu, so
//! that anything else which rewrites the cell states cannot leave a row selectable for longer than
//! the frame it did it in.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, safe_read_i32, safe_read_u8, safe_read_usize};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// Trampoline back to the original enable-and-style pass.
static APPLY_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Trampoline back to `FeSubStateTitleTopMenu`'s original update.
static UPDATE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Live module base, resolved once at install so neither detour repeats the lookup per frame.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// Bit `i` set means row `i` was computed unavailable by the game's own builder.
///
/// Written by the styling detour, which is the only place the descriptor list is in scope, and read
/// by the per-frame detour, which has no list of its own. Starts empty, so a per-frame pass that
/// runs before any styling pass does nothing at all rather than guessing.
static UNAVAILABLE_ROWS: AtomicU32 = AtomicU32::new(0);

/// Row count seen by the last styling pass, clamped to the vector's own capacity.
static ROW_COUNT: AtomicUsize = AtomicUsize::new(0);

/// How many rows have been drawn that the game would have hidden.
static SHOWN: AtomicUsize = AtomicUsize::new(0);

/// Whether the first styling pass has been logged.
static LOGGED: AtomicUsize = AtomicUsize::new(0);

/// `void apply_states(group, list)` -- both in registers, no stack arguments.
type ApplyStatesFn = unsafe extern "system" fn(*mut u8, *mut u8);

/// `void update(this, float delta)` -- `this` in RCX, the frame delta in XMM1.
///
/// The float is carried for the same reason every other update detour in this crate carries it:
/// the family's signature puts the delta in XMM1, and a detour that declared only `this` would be
/// free to clobber it.
type UpdateFn = unsafe extern "system" fn(*mut u8, f32);

/// `longlong cell_for_index(group, int index)` -- returns the cell whose id matches, or null.
type CellForIndexFn = unsafe extern "system" fn(*mut u8, i32) -> *mut u8;

/// Address of a row descriptor within the list, reproducing the game's own alignment fudge.
///
/// The builder and the styling pass both compute the element base as
/// `list + ((-list) & 7) + index * 0x38`, and the descriptors are only ever at those addresses. The
/// negate is done on the full pointer and masked to three bits, which is what the game's
/// `neg rdx; and edx,0x7` does.
fn row_at(list: usize, index: usize) -> usize {
    let align = (0usize.wrapping_sub(list)) & 7;
    list + align + index * ds2_rva::FE_TOP_MENU_ROW_STRIDE
}

/// Put every unavailable row's cell back to the state that keeps it out of cursor navigation.
///
/// Returns how many rows it wrote. Cells are looked up through the game's own by-index lookup
/// rather than by walking the group, because that lookup is the one the styling pass uses and it
/// returns null for an index with no cell -- which is the case this must skip rather than write
/// through.
///
/// # Safety
///
/// `group` must be the live `FeGroupTitleTopMenu`. Each cell is written only at the offset the
/// game's own styling pass writes, and only after a fault-tolerant read of that same offset has
/// succeeded.
unsafe fn reassert_unavailable(group: *mut u8) -> usize {
    let base = MODULE_BASE.load(Ordering::Acquire);
    let mask = UNAVAILABLE_ROWS.load(Ordering::Acquire);
    let count = ROW_COUNT.load(Ordering::Acquire);
    if base == 0 || mask == 0 || count == 0 || group.is_null() {
        return 0;
    }
    // SAFETY: resolved from the live module base; called with the group pointer and the index its
    // own call site in `FE_TOP_MENU_APPLY_STATES` passes.
    let cell_for_index: CellForIndexFn = unsafe {
        std::mem::transmute::<usize, CellForIndexFn>(
            base + ds2_rva::FE_TOP_MENU_CELL_FOR_INDEX as usize,
        )
    };
    let mut written = 0;
    for index in 0..count {
        if mask & (1 << index) == 0 {
            continue;
        }
        let cell = unsafe { cell_for_index(group, index as i32) };
        if cell.is_null() {
            continue;
        }
        let state = cell as usize + ds2_rva::FE_BUTTON_STATE_OFFSET;
        // Read before write: a cell the lookup handed back is mapped, and the read also rejects
        // the case where it is already correct, which is the common one on later frames.
        match unsafe { safe_read_i32(state) } {
            Some(current) if current == ds2_rva::FE_BUTTON_STATE_UNAVAILABLE => continue,
            Some(_) => {}
            None => continue,
        }
        // SAFETY: the same field the game's own pass writes at `0x1400f509a`, just read without
        // faulting, and written with the value that pass writes.
        unsafe {
            cell.add(ds2_rva::FE_BUTTON_STATE_OFFSET)
                .cast::<i32>()
                .write(ds2_rva::FE_BUTTON_STATE_UNAVAILABLE)
        };
        written += 1;
    }
    written
}

/// Style every row as available so all six are drawn, then put the unavailable ones back to a state
/// that cannot be selected.
///
/// The enable bytes are restored before returning. The list is the caller's stack buffer and this
/// is the only function that reads it, so leaving it edited would be harmless -- it is restored
/// anyway, because a detour that hands back a buffer it was not given the right to change is a
/// detour whose blast radius depends on a fact about a caller rather than on its own code.
unsafe extern "system" fn detour_apply_states(group: *mut u8, list: *mut u8) {
    let trampoline = APPLY_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 || group.is_null() || list.is_null() {
        // Nothing safe to do without the original: this detour's whole job is to bracket it.
        return;
    }
    // SAFETY: MinHook published this trampoline for this site, and both arguments are forwarded
    // exactly as received.
    let original: ApplyStatesFn =
        unsafe { std::mem::transmute::<usize, ApplyStatesFn>(trampoline) };

    let list_address = list as usize;
    let count = unsafe { safe_read_i32(list_address + ds2_rva::FE_TOP_MENU_LIST_COUNT_OFFSET) }
        .filter(|count| *count >= 0)
        .map(|count| (count as usize).min(ds2_rva::FE_TOP_MENU_ROW_CAPACITY))
        .unwrap_or(0);
    if count == 0 {
        unsafe { original(group, list) };
        return;
    }

    // Read the game's verdict before touching anything, and force every row to the available path
    // so the styling pass draws all six.
    let mut mask = 0u32;
    let mut saved = [1u8; ds2_rva::FE_TOP_MENU_ROW_CAPACITY];
    for (index, slot) in saved.iter_mut().enumerate().take(count) {
        let enabled = row_at(list_address, index) + ds2_rva::FE_TOP_MENU_ROW_ENABLED_OFFSET;
        let Some(value) = (unsafe { safe_read_u8(enabled) }) else {
            // A row whose own descriptor cannot be read is left exactly as the game built it.
            continue;
        };
        *slot = value;
        if value == 0 {
            mask |= 1 << index;
            // SAFETY: the byte was just read without faulting, and this writes the same field the
            // builder writes at `0x1400f437b`, with a value that field already takes.
            unsafe { (enabled as *mut u8).write(1) };
        }
    }
    UNAVAILABLE_ROWS.store(mask, Ordering::Release);
    ROW_COUNT.store(count, Ordering::Release);

    unsafe { original(group, list) };

    // Put the buffer back the way it was handed over.
    for (index, slot) in saved.iter().enumerate().take(count) {
        if mask & (1 << index) == 0 {
            continue;
        }
        let enabled = row_at(list_address, index) + ds2_rva::FE_TOP_MENU_ROW_ENABLED_OFFSET;
        // SAFETY: written one line above through the same address.
        unsafe { (enabled as *mut u8).write(*slot) };
    }

    // SYNCHRONOUSLY, not on the next frame: `FeGroupTitleTopMenu::v25` calls
    // `FexGridControl::FUN_140023690` immediately after this pass, and that is what settles the
    // cursor. The activate handler does no enable check of its own, so a cursor allowed to rest on
    // a row this mod made look selectable would be able to fire that row's action.
    let written = unsafe { reassert_unavailable(group) };
    let total = SHOWN.fetch_add(written, Ordering::Relaxed) + written;

    if LOGGED.swap(1, Ordering::Relaxed) == 0 {
        // The mask is the evidence for what the menu actually decided on this machine: which rows
        // the game would have hidden is the whole reason this feature exists, and a run where the
        // mask is 0 means it changed nothing and would otherwise be indistinguishable from a run
        // where the hook never fired.
        log(format_args!(
            "{LOG_PREFIX} shown screen=top-menu rows={count} unavailable-mask=0b{mask:06b} \
             state={} total={total}",
            ds2_rva::FE_BUTTON_STATE_UNAVAILABLE
        ));
    }
}

/// Re-assert the unavailable rows once per frame, after the game's own update has run.
///
/// The scene tick that handles this menu's input runs inside the original, so writing afterwards
/// means the states navigation reads on the next frame are the corrected ones.
unsafe extern "system" fn detour_top_menu_update(this: *mut u8, delta: f32) {
    let trampoline = UPDATE_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site, and `delta` is forwarded
        // untouched.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
        unsafe { original(this, delta) };
    }
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 || UNAVAILABLE_ROWS.load(Ordering::Acquire) == 0 {
        return;
    }
    let Some(globals) = (unsafe { safe_read_usize(base + ds2_rva::FE_TITLE_GLOBALS as usize) })
    else {
        return;
    };
    let Some(scene) = (unsafe { safe_read_usize(globals + ds2_rva::FE_TITLE_SCENE_OFFSET) }) else {
        return;
    };
    if scene == 0 {
        return;
    }
    let Some(group) = (unsafe { safe_read_usize(scene + ds2_rva::FE_TOP_MENU_GROUP_OFFSET) })
    else {
        return;
    };
    if group == 0 {
        return;
    }
    // SAFETY: the group pointer came from the field the scene's own builder writes at
    // `0x1400f4950`, through fault-tolerant reads that all succeeded.
    let written = unsafe { reassert_unavailable(group as *mut u8) };
    if written != 0 {
        SHOWN.fetch_add(written, Ordering::Relaxed);
    }
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Unavailable rows are now drawn rather than hidden.
    pub show_unavailable: bool,
}

/// Install the top-menu hooks.
///
/// Both sites go in together or not at all: the per-frame re-assert exists only to protect the
/// invariant the styling detour breaks, so installing one without the other would either change
/// nothing or leave a selectable row that should not be.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` and before the
/// title flow reaches the top menu.
pub unsafe fn install() -> Outcome {
    let mut outcome = Outcome {
        show_unavailable: false,
    };
    let base = match game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} menu-install-failed stage=module-base error={error}"
            ));
            return outcome;
        }
    };
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} menu-install-failed stage=MH_Initialize status={status:?}"
        ));
        return outcome;
    }
    // Published before either site is patched: a detour that fired with a zero here could not
    // resolve the cell lookup, and would leave rows it had just made visible still selectable.
    MODULE_BASE.store(base, Ordering::Release);

    let apply_site = base + ds2_rva::FE_TOP_MENU_APPLY_STATES as usize;
    let apply_hook = match unsafe {
        MhHook::new(
            apply_site as *mut c_void,
            detour_apply_states as *mut c_void,
        )
    } {
        Ok(hook) => {
            // Published BEFORE the site is patched: this detour brackets the original and does
            // nothing at all without it, so a zero would mean the menu drew no styles whatsoever.
            APPLY_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(apply_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                true
            } else {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed gate=top-menu-styles va=0x{apply_site:016x} \
                     stage=MH_EnableHook status={status:?}"
                ));
                false
            }
        }
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} hook-failed gate=top-menu-styles va=0x{apply_site:016x} \
                 stage=MH_CreateHook status={status:?}"
            ));
            false
        }
    };
    if !apply_hook {
        return outcome;
    }

    let update_site = base + ds2_rva::FE_TOP_MENU_UPDATE as usize;
    match unsafe {
        MhHook::new(
            update_site as *mut c_void,
            detour_top_menu_update as *mut c_void,
        )
    } {
        Ok(hook) => {
            UPDATE_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
            let status = unsafe { MH_EnableHook(update_site as *mut c_void) };
            if status == MH_STATUS::MH_OK {
                outcome.show_unavailable = true;
                log(format_args!(
                    "{LOG_PREFIX} hooked gate=top-menu rva=0x{:08x} va=0x{apply_site:016x} \
                     update-rva=0x{:08x} va=0x{update_site:016x}",
                    ds2_rva::FE_TOP_MENU_APPLY_STATES,
                    ds2_rva::FE_TOP_MENU_UPDATE
                ));
            } else {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed gate=top-menu-update va=0x{update_site:016x} \
                     stage=MH_EnableHook status={status:?}"
                ));
            }
        }
        Err(status) => log(format_args!(
            "{LOG_PREFIX} hook-failed gate=top-menu-update va=0x{update_site:016x} \
             stage=MH_CreateHook status={status:?}"
        )),
    }

    outcome
}
