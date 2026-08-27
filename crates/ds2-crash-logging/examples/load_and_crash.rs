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
//! wine load_and_crash.exe ds2_crash_logging.dll            # arm A: fault on the MAIN thread
//! wine load_and_crash.exe ds2_crash_logging.dll --thread   # arm B: fault on a SPAWNED thread
//! ```
//!
//! `0xc0000005` is raised directly rather than by dereferencing a null pointer, so the fault is
//! delivered at a known instruction and the example carries no UB of its own.
//!
//! # THE TWO ARMS, and the question they exist to settle
//!
//! `ds2-mods-rs-4tm` ran the logger inside DARK SOULS II and got a first-chance record and
//! nothing else: `fatal=false`, no fatal record, no minidump, so
//! `SetUnhandledExceptionFilter`'s callback never ran. The same logger under Wine, faulted from
//! `main`, had previously written all five artifacts including a rich-tier minidump.
//!
//! Between those two runs sit several differences at once -- the game, Arxan, Proton rather than
//! plain Wine, and *the thread the fault was raised on*. In-game the fault came from a thread this
//! project spawned with `std::thread`, inside a DLL linked `+crt-static`; under Wine it came from
//! `main`. `--thread` changes ONLY that last variable, with everything else held fixed, which is
//! the whole point: if arm A writes a fatal record and arm B does not, the thread is the cause and
//! no game launch was needed to learn it.
//!
//! A statically linked MSVC CRT installs its own per-module SEH wrapper around thread entry, which
//! is the mechanism that would explain a process dying without `UnhandledExceptionFilter` ever
//! being consulted. This example does not assume that -- it measures whether the difference is
//! real before anyone goes looking for a mechanism.

#[cfg(windows)]
use std::ffi::{CString, c_void};

#[cfg(windows)]
unsafe extern "system" {
    fn LoadLibraryA(path: *const u8) -> *mut c_void;
    fn RaiseException(code: u32, flags: u32, arg_count: u32, args: *const usize) -> !;
}

/// `EXCEPTION_ACCESS_VIOLATION`, the same code the in-game deliberate fault raises.
#[cfg(windows)]
const FAULT_CODE: u32 = 0xc000_0005;

#[cfg(windows)]
fn fault() -> ! {
    // SAFETY: no arguments, no continuable flag, nothing dereferenced. This raises a well-formed
    // exception at a known instruction rather than committing UB and hoping the fault lands where
    // intended. It does not return.
    unsafe { RaiseException(FAULT_CODE, 0, 0, std::ptr::null()) }
}

#[cfg(windows)]
fn main() {
    let mut args = std::env::args().skip(1);
    let dll_path = args
        .next()
        .expect("usage: load_and_crash.exe <path-to-ds2_crash_logging.dll> [--thread]");
    let on_thread = args.any(|a| a == "--thread");

    let dll_path = CString::new(dll_path).expect("DLL path contains NUL");
    let module = unsafe { LoadLibraryA(dll_path.as_ptr().cast()) };
    assert!(!module.is_null(), "LoadLibraryA failed");

    if on_thread {
        // Arm B: exactly how the in-game deliberate fault is raised -- `std::thread::spawn`, then
        // fault from inside it. `join` never returns, so the process dies here or not at all.
        println!("arm=spawned-thread raising 0x{FAULT_CODE:08x}");
        let handle = std::thread::spawn(fault);
        let _ = handle.join();
        unreachable!("the spawned thread does not return");
    } else {
        println!("arm=main-thread raising 0x{FAULT_CODE:08x}");
        fault()
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("load_and_crash is a Windows/Wine smoke helper");
}
