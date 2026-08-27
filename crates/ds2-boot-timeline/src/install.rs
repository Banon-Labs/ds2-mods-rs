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
        log(format_args!(
            "{LOG_PREFIX} milestone label={label} t={:.3}ms",
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
    log(format_args!(
        "{LOG_PREFIX} enter seq={seq} id=0x{id:02x} t={:.3}ms pending={pending}",
        at_us as f64 / 1000.0
    ));
    if id == ds2_rva::FE_SUBSTATE_ID_TITLE_TOP_MENU {
        log(format_args!(
            "{LOG_PREFIX} boot-complete reached=top-menu t={:.3}ms",
            at_us as f64 / 1000.0
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
