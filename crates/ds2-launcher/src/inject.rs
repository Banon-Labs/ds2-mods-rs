//! Starting the game suspended and putting DLLs in it before it runs an instruction.
//!
//! This is the half of the crate that cannot run on this workspace's machine, so the decisions
//! live in [`crate::plan`] and what is left here is the sequence itself, kept as close to the
//! shipped launcher's as the measurement allows.
//!
//! # The sequence, and where it was read from
//!
//! `ds2-seamless` records it, disassembled from `ds2sc_launcher.exe`: `CreateProcessA` with
//! `dwCreationFlags = 4` (`CREATE_SUSPENDED`; `movl $0x4,0x28(%rsp)` at `0x1400013b5`,
//! immediately before the call at `0x1400013d4`), then `VirtualAllocEx` / `WriteProcessMemory` /
//! `CreateRemoteThread` on `LoadLibraryA`, then `ResumeThread`.
//!
//! Two deliberate differences from that:
//!
//! - `CreateProcessW` and `LoadLibraryW` rather than the `A` forms. The path a player configures
//!   can contain anything their filesystem allows, and the `A` forms would mangle it through the
//!   process code page. Nothing about what the loaded DLL then sees is different.
//! - Every injected thread is waited on and its exit code read. `LoadLibraryW` returns the new
//!   module handle, which the thread's exit code carries, so a zero exit code is the loader
//!   saying the DLL did not go in. Skipping that check is how the in-process attempt this crate
//!   replaces looked like it worked: the call returned, a module was mapped, and the mod was
//!   inert.
//!
//! # Why the game's own `dinput8.dll` is not in the list
//!
//! It is a static import of `DarkSoulsII.exe`, so the loader maps it without being asked. The
//! ordering that falls out is worth stating, because it is the reverse of what "inject first"
//! suggests: a suspended process's initial thread is parked in `LdrInitializeThunk`, and the
//! first injected thread runs process initialisation -- static imports and their `DllMain`s --
//! before its own start routine. So this workspace's loader initialises first, and the injected
//! DLLs follow in list order.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, CreateRemoteThread, GetExitCodeThread, PROCESS_INFORMATION,
    ResumeThread, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::plan::{Injection, Plan};

/// The signature `CreateRemoteThread` starts a thread on.
///
/// `LPTHREAD_START_ROUTINE` is this wrapped in an `Option`; naming the inner type gives the
/// transmute in [`load_library_address`] something explicit to land on.
type ThreadStartRoutine = unsafe extern "system" fn(*mut core::ffi::c_void) -> u32;

/// How long one `LoadLibraryW` may take before the launch is abandoned.
///
/// Generous on purpose. A packed mod DLL is being paged in off whatever disk the player's game
/// lives on, through a Proton prefix, and a timeout that fired on a slow one would look exactly
/// like the mod refusing to load.
const LOAD_TIMEOUT_MS: u32 = 120_000;

/// Everything that can go wrong once a process exists.
///
/// Each one ends the launch: see [`crate::plan::Refusal`] for why a partially modded session is
/// not an outcome this launcher produces.
#[derive(Debug)]
pub enum InjectError {
    /// `CreateProcessW` refused.
    Spawn(String),
    /// `kernel32!LoadLibraryW` could not be located in this process.
    NoLoadLibrary,
    /// A remote allocation, write or thread failed.
    Remote {
        /// The DLL being injected when it happened.
        dll: String,
        /// Which call, and what it said.
        detail: String,
    },
    /// The injected thread ran and `LoadLibraryW` returned null.
    LoadFailed {
        /// The DLL that did not load.
        dll: String,
    },
    /// The injected thread did not finish in time.
    LoadTimedOut {
        /// The DLL that was still loading.
        dll: String,
    },
}

impl std::fmt::Display for InjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(detail) => write!(formatter, "could not start the game: {detail}"),
            Self::NoLoadLibrary => write!(
                formatter,
                "kernel32!LoadLibraryW not found in this process -- nothing can be injected"
            ),
            Self::Remote { dll, detail } => write!(formatter, "{dll}: {detail}"),
            Self::LoadFailed { dll } => write!(
                formatter,
                "{dll}: the injected thread ran and LoadLibraryW returned null -- the file is \
                 there but the loader refused it (wrong architecture, or a dependency of its \
                 own is missing)"
            ),
            Self::LoadTimedOut { dll } => write!(
                formatter,
                "{dll}: still loading after {}s",
                LOAD_TIMEOUT_MS / 1000
            ),
        }
    }
}

/// A null-terminated wide copy of a path, kept alive by the caller.
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

/// What a successful launch produced.
pub struct Launched {
    /// The process id, so a caller can say which session this was.
    pub process_id: u32,
}

/// Start the plan's executable suspended, inject every DLL, then let it run.
///
/// # Errors
///
/// Any failure after the process exists terminates it before returning, so a caller that gets an
/// error is looking at a machine with no half-modded game on it.
pub fn launch(plan: &Plan, game_dir: &Path) -> Result<Launched, InjectError> {
    // Resolved in this process and used in the other one. Kernel32 is mapped at the same base in
    // every process for the life of a boot, which is what makes the address portable; it is also
    // the assumption every injector makes, including the one being replaced here. If it were
    // ever untrue the injected thread would start at a bogus address and the game would die
    // immediately rather than run unmodded, so the failure is loud rather than silent.
    let load_library = load_library_address().ok_or(InjectError::NoLoadLibrary)?;

    let mut command = wide(plan.exe.as_os_str());
    let working = wide(game_dir.as_os_str());
    let startup = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(0),
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();

    // SAFETY: `command` and `working` are null-terminated wide buffers that outlive the call,
    // `startup` carries its own `cb`, and `process` is a live out-parameter. The optional
    // arguments are null, which the documented contract accepts.
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_SUSPENDED,
            None,
            PCWSTR(working.as_ptr()),
            &startup,
            &mut process,
        )
    }
    .map_err(|error| InjectError::Spawn(error.to_string()))?;

    for injection in &plan.injections {
        if let Err(failure) = inject_one(process.hProcess, load_library, injection) {
            // The contract from `plan::Refusal`: a session has the whole list, or does not exist.
            // SAFETY: `hProcess` is the handle `CreateProcessW` just returned, not yet closed.
            unsafe {
                let _ = TerminateProcess(process.hProcess, 1);
            }
            close(process.hThread);
            close(process.hProcess);
            return Err(failure);
        }
    }

    // SAFETY: `hThread` is the initial thread `CreateProcessW` returned, still suspended.
    unsafe { ResumeThread(process.hThread) };

    let process_id = process.dwProcessId;
    close(process.hThread);
    close(process.hProcess);
    Ok(Launched { process_id })
}

/// The address of `kernel32!LoadLibraryW` in this process.
fn load_library_address() -> Option<ThreadStartRoutine> {
    // SAFETY: both strings are null-terminated literals, and the handle `GetModuleHandleW`
    // returns for an already-loaded module is borrowed rather than owned, so it is not closed.
    let address = unsafe {
        let kernel32 = GetModuleHandleW(windows::core::w!("kernel32.dll")).ok()?;
        GetProcAddress(kernel32, windows::core::s!("LoadLibraryW"))
    }?;
    // SAFETY: `LoadLibraryW` takes one pointer-sized argument and returns a pointer-sized value,
    // which is the `LPTHREAD_START_ROUTINE` shape. This is the same reinterpretation every
    // injector performs, and the reason `CreateRemoteThread` can call it at all. Both types are
    // spelled out rather than inferred: a transmute between function pointers is exactly the
    // place where an inferred target silently becomes the wrong signature.
    let routine = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, ThreadStartRoutine>(address)
    };
    Some(routine)
}

/// Put one DLL into the suspended process and wait for its loader to answer.
fn inject_one(
    process: HANDLE,
    load_library: ThreadStartRoutine,
    injection: &Injection,
) -> Result<(), InjectError> {
    let path = wide(injection.resolved.as_os_str());
    let bytes = std::mem::size_of_val(path.as_slice());

    // SAFETY: `process` is a live handle with the access `CreateProcessW` grants its creator.
    let remote = unsafe {
        VirtualAllocEx(
            process,
            None,
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if remote.is_null() {
        return Err(InjectError::Remote {
            dll: injection.spelled.clone(),
            detail: format!("VirtualAllocEx of {bytes} bytes failed"),
        });
    }

    let outcome = write_and_run(process, remote, &path, bytes, load_library, injection);

    // SAFETY: `remote` is the allocation just made in `process`, and `MEM_RELEASE` requires a
    // size of zero, which is what is passed.
    unsafe {
        let _ = VirtualFreeEx(process, remote, 0, MEM_RELEASE);
    }
    outcome
}

/// Write the path into the remote allocation, run `LoadLibraryW` on it, and read the answer.
fn write_and_run(
    process: HANDLE,
    remote: *mut core::ffi::c_void,
    path: &[u16],
    bytes: usize,
    load_library: ThreadStartRoutine,
    injection: &Injection,
) -> Result<(), InjectError> {
    // SAFETY: `remote` is an allocation of exactly `bytes` in `process`, and `path` is that many
    // bytes long by construction.
    unsafe { WriteProcessMemory(process, remote, path.as_ptr().cast(), bytes, None) }.map_err(
        |error| InjectError::Remote {
            dll: injection.spelled.clone(),
            detail: format!("WriteProcessMemory failed: {error}"),
        },
    )?;

    // SAFETY: `load_library` is `kernel32!LoadLibraryW`, valid at the same address in the target,
    // and `remote` holds the wide path it will read.
    let thread =
        unsafe { CreateRemoteThread(process, None, 0, Some(load_library), Some(remote), 0, None) }
            .map_err(|error| InjectError::Remote {
                dll: injection.spelled.clone(),
                detail: format!("CreateRemoteThread failed: {error}"),
            })?;

    // SAFETY: `thread` is the handle just returned, and is closed exactly once below.
    let waited = unsafe { WaitForSingleObject(thread, LOAD_TIMEOUT_MS) };
    if waited != WAIT_OBJECT_0 {
        close(thread);
        return Err(InjectError::LoadTimedOut {
            dll: injection.spelled.clone(),
        });
    }

    let mut code = 0u32;
    // SAFETY: `thread` has finished, so its exit code is final, and `code` is a live out-param.
    let read = unsafe { GetExitCodeThread(thread, &mut code) };
    close(thread);
    read.map_err(|error| InjectError::Remote {
        dll: injection.spelled.clone(),
        detail: format!("GetExitCodeThread failed: {error}"),
    })?;

    // The exit code is the low half of whatever `LoadLibraryW` returned. That is not the module
    // handle on a 64-bit target -- the top half is gone -- so it cannot be reported as one. Zero
    // is still conclusive: no loader hands out a handle whose low half is all zero, and a null
    // return is exactly what a refusal looks like.
    if code == 0 {
        return Err(InjectError::LoadFailed {
            dll: injection.spelled.clone(),
        });
    }
    Ok(())
}

/// Close a handle, ignoring a failure there is nothing useful to do about.
fn close(handle: HANDLE) {
    if handle.is_invalid() {
        return;
    }
    // SAFETY: every handle reaching this is one this module created and has not already closed.
    unsafe {
        let _ = CloseHandle(handle);
    }
}
