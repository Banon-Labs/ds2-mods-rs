//! Smoke helper: load `ds2_crash_logging.dll` into a throwaway process and fault it.
//!
//! This exercises the crash logger WITHOUT the game -- no DS2, no loader, no Arxan. It proves
//! only that the DLL loads, that `DllMain` installs the vectored handler, and that the handler
//! writes its files; it proves nothing about behaviour inside DARK SOULS II, where the module
//! table, the thread that faults and the filters already installed are all different.
//!
//! Run it under Wine/Proton from the directory you want the log files written to, since the
//! logger writes next to the running executable:
//!
//! ```text
//! wine load_and_crash.exe ds2_crash_logging.dll
//! ```
//!
//! `0xc0000005` is raised directly rather than by dereferencing a null pointer, so the fault is
//! delivered at a known instruction and the example carries no UB of its own.

#[cfg(windows)]
use std::ffi::{CString, c_void};

#[cfg(windows)]
unsafe extern "system" {
    fn LoadLibraryA(path: *const u8) -> *mut c_void;
    fn RaiseException(code: u32, flags: u32, arg_count: u32, args: *const usize) -> !;
}

#[cfg(windows)]
fn main() {
    let dll_path = std::env::args()
        .nth(1)
        .expect("usage: load_and_crash.exe <path-to-ds2_crash_logging.dll>");
    let dll_path = CString::new(dll_path).expect("DLL path contains NUL");
    let module = unsafe { LoadLibraryA(dll_path.as_ptr().cast()) };
    assert!(!module.is_null(), "LoadLibraryA failed");
    unsafe { RaiseException(0xc000_0005, 0, 0, std::ptr::null()) }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("load_and_crash is a Windows/Wine smoke helper");
}
