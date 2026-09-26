//! Watchdogs for the failures a crash logger cannot see.
//!
//! A crash logger only sees a process that faults. A freeze and a long hitch raise no exception at
//! all, and each is invisible to the detector built for the other:
//!
//! | detector | question it answers | what it watches |
//! |---|---|---|
//! | stall | did the main loop stop? | the per-frame counter not advancing |
//! | frame drop | did the main loop miss a lot of frames without stopping? | frames owed vs delivered in a sliding window |
//!
//! Ported from `../er-mods-rs`'s `er-crash-logging-core/src/hang.rs`. The stall signal there is a
//! static dword `MainUpdate` increments. Here it is the dword at `GameManagerImp + 0x104`, reached
//! through the pointer at [`ds2_rva::GAME_MANAGER_IMP`] and the offset
//! [`ds2_rva::GAME_MANAGER_FRAME_COUNTER_OFFSET`]: a pointer first and a dword second. Measured at
//! runtime on 2026-09-26, it advanced exactly once per frame through a load and through play.
//!
//! When the counter stops for longer than the configured window, the watchdog snapshots every
//! thread in the process (instruction pointer, stack pointer, and a raw stack scan resolved
//! against the loaded-module table) and writes it out the same way a fault record is written. The
//! frame-drop path reports differently on purpose: a hitch is not a freeze, so it takes two cheap
//! CPU-time snapshots around the hitch, ranks threads by what they consumed, and captures stacks
//! for only the busiest few.
//!
//! # What did not come across
//!
//! Elden Ring's third detector, the loading-screen oracle, is gone. It read `CS::LoadingScreenData`
//! through a pointer published by the product DLL, and DARK SOULS II has no researched equivalent:
//! the candidate object, `FeOperatorNowLoading`, has not had its fields examined. The frame counter
//! keeps advancing through a load, so a load that stops progressing while the game stays healthy
//! is not something this module can see. `docs/DS2-FRAME-COUNTER.md` has the notes.
//!
//! # The rules it keeps
//!
//! * It proves its address before trusting it. The watchdog refuses to arm until it has watched
//!   the counter advance several times, so a game build that moves the field disarms the watchdog
//!   and says so in the log, instead of reporting a permanent fake hang. A pointer that is still
//!   null is "not yet", never a stall.
//! * It never allocates while a thread is suspended. Suspending a thread that holds the heap lock
//!   and then allocating would deadlock the process being diagnosed. Each thread is suspended,
//!   read into fixed storage, and resumed before any formatting happens.
//!
//! The watchdog is diagnostic: it never kills, never faults, and never changes game state.

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

// `StackScan::format_stack` is the only consumer outside the windows-only report paths.
#[cfg(any(windows, test))]
use crate::{LoadedModule, module_tag};
// `sample_thread` reads these out of a raw `CONTEXT`; the test module pins them against
// `CONTEXT_SIZE`. Neither consumer exists in a non-test host build.
#[cfg(any(windows, test))]
use crate::{CONTEXT_RIP_OFFSET, CONTEXT_RSP_OFFSET};
#[cfg(windows)]
use crate::{
    HangWatchdogConfig, MIN_VALID_PTR, append_log, config, loaded_modules, ms_since_install,
    path_for, safe_read_u32, utc_timestamp,
};

/// Executable the counter belongs to. Checked case-insensitively before anything is read, so the
/// watchdog stays inert if the crate is ever loaded into some other host process.
#[cfg(windows)]
const GAME_MODULE_NAME: &str = "DarkSoulsII.exe";

/// Let the game reach its main loop before the first sample.
#[cfg(windows)]
const STARTUP_DELAY_MS: u32 = 5_000;

/// Poll interval for every detector in this module.
///
/// 50ms, not 1s, because of the frame-drop detector. A hitch worth reporting is about a second of
/// lost frames, so a 1-second sampler would notice only after it ended, and a stack captured then
/// shows the recovery rather than the cause. The stall detector measures elapsed time, so the tick
/// rate does not change it.
#[cfg(windows)]
const SAMPLE_INTERVAL_MS: u32 = 50;

/// Frames of deficit that constitute a reportable hitch: about a second of lost time at 60fps.
#[cfg(windows)]
const FRAMEDROP_THRESHOLD_FRAMES: f64 = 60.0;

/// Plausible frame rates. A measured baseline outside this range means the counter is not a
/// per-frame counter on this build, so frame-drop detection disarms rather than reporting noise.
#[cfg(windows)]
const FRAMEDROP_MIN_FPS: f64 = 20.0;
#[cfg(windows)]
const FRAMEDROP_MAX_FPS: f64 = 360.0;

/// How long to watch the counter to learn the game's frame rate before arming.
#[cfg(windows)]
const FRAMEDROP_CALIBRATION_MS: u64 = 2_000;

/// Gap between the two CPU snapshots used to attribute a hitch.
#[cfg(windows)]
const FRAMEDROP_ATTRIBUTION_MS: u32 = 250;

/// Threads named in a hitch report, ranked by CPU consumed during the hitch.
#[cfg(windows)]
const FRAMEDROP_TOP_THREADS: usize = 5;

/// Hitches are common enough that an unbounded report would fill the log; a stall is not.
#[cfg(windows)]
const MAX_FRAMEDROP_REPORTS: usize = 5;

/// Minimum gap between hitch reports, so one bad streak cannot spend the whole budget at once.
#[cfg(windows)]
const FRAMEDROP_REPORT_COOLDOWN_SECS: u64 = 30;

/// How many distinct increments must be observed before the watchdog trusts the address.
#[cfg(any(windows, test))]
const ARM_ADVANCES_REQUIRED: u32 = 3;

/// Samples with a live `GameManagerImp` to wait for those increments before disarming: 60 s at
/// the 50ms tick.
///
/// Elden Ring waited 15 s. How long DARK SOULS II takes between creating `GameManagerImp` and
/// running its first update has not been measured, so this is a loose bound rather than a number
/// taken from the game. A wrong address still disarms; it just takes a minute to say so.
#[cfg(any(windows, test))]
const ARM_TIMEOUT_SAMPLES: u32 = 1_200;

/// Samples to wait for the `GameManagerImp` pointer to become non-null at all: 5 minutes.
///
/// A null pointer is boot, not a fault, so it gets its own and much longer budget than the
/// advance check above. Not measured either; boot has never been observed to take this long.
#[cfg(any(windows, test))]
const POINTER_TIMEOUT_SAMPLES: u32 = 6_000;

/// One stall is one report. Bounded so a game left frozen overnight cannot fill the disk.
#[cfg(windows)]
const MAX_HANG_REPORTS: usize = 3;

#[cfg(windows)]
const MAX_THREADS_SAMPLED: usize = 128;
#[cfg(any(windows, test))]
const THREAD_STACK_QWORDS: usize = 192;
#[cfg(any(windows, test))]
const THREAD_STACK_MAX_FRAMES: usize = 40;

#[cfg(windows)]
const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
#[cfg(windows)]
const THREAD_SUSPEND_RESUME: u32 = 0x0002;
#[cfg(windows)]
const THREAD_GET_CONTEXT: u32 = 0x0008;
#[cfg(windows)]
const THREAD_QUERY_INFORMATION: u32 = 0x0040;
#[cfg(windows)]
const INVALID_HANDLE_VALUE: isize = -1;
#[cfg(windows)]
const SUSPEND_FAILED: u32 = u32::MAX;

// x86-64 `CONTEXT`: 0x4d0 bytes, 16-byte aligned, `ContextFlags` at offset 0x30.
#[cfg(any(windows, test))]
const CONTEXT_SIZE: usize = 0x4d0;
#[cfg(any(windows, test))]
const CONTEXT_FLAGS_OFFSET: usize = 0x30;
#[cfg(windows)]
const CONTEXT_AMD64_CONTROL_INTEGER: u32 = 0x0010_0000 | 0x0000_0001 | 0x0000_0002;

/// Arming: whether the counter address deserves trust, decided from samples alone.
///
/// Pure so the rules can be tested without a game. The windows-only watchdog thread feeds it one
/// [`arming::Read`] per tick and acts on the verdict.
#[cfg(any(windows, test))]
pub(crate) mod arming {
    use super::{ARM_ADVANCES_REQUIRED, ARM_TIMEOUT_SAMPLES, POINTER_TIMEOUT_SAMPLES};

    /// One sample of `[[GAME_MANAGER_IMP] + 0x104]`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Read {
        /// The `GameManagerImp` pointer is null: early boot, not a stopped counter.
        NotYet,
        /// The pointer is non-null but the pointer or the dword behind it cannot be read. On a
        /// build where the field has moved this is one of the two shapes it takes.
        Unreadable,
        /// The counter's value.
        Value(u32),
    }

    /// Why arming gave up.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Refusal {
        /// The pointer stayed null for the whole pointer budget.
        PointerNeverSet,
        /// The pointer was set but the counter behind it could not be read.
        CounterUnreadable,
        /// The counter was readable but did not advance often enough: the other shape a moved
        /// field takes.
        NeverAdvanced,
    }

    impl Refusal {
        /// The `reason=` word written to the log.
        pub(crate) fn label(self) -> &'static str {
            match self {
                Self::PointerNeverSet => "game-manager-never-set",
                Self::CounterUnreadable => "frame-counter-unreadable",
                Self::NeverAdvanced => "frame-counter-never-advanced",
            }
        }
    }

    /// What one sample decided.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Verdict {
        /// Keep sampling.
        Waiting,
        /// The counter has been seen advancing; this is the last value read.
        Armed(u32),
        /// Stop, and log why.
        Disarmed(Refusal),
    }

    /// Counts advances and spends the two budgets.
    #[derive(Debug, Default)]
    pub(crate) struct Arming {
        previous: Option<u32>,
        advances: u32,
        live_samples: u32,
        null_samples: u32,
    }

    impl Arming {
        /// Feed one sample.
        pub(crate) fn observe(&mut self, read: Read) -> Verdict {
            match read {
                Read::NotYet => {
                    self.null_samples += 1;
                    // A pointer that goes null again after a value was read starts the advance
                    // count afresh against whatever object replaces it.
                    self.previous = None;
                    if self.null_samples >= POINTER_TIMEOUT_SAMPLES {
                        return Verdict::Disarmed(Refusal::PointerNeverSet);
                    }
                    Verdict::Waiting
                }
                Read::Unreadable => Verdict::Disarmed(Refusal::CounterUnreadable),
                Read::Value(value) => {
                    self.live_samples += 1;
                    if self.previous.is_some_and(|previous| previous != value) {
                        self.advances += 1;
                    }
                    self.previous = Some(value);
                    if self.advances >= ARM_ADVANCES_REQUIRED {
                        return Verdict::Armed(value);
                    }
                    if self.live_samples >= ARM_TIMEOUT_SAMPLES {
                        return Verdict::Disarmed(Refusal::NeverAdvanced);
                    }
                    Verdict::Waiting
                }
            }
        }
    }

    /// Turn the two reads of a sample into a [`Read`].
    ///
    /// `slot` is what was read from the static pointer slot: `None` if the slot itself could not
    /// be read. `counter` reads the dword at a given object address. A null pointer is
    /// [`Read::NotYet`]; a pointer into the reserved low range, an unreadable slot, or an
    /// unreadable dword are all [`Read::Unreadable`].
    pub(crate) fn classify(
        slot: Option<usize>,
        min_valid_ptr: usize,
        offset: usize,
        counter: impl FnOnce(usize) -> Option<u32>,
    ) -> Read {
        match slot {
            None => Read::Unreadable,
            Some(0) => Read::NotYet,
            Some(object) if object < min_valid_ptr => Read::Unreadable,
            Some(object) => match object.checked_add(offset).and_then(counter) {
                Some(value) => Read::Value(value),
                None => Read::Unreadable,
            },
        }
    }
}

/// Frame-drop detection: a leaky bucket of frames owed against frames delivered.
///
/// A stall watchdog answers "did the main loop stop". This answers "did the main loop miss a lot
/// of frames without stopping", the question behind every report of a hitch, a stutter, or a
/// second-long freeze that resolves itself.
///
/// The measure is frames owed within a sliding window, not since the session began. A running
/// total never forgets an isolated stutter, so a long healthy session would eventually cross any
/// threshold. `expected` comes from a frame rate measured at arming rather than an assumed 60: a
/// 30fps cap would otherwise read as a permanent 50% deficit.
#[cfg(any(windows, test))]
pub(crate) mod framedrop {
    /// Samples retained. At the module's 50ms tick this covers well over the window; the window
    /// is enforced by accumulated time, so a caller ticking at a different rate is still correct.
    const CAPACITY: usize = 128;

    /// How recent a deficit has to be to count toward a hitch.
    pub(crate) const WINDOW_SECONDS: f64 = 2.0;

    /// Frames owed inside the window, and the rate the debt is measured against.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Detector {
        baseline_fps: f64,
        threshold: f64,
        /// (deficit frames, seconds) per observation, oldest first. Fixed storage: a watchdog
        /// must not allocate while diagnosing a process that may be sick.
        samples: [(f64, f64); CAPACITY],
        len: usize,
    }

    /// What a crossing looked like, carried into the report.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub(crate) struct Hitch {
        /// Frames owed inside the window when it crossed.
        pub(crate) deficit_frames: f64,
        /// The rate the debt was measured against.
        pub(crate) baseline_fps: f64,
        /// Frames delivered in the tick that crossed the threshold.
        pub(crate) observed_frames: u32,
        /// Wall seconds that tick covered.
        pub(crate) window_seconds: f64,
    }

    impl Detector {
        /// `None` when the measured rate is not a plausible frame rate.
        #[must_use]
        pub(crate) fn new(
            baseline_fps: f64,
            threshold: f64,
            min_fps: f64,
            max_fps: f64,
        ) -> Option<Self> {
            if !baseline_fps.is_finite() || baseline_fps < min_fps || baseline_fps > max_fps {
                return None;
            }
            Some(Self {
                baseline_fps,
                threshold,
                samples: [(0.0, 0.0); CAPACITY],
                len: 0,
            })
        }

        /// Frames owed inside the current window.
        #[must_use]
        pub(crate) fn deficit(&self) -> f64 {
            self.samples[..self.len]
                .iter()
                .map(|(deficit, _)| *deficit)
                .sum::<f64>()
                .max(0.0)
        }

        /// Feed one observation. Returns a `Hitch` when the windowed deficit crosses the threshold.
        ///
        /// A surplus tick is recorded as a negative deficit so a burst partly recovered inside the
        /// window reads as the net loss, but the reported total floors at zero.
        pub(crate) fn observe(
            &mut self,
            frames_advanced: u32,
            window_seconds: f64,
        ) -> Option<Hitch> {
            if window_seconds <= 0.0 || !window_seconds.is_finite() {
                return None;
            }
            let expected = self.baseline_fps * window_seconds;
            self.push((expected - f64::from(frames_advanced), window_seconds));
            self.evict_older_than(WINDOW_SECONDS);

            let deficit = self.deficit();
            if deficit < self.threshold {
                return None;
            }
            let hitch = Hitch {
                deficit_frames: deficit,
                baseline_fps: self.baseline_fps,
                observed_frames: frames_advanced,
                window_seconds,
            };
            // Cleared after reporting, or the window stays over threshold and every later tick
            // re-fires the same hitch.
            self.reset();
            Some(hitch)
        }

        /// Forget the window: after a report, and when the counter has legitimately stopped (a
        /// stall the other detector owns) so recovery is not billed here.
        pub(crate) fn reset(&mut self) {
            self.len = 0;
        }

        fn push(&mut self, sample: (f64, f64)) {
            if self.len == CAPACITY {
                self.samples.copy_within(1.., 0);
                self.len -= 1;
            }
            self.samples[self.len] = sample;
            self.len += 1;
        }

        fn evict_older_than(&mut self, window: f64) {
            // Keep the newest samples whose accumulated time fits the window, walking from the end
            // so the most recent observation is retained even if it alone exceeds it.
            let mut total = 0.0;
            let mut keep = 0usize;
            for (_, seconds) in self.samples[..self.len].iter().rev() {
                total += *seconds;
                keep += 1;
                if total >= window {
                    break;
                }
            }
            if keep < self.len {
                let start = self.len - keep;
                self.samples.copy_within(start..self.len, 0);
                self.len = keep;
            }
        }
    }
}

#[cfg(windows)]
static HANG_REPORTS_WRITTEN: AtomicUsize = AtomicUsize::new(0);
#[cfg(windows)]
static WATCHDOG_STARTED: std::sync::Once = std::sync::Once::new();
#[cfg(windows)]
static HANG_CONFIG: OnceLock<HangWatchdogConfig> = OnceLock::new();

#[cfg(windows)]
type ThreadStart = unsafe extern "system" fn(*mut c_void) -> u32;

#[cfg(any(windows, test))]
#[repr(C)]
struct ThreadEntry32 {
    size: u32,
    usage: u32,
    thread_id: u32,
    owner_process_id: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}

#[repr(C, align(16))]
#[cfg(any(windows, test))]
struct ThreadContext([u8; CONTEXT_SIZE]);

#[cfg(windows)]
unsafe extern "system" {
    fn CreateThread(
        attributes: *mut c_void,
        stack_size: usize,
        start: ThreadStart,
        parameter: *mut c_void,
        flags: u32,
        thread_id: *mut u32,
    ) -> isize;
    fn Sleep(milliseconds: u32);
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> isize;
    fn Thread32First(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
    fn Thread32Next(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
    fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> isize;
    fn SuspendThread(thread: isize) -> u32;
    fn ResumeThread(thread: isize) -> u32;
    fn GetThreadContext(thread: isize, context: *mut c_void) -> i32;
    fn GetThreadTimes(
        thread: isize,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
}

#[cfg(windows)]
fn sleep_ms(milliseconds: u32) {
    // SAFETY: `Sleep` takes a plain integer and touches no memory of ours.
    unsafe { Sleep(milliseconds) };
}

#[cfg(windows)]
fn hang_config() -> HangWatchdogConfig {
    HANG_CONFIG.get().copied().unwrap_or_default()
}

/// Start the watchdog thread. Returns whether a thread was started by this call.
///
/// Call it once startup is over, not from `DllMain`: the thread is created and never joined. A
/// `stall_seconds` of 0 disables the watchdog. First call wins.
#[cfg(windows)]
pub(crate) fn start(new_config: HangWatchdogConfig) -> bool {
    if new_config.stall_seconds == 0 {
        return false;
    }
    let mut started = false;
    WATCHDOG_STARTED.call_once(|| {
        let _ = HANG_CONFIG.set(new_config);
        // SAFETY: `watchdog_thread` is a `'static` fn item of the thread-start signature and takes
        // no parameter, so nothing of ours has to outlive the call. The handle is closed below.
        let handle = unsafe {
            CreateThread(
                std::ptr::null_mut(),
                0,
                watchdog_thread,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == 0 {
            append_log(format_args!("hang watchdog CreateThread failed"));
            return;
        }
        // SAFETY: `handle` was just returned non-zero by `CreateThread` and is closed once.
        unsafe { CloseHandle(handle) };
        started = true;
    });
    started
}

/// The address of the static `GameManagerImp` pointer slot, refusing any host that is not the
/// expected exe.
#[cfg(windows)]
fn locate_game_manager_slot() -> Option<usize> {
    // SAFETY: a null name is the documented spelling of "this process's own image"; the returned
    // handle is borrowed and needs no release.
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    if base < MIN_VALID_PTR {
        return None;
    }
    let is_expected_host = loaded_modules()
        .iter()
        .any(|module| module.base == base && module.name.eq_ignore_ascii_case(GAME_MODULE_NAME));
    if !is_expected_host {
        return None;
    }
    base.checked_add(ds2_rva::GAME_MANAGER_IMP as usize)
}

/// One sample of the counter, reading the pointer afresh every time.
#[cfg(windows)]
fn read_counter(slot: usize) -> arming::Read {
    // SAFETY: `safe_read_usize` takes any address and fails closed on an unmapped one.
    let object = unsafe { ds2_game_base::mem::safe_read_usize(slot) };
    arming::classify(
        object,
        MIN_VALID_PTR,
        ds2_rva::GAME_MANAGER_FRAME_COUNTER_OFFSET,
        // SAFETY: same reader contract as above: any address, fails closed.
        |address| unsafe { safe_read_u32(address) },
    )
}

/// The live `GameManagerImp` pointer, for a report line. 0 when unreadable.
#[cfg(windows)]
fn read_game_manager(slot: usize) -> usize {
    // SAFETY: `safe_read_usize` takes any address and fails closed on an unmapped one.
    unsafe { ds2_game_base::mem::safe_read_usize(slot) }.unwrap_or(0)
}

#[cfg(windows)]
unsafe extern "system" fn watchdog_thread(_parameter: *mut c_void) -> u32 {
    sleep_ms(STARTUP_DELAY_MS);

    let Some(slot) = locate_game_manager_slot() else {
        append_log(format_args!(
            "hang watchdog disarmed reason=game-module-not-found expected={GAME_MODULE_NAME}"
        ));
        return 0;
    };

    let mut arming = arming::Arming::default();
    let mut last_value = loop {
        match arming.observe(read_counter(slot)) {
            arming::Verdict::Waiting => sleep_ms(SAMPLE_INTERVAL_MS),
            arming::Verdict::Armed(value) => break value,
            arming::Verdict::Disarmed(refusal) => {
                append_log(format_args!(
                    "hang watchdog disarmed reason={} slot=0x{slot:x} rva=0x{:x} offset=0x{:x} \
                     game_manager=0x{:x} note=a game build that moved the field looks like this",
                    refusal.label(),
                    ds2_rva::GAME_MANAGER_IMP,
                    ds2_rva::GAME_MANAGER_FRAME_COUNTER_OFFSET,
                    read_game_manager(slot),
                ));
                return 0;
            }
        }
    };

    let stall = hang_config().stall_seconds;
    append_log(format_args!(
        "hang watchdog armed slot=0x{slot:x} game_manager=0x{:x} offset=0x{:x} \
         frame_counter={last_value} stall_seconds={stall}",
        read_game_manager(slot),
        ds2_rva::GAME_MANAGER_FRAME_COUNTER_OFFSET,
    ));

    // Learn the frame rate before judging any deficit against it. The counter has already been
    // proven to advance, so this only measures how fast.
    let mut framedrop_detector = match calibrate_frame_rate(slot) {
        Some(fps) => match framedrop::Detector::new(
            fps,
            FRAMEDROP_THRESHOLD_FRAMES,
            FRAMEDROP_MIN_FPS,
            FRAMEDROP_MAX_FPS,
        ) {
            Some(detector) => {
                append_log(format_args!(
                    "hang watchdog framedrop detector armed baseline={fps:.1}fps \
                     threshold={FRAMEDROP_THRESHOLD_FRAMES:.0} frames"
                ));
                Some(detector)
            }
            None => {
                append_log(format_args!(
                    "hang watchdog framedrop detector inert: measured {fps:.1}fps is outside \
                     {FRAMEDROP_MIN_FPS:.0}..{FRAMEDROP_MAX_FPS:.0}, so this counter is probably \
                     not per-frame on this build"
                ));
                None
            }
        },
        None => {
            append_log(format_args!(
                "hang watchdog framedrop detector inert: frame rate could not be measured"
            ));
            None
        }
    };

    let mut last_change = std::time::Instant::now();
    let mut reported_this_stall = false;
    let mut framedrop_reports = 0usize;
    let mut last_framedrop_report: Option<std::time::Instant> = None;
    let mut last_tick = std::time::Instant::now();
    // Re-read so the first delta below starts after calibration, not two seconds before it.
    if let arming::Read::Value(value) = read_counter(slot) {
        last_value = value;
    }
    // Set while the pointer is null, so the first value after it is a new baseline rather than a
    // delta across the gap.
    let mut rebaseline = false;
    let mut paused_for_modal = false;

    loop {
        sleep_ms(SAMPLE_INTERVAL_MS);

        let value = match read_counter(slot) {
            arming::Read::Value(value) => value,
            arming::Read::NotYet => {
                // No `GameManagerImp` right now. That is "not yet" again, never a stall: nothing
                // is advancing a counter that does not exist.
                last_change = std::time::Instant::now();
                last_tick = std::time::Instant::now();
                rebaseline = true;
                if let Some(detector) = framedrop_detector.as_mut() {
                    detector.reset();
                }
                continue;
            }
            arming::Read::Unreadable => {
                append_log(format_args!(
                    "hang watchdog disarmed reason=frame-counter-unreadable slot=0x{slot:x} \
                     game_manager=0x{:x} note=the pointer or the field moved after arming",
                    read_game_manager(slot),
                ));
                return 0;
            }
        };
        // One of this workspace's own modal windows holds the game thread on purpose (ds2-save-file's
        // file dialog). Frames stop for as long as the player takes, which is not a hang: the
        // clocks restart from the moment it closes.
        if ds2_game_base::modal::active() {
            if !paused_for_modal {
                paused_for_modal = true;
                append_log(format_args!(
                    "hang watchdog paused: a modal window of ours holds the game thread"
                ));
            }
            rebaseline = true;
            if let Some(detector) = framedrop_detector.as_mut() {
                detector.reset();
            }
            continue;
        }
        if paused_for_modal {
            paused_for_modal = false;
            append_log(format_args!(
                "hang watchdog resumed: the modal window closed, the stall clock starts again"
            ));
        }
        if rebaseline {
            rebaseline = false;
            last_value = value;
            last_change = std::time::Instant::now();
            last_tick = std::time::Instant::now();
            continue;
        }

        // Frame-drop accounting runs every tick, before the stall logic, because a hitch is
        // frames arriving too slowly rather than not at all.
        let window_seconds = last_tick.elapsed().as_secs_f64();
        last_tick = std::time::Instant::now();
        if let Some(detector) = framedrop_detector.as_mut() {
            let advanced = value.wrapping_sub(last_value);
            let cooling = last_framedrop_report
                .is_some_and(|at| at.elapsed().as_secs() < FRAMEDROP_REPORT_COOLDOWN_SECS);
            // Over budget or inside the cooldown, `observe` has still cleared the debt, so the
            // next hitch is measured fresh instead of firing the instant the cooldown ends.
            if let Some(hitch) = detector.observe(advanced, window_seconds)
                && !cooling
                && framedrop_reports < MAX_FRAMEDROP_REPORTS
            {
                framedrop_reports += 1;
                last_framedrop_report = Some(std::time::Instant::now());
                report_framedrop(hitch);
                // Attribution slept; the next window would otherwise bill that sleep as missing
                // frames and fire again at once.
                last_tick = std::time::Instant::now();
                if let arming::Read::Value(now) = read_counter(slot) {
                    last_value = now;
                } else {
                    last_value = value;
                }
                detector.reset();
                continue;
            }
        }

        if value != last_value {
            last_value = value;
            last_change = std::time::Instant::now();
            reported_this_stall = false;
            continue;
        }
        if reported_this_stall {
            continue;
        }
        let stalled_for = last_change.elapsed();
        if stalled_for.as_secs() < stall {
            continue;
        }
        reported_this_stall = true;
        if HANG_REPORTS_WRITTEN.fetch_add(1, Ordering::SeqCst) >= MAX_HANG_REPORTS {
            append_log(format_args!(
                "hang watchdog report budget exhausted; further stalls will not be captured"
            ));
            return 0;
        }
        report_stall(slot, value, stalled_for.as_secs());
    }
}

/// Total CPU (kernel + user) each thread has consumed, in 100ns units.
///
/// Cheap enough to call twice around a hitch: it opens a handle per thread and reads a counter,
/// suspending nothing.
#[cfg(windows)]
fn thread_cpu_times() -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    for thread_id in enumerate_threads() {
        // SAFETY: `OpenThread` takes plain integers; a failure answers 0, checked next.
        let thread = unsafe { OpenThread(THREAD_QUERY_INFORMATION, 0, thread_id) };
        if thread == 0 {
            continue;
        }
        let (mut creation, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
        // SAFETY: `thread` is a live handle opened above, and the four outputs are locals this
        // frame owns, each the `FILETIME`-sized u64 the API writes.
        let ok =
            unsafe { GetThreadTimes(thread, &mut creation, &mut exit, &mut kernel, &mut user) };
        // SAFETY: `thread` was opened above and is closed exactly once.
        unsafe { CloseHandle(thread) };
        if ok != 0 {
            out.push((thread_id, kernel.saturating_add(user)));
        }
    }
    out
}

/// Rank threads by CPU consumed between two snapshots, busiest first.
///
/// Pure so it can be tested without a process: an attribution that ranks wrongly points the
/// reader at an idle thread with total confidence.
#[cfg(any(windows, test))]
fn rank_cpu_consumers(before: &[(u32, u64)], after: &[(u32, u64)]) -> Vec<(u32, u64)> {
    let mut deltas: Vec<(u32, u64)> = after
        .iter()
        .filter_map(|(thread_id, later)| {
            let earlier = before
                .iter()
                .find(|(candidate, _)| candidate == thread_id)
                .map(|(_, value)| *value)?;
            Some((*thread_id, later.saturating_sub(earlier)))
        })
        .filter(|(_, delta)| *delta > 0)
        .collect();
    // Tie-break on thread id so the same situation always reports the same order.
    deltas.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    deltas
}

/// Report a hitch, naming the threads that consumed the time.
///
/// The two CPU snapshots straddle a short window taken at detection, so they measure who is busy
/// while the hitch is still happening. Only the busiest few get their stacks captured, because
/// that capture suspends threads.
#[cfg(windows)]
fn report_framedrop(hitch: framedrop::Hitch) {
    let before = thread_cpu_times();
    sleep_ms(FRAMEDROP_ATTRIBUTION_MS);
    let after = thread_cpu_times();
    let ranked = rank_cpu_consumers(&before, &after);

    let window_ms = f64::from(FRAMEDROP_ATTRIBUTION_MS);
    let total: u64 = ranked.iter().map(|(_, delta)| *delta).sum();
    append_log(format_args!(
        "hang watchdog framedrop: {:.0} frames owed at a measured {:.1} fps baseline \
         ({} frame(s) delivered in {:.0}ms). Attributing over the next {:.0}ms: {} thread(s) \
         consumed CPU, {:.1}ms total.",
        hitch.deficit_frames,
        hitch.baseline_fps,
        hitch.observed_frames,
        hitch.window_seconds * 1000.0,
        window_ms,
        ranked.len(),
        total as f64 / 10_000.0,
    ));

    let modules = loaded_modules();
    for (thread_id, delta) in ranked.iter().take(FRAMEDROP_TOP_THREADS) {
        let cpu_ms = *delta as f64 / 10_000.0;
        let share = if total > 0 {
            (*delta as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        match sample_thread(*thread_id) {
            Some(sampled) => append_log(format_args!(
                "  framedrop thread {thread_id}: {cpu_ms:.1}ms CPU ({share:.0}% of measured) {}",
                sampled.scan.format_stack(&modules)
            )),
            None => append_log(format_args!(
                "  framedrop thread {thread_id}: {cpu_ms:.1}ms CPU ({share:.0}% of measured) \
                 <stack unavailable>"
            )),
        }
    }
}

/// Measure the game's frame rate so the deficit is judged against what the game delivers.
#[cfg(windows)]
fn calibrate_frame_rate(slot: usize) -> Option<f64> {
    let value = |read: arming::Read| match read {
        arming::Read::Value(value) => Some(value),
        arming::Read::NotYet | arming::Read::Unreadable => None,
    };
    let start = std::time::Instant::now();
    let first = value(read_counter(slot))?;
    let mut last = first;
    let mut wrapped = 0u64;
    while start.elapsed().as_millis() < u128::from(FRAMEDROP_CALIBRATION_MS) {
        sleep_ms(SAMPLE_INTERVAL_MS);
        let current = value(read_counter(slot))?;
        if current < last {
            // A u32 wraps eventually; account for it rather than measuring a huge negative rate
            // and disarming on a healthy game.
            wrapped += 1;
        }
        last = current;
    }
    let elapsed = start.elapsed().as_secs_f64();
    if elapsed <= 0.0 {
        return None;
    }
    let advanced =
        (u64::from(last) + wrapped * (u64::from(u32::MAX) + 1)).saturating_sub(u64::from(first));
    Some(advanced as f64 / elapsed)
}

#[cfg(windows)]
fn report_stall(slot: usize, frame_counter: u32, stalled_seconds: u64) {
    use std::fmt::Write as _;

    let modules = loaded_modules();
    let game_manager = read_game_manager(slot);
    let mut out = String::new();
    let _ = writeln!(out, "reason=main-thread-stall");
    let _ = writeln!(out, "module={}", config().module_label);
    let _ = writeln!(out, "utc={}", utc_timestamp());
    let _ = writeln!(out, "ms_since_install={}", ms_since_install());
    let _ = writeln!(out, "stalled_seconds={stalled_seconds}");
    let _ = writeln!(
        out,
        "stall_threshold_seconds={}",
        hang_config().stall_seconds
    );
    let _ = writeln!(out, "frame_counter={frame_counter}");
    let _ = writeln!(
        out,
        "frame_counter_addr=0x{:x}",
        game_manager.wrapping_add(ds2_rva::GAME_MANAGER_FRAME_COUNTER_OFFSET)
    );
    let _ = writeln!(
        out,
        "game_manager=0x{game_manager:x} game_manager_rva=0x{:x} frame_counter_offset=0x{:x} \
         host={GAME_MODULE_NAME}",
        ds2_rva::GAME_MANAGER_IMP,
        ds2_rva::GAME_MANAGER_FRAME_COUNTER_OFFSET,
    );
    let _ = writeln!(
        out,
        "report_index={}",
        HANG_REPORTS_WRITTEN
            .load(Ordering::SeqCst)
            .saturating_sub(1)
    );

    // Who is running, measured rather than guessed: two CPU snapshots straddling a short window.
    // A thread that burned no CPU across it is parked. Taken before the stacks so the window is
    // not lengthened by a hundred suspend/resume pairs.
    let cpu_before = thread_cpu_times();
    sleep_ms(FRAMEDROP_ATTRIBUTION_MS);
    let cpu_after = thread_cpu_times();
    let ranked = rank_cpu_consumers(&cpu_before, &cpu_after);
    let burned: std::collections::BTreeMap<u32, u64> = ranked.iter().copied().collect();

    let threads = enumerate_threads();
    let _ = writeln!(out, "thread_count={}", threads.len());

    // The first-created thread is labelled as that and nothing more. Elden Ring's port called it
    // the main thread, and under a mod loader that parks the first thread for the life of the
    // process the label sent two investigations the wrong way. The CPU delta beside each thread
    // says which ones were doing anything.
    let mut earliest_creation = u64::MAX;
    let mut first_thread_id = 0u32;
    let mut samples = Vec::with_capacity(threads.len());
    for thread_id in threads {
        if let Some(sample) = sample_thread(thread_id) {
            if sample.creation_time != 0 && sample.creation_time < earliest_creation {
                earliest_creation = sample.creation_time;
                first_thread_id = sample.thread_id;
            }
            samples.push(sample);
        }
    }

    let total_burned: u64 = burned.values().sum();
    let _ = writeln!(
        out,
        "cpu_window_ms={} threads_burning_cpu={} total_cpu_ms={:.1}",
        FRAMEDROP_ATTRIBUTION_MS,
        burned.values().filter(|delta| **delta > 0).count(),
        total_burned as f64 / 10_000.0,
    );
    for (thread_id, delta) in ranked.iter().take(FRAMEDROP_TOP_THREADS) {
        if *delta == 0 {
            break;
        }
        let _ = writeln!(
            out,
            "cpu_top tid={thread_id} cpu_ms={:.1}",
            *delta as f64 / 10_000.0
        );
    }

    for sample in &samples {
        let is_first = sample.thread_id == first_thread_id;
        let cpu_ms = burned.get(&sample.thread_id).copied().unwrap_or(0) as f64 / 10_000.0;
        let _ = writeln!(
            out,
            "thread tid={} first_thread={} cpu_ms={:.1} rip=0x{:x}{} rsp=0x{:x} stack={}",
            sample.thread_id,
            is_first,
            cpu_ms,
            sample.rip,
            module_tag(sample.rip, &modules),
            sample.rsp,
            sample.scan.format_stack(&modules)
        );
    }

    let _ = std::fs::write(path_for(hang_config().report_file_name), &out);
    append_log(format_args!("{out}---"));

    // The text snapshot is a raw stack scan; the dump carries the real unwind for every thread.
    // SAFETY: a null exception pointer is the documented "no faulting thread" form, which is what
    // a live, frozen process is.
    unsafe { crate::write_minidump_named(hang_config().minidump_file_name, std::ptr::null_mut()) };
}

/// Every thread id in this process, from a `ToolHelp` snapshot. Suspends nothing.
#[cfg(windows)]
fn enumerate_threads() -> Vec<u32> {
    let mut ids = Vec::new();
    // SAFETY: takes no pointer.
    let process_id = unsafe { GetCurrentProcessId() };
    // SAFETY: plain integer arguments; failure is checked on the next line.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
        return ids;
    }
    let mut entry = new_thread_entry();
    // SAFETY: `snapshot` is live and `entry` is a local of the `THREADENTRY32` layout with its
    // size field set, which the API requires.
    let mut ok = unsafe { Thread32First(snapshot, &mut entry) };
    while ok != 0 && ids.len() < MAX_THREADS_SAMPLED {
        if entry.owner_process_id == process_id {
            ids.push(entry.thread_id);
        }
        entry = new_thread_entry();
        // SAFETY: same as `Thread32First` above.
        ok = unsafe { Thread32Next(snapshot, &mut entry) };
    }
    // SAFETY: `snapshot` was opened above and is closed exactly once.
    unsafe { CloseHandle(snapshot) };
    ids
}

#[cfg(any(windows, test))]
fn new_thread_entry() -> ThreadEntry32 {
    ThreadEntry32 {
        size: std::mem::size_of::<ThreadEntry32>() as u32,
        usage: 0,
        thread_id: 0,
        owner_process_id: 0,
        base_priority: 0,
        delta_priority: 0,
        flags: 0,
    }
}

/// A raw stack scan in fixed storage, filled while its thread is suspended.
#[cfg(any(windows, test))]
struct StackScan {
    stack: [usize; THREAD_STACK_QWORDS],
    len: usize,
}

#[cfg(any(windows, test))]
impl StackScan {
    fn empty() -> Self {
        Self {
            stack: [0; THREAD_STACK_QWORDS],
            len: 0,
        }
    }

    /// The qwords that resolve into a loaded module, deduplicated in a row, as `[mod+0x..,...]`.
    fn format_stack(&self, modules: &[LoadedModule]) -> String {
        let mut out = String::from("[");
        let mut emitted = 0usize;
        let mut last = String::new();
        for value in self.stack.iter().take(self.len) {
            if emitted >= THREAD_STACK_MAX_FRAMES {
                break;
            }
            let tag = module_tag(*value, modules);
            if tag.is_empty() || tag == last {
                continue;
            }
            if emitted != 0 {
                out.push(',');
            }
            // `module_tag` wraps in braces for inline use; unwrap it for the list form.
            out.push_str(tag.trim_matches(['{', '}']));
            emitted += 1;
            last = tag;
        }
        out.push(']');
        out
    }
}

#[cfg(windows)]
struct ThreadSample {
    thread_id: u32,
    rip: usize,
    rsp: usize,
    creation_time: u64,
    scan: StackScan,
}

/// Snapshot one thread.
///
/// Everything between `SuspendThread` and `ResumeThread` writes into fixed-size storage only. A
/// heap allocation there could block on a lock the suspended thread owns and freeze the process
/// for real, which would be a diagnostic causing the failure it exists to observe.
///
/// What bounds the risk: the watchdog sleeps 5 s before it looks at anything, refuses to arm until
/// it has watched the frame counter advance, and suspends only after a detector has fired. One
/// thread is suspended at a time, and the budgets are three stall reports and five frame-drop
/// reports for the life of the process. This suspension is not serialised against `ds2-hook`'s
/// `MinHook` freezes, so a frame-drop report landing on a thread inside another module's freeze
/// window is the same hazard class, just an unlikely one.
#[cfg(windows)]
fn sample_thread(thread_id: u32) -> Option<ThreadSample> {
    // SAFETY: takes no pointer.
    if thread_id == unsafe { GetCurrentThreadId() } {
        return None;
    }
    let access = THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION;
    // SAFETY: plain integer arguments; failure answers 0, checked next.
    let thread = unsafe { OpenThread(access, 0, thread_id) };
    if thread == 0 {
        return None;
    }

    let mut sample = ThreadSample {
        thread_id,
        rip: 0,
        rsp: 0,
        creation_time: 0,
        scan: StackScan::empty(),
    };

    // SAFETY: `thread` is a live handle opened with suspend access. It is resumed below on every
    // path that suspended it, and nothing in between allocates.
    if unsafe { SuspendThread(thread) } != SUSPEND_FAILED {
        let mut context = ThreadContext([0u8; CONTEXT_SIZE]);
        context.0[CONTEXT_FLAGS_OFFSET..CONTEXT_FLAGS_OFFSET + 4]
            .copy_from_slice(&CONTEXT_AMD64_CONTROL_INTEGER.to_le_bytes());
        // SAFETY: `context` is a 16-byte-aligned local of the full x64 `CONTEXT` size with its
        // flags set, which is what `GetThreadContext` writes into.
        let got_context =
            unsafe { GetThreadContext(thread, context.0.as_mut_ptr().cast::<c_void>()) };
        if got_context != 0 {
            sample.rip = read_context_field(&context, CONTEXT_RIP_OFFSET);
            sample.rsp = read_context_field(&context, CONTEXT_RSP_OFFSET);
            if sample.rsp >= MIN_VALID_PTR {
                for slot in 0..THREAD_STACK_QWORDS {
                    let addr = sample.rsp + slot * std::mem::size_of::<usize>();
                    // SAFETY: `safe_read_usize` takes any address and fails closed.
                    match unsafe { ds2_game_base::mem::safe_read_usize(addr) } {
                        Some(value) => {
                            sample.scan.stack[slot] = value;
                            sample.scan.len = slot + 1;
                        }
                        None => break,
                    }
                }
            }
        }
        // SAFETY: the thread was suspended by this function just above.
        unsafe { ResumeThread(thread) };
    }

    let (mut creation, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
    // SAFETY: `thread` is live, and the four outputs are locals this frame owns.
    if unsafe { GetThreadTimes(thread, &mut creation, &mut exit, &mut kernel, &mut user) } != 0 {
        sample.creation_time = creation;
    }
    // SAFETY: `thread` was opened above and is closed exactly once.
    unsafe { CloseHandle(thread) };
    Some(sample)
}

#[cfg(any(windows, test))]
fn read_context_field(context: &ThreadContext, offset: usize) -> usize {
    let mut bytes = [0u8; std::mem::size_of::<usize>()];
    bytes.copy_from_slice(&context.0[offset..offset + std::mem::size_of::<usize>()]);
    usize::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::arming::{Arming, Read, Refusal, Verdict, classify};
    use super::framedrop::Detector;
    use super::*;

    /// A steady 60fps must never accumulate a deficit, however long it runs.
    #[test]
    fn framedrop_stays_quiet_at_a_steady_baseline() {
        let mut detector = Detector::new(60.0, 60.0, 20.0, 360.0).expect("60fps is plausible");
        for _ in 0..(20 * 300) {
            assert_eq!(detector.observe(3, 0.05), None);
        }
        assert_eq!(
            detector.deficit(),
            0.0,
            "a healthy run must not drift upward"
        );
    }

    /// A full stop crosses 60 owed frames after about a second, not before.
    #[test]
    fn framedrop_fires_after_roughly_one_second_of_lost_frames() {
        let mut detector = Detector::new(60.0, 60.0, 20.0, 360.0).expect("60fps is plausible");
        let mut ticks = 0;
        let hitch = loop {
            ticks += 1;
            assert!(ticks < 100, "must fire well within 5s of a total stop");
            if let Some(hitch) = detector.observe(0, 0.05) {
                break hitch;
            }
        };
        assert_eq!(ticks, 20, "fired after {ticks} ticks of 50ms");
        assert!(hitch.deficit_frames >= 60.0);
        assert_eq!(hitch.observed_frames, 0);
    }

    /// Half rate is a real hitch, just a slower-accumulating one.
    #[test]
    fn framedrop_fires_on_sustained_reduced_rate() {
        let mut detector = Detector::new(60.0, 60.0, 20.0, 360.0).expect("60fps is plausible");
        let mut fired = None;
        for tick in 1..=200 {
            if let Some(hitch) = detector.observe(1, 0.05) {
                fired = Some((tick, hitch));
                break;
            }
        }
        let (tick, hitch) = fired.expect("20fps against a 60fps baseline owes 40 frames/sec");
        assert_eq!(tick, 30, "2 owed per 50ms tick; 60 owed at tick 30");
        assert!(hitch.deficit_frames >= 60.0);
    }

    /// Isolated stutters must drain, or a long session would eventually false-fire on noise.
    #[test]
    fn framedrop_drains_isolated_stutters() {
        let mut detector = Detector::new(60.0, 60.0, 20.0, 360.0).expect("60fps is plausible");
        let mut peak: f64 = 0.0;
        for _ in 0..500 {
            assert_eq!(detector.observe(0, 0.05), None);
            for _ in 0..10 {
                assert_eq!(detector.observe(3, 0.05), None);
            }
            peak = peak.max(detector.deficit());
        }
        assert!(peak < 20.0, "isolated stutters peaked at {peak}");
    }

    /// Credit must not bank: a fast stretch cannot pay for a later hitch.
    #[test]
    fn framedrop_does_not_bank_credit_from_a_fast_stretch() {
        let mut detector = Detector::new(60.0, 60.0, 20.0, 360.0).expect("60fps is plausible");
        for _ in 0..200 {
            assert_eq!(detector.observe(30, 0.05), None, "far above baseline");
        }
        assert_eq!(
            detector.deficit(),
            0.0,
            "surplus never reads as a negative debt"
        );
        let mut ticks = 0;
        while detector.observe(0, 0.05).is_none() {
            ticks += 1;
            assert!(ticks < 80, "a hitch after a fast stretch must still fire");
        }
        assert!(
            (20..=60).contains(&(ticks + 1)),
            "fired after {} ticks",
            ticks + 1
        );
    }

    /// An implausible measured rate disarms rather than reporting noise forever.
    #[test]
    fn framedrop_refuses_an_implausible_baseline() {
        assert!(Detector::new(4.0, 60.0, 20.0, 360.0).is_none());
        assert!(Detector::new(5000.0, 60.0, 20.0, 360.0).is_none());
        assert!(Detector::new(f64::NAN, 60.0, 20.0, 360.0).is_none());
        assert!(Detector::new(60.0, 60.0, 20.0, 360.0).is_some());
        assert!(
            Detector::new(30.0, 60.0, 20.0, 360.0).is_some(),
            "a 30fps cap is legitimate"
        );
    }

    /// A zero or negative window must be ignored, not divided by.
    #[test]
    fn framedrop_ignores_a_degenerate_window() {
        let mut detector = Detector::new(60.0, 60.0, 20.0, 360.0).expect("60fps is plausible");
        assert_eq!(detector.observe(0, 0.0), None);
        assert_eq!(detector.observe(0, -1.0), None);
        assert_eq!(detector.observe(0, f64::NAN), None);
        assert_eq!(detector.deficit(), 0.0);
    }

    #[test]
    fn cpu_ranking_orders_by_consumption_and_drops_idle_threads() {
        let before = [(10u32, 1_000u64), (11, 5_000), (12, 0), (13, 700)];
        let after = [(10u32, 1_100u64), (11, 45_000), (12, 0), (13, 700)];
        assert_eq!(
            rank_cpu_consumers(&before, &after),
            vec![(11, 40_000), (10, 100)]
        );
    }

    #[test]
    fn cpu_ranking_handles_threads_appearing_and_vanishing() {
        let before = [(10u32, 500u64), (11, 900)];
        let after = [(10u32, 900u64), (99, 12_345)];
        assert_eq!(rank_cpu_consumers(&before, &after), vec![(10, 400)]);
        assert_eq!(rank_cpu_consumers(&[(1, 900)], &[(1, 100)]), vec![]);
    }

    /// A null `GameManagerImp` is boot, not a reading of zero.
    #[test]
    fn a_null_game_manager_is_not_yet() {
        let read = classify(Some(0), 0x10000, 0x104, |_| {
            panic!("must not dereference null")
        });
        assert_eq!(read, Read::NotYet);
    }

    /// The dword is read at the object plus the offset, from the pointer read this sample.
    #[test]
    fn the_counter_is_read_behind_the_pointer() {
        let read = classify(Some(0x7ff0_0000), 0x10000, 0x104, |address| {
            assert_eq!(address, 0x7ff0_0104);
            Some(42)
        });
        assert_eq!(read, Read::Value(42));
    }

    /// An unreadable slot, a pointer into the reserved low range, or an unreadable dword are all
    /// the shape a moved field takes, and none of them is a counter value.
    #[test]
    fn unreadable_shapes_are_unreadable() {
        assert_eq!(
            classify(None, 0x10000, 0x104, |_| Some(1)),
            Read::Unreadable
        );
        assert_eq!(
            classify(Some(0x40), 0x10000, 0x104, |_| panic!(
                "low pointers are not chased"
            )),
            Read::Unreadable
        );
        assert_eq!(
            classify(Some(0x7ff0_0000), 0x10000, 0x104, |_| None),
            Read::Unreadable
        );
        assert_eq!(
            classify(Some(usize::MAX), 0x10000, 0x104, |_| Some(1)),
            Read::Unreadable
        );
    }

    #[test]
    fn arming_waits_through_a_null_pointer_then_arms_on_advances() {
        let mut arming = Arming::default();
        for _ in 0..1_000 {
            assert_eq!(arming.observe(Read::NotYet), Verdict::Waiting);
        }
        assert_eq!(arming.observe(Read::Value(10)), Verdict::Waiting);
        assert_eq!(arming.observe(Read::Value(11)), Verdict::Waiting);
        assert_eq!(
            arming.observe(Read::Value(11)),
            Verdict::Waiting,
            "a repeat is no advance"
        );
        assert_eq!(arming.observe(Read::Value(12)), Verdict::Waiting);
        assert_eq!(arming.observe(Read::Value(13)), Verdict::Armed(13));
    }

    /// One advance could be noise in an unrelated dword; it must never arm on its own.
    #[test]
    fn a_single_advance_does_not_arm() {
        const _: () = assert!(ARM_ADVANCES_REQUIRED > 1);
        const _: () = assert!(ARM_TIMEOUT_SAMPLES > ARM_ADVANCES_REQUIRED);
        let mut arming = Arming::default();
        assert_eq!(arming.observe(Read::Value(1)), Verdict::Waiting);
        assert_eq!(arming.observe(Read::Value(2)), Verdict::Waiting);
        for _ in 0..(ARM_TIMEOUT_SAMPLES - 3) {
            assert_eq!(arming.observe(Read::Value(2)), Verdict::Waiting);
        }
        assert_eq!(
            arming.observe(Read::Value(2)),
            Verdict::Disarmed(Refusal::NeverAdvanced),
            "a field that sits still is a moved field, not a hang"
        );
    }

    /// Advances seen before the pointer went null again do not count toward the new object.
    #[test]
    fn a_pointer_that_goes_null_restarts_the_comparison() {
        let mut arming = Arming::default();
        assert_eq!(arming.observe(Read::Value(1)), Verdict::Waiting);
        assert_eq!(arming.observe(Read::Value(2)), Verdict::Waiting);
        assert_eq!(arming.observe(Read::NotYet), Verdict::Waiting);
        assert_eq!(
            arming.observe(Read::Value(500)),
            Verdict::Waiting,
            "a new object's value is a baseline, not an advance"
        );
    }

    #[test]
    fn an_unreadable_counter_disarms_at_once() {
        let mut arming = Arming::default();
        assert_eq!(
            arming.observe(Read::Unreadable),
            Verdict::Disarmed(Refusal::CounterUnreadable)
        );
    }

    #[test]
    fn a_pointer_that_never_appears_disarms_eventually() {
        let mut arming = Arming::default();
        for _ in 0..(POINTER_TIMEOUT_SAMPLES - 1) {
            assert_eq!(arming.observe(Read::NotYet), Verdict::Waiting);
        }
        assert_eq!(
            arming.observe(Read::NotYet),
            Verdict::Disarmed(Refusal::PointerNeverSet)
        );
        assert_eq!(Refusal::PointerNeverSet.label(), "game-manager-never-set");
    }

    /// The address is the measured one: `[[exe + 0x016148f0]] + 0x104`.
    #[test]
    fn the_counter_constants_match_the_measurement() {
        assert_eq!(ds2_rva::GAME_MANAGER_IMP, 0x0161_48f0);
        assert_eq!(ds2_rva::GAME_MANAGER_FRAME_COUNTER_OFFSET, 0x104);
    }

    #[test]
    fn context_layout_matches_windows_amd64() {
        const _: () = assert!(CONTEXT_FLAGS_OFFSET + 4 <= CONTEXT_SIZE);
        const _: () = assert!(CONTEXT_RIP_OFFSET + std::mem::size_of::<usize>() <= CONTEXT_SIZE);
        const _: () = assert!(CONTEXT_RSP_OFFSET + std::mem::size_of::<usize>() <= CONTEXT_SIZE);
        assert_eq!(std::mem::align_of::<ThreadContext>(), 16);
    }

    #[test]
    fn thread_entry_size_matches_toolhelp_layout() {
        assert_eq!(std::mem::size_of::<ThreadEntry32>(), 28);
        assert_eq!(new_thread_entry().size as usize, 28);
    }

    #[test]
    fn context_field_reads_little_endian_qword() {
        let mut context = ThreadContext([0u8; CONTEXT_SIZE]);
        context.0[CONTEXT_RIP_OFFSET..CONTEXT_RIP_OFFSET + 8]
            .copy_from_slice(&0x7ff6_1597_9999_usize.to_le_bytes());
        assert_eq!(
            read_context_field(&context, CONTEXT_RIP_OFFSET),
            0x7ff6_1597_9999
        );
    }

    #[test]
    fn stack_formatting_dedupes_and_drops_unresolved_addresses() {
        let modules = vec![LoadedModule {
            base: 0x1000_0000,
            size: 0x1000,
            name: String::from("DarkSoulsII.exe"),
        }];
        let mut scan = StackScan::empty();
        scan.stack[0] = 0x1000_0010;
        scan.stack[1] = 0x1000_0010; // repeat: collapsed
        scan.stack[2] = 0xdead_beef; // outside every module: dropped
        scan.stack[3] = 0x1000_0020;
        scan.len = 4;
        assert_eq!(
            scan.format_stack(&modules),
            "[DarkSoulsII.exe+0x10,DarkSoulsII.exe+0x20]"
        );
    }
}
