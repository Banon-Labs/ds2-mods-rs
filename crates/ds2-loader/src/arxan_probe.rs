//! **M1**: does a MinHook detour survive Arxan in DARK SOULS II?
//!
//! This module is an experiment, not a feature. It installs one detour on one deliberately
//! chosen function, then watches -- from a separate thread, once a second -- whether the bytes
//! it wrote are still there. Nothing in here is meant to survive into a shipping mod; it exists
//! to turn "we assume hooking works" into a fact or a refutation, because DS2 carries 48 Arxan
//! stubs and 286 Arxan-redirected functions and **nobody has hooked anything in this game yet**.
//! Every plan in this repo that involves a hook is unproven until this reports.
//!
//! # Why four measurements and not one
//!
//! The obvious experiment -- detour something, count the calls -- **cannot answer the question**.
//! A counter that stays at zero is equally consistent with "Arxan reverted our patch" and with
//! "that function was simply never called", and those two facts point in opposite directions.
//! So the probe reports four things, and it is the *combination* that is evidence:
//!
//! 1. **The hit counter** ([`HIT_COUNT`]), incremented inside the detour and logged with every
//!    heartbeat. Proves the detour fires.
//! 2. **The hook-site byte poller**. Re-reads the bytes at the patched prologue every second and
//!    compares them against *what MinHook actually wrote* -- not against a predicted `e9`, but
//!    against the bytes read back after `MH_EnableHook` returned. This is what separates
//!    reversion from non-invocation, and it works even if the function is never called once.
//! 3. **The trampoline byte poller**. Arxan could corrupt the trampoline while leaving the hook
//!    site pristine: the counter would go silent, the site would look perfect, and the honest
//!    reading of those two facts alone is "the function stopped being called", which would be
//!    wrong. MinHook's 64-byte slot also contains the *relay* holding the absolute jump to our
//!    detour, so this window covers the other half of the machinery too.
//! 4. **The arm** -- see below. Logged on every line that carries a verdict.
//!
//! # The A/B arm, and why "it survived" means nothing without it
//!
//! Suppose the probe runs, `dearxan::disabler::neuter_arxan` has already patched all 48 stubs,
//! and the detour survives for an hour. That result is compatible with two very different
//! worlds: *Arxan would have reverted the hook and dearxan is load-bearing*, or *Arxan never
//! looks at this page and dearxan was irrelevant here*. One run cannot tell them apart, so the
//! probe has two arms and both must be run:
//!
//! | arm | env | what runs | what a surviving detour means |
//! | --- | --- | --- | --- |
//! | [`Arm::NeuterArxan`] | `DS2_ARXAN_PROBE=1` | `neuter_arxan` patches the stubs, then the probe installs | hooking works *with dearxan* |
//! | [`Arm::SkipNeuterArxan`] | `DS2_ARXAN_PROBE=1 DS2_ARXAN_PROBE_SKIP_NEUTER=1` | stubs left intact; the probe installs anyway | Arxan was never a threat to this site |
//!
//! The skip arm does **not** simply drop the `neuter_arxan` call and install from `DllMain`.
//! That would change two variables at once -- the Arxan patching *and* the moment the hook goes
//! in. It calls [`schedule_after_arxan`](dearxan::disabler::schedule_after_arxan) instead, which
//! is the same scheduling machinery `neuter_arxan` itself is built on (`neuter_arxan` is
//! literally "schedule the stub patching before the Arxan entry stub" plus "schedule the report
//! after it"), minus the stub patching. Both arms therefore install the detour at the same point
//! in the CRT entry sequence, on the same thread, with the same SteamStub handling. Exactly one
//! thing differs, which is the only way the comparison means anything.
//!
//! # What this module deliberately does not do
//!
//! It does not re-apply the patch if it is reverted, and it does not fight back. The moment it
//! re-patched, the next observation would be measuring the fight rather than Arxan, and the
//! interesting facts -- *how long did it take, and did it happen more than once* -- would be
//! gone.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ds2_game_base::mem;
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

#[cfg(not(target_arch = "x86_64"))]
compile_error!(
    "the probe's detour is hand-written x86-64 assembly and DARK SOULS II ships x86-64 only"
);

/// `1` in `DS2_ARXAN_PROBE` installs the detour. Anything else -- including unset, `true`, `yes`
/// and `0` -- leaves this DLL behaving exactly as it does without this module.
pub const ENV_PROBE: &str = "DS2_ARXAN_PROBE";

/// `1` in `DS2_ARXAN_PROBE_SKIP_NEUTER` selects [`Arm::SkipNeuterArxan`]. Honoured **only** when
/// [`ENV_PROBE`] is also `1`: skipping the Arxan patch with no probe watching produces no
/// evidence at all, so a stale variable in a shell cannot quietly turn an ordinary run into an
/// unprotected one.
pub const ENV_SKIP_NEUTER: &str = "DS2_ARXAN_PROBE_SKIP_NEUTER";

/// The only value either variable accepts. Strict on purpose: a typo that silently read as "off"
/// would produce a run that looks like the probe never reported, which is the one failure mode
/// this experiment must never confuse with a real result. The raw string is logged either way,
/// so a rejected value is visible rather than inferred.
const ENV_TRUE: &str = "1";

/// Prefix on every line this module writes. Distinct from `ds2-loader:` so the probe's testimony
/// and the loader's stay separable, by a reader and by `scripts/ds2-run.py`.
pub const PROBE_LINE_PREFIX: &str = "ds2-probe:";

/// How often the two pollers re-read their windows.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How many polls between heartbeats. Every state *change* is logged the instant it is seen; the
/// heartbeat exists so that a log which says nothing is distinguishable from a log that stopped,
/// and so a crash has a time of death. Ten seconds keeps a multi-hour session's log small.
const POLLS_PER_HEARTBEAT: u32 = 10;

/// Bytes watched at the hook site. MinHook writes five (`e9 rel32`); this window is wider so a
/// divergence report shows the surrounding instructions rather than five bytes with no context,
/// and so MinHook's `patchAbove` variant -- a two-byte `eb` short jump at the target, with the
/// real jump five bytes earlier -- is still caught at the target.
const SITE_WINDOW: usize = 16;

/// Bytes watched at the trampoline. MinHook's x64 `MEMORY_SLOT_SIZE` is 64 and the whole slot
/// belongs to this one hook: the relocated prologue, the jump back into the function, and the
/// relay carrying the absolute jump to our detour all live inside it. Watching all 64 covers
/// every part of the machinery that is not the hook site itself.
const TRAMPOLINE_WINDOW: usize = 64;

/// Times the detour has been entered. Incremented by the `lock inc` in [`probe_detour`], read by
/// the poller, never reset.
static HIT_COUNT: AtomicU64 = AtomicU64::new(0);

/// MinHook's trampoline for the hook site -- where [`probe_detour`] jumps to reach the original
/// function. Written once, after `MH_CreateHook` and **before** `MH_EnableHook`, so it cannot be
/// read as zero by a detour that has fired: until `MH_EnableHook` returns, the site is unpatched
/// and the detour is unreachable.
static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Which of the two arms this process is running.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// `neuter_arxan` runs; the probe installs from its callback. The default.
    NeuterArxan,
    /// `neuter_arxan` is **skipped**; the probe installs from `schedule_after_arxan` instead, at
    /// the same point in the entry sequence, with Arxan's 48 stubs intact.
    SkipNeuterArxan,
}

impl Arm {
    /// The spelling that appears in the log and in `scripts/ds2-run.py`. Both sides parse it, so
    /// these two strings are a contract; changing one without the other makes every run report
    /// the wrong arm, which is worse than reporting none.
    pub const fn as_str(self) -> &'static str {
        match self {
            Arm::NeuterArxan => "neuter-arxan",
            Arm::SkipNeuterArxan => "skip-neuter-arxan",
        }
    }
}

/// What the environment asked for, resolved once at `DLL_PROCESS_ATTACH`.
pub struct ProbeConfig {
    /// Whether to install the detour at all.
    pub enabled: bool,
    /// Which arm to run. Forced to [`Arm::NeuterArxan`] when `enabled` is false.
    pub arm: Arm,
    /// The raw strings, kept so the log can show what was actually set rather than what was
    /// understood. A run that reports `probe=off` next to `DS2_ARXAN_PROBE="true"` diagnoses
    /// itself; one that reports only `probe=off` sends someone hunting Steam's environment
    /// propagation for an hour.
    raw_probe: Option<String>,
    raw_skip: Option<String>,
}

impl ProbeConfig {
    /// Read [`ENV_PROBE`] and [`ENV_SKIP_NEUTER`].
    ///
    /// Under Proton these arrive the same way `WINEDLLOVERRIDES` does -- through the environment
    /// Steam hands the Wine process -- so they share its failure mode: a Steam client that was
    /// **already running** launches the game from *its* environment, not from the one
    /// `scripts/ds2-run.py` set. That is why the resolved config is logged rather than assumed,
    /// and why the script refuses to report a verdict for an arm the log does not confirm.
    pub fn from_env() -> Self {
        let raw_probe = std::env::var(ENV_PROBE).ok();
        let raw_skip = std::env::var(ENV_SKIP_NEUTER).ok();
        let enabled = raw_probe.as_deref() == Some(ENV_TRUE);
        let arm = if enabled && raw_skip.as_deref() == Some(ENV_TRUE) {
            Arm::SkipNeuterArxan
        } else {
            Arm::NeuterArxan
        };
        Self {
            enabled,
            arm,
            raw_probe,
            raw_skip,
        }
    }

    /// The `probe=... arm=... DS2_ARXAN_PROBE=...` tail appended to the loader's attach line.
    pub fn describe(&self) -> String {
        format!(
            "probe={} arm={} {ENV_PROBE}={} {ENV_SKIP_NEUTER}={}",
            if self.enabled { "on" } else { "off" },
            self.arm.as_str(),
            quote_env(self.raw_probe.as_deref()),
            quote_env(self.raw_skip.as_deref()),
        )
    }
}

/// Render an environment value for the log: `<unset>`, or the value in quotes so an empty string
/// and a stray trailing space are visible instead of invisible.
fn quote_env(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("{value:?}"),
        None => "<unset>".to_string(),
    }
}

/// One byte window as `48 89 5c 24 08 ...`, for a line a human reads and a script greps.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 3);
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        // Infallible: writing to a `String`.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// ============================================================================================
// THE LOG SINK
//
// A function pointer installed by `lib.rs`, for the same reason `ds2-hook` has one: this module
// must not know where the log lives, and the poller thread outlives the call that started it, so
// a borrowed closure could not be used.
// ============================================================================================

/// Signature of the sink this module writes through.
pub type ProbeLogFn = fn(std::fmt::Arguments<'_>);

/// Zero means "no sink installed", and then the probe writes nothing at all -- silence rather
/// than a crash, which is the right failure for a diagnostic.
static PROBE_LOG: AtomicUsize = AtomicUsize::new(0);

/// Install the sink. Call once, before [`install`].
pub fn set_probe_logger(logger: ProbeLogFn) {
    PROBE_LOG.store(logger as usize, Ordering::Release);
}

fn log(args: std::fmt::Arguments<'_>) {
    let raw = PROBE_LOG.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `ProbeLogFn` stored by `set_probe_logger`.
        let logger: ProbeLogFn = unsafe { std::mem::transmute::<usize, ProbeLogFn>(raw) };
        logger(args);
    }
}

// ============================================================================================
// LIVE STATE
//
// The poller's running totals live in statics rather than in its own locals so that
// `detach_line` can report them from the process's exit path, which is a different thread. They
// are the only thing that distinguishes "the game was closed" from "the game died", and that
// distinction is worth four atomics.
// ============================================================================================

/// Result of comparing a watched window against its baseline.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Integrity {
    /// Byte-for-byte what was there when the probe finished installing.
    Intact = 0,
    /// Readable, and different. **This is the finding the experiment exists to detect.**
    Diverged = 1,
    /// `ReadProcessMemory` refused the range. Not the same as diverged: the page may have been
    /// unmapped or reprotected, which is its own kind of answer.
    Unreadable = 2,
}

impl Integrity {
    /// Lowercase for the good state, SHOUTING for the two bad ones, so a log skimmed by eye and a
    /// log grepped by machine agree about what is worth stopping on.
    const fn as_str(self) -> &'static str {
        match self {
            Integrity::Intact => "intact",
            Integrity::Diverged => "DIVERGED",
            Integrity::Unreadable => "UNREADABLE",
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Integrity::Diverged,
            2 => Integrity::Unreadable,
            _ => Integrity::Intact,
        }
    }
}

static SITE_STATE: AtomicU8 = AtomicU8::new(Integrity::Intact as u8);
static TRAMPOLINE_STATE: AtomicU8 = AtomicU8::new(Integrity::Intact as u8);
static SITE_DIVERGENCES: AtomicU32 = AtomicU32::new(0);
static TRAMPOLINE_DIVERGENCES: AtomicU32 = AtomicU32::new(0);

/// Set when the detour is enabled; `None` means the probe never got that far, and then
/// [`detach_line`] has nothing to say.
static WATCH_STARTED: OnceLock<(Instant, Arm)> = OnceLock::new();

/// Everything the poller thread needs. Addresses cross the thread boundary as `usize` because
/// that is what they are; nothing here is a raw pointer, so the whole struct is `Send`.
struct Watch {
    arm: Arm,
    site_address: usize,
    site_baseline: Vec<u8>,
    trampoline_address: usize,
    trampoline_baseline: Vec<u8>,
}

/// Install the detour and start the pollers. Called from the Arxan callback in either arm, on the
/// entry-point thread, after `DllMain` has returned.
///
/// Returns without installing anything if any step fails, having logged which step and why. It
/// never panics out into the game's startup: a probe that takes the game down proves nothing
/// except that the probe is broken.
///
/// # Safety
///
/// Patches a function prologue in the loaded game image through MinHook. The address comes from
/// [`ds2_rva::ARXAN_PROBE_HOOK_SITE`], whose derivation is recorded there; a wrong address means
/// five bytes of some unrelated function are overwritten and everything the game does afterwards
/// is undefined.
pub unsafe fn install(arm: Arm) {
    let base = match mem::game_module_base() {
        Ok(base) => base,
        Err(error) => {
            log(format_args!(
                "{PROBE_LINE_PREFIX} install-failed stage=module-base arm={} error={error}",
                arm.as_str()
            ));
            return;
        }
    };
    let site = base + ds2_rva::ARXAN_PROBE_HOOK_SITE as usize;
    log(format_args!(
        "{PROBE_LINE_PREFIX} install arm={} base=0x{base:016x} rva=0x{:08x} va=0x{site:016x}",
        arm.as_str(),
        ds2_rva::ARXAN_PROBE_HOOK_SITE,
    ));

    // READ BEFORE WRITING. If these bytes are not the prologue recorded in `ds2-rva`, something
    // reached this function before we did, and everything downstream would be a measurement of
    // that instead of of Arxan.
    let mut original = vec![0u8; SITE_WINDOW];
    if !unsafe { mem::read_bytes(site, &mut original) } {
        log(format_args!(
            "{PROBE_LINE_PREFIX} install-failed stage=read-original va=0x{site:016x}"
        ));
        return;
    }
    let expected = ds2_rva::ARXAN_PROBE_HOOK_SITE_PROLOGUE;
    let prologue_match = original[..expected.len()] == expected;
    log(format_args!(
        "{PROBE_LINE_PREFIX} install original=[{}] expected=[{}] prologue-match={prologue_match}",
        hex(&original),
        hex(&expected),
    ));
    if !prologue_match {
        log(format_args!(
            "{PROBE_LINE_PREFIX} VOID prologue-mismatch va=0x{site:016x} -- this function was \
             already patched before the probe touched it, so nothing this run reports is \
             evidence about Arxan"
        ));
        return;
    }

    // MinHook is statically linked into THIS DLL (see `ds2-hook`), so nothing else in the process
    // shares this instance and `MH_ERROR_ALREADY_INITIALIZED` could only mean this code ran
    // twice. Treat it as success rather than as a reason to stop.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{PROBE_LINE_PREFIX} install-failed stage=MH_Initialize status={status:?}"
        ));
        return;
    }

    let detour: unsafe extern "system" fn() = probe_detour;
    // SAFETY: `site` is a function start read out of `.pdata`, `probe_detour` is a naked
    // tail-jump that imposes no ABI on it, and `TRAMPOLINE` is published below before the
    // prologue is patched.
    let hook = match unsafe { MhHook::new(site as *mut c_void, detour as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{PROBE_LINE_PREFIX} install-failed stage=MH_CreateHook status={status:?}"
            ));
            return;
        }
    };
    let trampoline_address = hook.trampoline() as usize;
    // BEFORE the prologue is patched, so the detour cannot observe a zero here.
    TRAMPOLINE.store(trampoline_address, Ordering::SeqCst);

    // Not the queued API: this is one hook, and `MH_EnableHook` applies it immediately, so there
    // is no window in which the trampoline exists and the site is half-patched.
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{PROBE_LINE_PREFIX} install-failed stage=MH_EnableHook status={status:?}"
        ));
        return;
    }

    // THE BASELINE IS WHAT MINHOOK WROTE, READ BACK. Not a predicted `e9 rel32`: if MinHook took
    // its `patchAbove` path, or a future version writes something else, a predicted baseline
    // would report a divergence on the first poll and the run would be discarded for nothing.
    let mut site_baseline = vec![0u8; SITE_WINDOW];
    if !unsafe { mem::read_bytes(site, &mut site_baseline) } {
        log(format_args!(
            "{PROBE_LINE_PREFIX} install-failed stage=read-patched va=0x{site:016x}"
        ));
        return;
    }
    let mut trampoline_baseline = vec![0u8; TRAMPOLINE_WINDOW];
    if !unsafe { mem::read_bytes(trampoline_address, &mut trampoline_baseline) } {
        log(format_args!(
            "{PROBE_LINE_PREFIX} install-failed stage=read-trampoline \
             addr=0x{trampoline_address:016x}"
        ));
        return;
    }
    log(format_args!(
        "{PROBE_LINE_PREFIX} install minhook=ok trampoline=0x{trampoline_address:016x} \
         patched=[{}] site-jmp={}",
        hex(&site_baseline),
        site_baseline.first() == Some(&0xe9),
    ));
    log(format_args!(
        "{PROBE_LINE_PREFIX} install trampoline-baseline=[{}]",
        hex(&trampoline_baseline),
    ));

    // `MhHook` has no `Drop`, so letting it fall out of scope leaves the hook installed and
    // MinHook's own bookkeeping intact. Nothing here ever removes the hook: an experiment that
    // unhooks itself has removed the thing it is measuring.
    let watch = Watch {
        arm,
        site_address: site,
        site_baseline,
        trampoline_address,
        trampoline_baseline,
    };

    log(format_args!(
        "{PROBE_LINE_PREFIX} watching arm={} poll={:.1}s heartbeat={:.1}s \
         site-window={SITE_WINDOW} trampoline-window={TRAMPOLINE_WINDOW}",
        arm.as_str(),
        POLL_INTERVAL.as_secs_f64(),
        POLL_INTERVAL.as_secs_f64() * f64::from(POLLS_PER_HEARTBEAT),
    ));

    // A dedicated thread, not a hook on some game callback: the byte pollers have to keep
    // reporting whether or not the detour ever fires, which is exactly the case a game-driven
    // timer could not cover. Spawning here is safe -- this runs on the entry-point thread after
    // `DllMain` returned, so the loader lock is not held; spawning from `DllMain` itself is the
    // textbook deadlock.
    let _ = WATCH_STARTED.set((Instant::now(), arm));
    if let Err(error) = std::thread::Builder::new()
        .name("ds2-arxan-probe".to_string())
        .spawn(move || poll_forever(&watch))
    {
        log(format_args!(
            "{PROBE_LINE_PREFIX} install-failed stage=spawn-poller error={error}"
        ));
    }
}

/// **The detour.** Counts one entry and tail-jumps to MinHook's trampoline.
///
/// # Why this is hand-written assembly and not a Rust function
///
/// Nobody knows this function's signature. It was chosen for its call count and its prologue, not
/// because anyone knows what it does -- and a Rust detour would have to *declare* a signature.
/// Declaring the wrong one is not a cosmetic error: arguments past the fourth live on the
/// caller's stack at offsets relative to the return address, so a Rust detour that builds its own
/// frame and then calls the trampoline silently hands the original function different stack
/// arguments than its caller passed. Float arguments in `xmm0`-`xmm3`, and float returns in
/// `xmm0`, are lost the same way.
///
/// A naked tail-jump has no signature to get wrong. `lock inc` touches one static and the flags
/// -- which are volatile at a function entry under every x64 calling convention -- and `jmp`
/// leaves the stack, the return address and every register exactly as the caller arranged them.
/// The original function cannot tell it was called through us. For a probe whose entire purpose
/// is to observe without perturbing, that is the requirement rather than a nicety.
///
/// `lock` is deliberate. The counter is read from another thread and this function is called from
/// several, so a plain `inc` would lose increments under contention. That is tolerable for a
/// number in the millions and **not** tolerable for the difference between `hits=0` and
/// `hits=1`, which is the single most important bit this experiment produces. The cost is
/// contention on one cache line in a function with 2052 call sites; if a run comes back
/// unplayably slow, that is data too, and dropping the `lock` prefix is a one-word change.
///
/// # Safety
///
/// Reached only through MinHook's patched prologue. [`TRAMPOLINE`] must be non-zero before the
/// hook is enabled -- see its documentation for why that ordering holds.
#[unsafe(naked)]
unsafe extern "system" fn probe_detour() {
    core::arch::naked_asm!(
        "lock inc qword ptr [rip + {hits}]",
        "jmp qword ptr [rip + {trampoline}]",
        hits = sym HIT_COUNT,
        trampoline = sym TRAMPOLINE,
    )
}

/// Compare one window against its baseline, fault-safely.
///
/// [`mem::read_bytes`] goes through `ReadProcessMemory`, which **fails closed** on an unmapped or
/// reprotected range instead of faulting. That matters more here than anywhere else in this repo:
/// a raw dereference of a page Arxan had just torn down would crash the game, and a crashed game
/// destroys the evidence the poller exists to collect.
fn check(address: usize, baseline: &[u8], observed: &mut Vec<u8>) -> Integrity {
    observed.clear();
    observed.resize(baseline.len(), 0);
    // SAFETY: `read_bytes` has no precondition on `address`; it validates the range through the
    // kernel and returns false rather than faulting.
    if !unsafe { mem::read_bytes(address, observed) } {
        return Integrity::Unreadable;
    }
    if observed.as_slice() == baseline {
        Integrity::Intact
    } else {
        Integrity::Diverged
    }
}

/// The watch loop. Never returns; the thread dies with the process.
fn poll_forever(watch: &Watch) -> ! {
    let started = Instant::now();
    let mut polls = 0u32;
    let mut observed = Vec::new();

    loop {
        std::thread::sleep(POLL_INTERVAL);
        polls = polls.wrapping_add(1);
        let uptime = started.elapsed().as_secs_f64();
        let hits = HIT_COUNT.load(Ordering::Relaxed);

        let previous = Integrity::from_u8(SITE_STATE.load(Ordering::Relaxed));
        let now = check(watch.site_address, &watch.site_baseline, &mut observed);
        if now != previous {
            if now == Integrity::Diverged {
                SITE_DIVERGENCES.fetch_add(1, Ordering::Relaxed);
            }
            SITE_STATE.store(now as u8, Ordering::Relaxed);
            log(format_args!(
                "{PROBE_LINE_PREFIX} SITE uptime={uptime:.1}s arm={} state={} prev={} \
                 hits={hits} va=0x{:016x} expected=[{}] observed=[{}]",
                watch.arm.as_str(),
                now.as_str(),
                previous.as_str(),
                watch.site_address,
                hex(&watch.site_baseline),
                observed_hex(now, &observed),
            ));
        }

        let previous = Integrity::from_u8(TRAMPOLINE_STATE.load(Ordering::Relaxed));
        let now = check(
            watch.trampoline_address,
            &watch.trampoline_baseline,
            &mut observed,
        );
        if now != previous {
            if now == Integrity::Diverged {
                TRAMPOLINE_DIVERGENCES.fetch_add(1, Ordering::Relaxed);
            }
            TRAMPOLINE_STATE.store(now as u8, Ordering::Relaxed);
            log(format_args!(
                "{PROBE_LINE_PREFIX} TRAMP uptime={uptime:.1}s arm={} state={} prev={} \
                 hits={hits} addr=0x{:016x} expected=[{}] observed=[{}]",
                watch.arm.as_str(),
                now.as_str(),
                previous.as_str(),
                watch.trampoline_address,
                hex(&watch.trampoline_baseline),
                observed_hex(now, &observed),
            ));
        }

        // THE LINE TO READ. All four measurements are on it, so a verdict is the last heartbeat
        // in the file rather than a reconstruction from scattered lines. It is also what makes a
        // crash legible: the log simply stops, and the last `uptime` is when.
        if polls.is_multiple_of(POLLS_PER_HEARTBEAT) {
            log(format_args!(
                "{PROBE_LINE_PREFIX} heartbeat uptime={uptime:.1}s {}",
                summary(hits),
            ));
        }
    }
}

/// An `UNREADABLE` window has no bytes to show, and printing 16 zeroes there would read as
/// "we saw zeroes", which is a different and false claim.
fn observed_hex(state: Integrity, observed: &[u8]) -> String {
    if state == Integrity::Unreadable {
        "<unreadable>".to_string()
    } else {
        hex(observed)
    }
}

/// The `arm=... hits=... site=... tramp=... site-diverged=... tramp-diverged=...` tail shared by
/// the heartbeat and the detach line, so the two cannot drift apart and be compared wrongly.
fn summary(hits: u64) -> String {
    let arm = WATCH_STARTED
        .get()
        .map_or(Arm::NeuterArxan, |(_, arm)| *arm);
    format!(
        "arm={} hits={hits} site={} tramp={} site-diverged={} tramp-diverged={}",
        arm.as_str(),
        Integrity::from_u8(SITE_STATE.load(Ordering::Relaxed)).as_str(),
        Integrity::from_u8(TRAMPOLINE_STATE.load(Ordering::Relaxed)).as_str(),
        SITE_DIVERGENCES.load(Ordering::Relaxed),
        TRAMPOLINE_DIVERGENCES.load(Ordering::Relaxed),
    )
}

/// The final line, written from `DLL_PROCESS_DETACH`. `None` if the probe never installed, in
/// which case there is nothing to summarise and nothing is written.
///
/// # What its presence and absence actually mean
///
/// `DLL_PROCESS_DETACH` fires on an orderly teardown (`ExitProcess`) and does **not** fire on
/// `TerminateProcess`. So this line present means the process wound down through its normal exit
/// path; this line absent, with the log ending at some heartbeat, means it did not -- a crash, a
/// kill, or a Wine teardown that skipped the notification. That is weaker than a crash handler
/// and it is stated weakly on purpose: it is the difference between "closed" and "died", not a
/// cause of death.
pub fn detach_line() -> Option<String> {
    let (started, _) = WATCH_STARTED.get()?;
    Some(format!(
        "{PROBE_LINE_PREFIX} detach uptime={:.1}s {}",
        started.elapsed().as_secs_f64(),
        summary(HIT_COUNT.load(Ordering::Relaxed)),
    ))
}
