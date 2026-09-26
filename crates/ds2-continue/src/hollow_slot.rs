//! Stop a hollow save slot from hard-locking the game.
//!
//! A slot the character list names but whose two data entries are all zeros sends the game's own
//! section walk ([`ds2_rva::SL_SECTION_WALK`]) to the end of the stream without an end marker. The
//! read there fails and leaves the header as it was, the seek clamps, and the walk repeats the same
//! step forever on the game thread. Vanilla code, vanilla data shape; nothing but a kill ends it.
//!
//! # The escape
//!
//! When a header read made by the walk fails, write type [`ds2_rva::SL_SECTION_TYPE_ESCAPE`] into
//! the header. The walk's own `0xe < type` branch then returns
//! [`ds2_rva::SL_SECTION_WALK_RESULT_BAD_TYPE`], the load fails the way any corrupt section fails,
//! and the title shows "Failed to load character data." and goes back to the character list.
//! That exact write was proven on the hung game with `scripts/frida/load-hang-escape.js` before
//! this was written.
//!
//! # Why the walk is hooked and not the read
//!
//! The Frida prototype hooked `DLMemoryInputStream::read` and filtered on the return address. A
//! MinHook detour cannot do that cheaply: Rust has no return-address intrinsic, and the read is a
//! hot generic function with direct callers elsewhere in the save code (`FUN_1402d09c0` alone has
//! eight). The walk makes both its reads through the stream's vtable (`call [rax+0x18]`), so this
//! detours the walk instead and, for the length of that one call, points the stream at a shadow
//! copy of its own vtable whose read slot is a wrapper. Every read the wrapper sees is one the walk
//! made -- the same set the prototype's return-address window selected -- and no other caller of
//! the read pays anything. Nothing else in the workspace hooks either address.
//!
//! The shadow is built only when the stream's vtable is exactly
//! [`ds2_rva::DL_MEMORY_INPUT_STREAM_VTABLE`], the class the live hang used and the class whose
//! slot count is known. Any other stream runs the original untouched.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::{game_rva, read_bytes};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// `u32 walk(void *unused, DLMemoryInputStream *stream, void **handlers)`.
type WalkFn = unsafe extern "system" fn(usize, *mut *const usize, usize) -> u32;

/// `int read(DLMemoryInputStream *this, void *out, size_t len)`.
type ReadFn = unsafe extern "system" fn(*mut *const usize, *mut u8, usize) -> i32;

static WALK_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// `DLMemoryInputStream`'s vtable in the live image, or zero before install.
static STREAM_VTABLE: AtomicUsize = AtomicUsize::new(0);

/// How many walks the escape has ended.
static FIRED: AtomicUsize = AtomicUsize::new(0);

/// Slots in front of the shadow's function slots: the original read, then the RTTI locator.
///
/// The locator stays at `vtable[-1]` where MSVC expects it, so anything that asks the stream for
/// its type during the walk still gets a true answer. The original read sits one further back,
/// where only [`shadow_read`] looks.
const SHADOW_PREFIX: usize = 2;

/// Shadow vtable length: the prefix plus every function slot of the real one.
const SHADOW_LEN: usize = SHADOW_PREFIX + ds2_rva::DL_MEMORY_INPUT_STREAM_VTABLE_SLOTS;

/// Whether a read the walk made is the failed header read that would spin it.
///
/// The prototype's condition, unchanged: a header-sized read that came back failed. A short read
/// that returned a count is left alone, as is any read of another size.
fn escape_applies(len: usize, returned: i32) -> bool {
    len == ds2_rva::SL_SECTION_HEADER_SIZE && returned == ds2_rva::DL_INPUT_STREAM_READ_FAILED
}

/// Lay out the shadow vtable for `real` (its function slots) and `locator` (its `[-1]`).
fn shadow_vtable(
    real: &[usize; ds2_rva::DL_MEMORY_INPUT_STREAM_VTABLE_SLOTS],
    locator: usize,
    wrapper: usize,
) -> [usize; SHADOW_LEN] {
    let mut shadow = [0usize; SHADOW_LEN];
    shadow[0] = real[ds2_rva::DL_INPUT_STREAM_SLOT_READ];
    shadow[1] = locator;
    shadow[SHADOW_PREFIX..].copy_from_slice(real);
    shadow[SHADOW_PREFIX + ds2_rva::DL_INPUT_STREAM_SLOT_READ] = wrapper;
    shadow
}

/// The read slot of the shadow vtable: call the real read, and end the walk if it failed.
unsafe extern "system" fn shadow_read(this: *mut *const usize, out: *mut u8, len: usize) -> i32 {
    // SAFETY: only reachable through a shadow vtable `detour_walk` installed on `this`, so `*this`
    // points `SHADOW_PREFIX` slots into that shadow and slot 0 holds the real read.
    let original: ReadFn = unsafe {
        let vtable = *this;
        std::mem::transmute::<usize, ReadFn>(*vtable.sub(SHADOW_PREFIX))
    };
    // SAFETY: the real read, with the walk's own arguments.
    let returned = unsafe { original(this, out, len) };
    if escape_applies(len, returned) && !out.is_null() {
        // SAFETY: `out` is the walk's 0x20-byte header on its stack (`lea rdx,[rsp+0x20]`), and
        // `len` was just checked to be that size. The failed read left it untouched.
        let was = unsafe {
            let header = out.cast::<u32>();
            let was = header.read_unaligned();
            header.write_unaligned(ds2_rva::SL_SECTION_TYPE_ESCAPE);
            was
        };
        let fired = FIRED.fetch_add(1, Ordering::AcqRel) + 1;
        log(format_args!(
            "{LOG_PREFIX} hollow slot: save stream ended with no end marker; walk stopped instead \
             of spinning (header type 0x{was:x} -> 0x{:x}, fire {fired}). The game reports a \
             failed load and returns to the character list.",
            ds2_rva::SL_SECTION_TYPE_ESCAPE
        ));
    }
    returned
}

/// Detour on the walk: run it with the shadow read in place, then put the real vtable back.
unsafe extern "system" fn detour_walk(
    unused: usize,
    stream: *mut *const usize,
    handlers: usize,
) -> u32 {
    let trampoline = WALK_TRAMPOLINE.load(Ordering::Acquire);
    // SAFETY: stored before the hook was enabled, so it is MinHook's trampoline for this target.
    let original: WalkFn = unsafe { std::mem::transmute::<usize, WalkFn>(trampoline) };
    let expected = STREAM_VTABLE.load(Ordering::Acquire);
    if stream.is_null() || expected == 0 {
        // SAFETY: the original, with the arguments it was given.
        return unsafe { original(unused, stream, handlers) };
    }
    // SAFETY: the walk dereferences `stream`'s vtable on its first instruction that uses it, so a
    // stream the game passes here is a live object.
    let real = unsafe { *stream };
    if real as usize != expected {
        // SAFETY: as above.
        return unsafe { original(unused, stream, handlers) };
    }
    // SAFETY: `real` is the image's own `DLMemoryInputStream` vtable, whose function slots and
    // locator were read from the shipped binary; all of them are in `.rdata`.
    let (slots, locator) = unsafe {
        (
            *real.cast::<[usize; ds2_rva::DL_MEMORY_INPUT_STREAM_VTABLE_SLOTS]>(),
            *real.sub(1),
        )
    };
    let shadow = shadow_vtable(&slots, locator, shadow_read as *const () as usize);
    // SAFETY: `stream` is live (above). The shadow outlives the call: it is this frame's local,
    // and the real vtable is restored before the frame ends. The walk runs synchronously on this
    // thread, and the stream is the load's own object.
    unsafe {
        *stream = shadow.as_ptr().add(SHADOW_PREFIX);
        let result = original(unused, stream, handlers);
        *stream = real;
        result
    }
}

/// Detour the walk. Call from the loader's install position; it is on for every run, because the
/// only thing it changes is a walk that would otherwise never return.
///
/// # Safety
///
/// Patches executable memory in the game image. Must run where the other install paths run.
pub unsafe fn install_hollow_slot_guard() -> bool {
    let site = match game_rva(ds2_rva::SL_SECTION_WALK) {
        Ok(site) => site,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} hollow-slot guard not installed reason=no-module-base -- {error}"
            ));
            return false;
        }
    };
    let expected = ds2_rva::SL_SECTION_WALK_PROLOGUE;
    let mut prologue = [0u8; ds2_rva::SL_SECTION_WALK_PROLOGUE.len()];
    // SAFETY: a resolved RVA inside the loaded image; `read_bytes` reports an unmapped page rather
    // than faulting on it.
    let read = unsafe { read_bytes(site, &mut prologue) };
    if !read || prologue != expected {
        log(format_args!(
            "{LOG_PREFIX} hollow-slot guard not installed reason=prologue va=0x{site:016x} \
             read={read} saw={prologue:02x?} want={expected:02x?}"
        ));
        return false;
    }
    let vtable = match game_rva(ds2_rva::DL_MEMORY_INPUT_STREAM_VTABLE) {
        Ok(vtable) => vtable,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} hollow-slot guard not installed reason=no-vtable -- {error}"
            ));
            return false;
        }
    };
    STREAM_VTABLE.store(vtable, Ordering::Release);
    // SAFETY: `MH_Initialize` is safe to call again; the status distinguishes that case.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} hollow-slot guard not installed stage=MH_Initialize status={status:?}"
        ));
        return false;
    }
    // SAFETY: `site` holds the walk's own prologue, and `detour_walk` has its ABI.
    let hook = match unsafe { MhHook::new(site as *mut c_void, detour_walk as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} hollow-slot guard not installed stage=MH_CreateHook \
                 status={status:?}"
            ));
            return false;
        }
    };
    // Published before the site is patched, so the detour can never run without it.
    WALK_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: the target `MhHook::new` just registered.
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} hollow-slot guard not installed stage=MH_EnableHook status={status:?}"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} hooked site=section-walk rva=0x{:08x} va=0x{site:016x} -- a hollow save slot \
         fails to load instead of hanging",
        ds2_rva::SL_SECTION_WALK
    ));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_failed_header_read_ends_the_walk() {
        assert!(escape_applies(0x20, -1));
        // A read that returned bytes, even a short count, is a real header.
        assert!(!escape_applies(0x20, 0x20));
        assert!(!escape_applies(0x20, 4));
        assert!(!escape_applies(0x20, 0));
        // Another size is not the walk's header read.
        assert!(!escape_applies(0x10, -1));
    }

    #[test]
    fn the_escape_type_is_one_the_walk_rejects_and_not_its_end_marker() {
        // The walk returns on `0xe < type`, and must not mistake the escape for a clean end.
        const { assert!(ds2_rva::SL_SECTION_TYPE_ESCAPE > 0xe) };
        assert_ne!(
            ds2_rva::SL_SECTION_TYPE_ESCAPE,
            ds2_rva::SL_SECTION_TYPE_END
        );
    }

    #[test]
    fn the_shadow_keeps_every_slot_but_the_read() {
        let real: [usize; ds2_rva::DL_MEMORY_INPUT_STREAM_VTABLE_SLOTS] =
            std::array::from_fn(|index| 0x1000 + index);
        let shadow = shadow_vtable(&real, 0xc01, 0xbeef);
        let read = ds2_rva::DL_INPUT_STREAM_SLOT_READ;
        // The wrapper finds the real read two slots in front of the vtable pointer.
        assert_eq!(shadow[0], real[read]);
        // The RTTI locator stays at `vtable[-1]`.
        assert_eq!(shadow[SHADOW_PREFIX - 1], 0xc01);
        for (index, slot) in shadow[SHADOW_PREFIX..].iter().enumerate() {
            if index == read {
                assert_eq!(*slot, 0xbeef);
            } else {
                assert_eq!(*slot, real[index], "slot {index} must be the real one");
            }
        }
    }
}
