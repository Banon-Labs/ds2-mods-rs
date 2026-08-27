//! First-chance exception capture for the DLLs in this workspace, as a reusable library.
//!
//! A Win32 **vectored exception handler** sees every exception raised in the process before any
//! `try`/`except` gets a chance at it, so this crate can write a record of a fault even when
//! something downstream swallows it. A top-level unhandled-exception filter is installed
//! alongside, because a first-chance handler structurally cannot tell you whether the exception
//! it just saw was the one that killed the process.
//!
//! What lands on disk, in the game directory:
//!
//! | file | written when | contents |
//! |---|---|---|
//! | `ds2-crash-log.txt` | install, every recorded exception, every minidump attempt | append-only running log, one run per file |
//! | `ds2-crash-latest.txt` | every recorded exception, until a fatal one latches it | the single most interesting record on its own |
//! | `ds2-crash-breadcrumb-latest.txt` | install, host DLL detach | where the process got to, tiny, always current |
//! | `ds2-crash-modules.txt` | install, and again on a fatal exception | every loaded module: base, span, PE timestamp, name |
//! | `ds2-crash-minidump.dmp` | fatal exception only | every thread's stack, if `dbghelp` cooperates |
//!
//! Ported from `../er-mods-rs`'s `er-crash-logging-core`. Two things changed on the way and both
//! matter more than the line count suggests:
//!
//! # Nothing here knows what game it is running in
//!
//! Deliberately, and this is what makes crash logging the first mod worth porting: it exercises
//! the loader, the Arxan neutering and the log path end to end while needing **zero** knowledge
//! of DARK SOULS II structures. Every address in a record is resolved against the module table
//! the loader itself maintains (the PEB `InMemoryOrderModuleList`), so a fault reads as
//! `nvwgf2umx.dll+0xead0c0` without this crate ever being told what modules to expect. There is
//! no dependency on `ds2-rva` and no reason for one.
//!
//! # The hang watchdog did NOT come across
//!
//! `er-crash-logging-core` ships a second module, `hang.rs`, that reports freezes -- which raise
//! no exception and are therefore invisible to everything above. It is not ported, and not
//! because of its size. Its three detectors all hang off two Elden Ring facts:
//!
//! * a version-pinned RVA for the dword `MainUpdate` increments once per frame, and the name of
//!   the executable that dword lives in; and
//! * the field offsets of `CS::LoadingScreenData`, an object from a class hierarchy that
//!   postdates this engine.
//!
//! Strip those out and what remains is a thread-suspension harness with nothing to watch. The
//! equivalent for this game starts with finding DS2's own per-frame counter in the disassembly,
//! which is reverse engineering nobody has done yet, and the address it produces would belong in
//! `ds2-rva` rather than here. Writing the harness first would mean shipping a watchdog that can
//! only ever report that it is disarmed.

use std::{fmt, path::PathBuf, sync::OnceLock};

#[cfg(windows)]
use std::{
    ffi::c_void,
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

const DEFAULT_LOG_FILE: &str = "ds2-crash-log.txt";
const DEFAULT_LATEST_FILE: &str = "ds2-crash-latest.txt";
const DEFAULT_BREADCRUMB_FILE: &str = "ds2-crash-breadcrumb-latest.txt";
const DEFAULT_MODULES_FILE: &str = "ds2-crash-modules.txt";
const DEFAULT_MINIDUMP_FILE: &str = "ds2-crash-minidump.dmp";
const DEFAULT_MODULE_LABEL: &str = "ds2-crash-logging";

/// Which files a host DLL wants its crash evidence written to, and what to call itself in them.
///
/// Every name is relative to the game directory. A host DLL that installs this alongside its own
/// feature work should give them its own names, so two loaded mods cannot fight over one log.
#[derive(Clone, Copy, Debug)]
pub struct CrashLogConfig {
    pub log_file_name: &'static str,
    pub latest_file_name: &'static str,
    pub breadcrumb_file_name: &'static str,
    /// Full loaded-module inventory: what was in the process, where, and which build. Written at
    /// install and rewritten when the process dies, so an offset like `nvwgf2umx.dll+0xead0c0`
    /// can be tied back to an exact driver image later.
    pub modules_file_name: &'static str,
    /// Postmortem minidump, written only from the unhandled-exception path.
    pub minidump_file_name: &'static str,
    pub module_label: &'static str,
}

impl Default for CrashLogConfig {
    fn default() -> Self {
        Self {
            log_file_name: DEFAULT_LOG_FILE,
            latest_file_name: DEFAULT_LATEST_FILE,
            breadcrumb_file_name: DEFAULT_BREADCRUMB_FILE,
            modules_file_name: DEFAULT_MODULES_FILE,
            minidump_file_name: DEFAULT_MINIDUMP_FILE,
            module_label: DEFAULT_MODULE_LABEL,
        }
    }
}

/// How far the host DLL got before the process died.
///
/// Stamped into every record, so a crash during load reads differently from a crash after the
/// host was fully up. The numeric values are gapped on purpose: a host DLL's own phases can be
/// interleaved later without renumbering the ones already written into logs in the field.
#[repr(usize)]
#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Uninitialized = 0,
    DllAttach = 1,
    HandlerInstalled = 2,
    /// Set by the host DLL when its own feature work is live -- not by this crate, which has no
    /// way to know. (`er-crash-logging-core` called this `ClientReady`, after a networked
    /// feature that has no counterpart in this workspace.)
    HostReady = 10,
    ExceptionObserved = 90,
}

// `phase_label` is the only caller and it is windows-only; the test module pins the strings.
#[cfg(any(windows, test))]
impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::DllAttach => "dll-attach",
            Self::HandlerInstalled => "handler-installed",
            Self::HostReady => "host-ready",
            Self::ExceptionObserved => "exception-observed",
        }
    }
}

static CONFIG: OnceLock<CrashLogConfig> = OnceLock::new();

pub(crate) fn config() -> CrashLogConfig {
    *CONFIG.get_or_init(CrashLogConfig::default)
}

pub(crate) fn path_for(name: &str) -> PathBuf {
    ds2_game_base::log::game_directory_path()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(name)
}

pub fn append_log(args: fmt::Arguments<'_>) {
    ds2_game_base::log::append_line(&path_for(config().log_file_name), args);
}

pub fn write_breadcrumb(reason: &str, args: fmt::Arguments<'_>) {
    let mut body = String::new();
    use fmt::Write as _;
    let _ = writeln!(body, "reason={reason}");
    let _ = writeln!(body, "module={}", config().module_label);
    write_common_fields(&mut body);
    let _ = writeln!(body, "details={args}");
    #[cfg(windows)]
    let _ = fs::write(path_for(config().breadcrumb_file_name), body);
    #[cfg(not(windows))]
    let _ = std::fs::write(path_for(config().breadcrumb_file_name), body);
}

fn write_common_fields(out: &mut String) {
    {
        use fmt::Write as _;
        // Every record, not just the log's first line. The breadcrumb and `-latest` files are
        // written with a direct `fs::write` and never pass through the shared logger, and they
        // are precisely the two files a tester sends on their own -- a crash report that cannot
        // name its own build costs a round-trip to answer "is this already fixed".
        //
        // `../er-mods-rs` filled this from a git sha baked in by a `build.rs`. That machinery
        // (`er-game-base::build_id`) was not ported, so the value comes from the process-wide
        // identity slot in `ds2-game-base::log` instead -- which THIS crate fills, in `install`,
        // from the loaded DLL's own PE headers. A host DLL that set the line first keeps it.
        let _ = writeln!(
            out,
            "build={}",
            ds2_game_base::log::identity_line().unwrap_or("<unset>")
        );
    }
    #[cfg(windows)]
    {
        use fmt::Write as _;
        let phase = PHASE.load(Ordering::SeqCst);
        let _ = writeln!(out, "phase_id={phase}");
        let _ = writeln!(out, "phase={}", phase_label(phase));
        let _ = writeln!(
            out,
            "self_module_base=0x{:x}",
            SELF_MODULE_BASE.load(Ordering::SeqCst)
        );
        let _ = writeln!(
            out,
            "self_module_size=0x{:x}",
            SELF_MODULE_SIZE.load(Ordering::SeqCst)
        );
    }
    #[cfg(not(windows))]
    {
        use fmt::Write as _;
        let _ = writeln!(out, "phase_id=0");
        let _ = writeln!(out, "phase=uninitialized");
        let _ = writeln!(out, "self_module_base=0x0");
        let _ = writeln!(out, "self_module_size=0x0");
    }
}

#[cfg(windows)]
pub fn mark_phase(phase: Phase) {
    PHASE.store(phase as usize, Ordering::SeqCst);
}

#[cfg(not(windows))]
pub fn mark_phase(_phase: Phase) {}

#[cfg(windows)]
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
#[cfg(windows)]
const VECTORED_FIRST_HANDLER: u32 = 1;

// NTSTATUS exception codes and the record policy built on them are plain arithmetic, so they stay
// off `cfg(windows)`: the classification is the part most worth testing, and the tests run on the
// host toolchain.
const EXCEPTION_ACCESS_VIOLATION: u32 = 0xc000_0005;
const EXCEPTION_ILLEGAL_INSTRUCTION: u32 = 0xc000_001d;
const EXCEPTION_STACK_BUFFER_OVERRUN: u32 = 0xc000_0409;
const EXCEPTION_STACK_OVERFLOW: u32 = 0xc000_00fd;
const EXCEPTION_HEAP_CORRUPTION: u32 = 0xc000_0374;
const EXCEPTION_IN_PAGE_ERROR: u32 = 0xc000_0006;
const EXCEPTION_INT_DIVIDE_BY_ZERO: u32 = 0xc000_0094;
const EXCEPTION_PRIVILEGED_INSTRUCTION: u32 = 0xc000_0096;
const EXCEPTION_NONCONTINUABLE: u32 = 0xc000_0025;
const EXCEPTION_CPP_THROW: u32 = 0xe06d_7363;
const EXCEPTION_SEVERITY_MASK: u32 = 0xc000_0000;

/// Which budget an observed exception draws from.
///
/// A first-chance vectored handler sees every `0xe06d7363` the process raises, and Windows
/// user-mode graphics drivers throw and catch C++ exceptions as ordinary control flow. Sharing
/// one budget between those and real faults means a noisy render thread can exhaust the log
/// before the access violation that actually killed the process ever gets written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecordClass {
    /// `0xe06d7363` -- a C++/Rust throw. Usually caught by somebody; only interesting in bulk or
    /// when it turns out to be the one that went unhandled.
    Throw,
    /// A hard fault: access violation, heap corruption, fail-fast, and friends.
    Fault,
}

/// Throw records are capped low; they are noise until proven otherwise.
const MAX_THROW_RECORDS: usize = 8;
/// Faults get their own, larger budget that throws cannot touch.
const MAX_FAULT_RECORDS: usize = 24;
/// Distinct `(code, address)` sites tracked for repeat counting.
#[cfg(windows)]
const THROW_SITE_SLOTS: usize = 24;

pub fn record_class(code: u32) -> RecordClass {
    if code == EXCEPTION_CPP_THROW {
        RecordClass::Throw
    } else {
        RecordClass::Fault
    }
}

/// Whether a record of `class` may still be written, given how many of each class already were.
pub fn budget_allows(class: RecordClass, throws_written: usize, faults_written: usize) -> bool {
    match class {
        RecordClass::Throw => throws_written < MAX_THROW_RECORDS,
        RecordClass::Fault => faults_written < MAX_FAULT_RECORDS,
    }
}

/// Turn an MSVC RTTI `TypeDescriptor` name into something readable.
///
/// `.?AVbad_alloc@std@@` -> `std::bad_alloc`, `.?AV_com_error@@` -> `_com_error`. Anything that
/// does not match the expected decoration is returned unchanged rather than mangled further --
/// a raw name we cannot parse is still evidence.
pub fn format_thrown_type_name(raw: &str) -> String {
    let Some(body) = raw.strip_prefix('.') else {
        return raw.to_string();
    };
    let body = ["?AV", "?AU", "PEAV", "PEAU", "AV", "AU"]
        .iter()
        .find_map(|prefix| body.strip_prefix(prefix))
        .unwrap_or(body);
    let Some(body) = body.strip_suffix("@@") else {
        return raw.to_string();
    };
    if body.is_empty() {
        return raw.to_string();
    }
    // MSVC stores the qualified name innermost-first, so `bad_alloc@std` is `std::bad_alloc`.
    body.split('@').rev().collect::<Vec<_>>().join("::")
}

// x86-64 Windows `CONTEXT`, from the documented layout: six home slots (0x30), `ContextFlags`,
// `MxCsr`, six segment selectors, `EFlags`, six debug registers, then the integer file at 0x78 in
// `Rax Rcx Rdx Rbx Rsp Rbp Rsi Rdi R8..R15` order, then `Rip`. These are OS ABI, not game facts.
#[cfg(windows)]
const CONTEXT_RAX_OFFSET: usize = 0x78;
#[cfg(windows)]
const CONTEXT_RCX_OFFSET: usize = 0x80;
#[cfg(windows)]
const CONTEXT_RDX_OFFSET: usize = 0x88;
#[cfg(windows)]
const CONTEXT_RSP_OFFSET: usize = 0x98;
/// `Rbp` sits one slot past `Rsp`. `er-crash-logging-core` never read it and reported a hardcoded
/// `context_rbp=0x0` in every record -- a field that always lies is worse than an absent one.
#[cfg(windows)]
const CONTEXT_RBP_OFFSET: usize = 0xa0;
#[cfg(windows)]
const CONTEXT_R8_OFFSET: usize = 0xb8;
#[cfg(windows)]
const CONTEXT_R9_OFFSET: usize = 0xc0;
#[cfg(windows)]
const CONTEXT_RIP_OFFSET: usize = 0xf8;

// MSVC C++ EH payload, as raised by `_CxxThrowException`. `ExceptionInformation` is
// `[magic, thrown object, ThrowInfo, module base]`; on x64 every pointer inside `ThrowInfo` is a
// 32-bit RVA relative to that module base.
#[cfg(windows)]
const CPP_EH_PARAMS_KEPT: usize = 4;
#[cfg(windows)]
const CPP_EH_MAGIC: usize = 0x1993_0520;
#[cfg(windows)]
const CPP_EH_PARAM_OBJECT: usize = 1;
#[cfg(windows)]
const CPP_EH_PARAM_THROW_INFO: usize = 2;
#[cfg(windows)]
const CPP_EH_PARAM_MODULE_BASE: usize = 3;
#[cfg(windows)]
const THROW_INFO_CATCHABLE_ARRAY_OFFSET: usize = 0x0c;
#[cfg(windows)]
const CATCHABLE_ARRAY_FIRST_TYPE_OFFSET: usize = 0x04;
#[cfg(windows)]
const CATCHABLE_TYPE_DESCRIPTOR_OFFSET: usize = 0x04;
#[cfg(windows)]
const TYPE_DESCRIPTOR_NAME_OFFSET: usize = 0x10;
#[cfg(windows)]
const TYPE_DESCRIPTOR_NAME_MAX: usize = 96;
#[cfg(windows)]
const THROWN_OBJECT_PREVIEW_BYTES: usize = 32;

// Minidump: indirectly-referenced memory plus thread info keeps the dump small while still
// carrying the stacks and the objects those stacks point at.
#[cfg(windows)]
const MINIDUMP_NORMAL: u32 = 0x0000_0000;
#[cfg(windows)]
const MINIDUMP_WITH_UNLOADED_MODULES: u32 = 0x0000_0020;
#[cfg(windows)]
const MINIDUMP_WITH_INDIRECTLY_REFERENCED_MEMORY: u32 = 0x0000_0040;
#[cfg(windows)]
const MINIDUMP_WITH_PROCESS_THREAD_DATA: u32 = 0x0000_0100;
#[cfg(windows)]
const MINIDUMP_WITH_THREAD_INFO: u32 = 0x0000_1000;
#[cfg(windows)]
const GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
const CREATE_ALWAYS: u32 = 2;
#[cfg(windows)]
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
#[cfg(windows)]
const INVALID_HANDLE_VALUE: isize = -1;

// PE header walk. `e_lfanew` at DOS `+0x3c` points at `PE\0\0`; the COFF file header follows it
// with `TimeDateStamp` at `+0x08`, and the optional header begins 24 bytes past the signature.
// `SizeOfImage` is at optional-header `+0x38` in both PE32 and PE32+.
#[cfg(windows)]
const PE_DOS_MAGIC: u16 = 0x5a4d;
#[cfg(windows)]
const PE_E_LFANEW_OFFSET: usize = 0x3c;
#[cfg(windows)]
const PE_E_LFANEW_MAX: usize = 0x1000;
#[cfg(windows)]
const PE_TIMEDATESTAMP_FROM_NT: usize = 0x08;
#[cfg(windows)]
const PE_OPTIONAL_HEADER_FROM_NT: usize = 24;
#[cfg(windows)]
const PE_SIZE_OF_IMAGE_IN_OPTIONAL: usize = 0x38;

#[cfg(windows)]
const STACK_TRACE_FRAMES_TO_SKIP: u32 = 2;
#[cfg(windows)]
const STACK_TRACE_FRAME_COUNT: usize = 24;
#[cfg(windows)]
const STACK_SCAN_SLOTS: usize = 256;
#[cfg(windows)]
const STACK_SCAN_MAX_FRAMES: usize = 32;
#[cfg(windows)]
const STACK_RAW_QWORDS: usize = 8;
/// Windows reserves the low 64 KiB of every process's address space, so nothing below this is a
/// pointer worth chasing. Not a bound on any image -- see [`module_for_addr`] for that.
#[cfg(any(windows, test))]
pub(crate) const MIN_VALID_PTR: usize = 0x10000;

#[cfg(windows)]
static INSTALLED: std::sync::Once = std::sync::Once::new();
#[cfg(windows)]
static SELF_MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static SELF_MODULE_SIZE: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static HANDLER_HANDLE: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static PHASE: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static THROW_RECORDS_WRITTEN: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static FAULT_RECORDS_WRITTEN: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static RECORD_INDEX: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static PREVIOUS_UNHANDLED_FILTER: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static FATAL_REPORTED: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static INSTALL_INSTANT: OnceLock<std::time::Instant> = OnceLock::new();

/// One tracked `(code, address)` exception site and how often it has been seen.
///
/// The count is the difference between background noise and a smoking gun: a throw site at its
/// 400th occurrence is how the driver works, one at its 1st occurrence right before the process
/// dies is the lead.
#[cfg(windows)]
struct ThrowSite {
    code: AtomicUsize,
    address: AtomicUsize,
    count: AtomicUsize,
}

#[cfg(windows)]
static THROW_SITES: [ThrowSite; THROW_SITE_SLOTS] = [const {
    ThrowSite {
        code: AtomicUsize::new(0),
        address: AtomicUsize::new(0),
        count: AtomicUsize::new(0),
    }
}; THROW_SITE_SLOTS];

/// Count this `(code, address)` and return how many times it has now been seen (1 on first sight).
/// Returns 0 once the table is full, meaning "unknown, not tracked".
#[cfg(windows)]
fn note_exception_site(code: u32, address: usize) -> usize {
    let code = code as usize;
    for site in THROW_SITES.iter() {
        let claimed = site.address.load(Ordering::SeqCst);
        if claimed == address && site.code.load(Ordering::SeqCst) == code {
            return site.count.fetch_add(1, Ordering::SeqCst) + 1;
        }
        if claimed == 0
            && site
                .address
                .compare_exchange(0, address, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            site.code.store(code, Ordering::SeqCst);
            return site.count.fetch_add(1, Ordering::SeqCst) + 1;
        }
    }
    0
}

/// How many times this `(code, address)` has been seen, without counting this look.
///
/// The fatal filter re-observes the exception the vectored handler already counted; counting it
/// twice would report every fatal site as one occurrence more common than it really is.
#[cfg(windows)]
fn current_site_count(code: u32, address: usize) -> usize {
    let code = code as usize;
    THROW_SITES
        .iter()
        .find(|site| {
            site.address.load(Ordering::SeqCst) == address
                && site.code.load(Ordering::SeqCst) == code
        })
        .map(|site| site.count.load(Ordering::SeqCst))
        .unwrap_or(0)
}

/// Every tracked site with its repeat count, most-repeated first.
#[cfg(windows)]
fn exception_site_histogram(modules: &[LoadedModule]) -> String {
    let mut sites: Vec<(usize, u32, usize)> = THROW_SITES
        .iter()
        .filter_map(|site| {
            let address = site.address.load(Ordering::SeqCst);
            (address != 0).then(|| {
                (
                    site.count.load(Ordering::SeqCst),
                    site.code.load(Ordering::SeqCst) as u32,
                    address,
                )
            })
        })
        .collect();
    sites.sort_by_key(|site| std::cmp::Reverse(site.0));
    let mut out = String::from("[");
    for (index, (count, code, address)) in sites.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "0x{:08x}@0x{:x}{}x{}",
            code,
            address,
            module_tag(*address, modules),
            count
        ));
    }
    out.push(']');
    out
}

#[cfg(windows)]
#[repr(C)]
struct ExceptionRecordMin {
    exception_code: u32,
    exception_flags: u32,
    next_record: *mut ExceptionRecordMin,
    exception_address: *mut c_void,
    number_parameters: u32,
    exception_information: [usize; 15],
}

#[cfg(windows)]
#[repr(C)]
struct ExceptionPointersMin {
    exception_record: *mut ExceptionRecordMin,
    context_record: *mut c_void,
}

#[cfg(windows)]
type VectoredHandler = unsafe extern "system" fn(*mut ExceptionPointersMin) -> i32;

#[cfg(windows)]
type UnhandledExceptionFilter = unsafe extern "system" fn(*mut ExceptionPointersMin) -> i32;

#[cfg(windows)]
#[repr(C)]
struct SystemTimeMin {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

// Only the exception, timestamp, module-handle and file primitives are declared here. Every RAM
// read goes through `ds2-game-base::mem`, which already owns a `ReadProcessMemory` declaration --
// a hand-written Win32 ABI must have exactly one declaration in the workspace.
#[cfg(windows)]
unsafe extern "system" {
    fn AddVectoredExceptionHandler(first: u32, handler: VectoredHandler) -> *mut c_void;
    fn SetUnhandledExceptionFilter(filter: UnhandledExceptionFilter) -> *mut c_void;
    fn GetSystemTime(out: *mut SystemTimeMin);
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetCurrentProcess() -> isize;
    fn GetCurrentProcessId() -> u32;
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: isize,
    ) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn DeleteFileW(file_name: *const u16) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn RtlCaptureStackBackTrace(
        frames_to_skip: u32,
        frames_to_capture: u32,
        backtrace: *mut *mut c_void,
        backtrace_hash: *mut u32,
    ) -> u16;
}

/// Install the vectored handler and the top-level filter. Idempotent; first call wins.
///
/// `self_module_base` is the `HMODULE` the loader passed to the host DLL's `DllMain`, used to
/// report where this code was mapped and to date the build that wrote the log.
#[cfg(windows)]
pub fn install(new_config: CrashLogConfig, self_module_base: usize) {
    let _ = CONFIG.set(new_config);
    record_self_module(self_module_base);
    INSTALLED.call_once(|| {
        let _ = INSTALL_INSTANT.set(std::time::Instant::now());
        publish_identity_line();
        mark_phase(Phase::DllAttach);
        let handle =
            unsafe { AddVectoredExceptionHandler(VECTORED_FIRST_HANDLER, crash_vectored_handler) }
                as usize;
        HANDLER_HANDLE.store(handle, Ordering::SeqCst);
        // The vectored handler only ever sees first-chance exceptions, so on its own it cannot say
        // whether anything it logged was fatal. The top-level filter is what closes that gap: it
        // runs only when nothing handled the exception. Chain whatever was installed before us --
        // the game's own filter and Windows Error Reporting live here too, and silently swallowing
        // them would trade one blind spot for another.
        let previous = unsafe { SetUnhandledExceptionFilter(unhandled_exception_filter) } as usize;
        PREVIOUS_UNHANDLED_FILTER.store(previous, Ordering::SeqCst);
        if handle != 0 {
            mark_phase(Phase::HandlerInstalled);
            append_log(format_args!(
                "crash logger installed module={} self=0x{:x}+0x{:x} previous_unhandled_filter=0x{:x}",
                config().module_label,
                SELF_MODULE_BASE.load(Ordering::SeqCst),
                SELF_MODULE_SIZE.load(Ordering::SeqCst),
                previous
            ));
            write_breadcrumb("handler-installed", format_args!("handler=0x{handle:x}"));
        } else {
            append_log(format_args!(
                "crash logger AddVectoredExceptionHandler failed"
            ));
        }
        write_module_inventory("install");
    });
}

/// Fill `ds2-game-base`'s process-wide identity slot, so every log this run opens with a line
/// naming the binary that wrote it.
///
/// # What this can honestly say
///
/// `../er-mods-rs` composed the equivalent line out of a git description baked in by a
/// `build.rs`. That is a better answer and it is not available here: `er-game-base::build_id` was
/// not ported (it also carried release-tag and roster specifics), and this crate has no build
/// script. What IS available at runtime is the loaded DLL's own PE `TimeDateStamp` -- a
/// per-build watermark the linker writes into the image -- plus where the loader mapped it and
/// the crate version. That is enough to tell two builds apart, which is the whole job.
///
/// It is deliberately NOT called a date. The MSVC linker writes a content hash into this field
/// for reproducible builds rather than a wall clock, so reporting it as a timestamp would be a
/// claim about a build we would then have to defend.
///
/// First setter wins process-wide: a host DLL that already published a truer identity keeps it.
#[cfg(windows)]
fn publish_identity_line() {
    let base = SELF_MODULE_BASE.load(Ordering::SeqCst);
    let size = SELF_MODULE_SIZE.load(Ordering::SeqCst);
    let stamp = unsafe { pe_timedatestamp(base) }.unwrap_or(0);
    ds2_game_base::log::set_identity_line(format!(
        "identity module={} crate=ds2-crash-logging-core v{} image=0x{:x}+0x{:x} pe_watermark=0x{:08x}",
        config().module_label,
        env!("CARGO_PKG_VERSION"),
        base,
        size,
        stamp
    ));
}

/// Note that the host DLL is detaching.
///
/// A fatal record with no matching detach means the process died where it stood; a detach with no
/// fatal record means it shut down or was killed from outside. Neither is knowable from the crash
/// log alone.
#[cfg(windows)]
pub fn note_process_detach() {
    write_breadcrumb(
        "process-detach",
        format_args!(
            "throw_records={} fault_records={} fatal_reported={}",
            THROW_RECORDS_WRITTEN.load(Ordering::SeqCst),
            FAULT_RECORDS_WRITTEN.load(Ordering::SeqCst),
            FATAL_REPORTED.load(Ordering::SeqCst)
        ),
    );
}

#[cfg(not(windows))]
pub fn note_process_detach() {}

#[cfg(not(windows))]
pub fn install(config: CrashLogConfig, _self_module_base: usize) {
    let _ = CONFIG.set(config);
}

/// Record where the loader mapped this DLL and how far it spans.
///
/// # A size that cannot be read is reported as unknown, never guessed
///
/// `er-crash-logging-core` fell back to a hardcoded `0x0400_0000` -- 64 MB -- when the PE headers
/// were unreadable. No mod DLL is 64 MB; that number is slack, not a measurement, and a record
/// that prints a fabricated span is exactly the failure this port was told to watch for. If the
/// headers cannot be read the field stays `0x0`, which reads as "unknown" and is true.
#[cfg(windows)]
fn record_self_module(base: usize) {
    if base < MIN_VALID_PTR {
        return;
    }
    SELF_MODULE_BASE.store(base, Ordering::SeqCst);
    SELF_MODULE_SIZE.store(
        unsafe { pe_size_of_image(base) }.unwrap_or(0),
        Ordering::SeqCst,
    );
}

/// Fault-safe `u32` read.
///
/// `ds2-game-base::mem` exposes the signed reader; `cast_unsigned` reinterprets the same four
/// bytes rather than converting them, so this is exactly the `safe_read_u32` the sibling crate
/// declared for itself, without a second `ReadProcessMemory` declaration in the workspace.
///
/// # Safety
///
/// `addr` has no precondition -- the read is performed by `ReadProcessMemory` and fails closed on
/// an unmapped range. The caller owns the meaning of the bytes.
#[cfg(windows)]
unsafe fn safe_read_u32(addr: usize) -> Option<u32> {
    unsafe { ds2_game_base::mem::safe_read_i32(addr) }.map(i32::cast_unsigned)
}

/// Locate the `PE\0\0` signature of the image mapped at `base`, or `None` if it is not one.
///
/// `ds2-game-base` has this walk too, but only reachable through `module_image_size()`, which
/// resolves the GAME module. The crash logger needs the headers of an arbitrary module -- its own
/// DLL, and every entry in the loaded-module table -- so the walk is repeated here over the same
/// fault-safe readers rather than the bound being taken from the wrong image.
#[cfg(windows)]
unsafe fn pe_nt_headers(base: usize) -> Option<usize> {
    if unsafe { ds2_game_base::mem::safe_read_u16(base) }? != PE_DOS_MAGIC {
        return None;
    }
    let e_lfanew = unsafe { safe_read_u32(base + PE_E_LFANEW_OFFSET) }? as usize;
    if e_lfanew > PE_E_LFANEW_MAX {
        return None;
    }
    let nt = base.checked_add(e_lfanew)?;
    if unsafe { safe_read_u32(nt) }? != 0x0000_4550 {
        return None;
    }
    Some(nt)
}

/// `SizeOfImage` of the module mapped at `base`: how many bytes the loader mapped it over.
#[cfg(windows)]
unsafe fn pe_size_of_image(base: usize) -> Option<usize> {
    let nt = unsafe { pe_nt_headers(base) }?;
    let optional = nt + PE_OPTIONAL_HEADER_FROM_NT;
    unsafe { safe_read_u32(optional + PE_SIZE_OF_IMAGE_IN_OPTIONAL) }.map(|size| size as usize)
}

/// The COFF `TimeDateStamp` of the module mapped at `base`: a per-build watermark, not a date.
#[cfg(windows)]
unsafe fn pe_timedatestamp(base: usize) -> Option<u32> {
    let nt = unsafe { pe_nt_headers(base) }?;
    unsafe { safe_read_u32(nt + PE_TIMEDATESTAMP_FROM_NT) }
}

#[cfg(windows)]
unsafe extern "system" fn crash_vectored_handler(info: *mut ExceptionPointersMin) -> i32 {
    let Some(snapshot) = (unsafe { ExceptionSnapshot::from_raw(info) }) else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    if !is_reportable_exception(snapshot.code) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    // Count every occurrence, even ones too far over budget to write: the repeat count is what
    // separates "this driver throws constantly" from "this happened once, just before the end".
    let repeat = note_exception_site(snapshot.code, snapshot.address);
    let class = record_class(snapshot.code);
    let allowed = budget_allows(
        class,
        THROW_RECORDS_WRITTEN.load(Ordering::SeqCst),
        FAULT_RECORDS_WRITTEN.load(Ordering::SeqCst),
    );
    if !allowed {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    match class {
        RecordClass::Throw => THROW_RECORDS_WRITTEN.fetch_add(1, Ordering::SeqCst),
        RecordClass::Fault => FAULT_RECORDS_WRITTEN.fetch_add(1, Ordering::SeqCst),
    };
    mark_phase(Phase::ExceptionObserved);
    let report = exception_report(&snapshot, "veh-first-chance-exception", repeat);
    // A fatal record is the conclusion of the run; never let a later benign throw overwrite it.
    if FATAL_REPORTED.load(Ordering::SeqCst) == 0 {
        let _ = fs::write(path_for(config().latest_file_name), &report);
    }
    append_log(format_args!("{report}\n---"));
    EXCEPTION_CONTINUE_SEARCH
}

/// Top-level filter: reached only when no handler anywhere in the process claimed the exception.
///
/// This is the record that says "this one was fatal" -- the distinction a first-chance handler
/// structurally cannot make.
#[cfg(windows)]
unsafe extern "system" fn unhandled_exception_filter(info: *mut ExceptionPointersMin) -> i32 {
    // If reporting the fatal exception itself faults, do not recurse into reporting again -- chain
    // straight on so the next filter still gets its turn.
    let first_entry = FATAL_REPORTED
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok();
    if !first_entry {
        return chain_previous_unhandled_filter(info);
    }
    if let Some(snapshot) = unsafe { ExceptionSnapshot::from_raw(info) } {
        let repeat = current_site_count(snapshot.code, snapshot.address);
        let report = exception_report(&snapshot, "unhandled-exception-fatal", repeat);
        let _ = fs::write(path_for(config().latest_file_name), &report);
        append_log(format_args!("{report}\n---"));
    }
    write_module_inventory("fatal");
    unsafe { write_minidump_named(config().minidump_file_name, info) };
    chain_previous_unhandled_filter(info)
}

/// Hand the exception on exactly as we found it.
///
/// Whatever was registered before us -- the game's own filter, Windows Error Reporting, another
/// mod's crash handler -- still gets its turn. We observe; we do not consume.
#[cfg(windows)]
fn chain_previous_unhandled_filter(info: *mut ExceptionPointersMin) -> i32 {
    let previous = PREVIOUS_UNHANDLED_FILTER.load(Ordering::SeqCst);
    if previous != 0 {
        let chained: UnhandledExceptionFilter = unsafe { std::mem::transmute(previous) };
        return unsafe { chained(info) };
    }
    EXCEPTION_CONTINUE_SEARCH
}

#[cfg(windows)]
struct ExceptionSnapshot {
    code: u32,
    address: usize,
    access_kind: usize,
    access_address: usize,
    rip: usize,
    rsp: usize,
    rbp: usize,
    rax: usize,
    rcx: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
    thread_id: usize,
    /// The leading `ExceptionInformation` entries, kept verbatim. For `0xe06d7363` these carry the
    /// C++ EH magic, the thrown object, the `ThrowInfo`, and the module base its RVAs resolve
    /// against -- everything needed to name the thrown type.
    params: [usize; CPP_EH_PARAMS_KEPT],
    param_count: usize,
}

#[cfg(windows)]
impl ExceptionSnapshot {
    unsafe fn from_raw(info: *mut ExceptionPointersMin) -> Option<Self> {
        if info.is_null() {
            return None;
        }
        let pointers = unsafe { &*info };
        if pointers.exception_record.is_null() {
            return None;
        }
        let record = unsafe { &*pointers.exception_record };
        let mut out = Self {
            code: record.exception_code,
            address: record.exception_address as usize,
            access_kind: 0,
            access_address: 0,
            rip: 0,
            rsp: 0,
            rbp: 0,
            rax: 0,
            rcx: 0,
            rdx: 0,
            r8: 0,
            r9: 0,
            thread_id: unsafe { GetCurrentThreadId() as usize },
            params: [0; CPP_EH_PARAMS_KEPT],
            param_count: record.number_parameters as usize,
        };
        for (index, slot) in out.params.iter_mut().enumerate() {
            if index < out.param_count {
                *slot = record.exception_information[index];
            }
        }
        if record.number_parameters > 0 {
            out.access_kind = record.exception_information[0];
        }
        if record.number_parameters > 1 {
            out.access_address = record.exception_information[1];
        }
        if !pointers.context_record.is_null() {
            let base = pointers.context_record as *const u8;
            let read_reg = |off: usize| unsafe { *(base.add(off) as *const u64) as usize };
            out.rax = read_reg(CONTEXT_RAX_OFFSET);
            out.rcx = read_reg(CONTEXT_RCX_OFFSET);
            out.rdx = read_reg(CONTEXT_RDX_OFFSET);
            out.rsp = read_reg(CONTEXT_RSP_OFFSET);
            out.rbp = read_reg(CONTEXT_RBP_OFFSET);
            out.r8 = read_reg(CONTEXT_R8_OFFSET);
            out.r9 = read_reg(CONTEXT_R9_OFFSET);
            out.rip = read_reg(CONTEXT_RIP_OFFSET);
        }
        Some(out)
    }
}

#[cfg(windows)]
fn exception_report(snapshot: &ExceptionSnapshot, reason: &str, repeat: usize) -> String {
    let mut out = String::new();
    use fmt::Write as _;
    let modules = loaded_modules();
    let class = record_class(snapshot.code);
    let _ = writeln!(out, "reason={reason}");
    let _ = writeln!(out, "module={}", config().module_label);
    let _ = writeln!(
        out,
        "record_index={}",
        RECORD_INDEX.fetch_add(1, Ordering::SeqCst)
    );
    let _ = writeln!(out, "utc={}", utc_timestamp());
    let _ = writeln!(out, "ms_since_install={}", ms_since_install());
    let _ = writeln!(
        out,
        "record_class={}",
        match class {
            RecordClass::Throw => "throw",
            RecordClass::Fault => "fault",
        }
    );
    // The one question a first-chance record cannot answer about itself.
    let _ = writeln!(out, "fatal={}", reason == "unhandled-exception-fatal");
    let _ = writeln!(out, "site_repeat_count={repeat}");
    let _ = writeln!(out, "exception_code=0x{:08x}", snapshot.code);
    let _ = writeln!(
        out,
        "exception_label={}",
        exception_code_label(snapshot.code)
    );
    let _ = writeln!(
        out,
        "exception_address=0x{:x}{}",
        snapshot.address,
        module_tag(snapshot.address, &modules)
    );
    let _ = writeln!(out, "exception_access_kind={}", snapshot.access_kind);
    let _ = writeln!(
        out,
        "exception_access_address=0x{:x}",
        snapshot.access_address
    );
    let _ = writeln!(
        out,
        "context_rip=0x{:x}{}",
        snapshot.rip,
        module_tag(snapshot.rip, &modules)
    );
    let _ = writeln!(out, "context_rsp=0x{:x}", snapshot.rsp);
    let _ = writeln!(out, "context_rbp=0x{:x}", snapshot.rbp);
    let _ = writeln!(out, "context_rax=0x{:x}", snapshot.rax);
    let _ = writeln!(out, "context_rcx=0x{:x}", snapshot.rcx);
    let _ = writeln!(out, "context_rdx=0x{:x}", snapshot.rdx);
    let _ = writeln!(out, "context_r8=0x{:x}", snapshot.r8);
    let _ = writeln!(out, "context_r9=0x{:x}", snapshot.r9);
    let _ = writeln!(out, "thread_id={}", snapshot.thread_id);
    write_common_fields(&mut out);
    if snapshot.code == EXCEPTION_CPP_THROW {
        write_cpp_throw_fields(&mut out, snapshot, &modules);
    }
    if snapshot.code == EXCEPTION_STACK_OVERFLOW {
        // Everything below here formats into deeper stack; on an overflow there is none to spend.
        let _ = writeln!(
            out,
            "stack_note=stack-overflow; skipped stack scan/backtrace"
        );
        return out;
    }
    let _ = writeln!(out, "site_histogram={}", exception_site_histogram(&modules));
    let _ = writeln!(out, "callers={}", capture_callers(&modules));
    let _ = writeln!(
        out,
        "stack_modules={}",
        scan_stack_modules(snapshot.rsp, &modules)
    );
    let _ = writeln!(out, "stack_raw={}", scan_stack_raw(snapshot.rsp, &modules));
    out
}

/// Whether an exception is worth a record at all.
///
/// The enumerated codes are the ones with a useful label; the severity test is the catch-all, and
/// it is what keeps the handler from writing a record for every informational or warning-severity
/// status the process raises in normal operation.
pub fn is_reportable_exception(code: u32) -> bool {
    matches!(
        code,
        EXCEPTION_ACCESS_VIOLATION
            | EXCEPTION_ILLEGAL_INSTRUCTION
            | EXCEPTION_STACK_BUFFER_OVERRUN
            | EXCEPTION_STACK_OVERFLOW
            | EXCEPTION_HEAP_CORRUPTION
            | EXCEPTION_IN_PAGE_ERROR
            | EXCEPTION_INT_DIVIDE_BY_ZERO
            | EXCEPTION_PRIVILEGED_INSTRUCTION
            | EXCEPTION_NONCONTINUABLE
            | EXCEPTION_CPP_THROW
    ) || (code & EXCEPTION_SEVERITY_MASK) == EXCEPTION_SEVERITY_MASK
}

pub fn exception_code_label(code: u32) -> &'static str {
    match code {
        EXCEPTION_ACCESS_VIOLATION => "STATUS_ACCESS_VIOLATION",
        EXCEPTION_ILLEGAL_INSTRUCTION => "STATUS_ILLEGAL_INSTRUCTION",
        EXCEPTION_STACK_BUFFER_OVERRUN => "STATUS_STACK_BUFFER_OVERRUN / fail-fast",
        EXCEPTION_STACK_OVERFLOW => "STATUS_STACK_OVERFLOW",
        EXCEPTION_HEAP_CORRUPTION => "STATUS_HEAP_CORRUPTION",
        EXCEPTION_IN_PAGE_ERROR => "STATUS_IN_PAGE_ERROR",
        EXCEPTION_INT_DIVIDE_BY_ZERO => "STATUS_INTEGER_DIVIDE_BY_ZERO",
        EXCEPTION_PRIVILEGED_INSTRUCTION => "STATUS_PRIVILEGED_INSTRUCTION",
        EXCEPTION_NONCONTINUABLE => "STATUS_NONCONTINUABLE_EXCEPTION",
        EXCEPTION_CPP_THROW => "C++/Rust throw",
        _ => "unclassified-error-severity-exception",
    }
}

/// Name the thrown C++ type and preview the thrown object.
///
/// Without this a `0xe06d7363` record says only "somebody threw something". With it the record
/// names the type -- `_com_error`, `std::bad_alloc`, a driver-internal class -- and that name is
/// the lead. The walk is pure MSVC C++ ABI and is the same in any process on any game.
#[cfg(windows)]
fn write_cpp_throw_fields(
    out: &mut String,
    snapshot: &ExceptionSnapshot,
    modules: &[LoadedModule],
) {
    use fmt::Write as _;
    if snapshot.param_count < CPP_EH_PARAMS_KEPT || snapshot.params[0] != CPP_EH_MAGIC {
        let _ = writeln!(
            out,
            "cpp_throw_note=not-a-decodable-msvc-throw params={} magic=0x{:x}",
            snapshot.param_count, snapshot.params[0]
        );
        return;
    }
    let object = snapshot.params[CPP_EH_PARAM_OBJECT];
    let throw_info = snapshot.params[CPP_EH_PARAM_THROW_INFO];
    let image_base = snapshot.params[CPP_EH_PARAM_MODULE_BASE];
    let _ = writeln!(
        out,
        "cpp_throw_object=0x{:x}{}",
        object,
        module_tag(object, modules)
    );
    let _ = writeln!(
        out,
        "cpp_throw_info=0x{:x}{}",
        throw_info,
        module_tag(throw_info, modules)
    );
    let _ = writeln!(
        out,
        "cpp_throw_image_base=0x{:x}{}",
        image_base,
        module_tag(image_base, modules)
    );
    match unsafe { read_thrown_type_name(throw_info, image_base) } {
        Some(raw) => {
            let _ = writeln!(out, "cpp_throw_type={}", format_thrown_type_name(&raw));
            let _ = writeln!(out, "cpp_throw_type_raw={raw}");
        }
        None => {
            let _ = writeln!(out, "cpp_throw_type=<unresolved>");
        }
    }
    let _ = writeln!(out, "cpp_throw_object_bytes={}", unsafe {
        read_hex_preview(object, THROWN_OBJECT_PREVIEW_BYTES)
    });
}

/// Walk `ThrowInfo -> CatchableTypeArray -> CatchableType -> TypeDescriptor` and read the name.
///
/// Only the first catchable type is read: for `throw Derived` that is `Derived` itself, the rest
/// being the base classes it could also be caught as.
#[cfg(windows)]
unsafe fn read_thrown_type_name(throw_info: usize, image_base: usize) -> Option<String> {
    if throw_info < MIN_VALID_PTR || image_base < MIN_VALID_PTR {
        return None;
    }
    let array_rva = unsafe { safe_read_u32(throw_info + THROW_INFO_CATCHABLE_ARRAY_OFFSET) }?;
    if array_rva == 0 {
        return None;
    }
    let array = image_base + array_rva as usize;
    let count = unsafe { safe_read_u32(array) }?;
    if count == 0 {
        return None;
    }
    let first_rva = unsafe { safe_read_u32(array + CATCHABLE_ARRAY_FIRST_TYPE_OFFSET) }?;
    if first_rva == 0 {
        return None;
    }
    let catchable = image_base + first_rva as usize;
    let descriptor_rva = unsafe { safe_read_u32(catchable + CATCHABLE_TYPE_DESCRIPTOR_OFFSET) }?;
    if descriptor_rva == 0 {
        return None;
    }
    let name_addr = image_base + descriptor_rva as usize + TYPE_DESCRIPTOR_NAME_OFFSET;
    // Bounded and fault-safe by hand rather than through `ds2-game-base::mem::safe_read_cstr`,
    // which is the obvious substrate call and the wrong contract here: it treats a region with no
    // NUL inside the bound as unreadable and returns nothing, because its callers go on to USE the
    // string. This one only PRINTS it, and a truncated RTTI name is still the lead -- returning
    // `<unresolved>` for a long template name would throw away the only evidence in the record.
    let mut bytes = Vec::with_capacity(TYPE_DESCRIPTOR_NAME_MAX);
    for index in 0..TYPE_DESCRIPTOR_NAME_MAX {
        match unsafe { ds2_game_base::mem::safe_read_u8(name_addr + index) } {
            Some(0) | None => break,
            Some(byte) => bytes.push(byte),
        }
    }
    if bytes.is_empty() {
        return None;
    }
    // RTTI type names are ASCII by construction; anything else means we walked off target and the
    // raw bytes are more honest than a substituted replacement character.
    String::from_utf8(bytes).ok()
}

#[cfg(windows)]
unsafe fn read_hex_preview(addr: usize, len: usize) -> String {
    if addr < MIN_VALID_PTR {
        return String::from("<null>");
    }
    let mut out = String::with_capacity(len * 2);
    for index in 0..len {
        match unsafe { ds2_game_base::mem::safe_read_u8(addr + index) } {
            Some(byte) => {
                let _ = fmt::Write::write_fmt(&mut out, format_args!("{byte:02x}"));
            }
            None => {
                out.push_str("??");
                break;
            }
        }
    }
    out
}

#[cfg(windows)]
pub(crate) fn utc_timestamp() -> String {
    let mut now = SystemTimeMin {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    unsafe { GetSystemTime(&mut now) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        now.year, now.month, now.day, now.hour, now.minute, now.second, now.milliseconds
    )
}

#[cfg(windows)]
pub(crate) fn ms_since_install() -> u64 {
    INSTALL_INSTANT
        .get()
        .map(|start| start.elapsed().as_millis() as u64)
        .unwrap_or(0)
}

/// Write the full loaded-module table: name, base, size, and PE watermark.
///
/// The COFF `TimeDateStamp` identifies the exact build of each image, so a third-party offset in
/// a crash record stays resolvable even after the driver or overlay updates.
#[cfg(windows)]
fn write_module_inventory(reason: &str) {
    use fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "reason=module-inventory-{reason}");
    let _ = writeln!(out, "module={}", config().module_label);
    let _ = writeln!(out, "utc={}", utc_timestamp());
    let _ = writeln!(out, "ms_since_install={}", ms_since_install());
    let modules = loaded_modules();
    let _ = writeln!(out, "module_count={}", modules.len());
    for module in &modules {
        let stamp = unsafe { pe_timedatestamp(module.base) }.unwrap_or(0);
        let _ = writeln!(
            out,
            "module_entry base=0x{:x} size=0x{:x} pe_watermark=0x{:08x} name={}",
            module.base, module.size, stamp, module.name
        );
    }
    let _ = fs::write(path_for(config().modules_file_name), out);
}

/// Write a postmortem minidump next to the text log.
///
/// The text record is a summary of one thread; the dump carries every thread's stack, the full
/// module list with versions, and the memory those stacks reference. `dbghelp` is resolved lazily
/// so nothing is linked or loaded until the process is already dying.
#[cfg(windows)]
pub(crate) unsafe fn write_minidump_named(file_name: &str, info: *mut ExceptionPointersMin) {
    #[repr(C)]
    struct MinidumpExceptionInformation {
        thread_id: u32,
        exception_pointers: *mut ExceptionPointersMin,
        client_pointers: i32,
    }
    type MiniDumpWriteDumpFn = unsafe extern "system" fn(
        process: isize,
        process_id: u32,
        file: isize,
        dump_type: u32,
        exception_param: *const MinidumpExceptionInformation,
        user_stream_param: *const c_void,
        callback_param: *const c_void,
    ) -> i32;

    let dbghelp = unsafe { LoadLibraryA(c"dbghelp.dll".as_ptr().cast()) };
    if dbghelp.is_null() {
        return;
    }
    let entry = unsafe { GetProcAddress(dbghelp, c"MiniDumpWriteDump".as_ptr().cast()) };
    if entry.is_null() {
        return;
    }
    let write_dump: MiniDumpWriteDumpFn = unsafe { std::mem::transmute(entry) };

    let path = path_for(file_name);
    let mut wide: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    wide.push(0);
    let file = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null_mut(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if file == INVALID_HANDLE_VALUE {
        append_log(format_args!(
            "minidump create failed last_error={} path={}",
            unsafe { GetLastError() },
            path.display()
        ));
        return;
    }
    let exception = MinidumpExceptionInformation {
        thread_id: unsafe { GetCurrentThreadId() },
        exception_pointers: info,
        client_pointers: 0,
    };
    // A null `info` would mean nobody faulted. `MiniDumpWriteDump` accepts a null exception
    // parameter and captures every thread anyway, so the null case stays supported.
    let exception_param = if info.is_null() {
        std::ptr::null()
    } else {
        &exception as *const MinidumpExceptionInformation
    };
    let rich_type = MINIDUMP_WITH_UNLOADED_MODULES
        | MINIDUMP_WITH_INDIRECTLY_REFERENCED_MEMORY
        | MINIDUMP_WITH_PROCESS_THREAD_DATA
        | MINIDUMP_WITH_THREAD_INFO;
    let mut dump_type = rich_type;
    let mut ok = unsafe {
        write_dump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            file,
            dump_type,
            exception_param,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    let rich_error = unsafe { GetLastError() };
    if ok == 0 {
        // The rich flags walk far more of the process than the minimal dump does, and any one of
        // them can be refused -- by a partial `dbghelp` implementation, or by a process too far
        // gone to enumerate. A bare thread-and-module dump is worth vastly more than no dump, so
        // never let the expensive attempt be the only one.
        dump_type = MINIDUMP_NORMAL;
        ok = unsafe {
            write_dump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                file,
                dump_type,
                exception_param,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
    }
    let normal_error = unsafe { GetLastError() };
    if ok == 0 && !exception_param.is_null() {
        // Last tier: no exception parameter at all. This tier exists because of a measurement in
        // `../er-mods-rs`: Proton's `dbghelp` failed BOTH tiers above with `ERROR_NOACCESS` (998),
        // raised while it dereferenced the `MINIDUMP_EXCEPTION_INFORMATION` it was handed -- so
        // dropping that struct is the one variable left to change. Everything in this workspace
        // runs under Proton too, so keep it. The dump loses the faulting thread's registers, which
        // the text record already carries, and keeps every thread's stack, which nothing else does.
        ok = unsafe {
            write_dump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                file,
                dump_type,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
    }
    let final_error = unsafe { GetLastError() };
    unsafe { CloseHandle(file) };
    // `CreateFileW` above already created the file, so a failed dump leaves a 0-byte `.dmp` on
    // disk. That empty file is worse than no file: it is indistinguishable from a captured dump
    // until someone opens it, and users dutifully collect and send it. Delete it, so the ONLY
    // `.dmp` that ever exists is one with a dump in it.
    if ok == 0 {
        let removed = unsafe { DeleteFileW(wide.as_ptr()) } != 0;
        append_log(format_args!(
            "minidump write FAILED -- no dump exists. rich_last_error={rich_error} \
             normal_last_error={normal_error} last_error={final_error} \
             empty_file_deleted={removed} path={}",
            path.display()
        ));
        return;
    }
    append_log(format_args!(
        "minidump write ok=true type=0x{:x} rich_last_error={} last_error={} path={}",
        dump_type,
        rich_error,
        final_error,
        path.display()
    ));
}

#[cfg(windows)]
fn phase_label(phase: usize) -> &'static str {
    match phase {
        0 => Phase::Uninitialized.label(),
        1 => Phase::DllAttach.label(),
        2 => Phase::HandlerInstalled.label(),
        10 => Phase::HostReady.label(),
        90 => Phase::ExceptionObserved.label(),
        _ => "unknown",
    }
}

#[cfg(windows)]
fn capture_callers(modules: &[LoadedModule]) -> String {
    let mut frames = [std::ptr::null_mut::<c_void>(); STACK_TRACE_FRAME_COUNT];
    let captured = unsafe {
        RtlCaptureStackBackTrace(
            STACK_TRACE_FRAMES_TO_SKIP,
            frames.len() as u32,
            frames.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    } as usize;
    let mut out = String::from("[");
    for (index, frame) in frames.iter().take(captured).enumerate() {
        if index != 0 {
            out.push(',');
        }
        let addr = *frame as usize;
        out.push_str(&format!(
            "#{}=0x{:x}{}",
            index,
            addr,
            module_tag(addr, modules)
        ));
    }
    out.push(']');
    out
}

#[cfg(windows)]
fn scan_stack_modules(rsp: usize, modules: &[LoadedModule]) -> String {
    let mut out = String::from("[");
    if rsp < MIN_VALID_PTR {
        out.push(']');
        return out;
    }
    let mut emitted = 0usize;
    let mut last = String::new();
    for slot in 0..STACK_SCAN_SLOTS {
        if emitted >= STACK_SCAN_MAX_FRAMES {
            break;
        }
        let Some(value) = (unsafe {
            ds2_game_base::mem::safe_read_usize(rsp + slot * std::mem::size_of::<usize>())
        }) else {
            continue;
        };
        if let Some((name, offset)) = module_for_addr(value, modules) {
            let frame = format!("{name}+0x{offset:x}");
            if frame != last {
                if emitted != 0 {
                    out.push(',');
                }
                out.push_str(&frame);
                emitted += 1;
                last = frame;
            }
        }
    }
    out.push(']');
    out
}

#[cfg(windows)]
fn scan_stack_raw(rsp: usize, modules: &[LoadedModule]) -> String {
    let mut out = String::from("[");
    for i in 0..STACK_RAW_QWORDS {
        if i != 0 {
            out.push(',');
        }
        match unsafe { ds2_game_base::mem::safe_read_usize(rsp + i * std::mem::size_of::<usize>()) }
        {
            Some(value) => out.push_str(&format!("0x{:x}{}", value, module_tag(value, modules))),
            None => out.push_str("??"),
        }
    }
    out.push(']');
    out
}

/// One entry of the loader's own module table: where an image is mapped, how far it spans, and
/// what it is called.
///
/// This -- and not a table of addresses -- is how a fault gets symbolized here. The loader
/// maintains it for its own purposes and it is correct for whatever build is actually running, so
/// `ds2-rva` is not involved and does not need to be.
#[cfg(any(windows, test))]
#[derive(Clone, Debug)]
pub(crate) struct LoadedModule {
    pub(crate) base: usize,
    pub(crate) size: usize,
    pub(crate) name: String,
}

/// `{name+0xoffset}` when `addr` falls inside a loaded module, empty otherwise.
#[cfg(any(windows, test))]
pub(crate) fn module_tag(addr: usize, modules: &[LoadedModule]) -> String {
    module_for_addr(addr, modules)
        .map(|(name, offset)| format!("{{{name}+0x{offset:x}}}"))
        .unwrap_or_default()
}

/// Resolve `addr` to `(module name, offset into that module)`.
///
/// The bound is the module's OWN `SizeOfImage`, taken per entry from the loader's table. Compare
/// `er-game-base::mem::vtable_in_game_image`, which bounded every pointer against a hardcoded
/// `0x3000000` -- an Elden Ring image span, ~19 MB larger than this game's, and therefore a test
/// that would have called addresses past the end of the image "inside" it. Nothing here is
/// hardcoded, so nothing here goes stale when a module updates.
#[cfg(any(windows, test))]
fn module_for_addr(addr: usize, modules: &[LoadedModule]) -> Option<(&str, usize)> {
    modules.iter().find_map(|module| {
        addr.checked_sub(module.base)
            .filter(|offset| *offset < module.size)
            .map(|offset| (module.name.as_str(), offset))
    })
}

// PEB / `LDR_DATA_TABLE_ENTRY` walk. These are Windows x64 internals, documented by the debugger's
// own type information and stable across the OS versions this runs on. Nothing below is a fact
// about the game.
#[cfg(windows)]
const PEB_LDR_OFFSET: usize = 0x18;
#[cfg(windows)]
const PEB_LDR_IN_MEMORY_ORDER_LIST_OFFSET: usize = 0x20;
#[cfg(windows)]
const LDR_ENTRY_IN_MEMORY_ORDER_LINKS_OFFSET: usize = 0x10;
#[cfg(windows)]
const LDR_ENTRY_DLL_BASE_OFFSET: usize = 0x30;
#[cfg(windows)]
const LDR_ENTRY_SIZE_OF_IMAGE_OFFSET: usize = 0x40;
#[cfg(windows)]
const LDR_ENTRY_BASE_DLL_NAME_OFFSET: usize = 0x58;
#[cfg(windows)]
const UNICODE_STRING_BUFFER_OFFSET: usize = 0x8;
#[cfg(windows)]
const MODULE_WALK_MAX: usize = 512;
#[cfg(windows)]
const MODULE_NAME_MAX_WCHARS: usize = 260;

#[cfg(all(windows, target_arch = "x86_64"))]
#[inline(always)]
fn current_peb() -> usize {
    let peb: usize;
    unsafe {
        core::arch::asm!(
            "mov {peb}, gs:[0x60]",
            peb = out(reg) peb,
            options(nostack, preserves_flags),
        );
    }
    peb
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) fn loaded_modules() -> Vec<LoadedModule> {
    let mut modules = Vec::new();
    let peb = current_peb();
    if peb < MIN_VALID_PTR {
        return modules;
    }
    let Some(ldr) = (unsafe { ds2_game_base::mem::safe_read_usize(peb + PEB_LDR_OFFSET) }) else {
        return modules;
    };
    let list_head = ldr + PEB_LDR_IN_MEMORY_ORDER_LIST_OFFSET;
    let Some(mut node) = (unsafe { ds2_game_base::mem::safe_read_usize(list_head) }) else {
        return modules;
    };
    let mut count = 0usize;
    while node != list_head && node >= MIN_VALID_PTR && count < MODULE_WALK_MAX {
        let entry = node - LDR_ENTRY_IN_MEMORY_ORDER_LINKS_OFFSET;
        let base =
            unsafe { ds2_game_base::mem::safe_read_usize(entry + LDR_ENTRY_DLL_BASE_OFFSET) }
                .unwrap_or(0);
        // `SizeOfImage` is a `ULONG` in the entry, so only the low half of the qword read is the
        // span; the high half is the next field.
        let size =
            unsafe { safe_read_u32(entry + LDR_ENTRY_SIZE_OF_IMAGE_OFFSET) }.unwrap_or(0) as usize;
        if base != 0 && size != 0 {
            modules.push(LoadedModule {
                base,
                size,
                name: read_module_base_name(entry),
            });
        }
        let Some(next) = (unsafe { ds2_game_base::mem::safe_read_usize(node) }) else {
            break;
        };
        if next == node {
            break;
        }
        node = next;
        count += 1;
    }
    modules
}

#[cfg(all(windows, not(target_arch = "x86_64")))]
pub(crate) fn loaded_modules() -> Vec<LoadedModule> {
    Vec::new()
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn read_module_base_name(entry: usize) -> String {
    let name_field = entry + LDR_ENTRY_BASE_DLL_NAME_OFFSET;
    let len_bytes = unsafe { ds2_game_base::mem::safe_read_u16(name_field) }.unwrap_or(0) as usize;
    let Some(buffer) =
        (unsafe { ds2_game_base::mem::safe_read_usize(name_field + UNICODE_STRING_BUFFER_OFFSET) })
    else {
        return String::new();
    };
    if buffer < MIN_VALID_PTR || len_bytes == 0 {
        return String::new();
    }
    let wchars = (len_bytes / std::mem::size_of::<u16>()).min(MODULE_NAME_MAX_WCHARS);
    let mut units = Vec::with_capacity(wchars);
    for index in 0..wchars {
        match unsafe {
            ds2_game_base::mem::safe_read_u16(buffer + index * std::mem::size_of::<u16>())
        } {
            Some(unit) => units.push(unit),
            None => break,
        }
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(base: usize, size: usize, name: &str) -> LoadedModule {
        LoadedModule {
            base,
            size,
            name: name.to_string(),
        }
    }

    #[test]
    fn defaults_are_standalone_ship_names() {
        let cfg = CrashLogConfig::default();
        assert_eq!(cfg.log_file_name, DEFAULT_LOG_FILE);
        assert_eq!(cfg.latest_file_name, DEFAULT_LATEST_FILE);
        assert_eq!(cfg.breadcrumb_file_name, DEFAULT_BREADCRUMB_FILE);
        assert_eq!(cfg.modules_file_name, DEFAULT_MODULES_FILE);
        assert_eq!(cfg.minidump_file_name, DEFAULT_MINIDUMP_FILE);
        assert_eq!(cfg.module_label, DEFAULT_MODULE_LABEL);
    }

    /// The port must not have left an Elden Ring name in a default that lands on a player's disk.
    #[test]
    fn no_default_name_belongs_to_the_other_game() {
        let cfg = CrashLogConfig::default();
        for name in [
            cfg.log_file_name,
            cfg.latest_file_name,
            cfg.breadcrumb_file_name,
            cfg.modules_file_name,
            cfg.minidump_file_name,
            cfg.module_label,
        ] {
            assert!(
                name.starts_with("ds2-"),
                "{name} is not a name this workspace ships"
            );
        }
    }

    #[test]
    fn phase_labels_are_stable() {
        assert_eq!(Phase::DllAttach.label(), "dll-attach");
        assert_eq!(Phase::HostReady.label(), "host-ready");
        assert_eq!(Phase::ExceptionObserved.label(), "exception-observed");
    }

    #[test]
    fn cpp_throws_are_classified_apart_from_faults() {
        assert_eq!(record_class(EXCEPTION_CPP_THROW), RecordClass::Throw);
        assert_eq!(record_class(EXCEPTION_ACCESS_VIOLATION), RecordClass::Fault);
        assert_eq!(record_class(EXCEPTION_HEAP_CORRUPTION), RecordClass::Fault);
    }

    /// The failure the two budgets exist to prevent: a render thread throwing C++ exceptions as
    /// routine control flow must not be able to consume the records reserved for a real fault.
    #[test]
    fn exhausted_throw_budget_leaves_fault_budget_intact() {
        let throws = MAX_THROW_RECORDS * 100;
        assert!(!budget_allows(RecordClass::Throw, throws, 0));
        assert!(budget_allows(RecordClass::Fault, throws, 0));
        assert!(budget_allows(
            RecordClass::Fault,
            throws,
            MAX_FAULT_RECORDS - 1
        ));
        assert!(!budget_allows(
            RecordClass::Fault,
            throws,
            MAX_FAULT_RECORDS
        ));
    }

    #[test]
    fn thrown_type_names_are_readable() {
        assert_eq!(format_thrown_type_name(".?AV_com_error@@"), "_com_error");
        assert_eq!(
            format_thrown_type_name(".?AVbad_alloc@std@@"),
            "std::bad_alloc"
        );
        assert_eq!(
            format_thrown_type_name(".PEAVDeviceRemoved@nv@@"),
            "nv::DeviceRemoved"
        );
    }

    /// An unparseable name is still evidence; never mangle it further.
    #[test]
    fn unrecognized_type_names_pass_through_untouched() {
        assert_eq!(format_thrown_type_name("garbage"), "garbage");
        assert_eq!(
            format_thrown_type_name(".?AVunterminated"),
            ".?AVunterminated"
        );
        assert_eq!(format_thrown_type_name(".?AV@@"), ".?AV@@");
    }

    #[test]
    fn reportable_set_covers_cpp_throw_and_error_severity() {
        assert!(is_reportable_exception(EXCEPTION_CPP_THROW));
        assert!(is_reportable_exception(EXCEPTION_ACCESS_VIOLATION));
        // Error-severity codes we have not enumerated still report.
        assert!(is_reportable_exception(0xc000_0fff));
        // Informational/warning severities do not.
        assert!(!is_reportable_exception(0x4000_0001));
    }

    #[test]
    fn cpp_throw_has_a_distinct_label() {
        assert_eq!(exception_code_label(EXCEPTION_CPP_THROW), "C++/Rust throw");
        assert_eq!(
            exception_code_label(EXCEPTION_ACCESS_VIOLATION),
            "STATUS_ACCESS_VIOLATION"
        );
    }

    /// Symbolizing a fault is this function and nothing else -- no address table, no image-span
    /// constant. An address inside a module resolves to that module and its offset.
    #[test]
    fn an_address_resolves_to_its_module_and_offset() {
        let modules = [
            module(0x1_4000_0000, 0x01d7_6000, "DarkSoulsII.exe"),
            module(0x7ffb_0000_0000, 0x0002_0000, "ds2_crash_logging.dll"),
        ];
        assert_eq!(
            module_for_addr(0x1_4000_1234, &modules),
            Some(("DarkSoulsII.exe", 0x1234))
        );
        assert_eq!(
            module_tag(0x7ffb_0000_0010, &modules),
            "{ds2_crash_logging.dll+0x10}"
        );
    }

    /// THE bug the ported `vtable_in_game_image` would have shipped: a span borrowed from a bigger
    /// image accepts addresses past the end of a smaller one. Each entry's own `SizeOfImage` is
    /// the bound, so one byte past the end is outside and says so.
    #[test]
    fn an_address_past_the_end_of_a_module_belongs_to_no_module() {
        let modules = [module(0x1_4000_0000, 0x01d7_6000, "DarkSoulsII.exe")];
        // Last mapped byte is in; the first byte past the span is not.
        assert!(module_for_addr(0x1_4000_0000 + 0x01d7_5fff, &modules).is_some());
        assert!(module_for_addr(0x1_4000_0000 + 0x01d7_6000, &modules).is_none());
        // An Elden Ring-sized span would have swallowed this address whole.
        assert!(module_for_addr(0x1_4000_0000 + 0x02ff_ffff, &modules).is_none());
        assert_eq!(module_tag(0x1_4000_0000 + 0x02ff_ffff, &modules), "");
    }

    /// An address below a module's base must not wrap into a colossal offset.
    #[test]
    fn an_address_below_every_module_resolves_to_nothing() {
        let modules = [module(0x1_4000_0000, 0x01d7_6000, "DarkSoulsII.exe")];
        assert!(module_for_addr(0, &modules).is_none());
        assert!(module_for_addr(MIN_VALID_PTR, &modules).is_none());
        assert!(module_for_addr(0x1_3fff_ffff, &modules).is_none());
    }
}
