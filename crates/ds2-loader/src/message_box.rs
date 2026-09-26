//! Every Win32 message box anybody in this process opens, written to the log before it opens.
//!
//! # Why this exists
//!
//! A message box is the one failure every other line in this log reports as a success. On
//! 2026-09-26 `scripts/ds2-run.py` printed its RUNNING block over a game that was showing
//! Seamless Co-op's *"This version of Dark Souls II seamless co-op (0.0.1) is depreciated and
//! requires an update. The application will now exit."* The DLL had loaded, dearxan had reported,
//! every hook had applied, and the launcher's promise -- the game is up -- was false. That mod's
//! DLL is packed and its sentence is not in the file; it exists only in the process, and it passes
//! through `user32` on its way to the screen. So that is where it is caught.
//!
//! # What is hooked
//!
//! The six documented entry points: `MessageBoxA`, `MessageBoxW`, `MessageBoxExA`,
//! `MessageBoxExW`, `MessageBoxIndirectA` and `MessageBoxIndirectW`, as exported by the `user32`
//! this process has mapped. The prologue of the export is what is patched, not an import slot, so
//! it catches a caller however it found the function -- a static import, `GetProcAddress`, or a
//! packed DLL resolving its imports by hand, which is what `ds2sc.dll` does (its only static
//! `USER32` import is `GetDesktopWindow`).
//!
//! Wine implements the six as a chain (`A` -> `ExA` -> `ExW` -> `IndirectW`), so one call from a
//! caller runs several of these detours. A per-thread depth counter logs only the outermost, which
//! is the one whose return address is the real caller: one call, one line.
//!
//! # When it is installed, and why that is early enough for Seamless
//!
//! From `DllMain`, in the loader's `attach`, before `neuter_arxan` -- and not from the Arxan
//! callback with every other hook. The callback runs at the game's entry point, and that is too
//! late for a DLL injected by `ds2-launcher.exe`:
//!
//! - `ds2-launcher` (`crates/ds2-launcher/src/inject.rs`) creates the game `CREATE_SUSPENDED`,
//!   then for each DLL in its plan runs `CreateRemoteThread` on `LoadLibraryW` and waits for that
//!   thread to finish before the next one, and only then calls `ResumeThread` on the game's main
//!   thread. `[seamless] enabled = true` puts `SeamlessCoop/ds2sc.dll` first in that plan.
//! - The first injected thread runs process initialisation before its own start routine, which
//!   maps `DarkSoulsII.exe`'s static imports and runs their `DllMain`s -- ours included, since this
//!   DLL is the game's `DINPUT8.dll` import. Only after that does `LoadLibraryW` load `ds2sc.dll`
//!   and run its `DllMain`.
//! - So `ds2sc.dll`'s `DllMain`, and any thread it starts, can run before the game's entry point
//!   and therefore before the Arxan callback. A box opened there would be missed by a hook
//!   installed with the others. Installed from `DllMain`, this one is already live.
//!
//! Evidence that the ordering holds on this machine, not just on paper: in the 2026-09-26 15:21
//! run the crash logger, installed from the same `attach`, recorded
//! `previous_unhandled_filter=0x0`, and five seconds later found the slot held by `0x1801be5d0` --
//! an address inside `ds2sc.dll`. Our `attach` ran before that mod had initialised far enough to
//! set its filter.
//!
//! `user32` is safe to reach from here: it is a static import of both this DLL and the game, so it
//! is mapped before any `DllMain` runs, and it is looked up with `GetModuleHandleW` -- never
//! loaded -- so nothing here asks the loader for anything under its own lock.
//!
//! # What it does not do
//!
//! Change the call. Every argument goes to the original untouched and its return value comes back
//! untouched; the only effect is one line in the log, written and synced before the box opens, so
//! that it is on disk even if the box is the last thing the process does.

use core::cell::Cell;
use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

/// Every line this module writes opens with this. `scripts/ds2-run.py` matches it exactly
/// (`MESSAGEBOX_LINE_PREFIX` there) and withholds its RUNNING block when it sees one.
pub const LINE_PREFIX: &str = "ds2-loader: MESSAGEBOX ";

/// The install report's prefix. `scripts/ds2-run.py` reads it to say whether the watch was armed,
/// because a watch that never installed reports the same silence as a run with no dialog.
pub const INSTALL_PREFIX: &str = "ds2-loader: messagebox-watch";

/// The most UTF-16 units or bytes read out of one caption or message. A message box's text is a
/// sentence or a paragraph; this bound exists so a pointer to an unterminated buffer costs one
/// truncated line rather than a walk off the end of the caller's memory.
const MAX_UNITS: usize = 2048;

/// `GetModuleHandleExW` flag: the second argument is an address inside the module.
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
/// `GetModuleHandleExW` flag: do not add a reference, so there is no handle to release.
const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, address: *const c_void, module: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn RtlCaptureStackBackTrace(
        frames_to_skip: u32,
        frames_to_capture: u32,
        back_trace: *mut *mut c_void,
        back_trace_hash: *mut u32,
    ) -> u16;
}

/// `MSGBOXPARAMSA` / `MSGBOXPARAMSW`, generic over the character type. Only `text`, `caption` and
/// `style` are read; the rest are here so those three sit at the offsets `winuser.h` gives them.
#[repr(C)]
struct MsgBoxParams<C> {
    size: u32,
    owner: *mut c_void,
    instance: *mut c_void,
    text: *const C,
    caption: *const C,
    style: u32,
    icon: *const C,
    context_help_id: usize,
    callback: *const c_void,
    language_id: u32,
}

type MessageBoxAFn = unsafe extern "system" fn(*mut c_void, *const u8, *const u8, u32) -> i32;
type MessageBoxWFn = unsafe extern "system" fn(*mut c_void, *const u16, *const u16, u32) -> i32;
type MessageBoxExAFn =
    unsafe extern "system" fn(*mut c_void, *const u8, *const u8, u32, u16) -> i32;
type MessageBoxExWFn =
    unsafe extern "system" fn(*mut c_void, *const u16, *const u16, u32, u16) -> i32;
type MessageBoxIndirectAFn = unsafe extern "system" fn(*const MsgBoxParams<u8>) -> i32;
type MessageBoxIndirectWFn = unsafe extern "system" fn(*const MsgBoxParams<u16>) -> i32;

/// The trampolines, one per API, in [`APIS`] order. Zero until that API's hook is created.
static ORIGINALS: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];

const MESSAGE_BOX_A: usize = 0;
const MESSAGE_BOX_W: usize = 1;
const MESSAGE_BOX_EX_A: usize = 2;
const MESSAGE_BOX_EX_W: usize = 3;
const MESSAGE_BOX_INDIRECT_A: usize = 4;
const MESSAGE_BOX_INDIRECT_W: usize = 5;

/// The export names, null-terminated for `GetProcAddress`, indexed like [`ORIGINALS`].
const APIS: [&str; 6] = [
    "MessageBoxA\0",
    "MessageBoxW\0",
    "MessageBoxExA\0",
    "MessageBoxExW\0",
    "MessageBoxIndirectA\0",
    "MessageBoxIndirectW\0",
];

thread_local! {
    /// How many of these detours are on this thread's stack right now. See the module docs.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Holds this thread's depth up for as long as one detour is running.
struct Nesting {
    counted: bool,
}

impl Nesting {
    /// Enter a detour. The flag says whether this is the outermost one on the thread.
    fn enter() -> (Self, bool) {
        match DEPTH.try_with(|depth| {
            let now = depth.get();
            depth.set(now.saturating_add(1));
            now
        }) {
            Ok(before) => (Self { counted: true }, before == 0),
            // The thread's TLS is being torn down. Log rather than lose the call: a duplicate
            // line is cheap, a missing one is the failure this module exists to prevent.
            Err(_) => (Self { counted: false }, true),
        }
    }
}

impl Drop for Nesting {
    fn drop(&mut self) {
        if self.counted {
            let _ = DEPTH.try_with(|depth| depth.set(depth.get().saturating_sub(1)));
        }
    }
}

/// A caption or message, quoted for one log line: `\`, `"` and control characters escaped so the
/// line stays one line and the quotes stay unambiguous, or `<null>` for a null pointer.
///
/// `scripts/ds2-run.py`'s `parse_messagebox_line` undoes exactly this; the two are pinned by tests
/// on both sides.
pub fn quote(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "<null>".to_owned();
    };
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 || control == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The whole log line for one call.
pub fn format_line(
    api: &str,
    caller: &str,
    caption: Option<&str>,
    text: Option<&str>,
    kind: u32,
) -> String {
    format!(
        "{LINE_PREFIX}caller={caller} api={api} caption={} text={} type=0x{kind:x}",
        quote(caption),
        quote(text)
    )
}

/// `name+0xoffset` for an address inside a loaded module, or the bare address when it is in none
/// -- which is itself worth knowing: code running out of an anonymous allocation.
pub fn format_caller(address: usize, module: Option<(&str, usize)>) -> String {
    match module {
        Some((name, base)) if address >= base => format!("{name}+0x{:x}", address - base),
        _ => format!("0x{address:x}"),
    }
}

/// The file name alone from a module path, so the line says `ds2sc.dll` and not the whole prefix.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Units up to the first zero, or up to [`MAX_UNITS`], whichever comes first; plus whether the
/// bound was what stopped it.
///
/// # Safety
///
/// `pointer` is non-null and readable up to its terminator or [`MAX_UNITS`] units -- the same
/// contract the message box API it was passed to already places on it.
unsafe fn units<C: Copy + PartialEq + Default>(pointer: *const C) -> (Vec<C>, bool) {
    let mut out = Vec::new();
    for index in 0..MAX_UNITS {
        // SAFETY: the caller's contract; the loop stops at the terminator or the bound.
        let unit = unsafe { pointer.add(index).read_unaligned() };
        if unit == C::default() {
            return (out, false);
        }
        out.push(unit);
    }
    (out, true)
}

fn truncated(mut text: String, cut: bool) -> String {
    if cut {
        text.push_str(" <truncated>");
    }
    text
}

/// # Safety
///
/// `pointer` is null, or a string the caller handed to a message box API.
unsafe fn narrow(pointer: *const u8) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: non-null, and the caller's contract covers the rest.
    let (bytes, cut) = unsafe { units(pointer) };
    Some(truncated(String::from_utf8_lossy(&bytes).into_owned(), cut))
}

/// # Safety
///
/// `pointer` is null, or a string the caller handed to a message box API.
unsafe fn wide(pointer: *const u16) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: non-null, and the caller's contract covers the rest.
    let (units, cut) = unsafe { units(pointer) };
    Some(truncated(String::from_utf16_lossy(&units), cut))
}

/// Who called the API whose detour called this, as `module+0xoffset`.
///
/// Frame 0 of the capture is inside this function, frame 1 is inside [`report`], and frame 2 is
/// inside the detour. The detour's own return address -- frame 3 -- is the caller's: the patched
/// prologue jumps to the detour, so the detour returns straight to whoever called the API.
/// `inline(never)` on this and on [`report`] keeps that count true.
#[inline(never)]
fn caller() -> String {
    let mut frame: *mut c_void = core::ptr::null_mut();
    // SAFETY: one frame into a live local; a zero return means nothing was captured.
    let captured = unsafe { RtlCaptureStackBackTrace(3, 1, &mut frame, core::ptr::null_mut()) };
    if captured == 0 || frame.is_null() {
        return "<unknown>".to_owned();
    }
    let address = frame as usize;
    let mut module: *mut c_void = core::ptr::null_mut();
    // SAFETY: `FROM_ADDRESS` takes any address and fails with zero when no module holds it, and
    // `UNCHANGED_REFCOUNT` leaves no handle to release. `module` is a live local.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            frame,
            &mut module,
        )
    } != 0;
    if !found || module.is_null() {
        return format_caller(address, None);
    }
    let mut buffer = [0u16; 520];
    // SAFETY: `module` is a loaded module's handle and `buffer` is a live local of the size passed.
    let written = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) };
    let path = String::from_utf16_lossy(&buffer[..(written as usize).min(buffer.len())]);
    let name = if path.is_empty() {
        "<module>"
    } else {
        file_name(&path)
    };
    format_caller(address, Some((name, module as usize)))
}

/// Log one call. Called only from a detour, and only for the outermost one on the thread.
#[inline(never)]
fn report(api: &str, caption: Option<String>, text: Option<String>, kind: u32) {
    let caller = caller();
    let line = format_line(api, &caller, caption.as_deref(), text.as_deref(), kind);
    crate::log_line(format_args!("{line}"));
}

fn original(index: usize) -> usize {
    ORIGINALS[index].load(Ordering::Acquire)
}

unsafe extern "system" fn message_box_a(
    owner: *mut c_void,
    text: *const u8,
    caption: *const u8,
    kind: u32,
) -> i32 {
    let (_nesting, outermost) = Nesting::enter();
    if outermost {
        // SAFETY: both pointers are the caller's own arguments to `MessageBoxA`.
        let (read_caption, read_text) = unsafe { (narrow(caption), narrow(text)) };
        report("MessageBoxA", read_caption, read_text, kind);
    }
    // SAFETY: the trampoline MinHook returned for `MessageBoxA`, whose signature this is; it is
    // stored before the hook is enabled, so it is never zero when this runs.
    let call: MessageBoxAFn = unsafe { core::mem::transmute(original(MESSAGE_BOX_A)) };
    // SAFETY: the caller's arguments, passed through unchanged.
    unsafe { call(owner, text, caption, kind) }
}

unsafe extern "system" fn message_box_w(
    owner: *mut c_void,
    text: *const u16,
    caption: *const u16,
    kind: u32,
) -> i32 {
    let (_nesting, outermost) = Nesting::enter();
    if outermost {
        // SAFETY: both pointers are the caller's own arguments to `MessageBoxW`.
        let (read_caption, read_text) = unsafe { (wide(caption), wide(text)) };
        report("MessageBoxW", read_caption, read_text, kind);
    }
    // SAFETY: as in `message_box_a`, for `MessageBoxW`.
    let call: MessageBoxWFn = unsafe { core::mem::transmute(original(MESSAGE_BOX_W)) };
    // SAFETY: the caller's arguments, passed through unchanged.
    unsafe { call(owner, text, caption, kind) }
}

unsafe extern "system" fn message_box_ex_a(
    owner: *mut c_void,
    text: *const u8,
    caption: *const u8,
    kind: u32,
    language: u16,
) -> i32 {
    let (_nesting, outermost) = Nesting::enter();
    if outermost {
        // SAFETY: both pointers are the caller's own arguments to `MessageBoxExA`.
        let (read_caption, read_text) = unsafe { (narrow(caption), narrow(text)) };
        report("MessageBoxExA", read_caption, read_text, kind);
    }
    // SAFETY: as in `message_box_a`, for `MessageBoxExA`.
    let call: MessageBoxExAFn = unsafe { core::mem::transmute(original(MESSAGE_BOX_EX_A)) };
    // SAFETY: the caller's arguments, passed through unchanged.
    unsafe { call(owner, text, caption, kind, language) }
}

unsafe extern "system" fn message_box_ex_w(
    owner: *mut c_void,
    text: *const u16,
    caption: *const u16,
    kind: u32,
    language: u16,
) -> i32 {
    let (_nesting, outermost) = Nesting::enter();
    if outermost {
        // SAFETY: both pointers are the caller's own arguments to `MessageBoxExW`.
        let (read_caption, read_text) = unsafe { (wide(caption), wide(text)) };
        report("MessageBoxExW", read_caption, read_text, kind);
    }
    // SAFETY: as in `message_box_a`, for `MessageBoxExW`.
    let call: MessageBoxExWFn = unsafe { core::mem::transmute(original(MESSAGE_BOX_EX_W)) };
    // SAFETY: the caller's arguments, passed through unchanged.
    unsafe { call(owner, text, caption, kind, language) }
}

unsafe extern "system" fn message_box_indirect_a(params: *const MsgBoxParams<u8>) -> i32 {
    let (_nesting, outermost) = Nesting::enter();
    if outermost {
        let (caption, text, style) = if params.is_null() {
            (None, None, 0)
        } else {
            // SAFETY: a non-null `MSGBOXPARAMSA` the caller passed, and the strings in it.
            unsafe {
                let params = params.read_unaligned();
                (narrow(params.caption), narrow(params.text), params.style)
            }
        };
        report("MessageBoxIndirectA", caption, text, style);
    }
    // SAFETY: as in `message_box_a`, for `MessageBoxIndirectA`.
    let call: MessageBoxIndirectAFn =
        unsafe { core::mem::transmute(original(MESSAGE_BOX_INDIRECT_A)) };
    // SAFETY: the caller's argument, passed through unchanged.
    unsafe { call(params) }
}

unsafe extern "system" fn message_box_indirect_w(params: *const MsgBoxParams<u16>) -> i32 {
    let (_nesting, outermost) = Nesting::enter();
    if outermost {
        let (caption, text, style) = if params.is_null() {
            (None, None, 0)
        } else {
            // SAFETY: a non-null `MSGBOXPARAMSW` the caller passed, and the strings in it.
            unsafe {
                let params = params.read_unaligned();
                (wide(params.caption), wide(params.text), params.style)
            }
        };
        report("MessageBoxIndirectW", caption, text, style);
    }
    // SAFETY: as in `message_box_a`, for `MessageBoxIndirectW`.
    let call: MessageBoxIndirectWFn =
        unsafe { core::mem::transmute(original(MESSAGE_BOX_INDIRECT_W)) };
    // SAFETY: the caller's argument, passed through unchanged.
    unsafe { call(params) }
}

/// The detour for each entry of [`APIS`], as an address.
fn detours() -> [*mut c_void; 6] {
    [
        message_box_a as MessageBoxAFn as *mut c_void,
        message_box_w as MessageBoxWFn as *mut c_void,
        message_box_ex_a as MessageBoxExAFn as *mut c_void,
        message_box_ex_w as MessageBoxExWFn as *mut c_void,
        message_box_indirect_a as MessageBoxIndirectAFn as *mut c_void,
        message_box_indirect_w as MessageBoxIndirectWFn as *mut c_void,
    ]
}

/// The install report: how many of the six are live, and what stopped each one that is not.
pub fn install_line(armed: usize, failures: &[String]) -> String {
    let mut line = format!("{INSTALL_PREFIX} installed={armed}/{}", APIS.len());
    if failures.is_empty() {
        line.push_str(" -- every message box in this process is logged before it opens");
    } else {
        line.push_str(&format!(" failed=[{}]", failures.join(", ")));
    }
    line
}

/// Hook all six and return the install report line. Enables each hook at once rather than
/// through the boot batch: this runs from `DllMain`, before the batch exists.
///
/// # Safety
///
/// Patches `user32` code. Call once, from the loader's `attach`.
pub unsafe fn install() -> String {
    let user32: Vec<u16> = "user32.dll".encode_utf16().chain(Some(0)).collect();
    // SAFETY: a null-terminated literal; the handle of an already-mapped module is borrowed, not
    // owned, and `user32` is a static import of this DLL, so it is mapped.
    let module = unsafe { GetModuleHandleW(user32.as_ptr()) };
    if module.is_null() {
        return install_line(0, &["user32.dll not mapped".to_owned()]);
    }
    // SAFETY: takes no arguments; a second call answers `MH_ERROR_ALREADY_INITIALIZED`, which is
    // as good as `MH_OK` here.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        return install_line(0, &[format!("MH_Initialize={status:?}")]);
    }

    let mut armed = 0;
    let mut failures = Vec::new();
    for (index, (name, detour)) in APIS.iter().zip(detours()).enumerate() {
        let label = name.trim_end_matches('\0');
        // SAFETY: `module` is `user32`'s base and `name` is null-terminated.
        let target = unsafe { GetProcAddress(module, name.as_ptr()) };
        if target.is_null() {
            failures.push(format!("{label}: not exported"));
            continue;
        }
        // SAFETY: `target` is the export and `detour` has its exact signature.
        let hook = match unsafe { MhHook::new(target, detour) } {
            Ok(hook) => hook,
            Err(status) => {
                failures.push(format!("{label}: MH_CreateHook={status:?}"));
                continue;
            }
        };
        // Stored before the enable: the detour reads it, and can run the instant the patch lands.
        ORIGINALS[index].store(hook.trampoline() as usize, Ordering::Release);
        // SAFETY: `target` was accepted by `MH_CreateHook` just above.
        match unsafe { MH_EnableHook(target) } {
            MH_STATUS::MH_OK => armed += 1,
            status => failures.push(format!("{label}: MH_EnableHook={status:?}")),
        }
    }
    install_line(armed, &failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seamless_line_is_one_line_with_its_newlines_escaped() {
        let line = format_line(
            "MessageBoxA",
            "ds2sc.dll+0x1234",
            Some("Error"),
            Some(
                "This version of Dark Souls II seamless co-op (0.0.1) is depreciated and requires \
                 an update.\nThe application will now exit.",
            ),
            0x10,
        );
        assert!(!line.contains('\n'), "{line}");
        assert_eq!(
            line,
            "ds2-loader: MESSAGEBOX caller=ds2sc.dll+0x1234 api=MessageBoxA caption=\"Error\" \
             text=\"This version of Dark Souls II seamless co-op (0.0.1) is depreciated and \
             requires an update.\\nThe application will now exit.\" type=0x10"
        );
    }

    #[test]
    fn quotes_backslashes_and_controls_are_escaped_and_null_is_not_empty() {
        assert_eq!(
            quote(Some("a\"b\\c\r\t\u{1}")),
            "\"a\\\"b\\\\c\\r\\t\\x01\""
        );
        assert_eq!(quote(Some("")), "\"\"");
        assert_eq!(quote(None), "<null>");
    }

    #[test]
    fn a_caller_in_a_module_is_named_by_module_and_offset() {
        assert_eq!(
            format_caller(0x1_8000_47c9, Some(("ds2sc.dll", 0x1_8000_0000))),
            "ds2sc.dll+0x47c9"
        );
        assert_eq!(format_caller(0x7fff_0000, None), "0x7fff0000");
        assert_eq!(
            format_caller(0x10, Some(("x.dll", 0x20))),
            "0x10",
            "an address below the base is not in that module"
        );
    }

    #[test]
    fn a_module_path_is_reduced_to_its_file_name() {
        assert_eq!(
            file_name("Z:\\games\\DS2\\Game\\SeamlessCoop\\ds2sc.dll"),
            "ds2sc.dll"
        );
        assert_eq!(file_name("ds2sc.dll"), "ds2sc.dll");
    }

    #[test]
    fn the_install_line_says_how_many_are_live_and_why_the_rest_are_not() {
        assert_eq!(
            install_line(6, &[]),
            "ds2-loader: messagebox-watch installed=6/6 -- every message box in this process is \
             logged before it opens"
        );
        assert_eq!(
            install_line(5, &["MessageBoxA: not exported".to_owned()]),
            "ds2-loader: messagebox-watch installed=5/6 failed=[MessageBoxA: not exported]"
        );
    }

    #[test]
    fn units_stop_at_the_terminator_or_the_bound() {
        let text = [b'h', b'i', 0, b'x'];
        // SAFETY: a live local array with a terminator inside it.
        let (read, cut) = unsafe { units(text.as_ptr()) };
        assert_eq!(read, b"hi");
        assert!(!cut);
        let long = vec![b'a'; MAX_UNITS + 4];
        // SAFETY: a live vector longer than the bound, so the bound stops the read first.
        let (read, cut) = unsafe { units(long.as_ptr()) };
        assert_eq!(read.len(), MAX_UNITS);
        assert!(cut);
    }

    #[test]
    fn the_params_struct_puts_text_caption_and_style_where_winuser_h_does() {
        assert_eq!(core::mem::offset_of!(MsgBoxParams<u16>, text), 24);
        assert_eq!(core::mem::offset_of!(MsgBoxParams<u16>, caption), 32);
        assert_eq!(core::mem::offset_of!(MsgBoxParams<u16>, style), 40);
        assert_eq!(core::mem::size_of::<MsgBoxParams<u16>>(), 80);
    }
}
