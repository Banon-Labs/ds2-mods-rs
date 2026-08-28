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
/// `fn(topSelect, frameDelta)`, the pause-menu group's per-frame update.
type UpdateFn = unsafe extern "system" fn(*mut u8, f32);

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

/// The most UTF-16 units a caption can hold, terminator included.
///
/// Fixed rather than grown, because the buffer is leaked once and then REWRITTEN in place: a row
/// that reports what it is doing changes its text many times per session, and reallocating would
/// mean handing the game a new pointer each time. The shipped caption box is `274` units wide at
/// font size 22 (`ds2_rva::FLO_CAPTION_BOX`), so anything much past this would not be read anyway.
const CAPTION_CAPACITY: usize = 48;

/// One rewritable caption: the leaked buffer, and the `DlString` that points at it.
///
/// Leaked because the game keeps the pointer -- `FeElement::setText` only READS the string, and it
/// reads it whenever it redraws, so the bytes must outlive anything this crate can observe.
struct RowCaption {
    label_id: u32,
    units: *mut u16,
    text: *mut DlString,
}

// SAFETY: both pointers are leaked allocations owned by this module and reachable only under
// `ROW_CAPTIONS`'s lock, which is what serialises every read and write of them.
unsafe impl Send for RowCaption {}

/// Set when a caption has been rewritten and not yet pushed to the screen.
///
/// The push has to happen on the game thread, and [`set_caption`] can be called from anywhere, so
/// the two are joined by this flag rather than by a direct call. [`update_detour`] clears it.
static CAPTIONS_DIRTY: AtomicBool = AtomicBool::new(false);

/// The trampoline for the pause-menu group's per-frame update.
static UPDATE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Every registered row's caption, rewritable.
///
/// A `Mutex` rather than a `OnceLock` because these change: [`set_caption`] is what lets a row say
/// "Fetching..." and then say what it found. The lock is held across the `setText` call in
/// [`push_captions`], which is what makes a worker's write and the game's copy exclusive -- the
/// game copies the bytes DURING that call and never touches them afterwards.
static ROW_CAPTIONS: Mutex<Vec<RowCaption>> = Mutex::new(Vec::new());

/// Build the buffers from the registry, once.
fn ensure_row_captions(captions: &mut Vec<RowCaption>) {
    if !captions.is_empty() {
        return;
    }
    for row in crate::api::rows_for(crate::api::Tab::Quit) {
        let units: Box<[u16]> = vec![0u16; CAPTION_CAPACITY].into_boxed_slice();
        let units = Box::leak(units).as_mut_ptr();
        let text = Box::leak(Box::new(DlString {
            data: units,
            length: 0,
            slack: 0,
            // Above `DL_STRING_INLINE_CAPACITY`, which is what selects the pointer branch. A
            // caption of seven units or fewer would otherwise be read out of the inline bytes,
            // which hold a pointer rather than characters.
            capacity: CAPTION_CAPACITY as u64,
        })) as *mut DlString;
        let caption = RowCaption {
            label_id: row.label_id,
            units,
            text,
        };
        write_units(&caption, row.caption);
        captions.push(caption);
    }
}

/// Copy `text` into a caption's buffer as NUL-terminated UTF-16, truncating to fit.
///
/// Truncates rather than refuses: a caption is a label, and a short one is a better failure than a
/// stale one that still says "Fetching...".
fn write_units(caption: &RowCaption, text: &str) {
    let mut units: Vec<u16> = text.encode_utf16().take(CAPTION_CAPACITY - 1).collect();
    let length = units.len();
    units.push(0);
    // SAFETY: `units` is a leaked allocation of exactly `CAPTION_CAPACITY` units and `units.len()`
    // is at most that, terminator included. The caller holds `ROW_CAPTIONS`.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), caption.units, units.len());
        // The NUL is not part of the length. The setter measures the string itself and the
        // terminator is there for anything that does not.
        (*caption.text).length = length as u64;
    }
}

/// Change what a registered row says. Safe to call from any thread.
///
/// It only rewrites this crate's own buffer -- **nothing reaches the game until
/// [`crate::refresh_row_captions`] or the next pause-menu open**, both of which are on the game
/// thread. That split is the whole point: the crate that wants to report a network result has that
/// result on a worker, and touching the scene from there would be a race with the renderer.
pub(crate) fn set_caption(row: usize, text: &str) -> bool {
    let Ok(mut captions) = ROW_CAPTIONS.lock() else {
        return false;
    };
    ensure_row_captions(&mut captions);
    match captions.get(row) {
        Some(caption) => {
            write_units(caption, text);
            CAPTIONS_DIRTY.store(true, Ordering::Release);
            true
        }
        None => false,
    }
}

/// `FeGroupInGameTopSelect::v2`, detoured: run the game's update, then push any changed caption.
///
/// **This is what makes a caption change VISIBLE while the menu is still open.** Without it, a row
/// that learns something -- a fetch that finished, a link that was refused -- could not say so until
/// the player closed the pause menu and opened it again, because captions are otherwise written
/// once, at bind.
///
/// It does as little as a per-frame detour can: an `AtomicBool` load on every frame, and real work
/// only on the frames where something actually changed. The original runs FIRST, so a frame in which
/// the game itself rewrites the captions is not fought over.
unsafe extern "system" fn update_detour(top_select: *mut u8, delta: f32) {
    let trampoline = UPDATE_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is the
        // one the disassembled entry implements -- the group in RCX, the frame delta in XMM1.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(trampoline) };
        // SAFETY: both arguments are the caller's own.
        unsafe { original(top_select, delta) };
    }
    if CAPTIONS_DIRTY.swap(false, Ordering::AcqRel) {
        // The group is live and this is the game thread, which is exactly what `push_captions`
        // requires. Remember it too: a push can also be wanted on a frame this detour did not
        // start, and the captured path is what the write needs.
        TOP_SELECT.store(top_select as usize, Ordering::Release);
        // SAFETY: the game thread, inside the menu's own per-frame update, with the group it was
        // called for.
        unsafe { push_captions() };
    }
}

/// Push every caption onto its element. **Game thread only.**
///
/// # Safety
///
/// Calls into the game's scene machinery, so it must run on the thread the game builds its menus
/// on and only while the pause menu the captions belong to is up.
pub(crate) unsafe fn push_captions() -> usize {
    let bytes = match CAPTURED.lock() {
        Ok(captured) => *captured,
        Err(_) => None,
    };
    let Some(bytes) = bytes else {
        return 0;
    };
    let top = TOP_SELECT.load(Ordering::Acquire);
    let Ok(mut captions) = ROW_CAPTIONS.lock() else {
        return 0;
    };
    ensure_row_captions(&mut captions);
    let mut pushed = 0;
    for caption in captions.iter() {
        // SAFETY: `bytes` is a scene path copied out of a live one, `top` the group it belongs to,
        // and `caption.text` a leaked `DlString` this module owns. The lock is held across the
        // call, so the game's copy cannot race another thread's rewrite.
        unsafe { write_caption(&bytes, top, caption.label_id, &*caption.text) };
        pushed += 1;
    }
    pushed
}
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
    // Logged only for the first couple of opens. The pause menu is opened over and over in a
    // session and this sink calls `sync_all` per line, so a line that repeats forever is a stall
    // that repeats forever. The first two are the evidence; the rest are noise with a cost.
    if n <= 2 {
        log(format_args!(
            "{LOG_PREFIX} caption label={label:#x} written={n}"
        ));
    }

    // THE TREE DUMP IS NOT RUN. It found what it was built to find -- that the banner is a
    // `FeComponentTextureShape` sized by its own quad -- and it costs 117 log lines, each of which
    // `ds2-loader`'s sink follows with `sync_all()`. That is most of a second of frozen game on the
    // first pause-menu open, for a measurement that has already been taken and written down.
    //
    // `crate::tree::dump` is kept and still compiles; re-arm it here when the next question needs
    // the live tree rather than the file.

    // THE BANNER, once, and only from the pass that resolves the row this crate added -- the other
    // pass targets the shipped row and would do the same work twice.
    if crate::api::rows_for(crate::api::Tab::Quit)
        .first()
        .is_some_and(|first| label == first.label_id)
    {
        // THE CONTAINER PATH, NOT THE CAPTURED ONE. The caption binder seals its base with a
        // trailing `0x5f5b9f2` -- the text leaf every caption ends at -- so the capture is FIVE ids,
        // and appending the panel to it asks for `.../0x1eace6/0x5f5b9f2/0x1eac81`, which resolves
        // to nothing. The tree dump had already said so in as many words -- `prefix5 resolved to
        // NOTHING` -- and the banner refusal that followed named the consequence.
        let mut panel = ds2_rva::FE_QUIT_TAB_BASE_PATH.to_vec();
        panel.push(ds2_rva::FLO_QUIT_TAB_CHILD_IDS[ds2_rva::FLO_QUIT_TAB_PANEL]);
        // SAFETY: the accessor is filled and the path is the container's with the panel appended.
        let component = unsafe { crate::tree::resolve_path(accessor.as_ptr(), &panel) };
        // SAFETY: `component` came from the game's own lookup, or is null and is checked.
        unsafe { crate::banner::lengthen(component, base_module) };
    }
}

/// The quit tab's own path, as the ids the caption binder built it from.
///
/// Only the disarmed tree dump wants this; the banner uses [`ds2_rva::FE_QUIT_TAB_BASE_PATH`],
/// which is the container path by definition rather than by how this caller sealed its own.
#[allow(dead_code)]
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
    // THIS IS ALSO WHAT MAKES A CHANGED CAPTION APPEAR. A row that reports a result rewrites its
    // own buffer from whatever thread got that result; the text reaches the screen HERE, on the
    // game thread, the next time the menu opens -- or sooner, if the row pushes it itself while the
    // menu is still up.
    //
    // SAFETY: this is the caption bind, on the game thread, with the group the original just ran
    // for, and `bytes` is a scene path copied out of a live one.
    unsafe {
        push_captions();
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
    // THE PER-FRAME PUSH IS AN ENHANCEMENT AND FAILS SOFT. Without it every caption still gets
    // written at bind, so the row is labelled and works; what is lost is a caption that CHANGES
    // while the menu is open. So a refusal here logs and leaves `ok` alone -- degrading to
    // "the text updates when you reopen the menu" is not the same class of failure as a blank row.
    let update_site = base + ds2_rva::FE_INGAME_TOP_SELECT_UPDATE as usize;
    let mut prologue = [0u8; ds2_rva::FE_INGAME_TOP_SELECT_UPDATE_PROLOGUE.len()];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(update_site, &mut prologue) };
    if !read || prologue != ds2_rva::FE_INGAME_TOP_SELECT_UPDATE_PROLOGUE {
        // THE BYTES BEFORE THE PATCH. An RVA is a number, and on a build this was not read from it
        // points into the middle of something else that would accept the write.
        log(format_args!(
            "{LOG_PREFIX} caption-tick NOT installed reason=prologue read={read} saw={prologue:02x?} \
             want={:02x?} -- captions will update on the next menu OPEN rather than live",
            ds2_rva::FE_INGAME_TOP_SELECT_UPDATE_PROLOGUE
        ));
    } else {
        match unsafe { MhHook::new(update_site as *mut c_void, update_detour as *mut c_void) } {
            Ok(hook) => {
                UPDATE_TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
                let status = unsafe { MH_EnableHook(update_site as *mut c_void) };
                if status == MH_STATUS::MH_OK {
                    log(format_args!(
                        "{LOG_PREFIX} caption-tick hooked rva=0x{:08x} va=0x{update_site:016x} \
                         -- a changed caption now appears without reopening the menu",
                        ds2_rva::FE_INGAME_TOP_SELECT_UPDATE
                    ));
                } else {
                    log(format_args!(
                        "{LOG_PREFIX} caption-tick NOT installed stage=MH_EnableHook \
                         status={status:?} -- captions update on the next menu OPEN"
                    ));
                }
            }
            Err(status) => log(format_args!(
                "{LOG_PREFIX} caption-tick NOT installed stage=MH_CreateHook status={status:?} \
                 -- captions update on the next menu OPEN"
            )),
        }
    }

    if ok {
        // THE CAPTIONS THIS PRINTS ARE THE ONES IT WROTE. It used to print the literals
        // `"Quit Game"` and `"Quit to Menu"` no matter what was in the registry, which was true
        // exactly as long as there was one hardcoded row -- and then read as a wrong caption the
        // first time another crate registered one. A log line that hardcodes what it claims to have
        // done cannot report the bug it exists to report.
        let added = crate::api::rows_for(crate::api::Tab::Quit)
            .iter()
            .map(|row| format!("{:#x}=\"{}\"", row.label_id, row.caption))
            .collect::<Vec<_>>()
            .join(" ");
        log(format_args!(
            "{LOG_PREFIX} captions armed added-rows=[{added}] shipped-row={:#x}=\"Quit to Menu\"",
            ds2_rva::FE_QUIT_TAB_ROW_TITLE_LABEL_ID
        ));
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped row's replacement must be NUL-terminated, because the setter walks to the
    /// terminator itself and an unterminated buffer is a read off the end of this DLL's data. The
    /// registered rows' strings get their terminator in `write_units`, which pushes one.
    #[test]
    fn the_captions_are_nul_terminated() {
        assert_eq!(*QUIT_TO_MENU.last().unwrap(), 0);
    }

    /// The capacity must select the POINTER branch, or the setter reads the struct's own first
    /// bytes as characters -- which is the pointer, rendered as line noise.
    #[test]
    fn the_strings_use_the_out_of_line_layout() {
        assert!(TITLE_ROW_TEXT.capacity > ds2_rva::DL_STRING_INLINE_CAPACITY);
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
