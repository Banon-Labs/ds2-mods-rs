//! Point the **storage worker's** container directory at a folder, mid-session, by calling the
//! game's own setter.
//!
//! # This is the field a container read opens
//!
//! Three other seams were tried first and all three moved something the read does not consult:
//!
//! | seam | what it moves | how it failed |
//! |---|---|---|
//! | [`crate::session_dir`] | `SLLoadSession`'s directory virtual | the work method reaches the same field through the accessor, never the virtual -- `load-answered=1` on a read that still failed |
//! | `SAVE_DIR_BUILD` armed mid-session | the string session setup builds | session setup does not re-run for a re-read -- `session-dir-answered=0` |
//! | `SLLoadContent`'s own string | `[[system+0x30]]+0x08` | measured `<unreadable>`, and it is not the string the worker holds |
//!
//! What the game actually does is one line of the session pump's `0x18` arm ([`ds2_rva`] records
//! the transcription): `SAVE_DIR_BUILD` produces a directory, and
//! [`ds2_rva::SL_REQUEST_SET_DIRECTORY`] seats it on the worker that opens files. That is the
//! entire consumer list of `SAVE_DIR_BUILD`'s result, which closes the chain: the launch-time
//! redirect on `SAVE_DIR_BUILD` was measured reading a donor container end to end, so the string it
//! produced reached the file through here and nowhere else.
//!
//! # Why the set is called whole, and why the read-back is a hook
//!
//! [`ds2_rva::SL_REQUEST_SET_DIRECTORY`] is a lock/unlock pair around one mutation: it takes the
//! manager's lock and the worker's, writes, and releases both. The worker pointer exists only
//! between those two calls, so there is no way to read the directory back afterwards -- reaching
//! for the finder alone would leave the save system locked against its own next request.
//!
//! So the read-back is a detour on the worker-side writer instead ([`install`]), which is the only
//! writer of that field. It changes nothing; it records what was written, and it records the byte
//! at [`ds2_rva::SL_WORKER_SET_SKIPPED_OFFSET`] that makes the whole write a no-op. **A skipped set
//! is otherwise silent**, and silence is exactly how the last three attempts at this were mistaken
//! for working.

use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// `void SetRequestDirectory(holder, u32 index, const wchar_t *path)`.
type SetRequestDirectoryFn = unsafe extern "system" fn(*mut c_void, u32, *const u16);

/// `void SetWorkerDirectory(worker, u32 index, const wchar_t *path)` -- the detour's shape.
type SetWorkerDirectoryFn = unsafe extern "system" fn(*mut c_void, u32, *const u16);

/// MinHook's trampoline back to the original worker-side writer.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// What the last worker-side write left behind, recorded by the detour.
static LAST: Mutex<Option<Seated>> = Mutex::new(None);

/// Times the worker-side writer has run since the process started.
static WRITES: AtomicUsize = AtomicUsize::new(0);

/// One observed directory write, as the worker holds it afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seated {
    /// The worker the directory was written to.
    pub worker: usize,
    /// The index the write carried, which lands at `worker+0x3c`.
    pub index: u32,
    /// Whether [`ds2_rva::SL_WORKER_SET_SKIPPED_OFFSET`] was set, making the write do nothing.
    pub skipped: bool,
    /// The directory read back out of the worker after the original returned.
    pub directory: String,
}

/// The directory the storage worker was last given, whoever gave it.
///
/// `None` before the game has set one, which on a booted session it always has: session setup
/// writes it during the `0x18` arm of the pump.
pub fn last() -> Option<Seated> {
    LAST.lock().ok()?.clone()
}

/// How many times the worker-side writer has run, so a caller can tell a set that happened from a
/// set that never reached the game.
pub fn writes() -> usize {
    WRITES.load(Ordering::Relaxed)
}

/// Read a live MSVC `basic_string<wchar_t>` out of the worker, through the fault-safe reader.
///
/// The layout is the one [`ds2_rva::SL_SESSION_STRING_SET`] itself branches on: length at `+0x10`,
/// capacity at `+0x18`, characters inline until the capacity exceeds seven.
fn read_directory(string: usize) -> Option<String> {
    // SAFETY: a field inside a live worker, every read through `safe_read_*`, which reports a bad
    // address rather than faulting on it.
    unsafe {
        let length = ds2_game_base::mem::safe_read_usize(string + 0x10)?;
        let capacity = ds2_game_base::mem::safe_read_usize(string + 0x18)?;
        // A path longer than a Windows path can be says this is not the object it should be, and
        // reading it as one would be a long walk through somebody else's memory.
        if length > 0x8000 {
            return None;
        }
        let data = if capacity < 8 {
            string
        } else {
            ds2_game_base::mem::safe_read_usize(string)?
        };
        let mut units = Vec::with_capacity(length);
        for index in 0..length {
            units.push(ds2_game_base::mem::safe_read_u32(data + index * 2)? as u16);
        }
        Some(String::from_utf16_lossy(&units))
    }
}

/// The observer. Calls the original, then reads out what it left.
///
/// # Safety
///
/// Called by the game with the Microsoft x64 ABI, on its own thread: `worker` is a live storage
/// worker, `path` a null-terminated wide string.
unsafe extern "system" fn detour_set_worker_directory(
    worker: *mut c_void,
    index: u32,
    path: *const u16,
) {
    let trampoline = TRAMPOLINE.load(Ordering::Acquire);
    // The skip byte is read BEFORE the original runs, because the original is what consults it and
    // a write that lands may well clear it.
    let mut flag = [0u8; 1];
    // SAFETY: one byte inside the live worker, through the fault-safe reader.
    let read = unsafe {
        ds2_game_base::mem::read_bytes(
            worker as usize + ds2_rva::SL_WORKER_SET_SKIPPED_OFFSET,
            &mut flag,
        )
    };
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site, and the signature is the one the
        // call site at `0x140a89a24` uses.
        let original: SetWorkerDirectoryFn =
            unsafe { std::mem::transmute::<usize, SetWorkerDirectoryFn>(trampoline) };
        unsafe { original(worker, index, path) };
    }
    let seated = Seated {
        worker: worker as usize,
        index,
        skipped: read && flag[0] != 0,
        directory: read_directory(worker as usize + ds2_rva::SL_WORKER_DIRECTORY_OFFSET)
            .unwrap_or_else(|| String::from("<unreadable>")),
    };
    let count = WRITES.fetch_add(1, Ordering::Relaxed) + 1;
    log(format_args!(
        "{LOG_PREFIX} worker-directory count={count} worker=0x{:016x} index={index} \
         skipped={} path={}",
        seated.worker, seated.skipped, seated.directory
    ));
    if let Ok(mut held) = LAST.lock() {
        *held = Some(seated);
    }
}

/// Why a directory set did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSet {
    /// The module base could not be resolved, so no address in the image can be.
    NoModuleBase,
    /// `[system + 0x38]` is null: the save system has no request holder yet.
    NoRequestHolder,
    /// The bytes at [`ds2_rva::SL_REQUEST_SET_DIRECTORY`] are not the recorded prologue, so that
    /// address is some other function on this build.
    WrongPrologue,
}

/// Point the storage worker at `windows_path`, through the game's own setter.
///
/// `system` is a live `SaveLoadSystem`. A trailing separator is added if the caller left one off,
/// because the read appends a filename and would otherwise look beside the folder rather than
/// inside it -- which is what [`ds2_rva::SAVE_DIR_BUILD`] does for the game's own calls.
///
/// Returns what the worker reads back as afterwards, observed by [`install`]'s detour rather than
/// assumed. `Ok(None)` means the set was made and the observer was not installed to see it, which
/// is a weaker answer than a path but a stronger one than a bare `true`.
///
/// # Safety
///
/// Game thread, with a `SaveLoadSystem` pointer the caller resolved this frame, and only while no
/// container request is in flight -- the setter takes the worker's own lock, and calling it under a
/// running request would block the game thread on its own worker.
pub unsafe fn set(system: usize, windows_path: &str) -> Result<Option<Seated>, NotSet> {
    let Ok(address) = ds2_game_base::mem::game_rva(ds2_rva::SL_REQUEST_SET_DIRECTORY) else {
        return Err(NotSet::NoModuleBase);
    };
    let expected = ds2_rva::SL_REQUEST_SET_DIRECTORY_PROLOGUE;
    let mut prologue = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(address, &mut prologue) };
    if !read || prologue != expected {
        log(format_args!(
            "{LOG_PREFIX} worker-directory REFUSED reason=prologue va=0x{address:016x} \
             read={read} saw={prologue:02x?} want={expected:02x?} -- that address is not the \
             request directory setter on this build"
        ));
        return Err(NotSet::WrongPrologue);
    }
    // The VALUE of the field, not its address: the setter reads the manager from `[holder]` and the
    // worker id from `[holder+8]`.
    // SAFETY: one read inside a live `SaveLoadSystem`, through the fault-safe reader.
    let holder =
        unsafe { ds2_game_base::mem::safe_read_usize(system + ds2_rva::SL_REQUEST_HOLDER_OFFSET) };
    let Some(holder) = holder.filter(|holder| *holder != 0) else {
        return Err(NotSet::NoRequestHolder);
    };

    let mut wide: Vec<u16> = windows_path.encode_utf16().collect();
    if !matches!(wide.last(), Some(&unit) if unit == u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }
    // The terminator is not optional: the setter measures the string by scanning for it.
    wide.push(0);

    let before = writes();
    // SAFETY: the prologue matches the function `ds2-rva` transcribed, the signature is the one the
    // pump's `0x18` arm calls it with, `holder` came from the field that arm passes, and `wide`
    // outlives the call. Called on the game thread, which is where the pump calls it from.
    let assign: SetRequestDirectoryFn =
        unsafe { std::mem::transmute::<usize, SetRequestDirectoryFn>(address) };
    // SAFETY: as above.
    unsafe {
        assign(
            holder as *mut c_void,
            ds2_rva::SL_REQUEST_DIRECTORY_INDEX_SESSION,
            wide.as_ptr(),
        )
    };
    if writes() == before {
        // The setter ran and the worker-side writer did not, which means the finder returned no
        // worker for this holder's id. Reported rather than swallowed: it is the one failure that
        // leaves the directory untouched while every call above succeeded.
        log(format_args!(
            "{LOG_PREFIX} worker-directory set-made-no-write holder=0x{holder:016x} \
             asked={windows_path} -- the request manager had no worker for its id"
        ));
        return Ok(None);
    }
    Ok(last())
}

/// Install the observer on the worker-side directory writer.
///
/// Returns whether the detour is live. A `false` costs nothing but the read-back: [`set`] still
/// calls the game's setter, and the game still sets its own directories.
///
/// # Safety
///
/// One MinHook detour on [`ds2_rva::SL_WORKER_SET_DIRECTORY`], installed after Arxan has been
/// neutered and from the loader's thread rather than from `DllMain`.
pub unsafe fn install() -> bool {
    let Ok(address) = ds2_game_base::mem::game_rva(ds2_rva::SL_WORKER_SET_DIRECTORY) else {
        log(format_args!(
            "{LOG_PREFIX} worker-directory observer NOT INSTALLED reason=no-module-base"
        ));
        return false;
    };
    let expected = ds2_rva::SL_WORKER_SET_DIRECTORY_PROLOGUE;
    let mut prologue = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(address, &mut prologue) };
    if !read || prologue != expected {
        log(format_args!(
            "{LOG_PREFIX} worker-directory observer NOT INSTALLED reason=prologue \
             va=0x{address:016x} read={read} saw={prologue:02x?} want={expected:02x?}"
        ));
        return false;
    }
    // MinHook is statically linked into this DLL, so ALREADY_INITIALIZED can only mean this ran
    // twice. Treat it as success, exactly as the other feature crates do.
    // SAFETY: MinHook's own initialiser, idempotent across the crates that call it.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} worker-directory observer NOT INSTALLED stage=MH_Initialize \
             status={status:?}"
        ));
        return false;
    }
    // SAFETY: a function start in the loaded image, checked against its recorded prologue above.
    let hook = match unsafe {
        MhHook::new(
            address as *mut c_void,
            detour_set_worker_directory as *mut c_void,
        )
    } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} worker-directory observer NOT INSTALLED va=0x{address:016x} \
                 stage=MH_CreateHook status={status:?}"
            ));
            return false;
        }
    };
    // Published BEFORE the site is patched: the detour calls straight back through it, and a
    // detour that read a zero here would silently drop every directory the game sets on itself.
    TRAMPOLINE.store(hook.trampoline() as usize, Ordering::Release);
    // SAFETY: the hook was created for this address by the call above.
    let status = unsafe { MH_EnableHook(address as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} worker-directory observer NOT INSTALLED va=0x{address:016x} \
             stage=MH_EnableHook status={status:?}"
        ));
        return false;
    }
    log(format_args!(
        "{LOG_PREFIX} worker-directory observer installed rva=0x{:08x} va=0x{address:016x}",
        ds2_rva::SL_WORKER_SET_DIRECTORY
    ));
    true
}
