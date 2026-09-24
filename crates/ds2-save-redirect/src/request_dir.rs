//! Point the container directory at a folder, mid-session, by writing the field every request is
//! built from.
//!
//! # The field, and how it was found
//!
//! `FUN_140a8a6f0` builds every `SLLoadSession`. It reads the directory through the accessor
//! `FUN_140a8a180` -- `lea rax,[rcx+8]; ret` -- applied to the `SLLoadContent` at `[system+0x30]`,
//! and hands the result to the constructor. So `content+0x08` is the durable copy, and everything
//! else that holds a directory holds a seeding from it.
//!
//! Four seams were tried before this was read. Each is here because a reader deserves to know the
//! field was found rather than guessed at, and because three of them look right:
//!
//! | seam | why it missed |
//! |---|---|
//! | [`crate::session_dir`]: `SLLoadSession`'s directory virtual | the work method reaches the string through the accessor, never the virtual -- `load-answered=1` on a read that still failed |
//! | [`ds2_rva::SAVE_DIR_BUILD`] re-pointed mid-session | it runs at session setup, not per request -- `session-dir-answered=0` |
//! | `content+0x08`, read without diagnostics | the right field; the read returned `<unreadable>` and the bare word named none of the four reads it could have been |
//! | the storage worker's own string, via [`ds2_rva::SL_REQUEST_SET_DIRECTORY`] | a worker exists only for a live request, and at the title its id names one that finished -- `set-made-no-write` |
//!
//! # The worker-side detour is kept, as an observer
//!
//! [`install`] hooks [`ds2_rva::SL_WORKER_SET_DIRECTORY`], the only writer of the worker's copy,
//! and changes nothing. It is what makes the game's own session setup visible -- the baseline line
//! a later one is read against -- and it reports the byte at
//! [`ds2_rva::SL_WORKER_SET_SKIPPED_OFFSET`] that would make a write a no-op. A skipped set is
//! otherwise silent, and silence is how three of the four seams above were mistaken for working.

use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::install::log;

/// `fn(string, chars, len)` -- [`ds2_rva::SL_SESSION_STRING_SET`], the game's own wstring assign.
type StringSetFn = unsafe extern "system" fn(usize, *const u16, usize);

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
    /// `[system + 0x30]` is null: the save system has no load content to build requests from.
    NoLoadContent,
    /// The bytes at [`ds2_rva::SL_SESSION_STRING_SET`] are not the recorded prologue, so that
    /// address is some other function on this build.
    WrongPrologue,
}

/// The content's directory string, with the numbers the read was made of.
///
/// A bare `None` is what the previous attempt at this reported, and `<unreadable>` in a log does
/// not say which of the four reads failed. Every field here is one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentDirectory {
    /// `[system + 0x30]`, the `SLLoadContent` every request is built from.
    pub content: usize,
    /// The string's length in characters, from `content+0x18`, or `None` if that read faulted.
    pub length: Option<usize>,
    /// The string's capacity, from `content+0x20`, or `None` if that read faulted.
    pub capacity: Option<usize>,
    /// The characters, when all of it could be read.
    pub text: Option<String>,
}

impl core::fmt::Display for ContentDirectory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.text {
            Some(text) => write!(f, "{text}"),
            None => write!(
                f,
                "<unread content=0x{:016x} length={:?} capacity={:?}>",
                self.content, self.length, self.capacity
            ),
        }
    }
}

/// Read the directory every new session is seeded from, reporting the numbers either way.
///
/// # Why this field
///
/// `FUN_140a8a6f0`, which builds every `SLLoadSession`, reads it through the two-instruction
/// accessor `FUN_140a8a180` -- `lea rax,[rcx+8]; ret` -- applied to the `SLLoadContent` at
/// `[system+0x30]`, and hands the result to the session constructor. So this is the durable copy,
/// and the storage worker's own string is a per-request seeding from it that dies with the request.
///
/// # Safety
///
/// Game thread, with a `SaveLoadSystem` pointer the caller resolved this frame.
pub unsafe fn content_directory(system: usize) -> Option<ContentDirectory> {
    // SAFETY: one read inside a live `SaveLoadSystem`, through the fault-safe reader.
    let content = unsafe {
        ds2_game_base::mem::safe_read_usize(system + ds2_rva::SAVE_LOAD_SYSTEM_CONTENT_OFFSET)?
    };
    if content == 0 {
        return None;
    }
    let string = content + ds2_rva::SL_CONTENT_DIRECTORY_OFFSET;
    // SAFETY: the MSVC layout `SL_SESSION_STRING_SET` itself branches on.
    let (length, capacity) = unsafe {
        (
            ds2_game_base::mem::safe_read_usize(string + 0x10),
            ds2_game_base::mem::safe_read_usize(string + 0x18),
        )
    };
    let text = match (length, capacity) {
        (Some(length), Some(capacity)) if length <= 0x8000 => {
            // SAFETY: above the small-string maximum the first field is the pointer; at or below
            // it, the characters are the first field.
            let data = if capacity < 8 {
                Some(string)
            } else {
                unsafe { ds2_game_base::mem::safe_read_usize(string) }
            };
            data.and_then(|data| {
                let mut units = Vec::with_capacity(length);
                for index in 0..length {
                    // SAFETY: `length` characters from the string's own buffer.
                    units.push(
                        unsafe { ds2_game_base::mem::safe_read_u32(data + index * 2) }? as u16,
                    );
                }
                Some(String::from_utf16_lossy(&units))
            })
        }
        _ => None,
    };
    Some(ContentDirectory {
        content,
        length,
        capacity,
        text,
    })
}

/// Point the load content at `windows_path`, so every session built after this reads from there.
///
/// `system` is a live `SaveLoadSystem`. A trailing separator is added if the caller left one off,
/// because the read appends a filename and would otherwise look beside the folder rather than
/// inside it -- which is what [`ds2_rva::SAVE_DIR_BUILD`] does for the game's own calls.
///
/// Returns what the field reads back as, which is the only honest report: the two differ exactly
/// when this is broken.
///
/// # Why not the storage worker
///
/// [`ds2_rva::SL_REQUEST_SET_DIRECTORY`] writes the worker's own copy and is what the pump's `0x18`
/// arm calls, so it looked like the seam. A worker exists only for a live request, though, and it
/// is found by the id at `[[system+0x38]+8]` -- which at the title names a request that has already
/// finished. A run measured that as `set-made-no-write ... the request manager had no worker for
/// its id`, and `FUN_140a89940` agrees in the disassembly: the same failed lookup is what makes it
/// report session type `0x14`, done.
///
/// # Safety
///
/// Game thread, with a `SaveLoadSystem` pointer the caller resolved this frame, and only while no
/// container request is in flight -- a session already built has taken its own copy, and changing
/// this under one would describe a container it is not reading.
pub unsafe fn set(system: usize, windows_path: &str) -> Result<ContentDirectory, NotSet> {
    let Ok(address) = ds2_game_base::mem::game_rva(ds2_rva::SL_SESSION_STRING_SET) else {
        return Err(NotSet::NoModuleBase);
    };
    let expected = ds2_rva::SL_SESSION_STRING_SET_PROLOGUE;
    let mut prologue = [0u8; 5];
    // SAFETY: a resolved RVA inside the loaded game image; `read_bytes` faults safely.
    let read = unsafe { ds2_game_base::mem::read_bytes(address, &mut prologue) };
    if !read || prologue != expected {
        log(format_args!(
            "{LOG_PREFIX} content-directory REFUSED reason=prologue va=0x{address:016x} \
             read={read} saw={prologue:02x?} want={expected:02x?} -- that address is not the \
             string setter on this build"
        ));
        return Err(NotSet::WrongPrologue);
    }
    // SAFETY: one read inside a live `SaveLoadSystem`, through the fault-safe reader.
    let content = unsafe {
        ds2_game_base::mem::safe_read_usize(system + ds2_rva::SAVE_LOAD_SYSTEM_CONTENT_OFFSET)
    };
    let Some(content) = content.filter(|content| *content != 0) else {
        return Err(NotSet::NoLoadContent);
    };

    let mut wide: Vec<u16> = windows_path.encode_utf16().collect();
    if !matches!(wide.last(), Some(&unit) if unit == u16::from(b'\\')) {
        wide.push(u16::from(b'\\'));
    }

    // SAFETY: the prologue matches the function `ds2-rva` transcribed, the signature is the one its
    // callers use, the string object is the content's own, and `wide` outlives the call. On the
    // game thread, which is where the session builder calls it from.
    let assign: StringSetFn = unsafe { std::mem::transmute::<usize, StringSetFn>(address) };
    // SAFETY: as above.
    unsafe {
        assign(
            content + ds2_rva::SL_CONTENT_DIRECTORY_OFFSET,
            wide.as_ptr(),
            wide.len(),
        )
    };
    // SAFETY: same thread, same object, immediately after the game's own assign seated it.
    match unsafe { content_directory(system) } {
        Some(seated) => Ok(seated),
        None => Err(NotSet::NoLoadContent),
    }
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
