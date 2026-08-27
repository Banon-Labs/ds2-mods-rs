//! Standalone crash-logging DLL: `ds2_crash_logging.dll`.
//!
//! The entire crate is a `DllMain` that hands `ds2-crash-logging-core` a set of file names and
//! the module handle the loader just gave it. Everything interesting is in the core crate; this
//! one exists so the crash logger can be loaded on its own, with no feature DLL involved.
//!
//! # What it writes, and when
//!
//! Files land next to the game executable -- `.../Dark Souls II Scholar of the First
//! Sin/Game/` on a Steam install. `ds2-crash-log.txt` gets a line at install and a full record
//! for every reportable exception; `ds2-crash-latest.txt` holds the most interesting single
//! record; `ds2-crash-breadcrumb-latest.txt` says how far the process got;
//! `ds2-crash-modules.txt` is the loaded-module inventory; `ds2-crash-minidump.dmp` appears only
//! if the process dies on an exception nobody handled. Each file describes exactly one run --
//! the previous run is rotated to `<name>.prev` and nothing older survives.
//!
//! # It has not been loaded into the game yet
//!
//! Nothing in this workspace has run against DARK SOULS II. The DLL builds and its host-testable
//! logic is under test, and that is the whole of what is known. It needs a loader to get into the
//! process, and it needs Arxan's integrity checks neutered before anything detours the game --
//! both of which are somebody else's crate.

use std::sync::Once;

const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_PROCESS_DETACH: u32 = 0;
const DLL_MAIN_SUCCESS: i32 = 1;

static START: Once = Once::new();

#[unsafe(no_mangle)]
/// # Safety
///
/// Called by the Windows loader. Do not call directly.
pub unsafe extern "system" fn DllMain(
    module: *mut core::ffi::c_void,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        START.call_once(|| {
            // Named explicitly rather than taken from `CrashLogConfig::default()`: these strings
            // are what a player is asked to send back, so the DLL that ships them says them out
            // loud instead of inheriting them from a library that could change underneath it.
            ds2_crash_logging_core::install(
                ds2_crash_logging_core::CrashLogConfig {
                    log_file_name: "ds2-crash-log.txt",
                    latest_file_name: "ds2-crash-latest.txt",
                    breadcrumb_file_name: "ds2-crash-breadcrumb-latest.txt",
                    modules_file_name: "ds2-crash-modules.txt",
                    minidump_file_name: "ds2-crash-minidump.dmp",
                    module_label: "ds2-crash-logging",
                },
                module as usize,
            );
            ds2_crash_logging_core::write_breadcrumb(
                "dll-attach",
                format_args!("standalone loaded"),
            );
        });
    }
    if reason == DLL_PROCESS_DETACH {
        // Distinguishes an orderly shutdown from a process that died where it stood.
        ds2_crash_logging_core::note_process_detach();
    }
    DLL_MAIN_SUCCESS
}

/// A `cdylib` with no exported symbol is not worth linking. On the host there is no `DllMain` to
/// call, so this keeps the non-Windows build meaningful for `cargo test`.
#[cfg(not(windows))]
#[unsafe(no_mangle)]
pub extern "C" fn ds2_crash_logging_host_stub() -> i32 {
    DLL_MAIN_SUCCESS
}
