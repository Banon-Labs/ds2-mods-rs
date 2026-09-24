//! Hand the game a different file when it opens its own save container, for as long as a window is
//! armed.
//!
//! # Why this and not a field inside the game
//!
//! Four attempts pointed a `SaveLoadSystem` field at another folder and all four were disproved by
//! a live run -- the table in [`crate::request_dir`] has them. The last of those found the shape of
//! the problem: the directory a container read opens belongs to a storage worker that exists only
//! for a live request, so at the title, which is the one moment a swap can act, there is nothing to
//! write.
//!
//! This is the seam `er-mods-rs` uses for the same job, and it does not ask the game anything. The
//! game keeps its own directory, its own file name and its own session machinery; one `CreateFileW`
//! answers a different path. `er-quit-menu-core::save_dest_open_redirect` is the module this
//! mirrors, down to the re-entry guard and the full-path match.
//!
//! # It diverts reads, and that is what keeps a save safe
//!
//! [`arm`] takes the container the game will ask for and the one to answer with, and only opens
//! that ask for read access are diverted. A write-open of the same path passes through untouched,
//! so the player's own container stays the thing their progress is written to and this cannot
//! damage it. That also means a character loaded out of a donor file saves into the player's own
//! container -- a separate decision, and one that belongs to whoever arms the save side, not here.
//!
//! # The window is narrow on purpose
//!
//! Armed at the title before a container re-read, disarmed the moment it finishes. Outside that
//! window every path in the process reaches the original API with no comparison made at all.

use core::ffi::c_void;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// Win32 `CreateFileW`, as the detour and the trampoline both see it.
type CreateFileWFn =
    unsafe extern "system" fn(*const u16, u32, u32, isize, u32, u32, isize) -> isize;

/// `INVALID_HANDLE_VALUE`, which is what a failed open returns.
const INVALID_HANDLE: isize = -1;

/// `GENERIC_WRITE`. An open carrying it is the game writing, and is never diverted.
const GENERIC_WRITE: u32 = 0x4000_0000;

/// Win32's own cap on a path, even in its extended form.
const MAX_PATH_UNITS: usize = 0x8000;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(name: *const std::ffi::c_char) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
}

/// MinHook's trampoline back to the real `CreateFileW`.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// The armed window: the path the game will ask for, and the one to answer with.
static WINDOW: Mutex<Option<(PathBuf, Vec<u16>)>> = Mutex::new(None);

/// Reads diverted since the window was armed.
static DIVERTED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Detour depth on this thread. The body writes log lines, and logging opens a file, which
    /// re-enters here. A nested entry is our own open of a path already decided on, so it wants the
    /// original API and none of the decision.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Point opens of `asked` at `answer`, until [`disarm`].
///
/// Both are Windows paths as the game spells them. Returns whether the window was armed, which is
/// `false` only when the lock is poisoned.
pub fn arm(asked: &Path, answer: &Path) -> bool {
    let mut wide: Vec<u16> = answer
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .collect();
    wide.push(0);
    let Ok(mut window) = WINDOW.lock() else {
        return false;
    };
    *window = Some((asked.to_path_buf(), wide));
    DIVERTED.store(0, Ordering::Relaxed);
    log(format_args!(
        "{LOG_PREFIX} open-redirect armed asked={} answer={}",
        asked.display(),
        answer.display()
    ));
    true
}

/// Stop diverting. Idempotent; returns how many reads were diverted while armed.
pub fn disarm() -> usize {
    if let Ok(mut window) = WINDOW.lock() {
        *window = None;
    }
    let diverted = DIVERTED.load(Ordering::Relaxed);
    log(format_args!(
        "{LOG_PREFIX} open-redirect disarmed diverted={diverted}"
    ));
    diverted
}

/// How many reads have been diverted since the window was armed.
///
/// **This is the number that says whether the swap read the file the player picked.** Zero with a
/// window armed means the game never opened the container, so nothing about that file explains what
/// happened next.
pub fn diverted() -> usize {
    DIVERTED.load(Ordering::Relaxed)
}

/// Whether a window is armed.
pub fn armed() -> bool {
    WINDOW
        .lock()
        .map(|window| window.is_some())
        .unwrap_or(false)
}

/// Read a NUL-terminated wide string, bounded so a non-terminated buffer cannot walk the heap.
///
/// # Safety
///
/// `text` must be null, or point at a NUL-terminated UTF-16 string.
unsafe fn wide_to_string(text: *const u16) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let mut units = Vec::new();
    for index in 0..MAX_PATH_UNITS {
        // SAFETY: the caller promises a terminated string; the bound covers one that is not.
        let unit = unsafe { *text.add(index) };
        if unit == 0 {
            return Some(String::from_utf16_lossy(&units));
        }
        units.push(unit);
    }
    None
}

/// Whether this open is the armed one: the same path, asked for reading.
///
/// The comparison is case-insensitive because Windows paths are, and the game's own spelling of its
/// container is not guaranteed to match the one a directory builder produced character for
/// character.
fn answer_for(path: &str, access: u32) -> Option<Vec<u16>> {
    if access & GENERIC_WRITE != 0 {
        return None;
    }
    let window = WINDOW.lock().ok()?;
    let (asked, answer) = window.as_ref()?;
    let asked = asked.as_os_str().to_string_lossy();
    asked.eq_ignore_ascii_case(path).then(|| answer.clone())
}

/// The detour.
///
/// # Safety
///
/// Called by the process with the Microsoft x64 ABI, as `CreateFileW`.
unsafe extern "system" fn detour_create_file_w(
    file_name: *const u16,
    access: u32,
    share: u32,
    security: isize,
    disposition: u32,
    flags: u32,
    template: isize,
) -> isize {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return INVALID_HANDLE;
    }
    // SAFETY: MinHook published this trampoline for `CreateFileW`, whose signature this is.
    let original: CreateFileWFn =
        unsafe { std::mem::transmute::<usize, CreateFileWFn>(trampoline) };
    let pass_through = |name: *const u16| unsafe {
        original(name, access, share, security, disposition, flags, template)
    };

    let nested = DEPTH.with(|depth| {
        let entered = depth.get();
        depth.set(entered + 1);
        entered != 0
    });
    if nested {
        DEPTH.with(|depth| depth.set(depth.get() - 1));
        return pass_through(file_name);
    }

    // SAFETY: Win32 hands this detour a NUL-terminated path or null.
    let answer = unsafe { wide_to_string(file_name) }
        .as_deref()
        .and_then(|path| answer_for(path, access));
    let handle = match answer {
        Some(answer) => {
            let count = DIVERTED.fetch_add(1, Ordering::Relaxed) + 1;
            log(format_args!(
                "{LOG_PREFIX} open-redirect diverted count={count} -- the game asked for its own \
                 container and was handed the staged one"
            ));
            pass_through(answer.as_ptr())
        }
        None => pass_through(file_name),
    };
    DEPTH.with(|depth| depth.set(depth.get() - 1));
    handle
}

/// Install the detour, in pass-through mode until [`arm`] is called.
///
/// Returns whether it is live. A `false` costs the in-session swap and nothing else.
///
/// # Safety
///
/// One MinHook detour on `kernel32!CreateFileW`, installed after Arxan has been neutered and from
/// the loader's own thread rather than from `DllMain`.
pub unsafe fn install() -> bool {
    // SAFETY: two `kernel32` lookups with NUL-terminated names; both return null rather than fault.
    let address = unsafe {
        let module = GetModuleHandleA(c"kernel32.dll".as_ptr());
        if module.is_null() {
            0
        } else {
            GetProcAddress(module, c"CreateFileW".as_ptr()) as usize
        }
    };
    if address == 0 {
        log(format_args!(
            "{LOG_PREFIX} open-redirect NOT INSTALLED reason=no-createfilew -- the in-session swap \
             cannot hand the game another container"
        ));
        return false;
    }
    // SAFETY: MinHook's own initialiser, idempotent across the crates that call it.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} open-redirect NOT INSTALLED stage=MH_Initialize status={status:?}"
        ));
        return false;
    }
    // SAFETY: a resolved export in a loaded system module.
    let hook =
        match unsafe { MhHook::new(address as *mut c_void, detour_create_file_w as *mut c_void) } {
            Ok(hook) => hook,
            Err(status) => {
                log(format_args!(
                    "{LOG_PREFIX} open-redirect NOT INSTALLED va=0x{address:016x} \
                 stage=MH_CreateHook status={status:?}"
                ));
                return false;
            }
        };
    // Published BEFORE the site is patched: every path in the process reaches the original through
    // this, and a detour that read a zero here would fail every file open in the game.
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: the hook was created for this address by the call above.
    let status = unsafe { MH_EnableHook(address as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} open-redirect NOT INSTALLED va=0x{address:016x} stage=MH_EnableHook \
             status={status:?}"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} open-redirect installed va=0x{address:016x} -- pass-through until a swap arms \
         a window"
    ));
    true
}

/// Whether [`install`] put the detour in.
///
/// Read by the in-session swap before it takes the player out of their game: without this there is
/// no way to answer a container open, so the flow has to refuse rather than strand them at the
/// title. `false` on a build where the export could not be resolved or MinHook refused the site.
pub fn installed() -> bool {
    TRAMPOLINE.load(Ordering::Acquire) != 0
}
