//! The two detours, the clock, and the one-resident-at-a-time bookkeeping between them.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
}

/// `t = 0` for every timestamp in this log.
///
/// Set from the loader's `DllMain`, which is the earliest instant this DLL can observe -- a
/// statically imported DLL's `DLL_PROCESS_ATTACH` runs during import resolution, before
/// `DarkSoulsII.exe`'s entry point. That matters more than it looks: the gap between this instant
/// and the FIRST substate line is the engine bringing up D3D11, mounting archives and starting
/// audio, and under Proton it may well be larger than everything the title flow does afterwards.
/// A timeline that starts at the first substate would hide the biggest number on the page.
static ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Record `t = 0`. Call from `DllMain`; calling it later just moves the origin later.
///
/// Safe to call under the loader lock: it reads a performance counter and stores it. It allocates
/// nothing, takes no lock of its own beyond the `OnceLock`, and calls into no other module.
pub fn mark_origin() {
    let _ = ORIGIN.set(Instant::now());
}

/// Microseconds since [`ORIGIN`]. Returns 0 before the origin is set, which cannot happen in a real
/// run -- the loader marks it in `DllMain`, long before any hook is installed.
fn now_us() -> u64 {
    ORIGIN
        .get()
        .map_or(0, |origin| origin.elapsed().as_micros() as u64)
}

// ============================================================================================
// MILESTONES. The substate timeline covers the title flow; these cover what happens BEFORE it,
// which the first two measured runs showed to be 3.86s -- 56% of the whole boot, reproducible to
// 0.67%, and therefore steady startup work rather than shader compilation.
//
// Nothing here hooks anything. Both milestones are positions the loader already occupies: the
// Arxan callback runs at the game's entry point, and `DirectInput8Create` is our own proxy export.
// Free instruments come first; the IAT-patching one that would time D3D11 and FMOD is only worth
// building if these leave a large undifferentiated gap. See `ds2-mods-rs-a8b`.
// ============================================================================================

/// Milestones recorded before the log sink and the config existed.
///
/// `DirectInput8Create` can be called by the game before the Arxan callback has read the config
/// and installed the logger, so a milestone cannot simply log itself. It records into this buffer
/// instead, and [`install`] flushes it. A milestone that arrived too early to be logged is still a
/// milestone; dropping it would silently shorten the very interval it exists to measure.
static PENDING: [(AtomicUsize, AtomicU64); MAX_PENDING] =
    [const { (AtomicUsize::new(0), AtomicU64::new(0)) }; MAX_PENDING];
static PENDING_COUNT: AtomicU32 = AtomicU32::new(0);
/// Enough for every milestone this crate defines, several times over. Overflow is dropped and
/// reported rather than growing: this runs on the game's startup thread and must not allocate.
const MAX_PENDING: usize = 16;
/// Set once [`install`] has flushed [`PENDING`]; later milestones log immediately.
static FLUSHED: AtomicU32 = AtomicU32::new(0);

/// Record that a named point in startup has been reached.
///
/// Safe to call from `DllMain` and from the proxy export. Allocates nothing, takes no lock, and
/// before [`install`] has run it does not touch the filesystem either.
pub fn mark(label: &'static str) {
    let at_us = now_us();
    if FLUSHED.load(Ordering::Acquire) != 0 {
        let (calls, ms) = sleep_totals();
        log(format_args!(
            "{LOG_PREFIX} milestone label={label} t={:.3}ms sleep-calls={calls} sleep-ms={ms}",
            at_us as f64 / 1000.0
        ));
        return;
    }
    let slot = PENDING_COUNT.fetch_add(1, Ordering::Relaxed) as usize;
    if slot < MAX_PENDING {
        // The label is a `&'static str`; only its pointer and length are needed, and every label
        // this crate is called with is a literal, so storing the pointer is sound for the life of
        // the process. Length is recovered by the flush from the same literal set.
        PENDING[slot]
            .0
            .store(label.as_ptr() as usize, Ordering::Relaxed);
        PENDING[slot]
            .1
            .store(at_us | ((label.len() as u64) << 48), Ordering::Relaxed);
    }
}

/// Emit everything [`mark`] recorded before the logger existed, then switch to immediate logging.
fn flush_milestones() {
    let recorded = PENDING_COUNT.load(Ordering::Relaxed) as usize;
    for slot in PENDING.iter().take(recorded.min(MAX_PENDING)) {
        let pointer = slot.0.load(Ordering::Relaxed);
        let packed = slot.1.load(Ordering::Relaxed);
        if pointer == 0 {
            continue;
        }
        let at_us = packed & 0x0000_ffff_ffff_ffff;
        let len = (packed >> 48) as usize;
        // SAFETY: `pointer`/`len` came from a `&'static str` literal passed to `mark`, which lives
        // for the whole process.
        let label = unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(pointer as *const u8, len))
        };
        log(format_args!(
            "{LOG_PREFIX} milestone label={label} t={:.3}ms",
            at_us as f64 / 1000.0
        ));
    }
    if recorded > MAX_PENDING {
        log(format_args!(
            "{LOG_PREFIX} milestone-overflow recorded={recorded} kept={MAX_PENDING}"
        ));
    }
    FLUSHED.store(1, Ordering::Release);
}

// ============================================================================================
// SLEEP ACCOUNTING. Does the boot spend its time asleep? `/proc` says the process is neither
// disk-bound nor CPU-bound through the engine block, and the binary holds a `PeekMessageW` /
// `Sleep(1)` / check-a-flag loop at `0x140fecdd6`. Counting is the way to find out.
//
// The patch is a POINTER WRITE IN `.idata`, not a code patch: no instruction is modified, so
// Arxan's `.text` integrity checks have nothing to react to, and only this executable's calls are
// counted rather than every module in the process.
// ============================================================================================

unsafe extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
}
const PAGE_READWRITE: u32 = 0x04;

/// `void Sleep(DWORD)` -- one integer argument, none on the stack, no return value. That is the
/// whole reason this import can be fronted by an ordinary Rust function.
type SleepFn = unsafe extern "system" fn(u32);

static ORIGINAL_SLEEP: AtomicUsize = AtomicUsize::new(0);
static SLEEP_CALLS: AtomicU64 = AtomicU64::new(0);
static SLEEP_REQUESTED_MS: AtomicU64 = AtomicU64::new(0);

/// Calls bucketed by requested duration: `0`, `1`, `2..=9`, `10`, `>10`.
///
/// The buckets are the binary's own call sites rather than round numbers. Of the thirteen `Sleep`
/// sites, two pass `0` (yield loops), one passes `1` (the `PeekMessageW` pump at `0x140fecdd6`),
/// five pass `10`, and the rest pass a register. Splitting on exactly those values turns a total
/// into an attribution: 4000 calls means nothing, 4000 calls of `1` names one loop.
static SLEEP_BUCKETS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

fn sleep_bucket(milliseconds: u32) -> usize {
    match milliseconds {
        0 => 0,
        1 => 1,
        2..=9 => 2,
        10 => 3,
        _ => 4,
    }
}

/// Count, then sleep exactly as asked. **This changes no timing of its own**: the requested
/// duration is passed through untouched, because the question is where the boot's time goes and an
/// instrument that shortened the sleeps would be answering a different one.
unsafe extern "system" fn detour_sleep(milliseconds: u32) {
    SLEEP_CALLS.fetch_add(1, Ordering::Relaxed);
    SLEEP_REQUESTED_MS.fetch_add(u64::from(milliseconds), Ordering::Relaxed);
    SLEEP_BUCKETS[sleep_bucket(milliseconds)].fetch_add(1, Ordering::Relaxed);
    let original = ORIGINAL_SLEEP.load(Ordering::Acquire);
    if original != 0 {
        // SAFETY: read out of the import slot before it was overwritten, so it is whatever the
        // Windows loader resolved `KERNEL32!Sleep` to.
        let original: SleepFn = unsafe { std::mem::transmute::<usize, SleepFn>(original) };
        unsafe { original(milliseconds) };
    }
}

/// Point the game's `Sleep` import at [`detour_sleep`]. Returns false and logs on any failure.
///
/// # Safety
///
/// `base` must be the live game module base; the RVA must be the import slot it names.
unsafe fn hook_sleep_import(base: usize) -> bool {
    let slot = (base + ds2_rva::SLEEP_IAT_THUNK as usize) as *mut usize;
    let mut old_protect = 0u32;
    // SAFETY: one pointer-sized slot inside the image's own `.idata`.
    let ok = unsafe {
        VirtualProtect(
            slot.cast::<c_void>(),
            std::mem::size_of::<usize>(),
            PAGE_READWRITE,
            &raw mut old_protect,
        )
    };
    if ok == 0 {
        log(format_args!(
            "{LOG_PREFIX} sleep-hook-failed stage=VirtualProtect slot=0x{:016x}",
            slot as usize
        ));
        return false;
    }
    // SAFETY: the slot is now writable, and it holds the resolved import the loader wrote.
    unsafe {
        // PUBLISHED BEFORE THE SLOT IS OVERWRITTEN. Another thread can be inside `Sleep` already;
        // one that entered the detour with a zero here would return without sleeping at all, which
        // would change the game's behaviour rather than measure it.
        ORIGINAL_SLEEP.store(slot.read(), Ordering::Release);
        // Through the fn-pointer type first: a direct `as usize` on a function ITEM is a
        // zero-sized cast that clippy rejects, and rightly -- it reads as an address but is not
        // one until the item has been coerced to a pointer.
        let detour: SleepFn = detour_sleep;
        slot.write(detour as usize);
        let mut restored = 0u32;
        VirtualProtect(
            slot.cast::<c_void>(),
            std::mem::size_of::<usize>(),
            old_protect,
            &raw mut restored,
        );
    }
    log(format_args!(
        "{LOG_PREFIX} hooked import=KERNEL32!Sleep slot=0x{:016x} original=0x{:016x}",
        slot as usize,
        ORIGINAL_SLEEP.load(Ordering::Acquire)
    ));
    true
}

/// `sleep0=..` etc, for the lines that report a breakdown.
fn sleep_histogram() -> [u64; 5] {
    std::array::from_fn(|i| SLEEP_BUCKETS[i].load(Ordering::Relaxed))
}

/// `calls=N ms=M` for the milestone and boot-complete lines.
fn sleep_totals() -> (u64, u64) {
    (
        SLEEP_CALLS.load(Ordering::Relaxed),
        SLEEP_REQUESTED_MS.load(Ordering::Relaxed),
    )
}

/// The id of the substate currently resident, or [`NO_SUBSTATE`].
static CURRENT_ID: AtomicU32 = AtomicU32::new(NO_SUBSTATE);
/// When [`CURRENT_ID`] was entered, in microseconds since [`ORIGIN`].
static CURRENT_ENTERED_US: AtomicU64 = AtomicU64::new(0);
/// Monotonic event counter, so a reader can tell a dropped line from a quiet stretch.
static SEQ: AtomicU32 = AtomicU32::new(0);
/// Whether the one-shot line describing the flow itself has been written.
static DESCRIBED: AtomicU32 = AtomicU32::new(0);

/// Sentinel for "no substate resident". `u32::MAX` and not `0`, because `0` is a real id --
/// `FeSubStateTitleInitBranch`, the first step of the boot chain.
const NO_SUBSTATE: u32 = u32::MAX;

/// `FeStateFlow::update(this, delta)`.
///
/// **Two arguments, and that is established rather than assumed.** The body reads its `this` from
/// RCX and the frame delta from XMM1, and every other register it uses it loads from the object
/// first (`mov r8,[rbx+0x30]`, `mov rdx,[rbx+0x28]`) rather than reading an incoming value. So
/// there is no third argument for a detour to clobber. This is why the crate uses `MhHook`
/// directly instead of `ds2-hook`'s union, whose shared signature is four integers and explicitly
/// cannot carry a float.
type FlowUpdateFn = unsafe extern "system" fn(*mut u8, f32);

/// `FeSubStateBase::v6(this, transitions, context)` -- "drop the transitions I published".
///
/// Three integer arguments, taken from the two call sites in `FeStateFlow::update`, which both set
/// RDX from `[flow+0x28]` and R8 from `[flow+0x30]` before calling through the slot.
type DropTransitionsFn = unsafe extern "system" fn(*mut u8, *mut u8, *mut u8);

static FLOW_UPDATE_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
static DROP_TRANSITIONS_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Read a substate's id from `+0x0c`, the field `FeStateFlow`'s own transition search compares
/// against. Returns [`NO_SUBSTATE`] for a null pointer.
///
/// # Safety
///
/// `substate` must be null or a live `FeSubStateBase`-derived object. Every caller here got it
/// either out of `[flow+0x10]` or as the `this` of a virtual call the game just made.
unsafe fn substate_id(substate: *const u8) -> u32 {
    if substate.is_null() {
        return NO_SUBSTATE;
    }
    // SAFETY: the game reads this same offset on this same pointer in its transition search.
    unsafe {
        substate
            .add(ds2_rva::FE_SUBSTATE_ID_OFFSET)
            .cast::<u32>()
            .read()
    }
}

/// Write the one-shot line that describes the flow object itself: how many substates are
/// registered, and where. The count is the denominator a loading bar needs, and reading it at
/// runtime is what turns "64 substates are constructed in `FeStateTitle::v6`" from a static claim
/// into a measured one.
///
/// # Safety
///
/// `flow` must be a live `FeStateFlow`.
unsafe fn describe_once(flow: *const u8) {
    if DESCRIBED.swap(1, Ordering::Relaxed) != 0 || flow.is_null() {
        return;
    }
    // SAFETY: `+0x20` is the substate list the flow's own transition search indexes, and `+0x2c8`
    // is the count that search bounds itself with.
    let (list, count) = unsafe {
        let list = flow
            .add(ds2_rva::FE_STATE_FLOW_SUBSTATE_LIST_OFFSET)
            .cast::<*const u8>()
            .read();
        let count = if list.is_null() {
            -1
        } else {
            list.add(ds2_rva::FE_SUBSTATE_LIST_COUNT_OFFSET)
                .cast::<i32>()
                .read()
        };
        (list, count)
    };
    log(format_args!(
        "{LOG_PREFIX} flow flow=0x{:016x} list=0x{:016x} registered={count}",
        flow as usize, list as usize
    ));
}

/// A substate became resident.
fn on_enter(id: u32, pending: i32) {
    let at_us = now_us();
    CURRENT_ID.store(id, Ordering::Relaxed);
    CURRENT_ENTERED_US.store(at_us, Ordering::Relaxed);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    // `pending` is the flow's `+0x48`, the id an outside caller asked for -- `FeOperatorTitle`
    // writes `0x17` there to return to the title. It is logged because a transition driven from
    // outside the substate graph looks identical to one driven by a transition object otherwise.
    let (calls, ms) = sleep_totals();
    log(format_args!(
        "{LOG_PREFIX} enter seq={seq} id=0x{id:02x} t={:.3}ms pending={pending} \
         sleep-calls={calls} sleep-ms={ms}",
        at_us as f64 / 1000.0
    ));
    if id == ds2_rva::FE_SUBSTATE_ID_TITLE_TOP_MENU {
        let (calls, ms) = sleep_totals();
        let h = sleep_histogram();
        log(format_args!(
            "{LOG_PREFIX} boot-complete reached=top-menu t={:.3}ms sleep-calls={calls} \
             sleep-ms={ms} sleep0={} sleep1={} sleep2-9={} sleep10={} sleep-gt10={}",
            at_us as f64 / 1000.0,
            h[0],
            h[1],
            h[2],
            h[3],
            h[4]
        ));
    }
}

/// A substate is about to be left.
fn on_leave(id: u32) {
    let at_us = now_us();
    let current = CURRENT_ID.swap(NO_SUBSTATE, Ordering::Relaxed);
    let entered = CURRENT_ENTERED_US.load(Ordering::Relaxed);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    // THE INTEGRITY CHECK, and the reason both hooks exist. If the substate being dropped is not
    // the one the sampler last saw arrive, the sampler missed a transition and every duration
    // after this point is attributed to the wrong step. Saying so in the line is the difference
    // between a log that is wrong and a log that says it is wrong.
    let mismatch = current != id;
    let dwell_us = at_us.saturating_sub(entered);
    log(format_args!(
        "{LOG_PREFIX} leave seq={seq} id=0x{id:02x} t={:.3}ms dwell={:.3}ms mismatch={mismatch}",
        at_us as f64 / 1000.0,
        dwell_us as f64 / 1000.0
    ));
}

// ============================================================================================
// WATCHES. The two substates that take ~1.01s reproducibly are waiting on a state word owned by
// something else. Sampling that word once per frame, and logging only when it CHANGES, says which
// side of the boundary the second is spent on -- the substate sitting on a result that arrived
// long ago, or the service genuinely taking a second to produce it.
//
// Only-on-change is what makes this affordable: this runs inside a per-frame detour, and the log
// sink opens, writes and fsyncs per line.
// ============================================================================================

/// Last value seen for each watch, plus a "never sampled" flag in the high bits.
static WATCHED: [AtomicU64; 3] = [const { AtomicU64::new(u64::MAX) }; 3];
const WATCH_SAVE_LOAD: usize = 0;
const WATCH_INFORMATION: usize = 1;
const WATCH_PHASE: usize = 2;

/// Substates whose own phase field is worth tracing frame by frame.
///
/// **This is the field that actually decides**, and aiming at anything else first was a mistake
/// worth not repeating. The interlock watch showed `SaveLoadSystem` going idle 88 ms in and
/// concluded no further request was issued — but a watch that logs only on CHANGE, sampled once
/// per frame, cannot see a request that starts and finishes between two samples, so it could not
/// support that conclusion. The substate's own phase can: it says which of the thirteen branches
/// the 890 ms is spent in, and every branch is a few lines of already-read disassembly.
const PHASE_WATCHED: [u32; 2] = [0x05, 0x44];

/// Follow `base_rva -> [+first] -> read u32 at +second`, with a null check at every hop.
///
/// # Safety
///
/// `base` is the live game module base. Each offset is one the game itself dereferences at the
/// same point in the flow, so the chain is valid whenever the watched substate is resident.
unsafe fn read_through(base: usize, base_rva: u32, first: usize, second: usize) -> Option<u32> {
    unsafe {
        let global = (base + base_rva as usize) as *const *const u8;
        let root = global.read();
        if root.is_null() {
            return None;
        }
        let object = root.add(first).cast::<*const u8>().read();
        if object.is_null() {
            return None;
        }
        Some(object.add(second).cast::<u32>().read())
    }
}

/// Sample whichever state word the resident substate is waiting on, and log a change.
///
/// # Safety
///
/// `base` must be the live game module base.
unsafe fn sample_watch(base: usize, resident_id: u32) {
    let (index, label, value) = match resident_id {
        // 0x05 SteamLoadSystemData waits on SaveLoadSystem's request state -- the same word the
        // service's own start path refuses on. Both halves of the interlock are packed into one
        // sample so a change in either is visible.
        0x05 => {
            let state = unsafe {
                read_through(
                    base,
                    ds2_rva::GAME_MANAGER_IMP,
                    ds2_rva::SAVE_LOAD_SYSTEM_OFFSET,
                    ds2_rva::SAVE_LOAD_SYSTEM_STATE_OFFSET,
                )
            };
            let sub = unsafe {
                read_through(
                    base,
                    ds2_rva::GAME_MANAGER_IMP,
                    ds2_rva::SAVE_LOAD_SYSTEM_OFFSET,
                    ds2_rva::SAVE_LOAD_SYSTEM_SUBSTATE_OFFSET,
                )
            };
            match (state, sub) {
                (Some(state), Some(sub)) => (
                    WATCH_SAVE_LOAD,
                    "saveload-state",
                    u64::from(state) | (u64::from(sub) << 32),
                ),
                _ => return,
            }
        }
        // 0x44 Information waits on the download job's own state, testing it for 5 or 6.
        0x44 => {
            let Some(state) = (unsafe {
                read_through(
                    base,
                    ds2_rva::FE_TITLE_CONTEXT,
                    ds2_rva::FE_TITLE_INFORMATION_JOB_OFFSET,
                    ds2_rva::FE_INFORMATION_JOB_STATE_OFFSET,
                )
            }) else {
                return;
            };
            (WATCH_INFORMATION, "information-job-state", u64::from(state))
        }
        _ => return,
    };
    if WATCHED[index].swap(value, Ordering::Relaxed) != value {
        log(format_args!(
            "{LOG_PREFIX} watch id=0x{resident_id:02x} field={label} value=0x{value:x} t={:.3}ms",
            now_us() as f64 / 1000.0
        ));
    }
}

/// Trace the resident substate's own phase field, for the ids in [`PHASE_WATCHED`].
///
/// # Safety
///
/// `substate` must be the live resident substate.
unsafe fn sample_phase(substate: *const u8, resident_id: u32) {
    if !PHASE_WATCHED.contains(&resident_id) {
        return;
    }
    // SAFETY: `+0x10` is the phase every one of these classes' own `update` switches on, and the
    // flow has just called that update against this pointer.
    let phase = unsafe {
        substate
            .add(ds2_rva::FE_SUBSTATE_PHASE_OFFSET)
            .cast::<u32>()
            .read()
    };
    let value = u64::from(phase) | (u64::from(resident_id) << 32);
    if WATCHED[WATCH_PHASE].swap(value, Ordering::Relaxed) != value {
        log(format_args!(
            "{LOG_PREFIX} watch id=0x{resident_id:02x} field=phase value={phase} t={:.3}ms",
            now_us() as f64 / 1000.0
        ));
    }
}

/// Sample the resident substate around the original, and report a change.
unsafe extern "system" fn detour_flow_update(flow: *mut u8, delta: f32) {
    let trampoline = FLOW_UPDATE_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 || flow.is_null() {
        return;
    }
    // SAFETY: MinHook published this trampoline for this site, and the signature is the one the
    // function's own body establishes -- RCX and XMM1 in, nothing else read.
    let original: FlowUpdateFn = unsafe { std::mem::transmute::<usize, FlowUpdateFn>(trampoline) };

    // SAFETY: `+0x10` is the resident-substate pointer the flow reads and writes throughout its
    // own update; reading it is exactly what the game does at `0x1401045e3`.
    let resident = |flow: *const u8| unsafe {
        flow.add(ds2_rva::FE_STATE_FLOW_RESIDENT_SUBSTATE_OFFSET)
            .cast::<*const u8>()
            .read()
    };

    unsafe { describe_once(flow) };
    let before = resident(flow);
    // SAFETY: `+0x48` is the pending-request id; the flow compares it against `0` as a signed
    // value and writes `-1` once consumed, so `i32` is its type.
    let pending = unsafe {
        flow.add(ds2_rva::FE_STATE_FLOW_PENDING_ID_OFFSET)
            .cast::<i32>()
            .read()
    };

    unsafe { original(flow, delta) };

    let after = resident(flow);
    // The watch runs every frame the substate is resident, not only on a transition: the whole
    // question is what the state word does DURING the second, and a transition-only sample would
    // show only its value at each end.
    if !after.is_null()
        && let Ok(base) = ds2_game_base::mem::game_module_base()
    {
        // SAFETY: `after` is the substate the flow has just updated, and `base` is the live module
        // base the RVAs in `ds2-rva` are relative to.
        let id = unsafe { substate_id(after) };
        unsafe { sample_watch(base, id) };
        unsafe { sample_phase(after, id) };
    }
    if after != before && !after.is_null() {
        // SAFETY: the flow has just stored this pointer as its resident substate.
        let id = unsafe { substate_id(after) };
        on_enter(id, pending);
    }
}

/// Report the departure, then let the original drop the transition list.
unsafe extern "system" fn detour_drop_transitions(
    this: *mut u8,
    transitions: *mut u8,
    context: *mut u8,
) {
    if !this.is_null() {
        // SAFETY: the flow is calling a virtual on this object, so it is live.
        on_leave(unsafe { substate_id(this) });
    }
    let trampoline = DROP_TRANSITIONS_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook published this trampoline for this site; the three integer arguments are
        // the ones both call sites in `FeStateFlow::update` set.
        let original: DropTransitionsFn =
            unsafe { std::mem::transmute::<usize, DropTransitionsFn>(trampoline) };
        unsafe { original(this, transitions, context) };
    }
}

/// One hook site: where it is, what to call it in a log, and the trampoline slot it publishes to.
struct Site {
    name: &'static str,
    rva: u32,
    detour: *mut c_void,
    trampoline: &'static AtomicUsize,
}

// SAFETY: the only non-`Send` field is a function pointer to a `'static` detour, which is
// immutable for the life of the process.
unsafe impl Sync for Site {}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Sites now detoured.
    pub installed: usize,
    /// Sites attempted. Carried so a caller reporting "1 of 2" need not know the total.
    pub attempted: usize,
}

/// Detour the flow's update and the shared `v6`. Call from the post-Arxan callback, never
/// `DllMain`.
///
/// **A partial install is reported and kept**, unlike a feature where half the patch is half the
/// behaviour. One hook alone still produces a usable timeline -- arrivals without departures, or
/// the reverse -- it just loses the cross-check between them. The `installed` count is what says
/// which of those a log is.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`) and before the title flow starts, which in practice is the loader's
/// Arxan callback. Both sites were checked with `scripts/ds2-arxan-chain.py` and are ordinary
/// prologues, not Arxan redirects.
pub unsafe fn install() -> Outcome {
    let sites: [Site; 2] = [
        Site {
            name: "flow-update",
            rva: ds2_rva::FE_STATE_FLOW_UPDATE,
            detour: detour_flow_update as *mut c_void,
            trampoline: &FLOW_UPDATE_TRAMPOLINE,
        },
        Site {
            name: "drop-transitions",
            rva: ds2_rva::FE_SUBSTATE_DROP_TRANSITIONS,
            detour: detour_drop_transitions as *mut c_void,
            trampoline: &DROP_TRANSITIONS_TRAMPOLINE,
        },
    ];

    // FIRST: everything `mark` recorded before there was a sink to write it to. `install` runs
    // from the Arxan callback, which is the entry point, so this is also the milestone that says
    // how much of the boot went by before the game's own code started.
    mark("entry-point");
    flush_milestones();

    let base = match ds2_game_base::mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} install-failed stage=module-base error={error}"
            ));
            return Outcome {
                installed: 0,
                attempted: sites.len(),
            };
        }
    };

    // MinHook is statically linked into this DLL, so ALREADY_INITIALIZED can only mean this ran
    // twice. Treat it as success, exactly as the other feature crates do.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return Outcome {
            installed: 0,
            attempted: sites.len(),
        };
    }

    // SAFETY: `base` is the live module base; the RVA names this image's own `Sleep` import slot.
    unsafe { hook_sleep_import(base) };

    let mut installed = 0;
    for site in &sites {
        let address = base + site.rva as usize;
        let hook = match unsafe { MhHook::new(address as *mut c_void, site.detour) } {
            Ok(hook) => hook,
            Err(status) => {
                log(format_args!(
                    "{LOG_PREFIX} hook-failed site={} va=0x{address:016x} stage=MH_CreateHook \
                     status={status:?}",
                    site.name
                ));
                continue;
            }
        };
        // Published BEFORE the site is patched. `FeStateFlow::update` runs every frame, so a
        // detour that observed a zero here would skip the original and freeze the title flow --
        // the one ordering mistake in this file that would be fatal rather than merely lossy.
        site.trampoline
            .store(hook.trampoline() as usize, Ordering::Release);
        let status = unsafe { MH_EnableHook(address as *mut c_void) };
        if status != MH_STATUS::MH_OK {
            log(format_args!(
                "{LOG_PREFIX} hook-failed site={} va=0x{address:016x} stage=MH_EnableHook \
                 status={status:?}",
                site.name
            ));
            continue;
        }
        // The handle falls out of scope; `MhHook` has no `Drop`, so the patch stays for the life
        // of the process, which is what is wanted.
        installed += 1;
        log(format_args!(
            "{LOG_PREFIX} hooked site={} rva=0x{:08x} va=0x{address:016x}",
            site.name, site.rva
        ));
    }

    log(format_args!(
        "{LOG_PREFIX} install installed={installed}/{} t={:.3}ms origin={}",
        sites.len(),
        now_us() as f64 / 1000.0,
        if ORIGIN.get().is_some() {
            "dll-main"
        } else {
            "UNSET-timestamps-will-read-zero"
        }
    ));
    Outcome {
        installed,
        attempted: sites.len(),
    }
}
