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
const PAYLOAD: (u32, u32) = (
    ds2_rva::FE_INGAME_MENU_ACTION_KEY_BINDINGS_UNUSED,
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
    Outcome { installed: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The payload must be an action the dispatch already has a case for. A typo here produces a
    /// row that plays the reject sound, which looks identical to a gated row.
    #[test]
    fn payload_action_is_the_unlisted_one() {
        assert_eq!(
            PAYLOAD.0,
            ds2_rva::FE_INGAME_MENU_ACTION_KEY_BINDINGS_UNUSED
        );
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
