//! The captions: one for the added row, and a corrected one for the row above it.
//!
//! # Why two, and why the one above changes
//!
//! The shipped bottom row says **"Quit Game"** and returns to the title screen, offering to save on
//! the way. The row this crate adds quits to the desktop. Two rows labelled "Quit Game", one of
//! which discards progress without asking, is a trap wearing the label of the thing it is not — so
//! the shipped row becomes **"Quit to Menu"**, which is what it does, and the added row takes
//! **"Quit Game"**, which is what *it* does.
//!
//! Neither string is in the game's text. Every English FMG was searched (13023 strings across 26
//! files) for anything meaning quit-to-desktop or return-to-title; the nearest are "Quit Game"
//! itself, "Quit" in `win32onlymessage.fmg` — whose id collides with a null entry in `shop.fmg` —
//! and "Close". So these two are supplied, and the cost is that they stay English while the FMG the
//! game would have used is localised.
//!
//! # How a caption is set, and why this does not rebuild the path
//!
//! [`ds2_rva::FE_INGAME_TOP_SELECT_CAPTIONS`] builds ten `(path, FMG id)` pairs on its stack and
//! loops over them exactly ten times. The table cannot be substituted the way the container's child
//! list can, so an eleventh caption has to be bound by making the same calls again.
//!
//! It does **not** rebuild the scene path. Rebuilding means reproducing a four-id builder, a seal
//! with a two-element suffix vector, and the exact stack shapes of both — three chances to be
//! wrong about a structure, in a function that has already killed this game once. Instead the
//! append itself is detoured: when the original asks for the quit tab's bottom label, the base path
//! is sitting in `rcx`, live and correct, and one more append against it costs nothing and cannot
//! disagree with the game about the path's shape.
//!
//! The two text writes then happen **after** the original's loop, because that loop would otherwise
//! overwrite them with the FMG text it was always going to apply.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// `fn(topSelect)`, the caption binder.
type BindCaptionsFn = unsafe extern "system" fn(*mut u8);
/// `fn(path, out, id) -> out`, one component appended to a scene path.
type PathAppendFn = unsafe extern "system" fn(*const u8, *mut u8, u32) -> *mut u8;
/// `fn(sceneHolder, out, path) -> out`, the accessor builder.
type BindProxyFn = unsafe extern "system" fn(*mut u8, *mut u8, *const u8) -> *mut u8;
/// `fn(accessor + 0x30, string)`, literal text onto an element.
type SetTextFn = unsafe extern "system" fn(*mut u8, *const DlString);

static BIND_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static APPEND_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The `FeGroupInGameTopSelect` the current bind is running for, and whether one is running.
///
/// The append detour fires for scene paths all over the game; without the flag it would have to
/// recognise the quit tab by its ids alone, and the ids are shared across screens.
static TOP_SELECT: AtomicUsize = AtomicUsize::new(0);
static BINDING: AtomicBool = AtomicBool::new(false);

/// How many captions were written, and how many binds ran without capturing a path.
static WRITTEN: AtomicUsize = AtomicUsize::new(0);
static MISSED: AtomicUsize = AtomicUsize::new(0);

/// Bytes of one scene path object. Five `u32` ids, slack, then the length at `+0x28`.
const PATH_SIZE: usize = ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE;

/// The quit tab's container path, copied out of the original's stack while it is still live.
///
/// Copied rather than pointed at: the original returns before the text is written, and by then its
/// frame is gone. A path is a value -- ids and a length, no pointers -- so a copy is the whole
/// thing. Behind a lock rather than a `static mut` because the append site is one of the busiest
/// functions in the frontend; the lock is only reached once [`BINDING`] says a caption bind is
/// actually running.
static CAPTURED: Mutex<Option<[u8; PATH_SIZE]>> = Mutex::new(None);

/// The string layout [`ds2_rva::FE_ELEMENT_SET_TEXT`] reads.
///
/// `capacity` above [`ds2_rva::DL_STRING_INLINE_CAPACITY`] is what selects the pointer branch, so
/// `data` is read and the inline bytes are not. Nothing here is ever written by the game.
#[repr(C)]
struct DlString {
    data: *const u16,
    length: u64,
    slack: u64,
    capacity: u64,
}

// SAFETY: every field is plain data pointing at a `static` in this DLL, and the game only reads it.
unsafe impl Sync for DlString {}

/// NUL-terminated UTF-16, because the setter measures the string itself.
static QUIT_GAME: [u16; 10] = [
    b'Q' as u16,
    b'u' as u16,
    b'i' as u16,
    b't' as u16,
    b' ' as u16,
    b'G' as u16,
    b'a' as u16,
    b'm' as u16,
    b'e' as u16,
    0,
];
static QUIT_TO_MENU: [u16; 13] = [
    b'Q' as u16,
    b'u' as u16,
    b'i' as u16,
    b't' as u16,
    b' ' as u16,
    b't' as u16,
    b'o' as u16,
    b' ' as u16,
    b'M' as u16,
    b'e' as u16,
    b'n' as u16,
    b'u' as u16,
    0,
];

static ADDED_ROW_TEXT: DlString = DlString {
    data: QUIT_GAME.as_ptr(),
    length: (QUIT_GAME.len() - 1) as u64,
    slack: 0,
    capacity: 32,
};
static TITLE_ROW_TEXT: DlString = DlString {
    data: QUIT_TO_MENU.as_ptr(),
    length: (QUIT_TO_MENU.len() - 1) as u64,
    slack: 0,
    capacity: 32,
};

/// Resolve `base + label` and write `text` onto it.
///
/// # Safety
///
/// `base` must be a scene path for the quit tab's container and `top_select` the group the
/// original was called with. Both callees are the game's own, called with the buffer sizes the
/// original gives them.
unsafe fn write_caption(
    base: &[u8; PATH_SIZE],
    top_select: usize,
    label: u32,
    text: &'static DlString,
) {
    let Some(base_module) = ds2_game_base::mem::game_module_base().ok() else {
        return;
    };
    let append = APPEND_TRAMPOLINE.load(Ordering::Acquire);
    if append == 0 || top_select == 0 {
        return;
    }
    // SAFETY: MinHook published this trampoline for the append site.
    let append: PathAppendFn = unsafe { std::mem::transmute::<usize, PathAppendFn>(append) };
    // SAFETY: both RVAs are `.pdata` function starts recorded in `ds2-rva`.
    let bind_proxy: BindProxyFn =
        unsafe { std::mem::transmute(base_module + ds2_rva::FE_BIND_SCENE_OBJ_PROXY as usize) };
    // SAFETY: same.
    let set_text: SetTextFn =
        unsafe { std::mem::transmute(base_module + ds2_rva::FE_ELEMENT_SET_TEXT as usize) };

    // Generous, and deliberately so: the original gives the accessor 0x90 bytes and a path 0x30,
    // and a buffer that is too large costs a few bytes of stack while one that is too small is a
    // corruption whose crash names someone else's function.
    let mut path = [0u8; PATH_SIZE * 2];
    let mut accessor = [0u8; ds2_rva::FE_ELEMENT_ACCESSOR_SIZE * 2];
    // SAFETY: the three calls are the loop body of `FE_INGAME_TOP_SELECT_CAPTIONS`, transcribed,
    // with our own buffers in place of its stack slots.
    unsafe {
        append(base.as_ptr(), path.as_mut_ptr(), label);
        bind_proxy(
            (top_select + ds2_rva::FE_INGAME_TOP_SELECT_SCENE_HOLDER_OFFSET) as *mut u8,
            accessor.as_mut_ptr(),
            path.as_ptr(),
        );
        set_text(
            accessor
                .as_mut_ptr()
                .add(ds2_rva::FE_ELEMENT_ACCESSOR_TEXT_SLOT_OFFSET),
            text,
        );
    }
    let n = WRITTEN.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} caption label={label:#x} written={n}"
    ));

    // THE TREE, from the accessor that is already built and already correct. Three stretch factors
    // on the panel record changed nothing on screen, so what draws the banner is still unknown and
    // the live tree is the only thing that can name it. Read-only, and once per process.
    // SAFETY: `accessor` was just filled by the game's own binder, and the ids are the path it was
    // filled from.
    unsafe { crate::tree::dump(accessor.as_ptr(), &path_ids(base)) };
}

/// The quit tab's own path, as the ids the caption binder built it from.
///
/// Read out of the captured path rather than written down twice: the capture is the object the
/// original built, and its ids are at `+0x00` upwards with the length at
/// [`ds2_rva::FE_SCENE_NAMER_ENTRY_LEN_OFFSET`].
fn path_ids(base: &[u8; PATH_SIZE]) -> Vec<u32> {
    let count = u32::from_le_bytes(
        base[ds2_rva::FE_SCENE_NAMER_ENTRY_LEN_OFFSET..][..4]
            .try_into()
            .expect("four bytes"),
    )
    .min(5) as usize;
    (0..count)
        .map(|i| u32::from_le_bytes(base[i * 4..][..4].try_into().expect("four bytes")))
        .filter(|id| *id != 0)
        .collect()
}

unsafe extern "system" fn append_detour(path: *const u8, out: *mut u8, id: u32) -> *mut u8 {
    let trampoline = APPEND_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return out;
    }
    // SAFETY: MinHook published this trampoline for exactly this site.
    let original: PathAppendFn = unsafe { std::mem::transmute::<usize, PathAppendFn>(trampoline) };
    // SAFETY: every argument is the caller's own, passed through unchanged.
    let returned = unsafe { original(path, out, id) };

    // THE ONE APPEND WORTH NOTICING: the quit tab's bottom label, during the caption bind. At this
    // instant `path` is the container's scene path, built by the original and still live. It is
    // copied here because the text has to be written after the original's loop, by which time this
    // frame is gone.
    if BINDING.load(Ordering::Acquire)
        && id == ds2_rva::FE_QUIT_TAB_ROW_TITLE_LABEL_ID
        && !path.is_null()
        && let Ok(mut captured) = CAPTURED.lock()
        && captured.is_none()
    {
        let mut bytes = [0u8; PATH_SIZE];
        // SAFETY: `path` is the scene path the original was just handed, and a path object is
        // `PATH_SIZE` bytes -- the stride the game's own namer addresses these at.
        unsafe { std::ptr::copy_nonoverlapping(path, bytes.as_mut_ptr(), PATH_SIZE) };
        *captured = Some(bytes);
    }
    returned
}

unsafe extern "system" fn bind_detour(top_select: *mut u8) {
    TOP_SELECT.store(top_select as usize, Ordering::Release);
    if let Ok(mut captured) = CAPTURED.lock() {
        *captured = None;
    }
    BINDING.store(true, Ordering::Release);

    let trampoline = BIND_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site.
        let original: BindCaptionsFn =
            unsafe { std::mem::transmute::<usize, BindCaptionsFn>(trampoline) };
        // SAFETY: the argument is the caller's own.
        unsafe { original(top_select) };
    }
    BINDING.store(false, Ordering::Release);

    // AFTER the original, never before. Its loop applies the FMG text to the shipped row, and a
    // literal written first would simply be overwritten by it.
    let bytes = match CAPTURED.lock() {
        Ok(captured) => *captured,
        Err(_) => None,
    };
    let Some(bytes) = bytes else {
        let n = MISSED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} caption REFUSED reason=no-path label={:#x} misses={n} \
             -- the added row stays blank and the shipped row keeps its own caption",
            ds2_rva::FE_QUIT_TAB_ROW_TITLE_LABEL_ID
        ));
        return;
    };
    let top = TOP_SELECT.load(Ordering::Acquire);
    // SAFETY: `bytes` is a scene path copied out of a live one, and `top` the group the original
    // just ran for.
    unsafe {
        write_caption(
            &bytes,
            top,
            ds2_rva::FLO_ADDED_ROW_LABEL_ID,
            &ADDED_ROW_TEXT,
        );
        write_caption(
            &bytes,
            top,
            ds2_rva::FE_QUIT_TAB_ROW_TITLE_LABEL_ID,
            &TITLE_ROW_TEXT,
        );
    }
}

/// Detour the caption bind and the path append. Returns whether the added row will be labelled.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan`, and before
/// the pause menu is built.
pub unsafe fn install(base: usize) -> bool {
    let mut ok = true;
    for (rva, detour, trampoline, what) in [
        (
            ds2_rva::FE_SCENE_PATH_APPEND,
            append_detour as *mut c_void,
            &APPEND_TRAMPOLINE,
            "path-append",
        ),
        (
            ds2_rva::FE_INGAME_TOP_SELECT_CAPTIONS,
            bind_detour as *mut c_void,
            &BIND_TRAMPOLINE,
            "caption-bind",
        ),
    ] {
        let site = base + rva as usize;
        match unsafe { MhHook::new(site as *mut c_void, detour) } {
            Ok(hook) => {
                // Published BEFORE the site is patched. The append detour returns its `out` on a
                // zero trampoline, which is the argument the caller already holds; anything else
                // would hand the game a path it never built.
                trampoline.store(hook.trampoline() as usize, Ordering::Release);
                let status = unsafe { MH_EnableHook(site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    log(format_args!(
                        "{LOG_PREFIX} {what} hooked rva=0x{rva:08x} va=0x{site:016x}"
                    ));
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} {what} NOT installed stage=MH_EnableHook status={status:?} \
                         -- the added row stays blank"
                    ));
                    ok = false;
                }
            }
            Err(status) => {
                log(format_args!(
                    "{LOG_PREFIX} {what} NOT installed stage=MH_CreateHook status={status:?} \
                     -- the added row stays blank"
                ));
                ok = false;
            }
        }
    }
    if ok {
        log(format_args!(
            "{LOG_PREFIX} captions armed added-row={:#x}=\"Quit Game\" \
             shipped-row={:#x}=\"Quit to Menu\"",
            ds2_rva::FLO_ADDED_ROW_LABEL_ID,
            ds2_rva::FE_QUIT_TAB_ROW_TITLE_LABEL_ID
        ));
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both strings must be NUL-terminated, because the setter walks to the terminator itself and
    /// an unterminated buffer is a read off the end of this DLL's data.
    #[test]
    fn the_captions_are_nul_terminated() {
        assert_eq!(*QUIT_GAME.last().unwrap(), 0);
        assert_eq!(*QUIT_TO_MENU.last().unwrap(), 0);
    }

    /// The capacity must select the POINTER branch, or the setter reads the struct's own first
    /// bytes as characters -- which is the pointer, rendered as line noise.
    #[test]
    fn the_strings_use_the_out_of_line_layout() {
        for s in [&ADDED_ROW_TEXT, &TITLE_ROW_TEXT] {
            assert!(s.capacity > ds2_rva::DL_STRING_INLINE_CAPACITY);
        }
        assert_eq!(
            ds2_rva::DL_STRING_CAPACITY_OFFSET,
            std::mem::offset_of!(DlString, capacity)
        );
    }

    /// The two rows must not end up with the same label element, or one write lands on the other.
    #[test]
    fn the_two_captions_go_to_different_elements() {
        assert_ne!(
            ds2_rva::FLO_ADDED_ROW_LABEL_ID,
            ds2_rva::FE_QUIT_TAB_ROW_TITLE_LABEL_ID
        );
    }

    /// The added row's label has to be one the quit tab's container does not already carry.
    #[test]
    fn the_added_label_is_not_a_shipped_child() {
        assert!(!ds2_rva::FLO_QUIT_TAB_CHILD_IDS.contains(&ds2_rva::FLO_ADDED_ROW_LABEL_ID));
        assert!(ds2_rva::FLO_QUIT_TAB_CHILD_IDS.contains(&ds2_rva::FE_QUIT_TAB_ROW_TITLE_LABEL_ID));
    }

    /// A captured path is a whole path, not a prefix of one.
    #[test]
    fn the_capture_is_a_whole_path() {
        assert_eq!(PATH_SIZE, ds2_rva::FE_SCENE_NAMER_ENTRY_STRIDE);
        const { assert!(PATH_SIZE > ds2_rva::FE_SCENE_NAMER_ENTRY_LEN_OFFSET) };
    }
}
