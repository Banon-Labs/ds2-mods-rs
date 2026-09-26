//! A sampling profile of the boot thread, from the entry point to the first substate.
//!
//! # Why
//!
//! Between `DirectInput8Create` returning and the first substate the boot thread spends about
//! three seconds, and the blocking-call accounting in `install` found it inside `Sleep` and the
//! wait imports for only about 0.6 s of them. The rest is the thread running, and a count of
//! calls cannot say where. A sampler can: every millisecond it stops the boot thread, reads its
//! instruction pointer, and lets it go, and the pointers are bucketed by the function they fall
//! in afterwards.
//!
//! # Why it cannot deadlock the thread it stops
//!
//! Between `SuspendThread` and `ResumeThread` this does exactly one thing, `GetThreadContext`
//! into a buffer allocated before sampling started, and pushes into a `Vec` whose capacity was
//! reserved up front. Nothing in that window allocates, logs or takes a lock the stopped thread
//! could be holding. Every lookup -- which module, which function -- happens after the last
//! sample, on this thread's own time.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::LOG_PREFIX;
use crate::install::log;

unsafe extern "system" {
    fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> *mut c_void;
    fn SuspendThread(thread: *mut c_void) -> u32;
    fn ResumeThread(thread: *mut c_void) -> u32;
    fn GetThreadContext(thread: *mut c_void, context: *mut c_void) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn RtlLookupFunctionEntry(
        pc: u64,
        image_base: *mut u64,
        history: *mut c_void,
    ) -> *const RuntimeFunction;
    fn RtlPcToFileHeader(pc: *mut c_void, base: *mut *mut c_void) -> *mut c_void;
    fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
}

/// `RUNTIME_FUNCTION`: one `.pdata` entry, RVAs of a function's start and end and its unwind info.
#[repr(C)]
struct RuntimeFunction {
    begin: u32,
    end: u32,
    unwind: u32,
}

const THREAD_SUSPEND_RESUME: u32 = 0x0002;
const THREAD_GET_CONTEXT: u32 = 0x0008;
/// `CONTEXT_AMD64 | CONTEXT_CONTROL`: enough for `Rip`, nothing more to fill.
const CONTEXT_CONTROL: u32 = 0x0010_0001;
/// `CONTEXT.ContextFlags` and `CONTEXT.Rip` in the x64 `CONTEXT`, whose size is `0x4d0`.
const CONTEXT_FLAGS_OFFSET: usize = 0x30;
const CONTEXT_RIP_OFFSET: usize = 0xf8;
/// `CONTEXT.Rsp`, filled by the same `CONTEXT_CONTROL`.
const CONTEXT_RSP_OFFSET: usize = 0x98;
const CONTEXT_SIZE: usize = 0x4d0;
// How much of the stopped thread's stack is copied per sample, in `u64`s (2 KiB).
const STACK_WORDS: usize = 256;
/// `SuspendThread`'s failure value.
const SUSPEND_FAILED: u32 = u32::MAX;

/// The x64 `CONTEXT` must be 16-byte aligned for `GetThreadContext`.
#[repr(C, align(16))]
struct Context([u8; CONTEXT_SIZE]);

/// Time between samples. Five milliseconds, not one: a run at one millisecond took the boot from
/// about 4.2 s to 10.5 s, because under Wine every suspend and resume is a signal round trip that
/// the stopped thread pays for.
const INTERVAL: Duration = Duration::from_millis(5);
/// A cap, so a boot that never reaches a substate stops sampling on its own: twenty seconds.
const MAX_SAMPLES: usize = 4_000;
/// Rows reported, most-sampled first.
const REPORTED: usize = 20;

static STOP: AtomicBool = AtomicBool::new(false);

/// Stop sampling. Called when the first substate is entered; the report follows on the sampler's
/// own thread.
pub(crate) fn stop() {
    STOP.store(true, Ordering::Relaxed);
}

/// The calling thread's stack base (the highest address of its stack), from its TEB.
///
/// `NT_TIB.StackBase` is at `gs:[0x08]` on x64. Called by `install` on the boot thread itself, so
/// the sampler knows how far above the stopped thread's `Rsp` it may copy without leaving its
/// stack -- a plain copy, because `ReadProcessMemory` could wait on a lock the stopped thread
/// holds.
pub(crate) fn current_stack_base() -> u64 {
    let base: u64;
    // SAFETY: reads one pointer out of this thread's own TEB, which is always mapped.
    unsafe {
        core::arch::asm!("mov {}, gs:[0x08]", out(reg) base, options(nostack, readonly, preserves_flags));
    }
    base
}

/// Start sampling `thread_id`, whose stack ends at `stack_base`, on a thread of its own.
pub(crate) fn start(thread_id: u32, stack_base: u64) {
    let spawned = std::thread::Builder::new()
        .name("ds2-boot-sampler".to_string())
        .spawn(move || run(thread_id, stack_base));
    if let Err(error) = spawned {
        log(format_args!(
            "{LOG_PREFIX} sampler-failed stage=spawn error={error}"
        ));
    }
}

fn context_u64(context: &Context, offset: usize) -> u64 {
    u64::from_le_bytes(context.0[offset..][..8].try_into().unwrap_or([0; 8]))
}

fn run(thread_id: u32, stack_base: u64) {
    // SAFETY: plain handle request for a thread id this process owns; null on failure.
    let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT, 0, thread_id) };
    if handle.is_null() {
        log(format_args!(
            "{LOG_PREFIX} sampler-failed stage=OpenThread thread={thread_id}"
        ));
        return;
    }
    // The helper answers `(start, len)`; the scan wants `(start, end)`.
    let text =
        ds2_game_base::mem::module_text_range().map_or((0, 0), |(start, len)| (start, start + len));
    // All three allocated before the first suspend, and never again while one is in force.
    let mut samples: Vec<(u64, u64)> = Vec::with_capacity(MAX_SAMPLES);
    let mut context = Box::new(Context([0; CONTEXT_SIZE]));
    let mut stack = Box::new([0u64; STACK_WORDS]);
    let mut failed = 0u64;
    let mut no_stack = 0u64;
    let mut first_rsp = 0u64;
    while !STOP.load(Ordering::Relaxed) && samples.len() < MAX_SAMPLES {
        std::thread::sleep(INTERVAL);
        context.0[CONTEXT_FLAGS_OFFSET..][..4].copy_from_slice(&CONTEXT_CONTROL.to_le_bytes());
        // SAFETY: `handle` was opened with suspend rights and is closed only after the loop.
        if unsafe { SuspendThread(handle) } == SUSPEND_FAILED {
            failed += 1;
            continue;
        }
        // SAFETY: the buffer is a 16-byte-aligned x64 `CONTEXT` with its flags set, and the thread
        // is suspended, which `GetThreadContext` requires for a coherent read.
        let read = unsafe { GetThreadContext(handle, context.0.as_mut_ptr().cast()) } != 0;
        let mut words = 0;
        if read {
            let rsp = context_u64(&context, CONTEXT_RSP_OFFSET);
            if rsp != 0 && rsp < stack_base && rsp.is_multiple_of(8) {
                words = (((stack_base - rsp) / 8) as usize).min(STACK_WORDS);
                // SAFETY: `rsp..rsp + words*8` lies inside the stopped thread's own committed stack
                // (below its base from the TEB), which cannot change while it is suspended; a plain
                // copy takes no lock.
                unsafe {
                    std::ptr::copy_nonoverlapping(rsp as *const u64, stack.as_mut_ptr(), words);
                }
            }
        }
        // SAFETY: balances the successful suspend above.
        unsafe { ResumeThread(handle) };
        if read {
            if first_rsp == 0 {
                first_rsp = context_u64(&context, CONTEXT_RSP_OFFSET);
            }
            if words == 0 {
                no_stack += 1;
            }
            let caller = first_game_return(&stack[..words], text);
            samples.push((context_u64(&context, CONTEXT_RIP_OFFSET), caller));
        } else {
            failed += 1;
        }
    }
    // SAFETY: the handle opened above, closed once.
    unsafe { CloseHandle(handle) };
    // Why a caller can be missing: no stack was copied at all (the stopped `Rsp` was outside the
    // bounds below), or a stack was copied and held no return address into the game's `.text`.
    log(format_args!(
        "{LOG_PREFIX} sampler stack first-rsp=0x{first_rsp:016x} stack-base=0x{stack_base:016x} \
         text=0x{:016x}..0x{:016x} no-stack-copied={no_stack}",
        text.0, text.1
    ));
    report(&samples, failed);
}

/// The first word on a copied stack that is a return address into the game's `.text`: a value in
/// range whose preceding bytes are a `call`. Zero when there is none.
///
/// The call check is what keeps a stray pointer into the image from counting: `e8 rel32` five
/// bytes back, or an `ff /2` indirect call two, three or six bytes back.
fn first_game_return(stack: &[u64], (text_start, text_end): (usize, usize)) -> u64 {
    for &value in stack {
        let address = value as usize;
        if address < text_start + 6 || address >= text_end {
            continue;
        }
        // SAFETY: `address - 6 .. address` lies inside the game's mapped `.text`.
        let before = unsafe { std::slice::from_raw_parts((address - 6) as *const u8, 6) };
        let direct = before[1] == 0xe8;
        let indirect = [0usize, 3, 4]
            .iter()
            .any(|&at| before[at] == 0xff && (before[at + 1] >> 3) & 7 == 2);
        if direct || indirect {
            return value;
        }
    }
    0
}

/// Where a sample landed: a function of the game image by its start RVA, or a whole module.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Place {
    GameFunction(u32),
    GameUnwound,
    Module(u64),
    Unknown,
}

fn place(pc: u64, game_base: u64) -> Place {
    let mut module: *mut c_void = std::ptr::null_mut();
    // SAFETY: a lookup on an address; returns null for one in no loaded image.
    unsafe { RtlPcToFileHeader(pc as *mut c_void, &raw mut module) };
    if module.is_null() {
        return Place::Unknown;
    }
    if module as u64 != game_base {
        return Place::Module(module as u64);
    }
    let mut image_base = 0u64;
    // SAFETY: a `.pdata` lookup for an address inside the game image; null when no entry covers it.
    let entry = unsafe { RtlLookupFunctionEntry(pc, &raw mut image_base, std::ptr::null_mut()) };
    if entry.is_null() {
        return Place::GameUnwound;
    }
    // SAFETY: non-null entries point into the image's own `.pdata`.
    Place::GameFunction(unsafe { (*entry).begin })
}

fn module_name(base: u64) -> String {
    let mut buffer = [0u16; 260];
    // SAFETY: the buffer and its length in `u16`s; the module is one `RtlPcToFileHeader` found.
    let len = unsafe {
        GetModuleFileNameW(
            base as *mut c_void,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    let full = String::from_utf16_lossy(&buffer[..len.min(buffer.len())]);
    full.rsplit(['\\', '/']).next().unwrap_or("?").to_string()
}

fn describe(where_: Place) -> String {
    match where_ {
        Place::GameFunction(rva) => format!("DarkSoulsII.exe fn=0x{rva:08x}"),
        Place::GameUnwound => "DarkSoulsII.exe no-pdata-entry".to_string(),
        Place::Module(base) => format!("module={}", module_name(base)),
        Place::Unknown => "no-module".to_string(),
    }
}

/// Log one ranked table: `label` names it, `total` is what the shares are a share of.
fn log_table(label: &str, counts: HashMap<Place, u64>, total: u64) {
    let mut rows: Vec<(Place, u64)> = counts.into_iter().collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.1));
    for (rank, (where_, samples)) in rows.into_iter().take(REPORTED).enumerate() {
        let share = if total == 0 {
            0.0
        } else {
            samples as f64 * 100.0 / total as f64
        };
        log(format_args!(
            "{LOG_PREFIX} sampler {label} rank={} samples={samples} share={share:.1}% {}",
            rank + 1,
            describe(where_)
        ));
    }
}

/// Two tables. `at` is where the instruction pointer was. `waiting-in` is, for samples whose
/// instruction pointer was outside the game image, the game function nearest the top of the stack
/// -- the game code that called into the system and was waiting for it to return.
fn report(samples: &[(u64, u64)], failed: u64) {
    let game_base = ds2_game_base::mem::game_module_base().unwrap_or(0) as u64;
    let mut at: HashMap<Place, u64> = HashMap::new();
    let mut waiting_in: HashMap<Place, u64> = HashMap::new();
    let mut outside = 0u64;
    for &(pc, caller) in samples {
        let here = place(pc, game_base);
        *at.entry(here).or_default() += 1;
        let in_game = matches!(here, Place::GameFunction(_) | Place::GameUnwound);
        if !in_game {
            outside += 1;
            let from = if caller == 0 {
                Place::Unknown
            } else {
                place(caller, game_base)
            };
            *waiting_in.entry(from).or_default() += 1;
        }
    }
    let total = samples.len() as u64;
    log(format_args!(
        "{LOG_PREFIX} sampler samples={total} failed={failed} interval-ms={} outside-game={outside}",
        INTERVAL.as_millis()
    ));
    log_table("at", at, total);
    log_table("waiting-in", waiting_in, outside);
}
