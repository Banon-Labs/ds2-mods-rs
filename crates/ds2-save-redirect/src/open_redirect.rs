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
//! # It diverts every open of that path, writes included
//!
//! [`arm`] takes the container the game will ask for and the one to answer with, and an open of
//! that path is answered whatever it asked for -- see [`GENERIC_WRITE`], which used to gate this on
//! read access and was measured doing the opposite of what it promised. So a character loaded out
//! of a donor file both reads and saves there, and the player's own container is the file nothing
//! touches for as long as the window is armed.
//!
//! Which makes this the only seam that knows where the saves are. No directory inside the game
//! moves, so `SAVE_DIR_BUILD`'s recorded answer goes on naming the player's own folder the whole
//! time. [`diverted_path`] is what a caller asks instead of believing that folder.
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

/// `GENERIC_WRITE`. Reported on every open so the log says which ones were writes.
///
/// It used to gate the diversion: an open carrying it was passed straight through, on the reasoning
/// that a swap has no business writing. That reasoning was backwards. The game saves on its own the
/// moment a character enters the world, and an undiverted write goes to the path the game asked for
/// -- the player's own container -- so the donor character would have been written over the very
/// save this flow tells them it left untouched.
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
struct Window {
    /// The container the game will ask for.
    asked: PathBuf,
    /// The container it is answered with.
    answer: PathBuf,
    /// `answer`, NUL-terminated, ready to hand straight to the original API.
    answer_wide: Vec<u16>,
}

static WINDOW: Mutex<Option<Window>> = Mutex::new(None);

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
    // THE LOCK IS RELEASED BEFORE THE LINE IS WRITTEN, and this is not tidiness. Writing a log line
    // opens a file, which enters the detour, which asks this same lock whether the path is the
    // armed one -- and `std::sync::Mutex` is not reentrant, so holding it across the log wedges the
    // game thread against itself. Measured 2026-09-23: the flow logged `swap at the title` and the
    // process sat there with eighty-five live threads and no further line.
    {
        let Ok(mut window) = WINDOW.lock() else {
            return false;
        };
        *window = Some(Window {
            asked: asked.to_path_buf(),
            answer: answer.to_path_buf(),
            answer_wide: wide,
        });
        DIVERTED.store(0, Ordering::Relaxed);
    }
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

/// Run `body` with this thread's opens exempt from the window.
///
/// # Why anything would want that
///
/// The window is armed on the path the game's own directory builder produced, and with
/// `[save_redirect] directory` pointing at a folder the player also exports into, that is a path
/// this DLL itself writes. `Save Game to File` copies the live container to a destination the
/// player named -- and if the destination open is diverted, the copy's source and target become one
/// file and `std::fs::copy` truncates it. Measured 2026-09-24: `exported bytes=0`, with the staged
/// save left empty behind it.
///
/// The refusal that guards against copying a container onto itself cannot see this. It compares the
/// two paths the caller holds, and they differ; what makes them the same file is a detour one layer
/// below.
///
/// # Thread-local, and the same counter the logger uses
///
/// [`DEPTH`] already exists so the detour's own log writes reach the original API instead of
/// re-entering. This is that mechanism, made available to a caller that knows its own opens are not
/// the game asking for its container. It covers this thread only, so a concurrent read on the game
/// thread is still diverted.
pub fn bypass<T>(body: impl FnOnce() -> T) -> T {
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let produced = body();
    DEPTH.with(|depth| depth.set(depth.get() - 1));
    produced
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

/// Whether this open is the armed one: the same path, whatever it is being opened for.
///
/// Reads and writes alike, because both belong to the staged copy -- see [`GENERIC_WRITE`].
///
/// The comparison is case-insensitive because Windows paths are, and the game's own spelling of its
/// container is not guaranteed to match the one a directory builder produced character for
/// character.
/// It never blocks -- `try_lock`, not `lock`.
///
/// This runs on every file open in the process, including the ones made by whoever is holding the
/// lock. A contended lock means somebody is arming or disarming right now, and the honest answer to
/// that is to let the open through rather than to stop the game until they are done: a missed
/// diversion is a failed swap, a blocked one is a game that never comes back.
fn answer_for(path: &str) -> Option<Vec<u16>> {
    let window = WINDOW.try_lock().ok()?;
    let window = window.as_ref()?;
    window
        .asked
        .as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(path)
        .then(|| window.answer_wide.clone())
}

/// What an open of `asked` would be handed instead, or `None` when no armed window is about it.
///
/// **This is how a caller finds out where the saves are going.** A swap arms this window and
/// nothing else -- no directory field inside the game moves, and the save-session override is
/// deliberately left unarmed -- so anything that asks the game which folder it built is told the
/// player's own container while every write lands in the staged one.
///
/// The same case-insensitive full-path comparison [`answer_for`] makes, because the answer has to
/// be the one the detour will actually give. `lock` rather than `try_lock`: this runs on the game
/// thread outside the detour, where a missed answer is the wrong file rather than a slow one.
pub fn diverted_path(asked: &Path) -> Option<PathBuf> {
    let window = WINDOW.lock().ok()?;
    let window = window.as_ref()?;
    window
        .asked
        .as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&asked.as_os_str().to_string_lossy())
        .then(|| window.answer.clone())
}

/// Which of the two container paths this open names, if either.
///
/// Reported for every open of either one while a window is armed, whatever its access. `Failed to
/// save game.` on 2026-09-23 came with the save session having been answered the staged directory
/// and the staged file unmodified, and nothing in the log said which open failed or with what
/// flags. This is that line.
fn container_role(path: &str) -> Option<&'static str> {
    // Any save-shaped path, wherever it lives. Measured 2026-09-23: on the run that showed
    // `Failed to save game.` there were 105 opens of the player's own container and every one was
    // a read that succeeded -- no write-open, no failure. So the writer is not reaching this API
    // by the container's own name, and narrowing the report to the two known paths is what hid
    // that. A temporary file, a `.bak`, or a different directory all show up here now.
    let lowered = path.to_ascii_lowercase();
    if lowered.contains("ds2sofs") || lowered.ends_with(".sl2") || lowered.ends_with(".sl2.bak") {
        let window = WINDOW.try_lock().ok()?;
        let Some(window) = window.as_ref() else {
            return Some("save-shaped");
        };
        if window
            .asked
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(path)
        {
            return Some("own");
        }
        let staged = window.answer.as_os_str().to_string_lossy();
        return Some(if staged.eq_ignore_ascii_case(path) {
            "staged"
        } else {
            "save-shaped"
        });
    }
    None
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
    let asked_path = unsafe { wide_to_string(file_name) };
    let role = asked_path.as_deref().and_then(container_role);
    let answer = asked_path.as_deref().and_then(answer_for);
    let diverted = answer.is_some();
    let handle = match &answer {
        Some(answer) => {
            DIVERTED.fetch_add(1, Ordering::Relaxed);
            pass_through(answer.as_ptr())
        }
        None => pass_through(file_name),
    };
    // Every open of either container is reported, whatever its access and whether or not it was
    // diverted. The flags and the handle are the evidence: a `-1` here is the open that failed, and
    // its access and share bits say why without anyone having to reason about which one it was.
    if let Some(role) = role {
        // The path is printed for anything that is not one of the two known containers, because on
        // a `save-shaped` line the path IS the finding.
        let named = if role == "save-shaped" {
            asked_path.clone().unwrap_or_default()
        } else {
            String::new()
        };
        log(format_args!(
            "{LOG_PREFIX} open-redirect container={role} diverted={diverted} write={} \
             access=0x{access:08x} share=0x{share:08x} disposition={disposition} \
             handle={handle} count={} {named}",
            access & GENERIC_WRITE != 0,
            DIVERTED.load(Ordering::Relaxed)
        ));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// [`diverted_path`] answers what the detour would answer, and only for the path it is armed
    /// for.
    ///
    /// One test rather than four because [`WINDOW`] is process-wide and `cargo test` runs cases in
    /// parallel: split up, they would arm and disarm each other's window.
    #[test]
    fn the_window_reports_the_container_it_will_hand_over() {
        let own = Path::new(r"C:\users\steamuser\AppData\Roaming\DarkSoulsII\aaa\DS2SOFS0000.sl2");
        let staged = Path::new(r"S:\Game\ds2-swapped-save\DS2SOFS0000.sl2");

        assert_eq!(
            diverted_path(own),
            None,
            "nothing is armed in a fresh process"
        );

        assert!(arm(own, staged));
        assert_eq!(diverted_path(own).as_deref(), Some(staged));
        // Windows paths are case-insensitive, and the game's spelling of its own container is not
        // guaranteed to match the directory builder's character for character. The detour compares
        // this way, so this has to as well or the two would disagree about the same open.
        let shouted = PathBuf::from(own.as_os_str().to_string_lossy().to_uppercase());
        assert_eq!(diverted_path(&shouted).as_deref(), Some(staged));
        assert_eq!(
            diverted_path(staged),
            None,
            "the answer is not itself diverted, or a caller would follow it in a circle"
        );

        disarm();
        assert_eq!(diverted_path(own), None);
    }
}
