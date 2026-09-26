//! Moving DARK SOULS II's camera from code, and keeping a human's hands out of the measurement.
//!
//! # The one question this crate had to answer first
//!
//! *Where does DARK SOULS II read input?* Injecting anywhere else does nothing -- that is the
//! lesson `../er-mods-rs` paid for twice on Elden Ring, and it is written on its
//! `er-input-harness` in capitals. So the answer here was read out of the binary before a line
//! of this crate existed, and it is recorded with its derivation in `ds2-rva` beside the
//! offsets. In short:
//!
//! * **`DarkSoulsII.exe` imports `DINPUT8.dll` (one thunk) and `XINPUT1_3.dll` (two).** There is
//!   no raw-input API in the import table at all.
//! * **Three `DLUID::*Device` classes own the DirectInput/XInput reads**, each overriding vtable
//!   slot 23: `PadDevice` (`0x140f05540`), `MouseDevice` (`0x140f074a0`), `KeyboardDevice`
//!   (`0x140f06dd0`).
//! * **The keyboard is `GetDeviceState(0x100, ...)`** -- the 256-byte DIK table.
//! * **The pad has three backends** -- `XInputGetState`, `GetDeviceState(0x50, ...)` for a
//!   DirectInput joystick, and a third HID-shaped one -- **and all three normalise into the same
//!   six floats on the device object.**
//! * **The camera's mouse-look is none of those.** It is `WindowsMouseDevice`
//!   (`0x140b5c1f0`), which calls `GetCursorPos` -> `ScreenToClient` -> `GetClientRect` and
//!   stores a clamped client-space position; `parseCameraInput` (`0x140b0c950`) turns the
//!   DIFFERENCE between two successive values of it into camera motion. `DLUID::MouseDevice` is
//!   a real device the engine polls, but thirty frames of authored delta in it moved the
//!   published camera yaw by exactly zero. That was a miss in the first version of this crate,
//!   caught by a live run; the whole chain is now traced in `ds2-rva`.
//!
//! The engine downstream of these objects reads the DEVICE, never the API, so one write per
//! device after its own poll has run covers every backend a player might have plugged in, and a
//! value written there is indistinguishable from one the hardware produced.
//!
//! **After, not before.** Each poll rewrites its device's fields from scratch, so a value
//! written ahead of the original is simply overwritten. Same edge, same fix, as
//! `er-input-harness`'s `pad_inject`.
//!
//! # What this crate does NOT claim
//!
//! It does not claim which input a given session's camera follows. Mouse-look's chain is traced
//! end to end and is the default; whether a pad axis also drives the camera depends on the
//! mapper's bindings, on the player's settings and on whether anything is plugged in. What is
//! here instead is a way to MEASURE it: `probe` holds each channel in turn -- both mouse
//! components and all six pad axes -- and reports what the camera's yaw did, and `turn` is a
//! closed loop on that same yaw which reports `NO RESPONSE` rather than success when the channel
//! it drives moves nothing. See [`drive::Channel`] and [`turn`].
//!
//! # How an agent drives it
//!
//! Two ways, and they are the same machine:
//!
//! * **From outside the process**, with the game already running: write a sequence number and a
//!   command to `<Game>/ds2-input-harness-cmd.txt`. See [`command`] for the grammar. This is the
//!   one that matters, because it costs no relaunch.
//! * **From inside**, from another crate in the same DLL: [`request`].
//!
//! # Safety valve
//!
//! Every command is frame-bounded and there is no unbounded form of any of them, including the
//! block. A harness that wedges stops pressing on its own and gives the player their controller
//! back; see [`command::MAX_BLOCK_FRAMES`].

pub mod authored;
pub mod command;
pub mod drive;
pub mod log;
pub mod turn;

#[cfg(windows)]
mod device;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub use crate::command::Command;
pub use crate::log::{LOG_PREFIX, LogFn, set_logger};

use crate::drive::Session;

/// The harness's whole state. A `Mutex` rather than atomics because this is a state machine and
/// not a value, and because its users are two game threads at most -- the `Present` tick, which
/// is the only thing that advances it, and whatever calls [`request`] -- with a critical section
/// of a few dozen arithmetic instructions, no allocation and nothing that can panic.
static SESSION: Mutex<Session> = Mutex::new(Session::new());

/// Whether the detours should blank what the hardware produced. Published out of [`SESSION`]
/// once per frame so the device detours never need the lock: they run on whatever thread the
/// engine polls from, and a detour on the input path that can block on a render-thread tick is
/// a hitch waiting to happen.
static BLOCKING: AtomicBool = AtomicBool::new(false);

/// A source of the camera's yaw in degrees, installed by the loader. `0` when none is.
///
/// This is a SEAM and not a camera search, deliberately. `ds2-invasion-path` already resolves
/// the camera the overlay draws through, validated against the projection matrix's own shape and
/// against where the game says the player is -- so wiring that in gives the closed loop the
/// exact camera the measurement is about, instead of a second resolution that could disagree
/// with it. A harness with no yaw source still holds sticks; it just refuses to claim degrees.
static YAW_SOURCE: AtomicUsize = AtomicUsize::new(0);

/// Signature of a yaw source: degrees, or `None` when no camera is resolvable this frame.
pub type YawFn = fn() -> Option<f32>;

/// Install the camera-yaw source the closed loop measures against.
///
/// Call before [`install`]. Without it, `turn` and `probe` refuse rather than guess.
pub fn set_yaw_source(source: YawFn) {
    YAW_SOURCE.store(source as usize, Ordering::Release);
}

/// The camera's yaw this frame, if anything can say.
#[must_use]
pub fn yaw() -> Option<f32> {
    let raw = YAW_SOURCE.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    // SAFETY: `raw` is only ever a `YawFn` stored by `set_yaw_source` above.
    let source: YawFn = unsafe { std::mem::transmute::<usize, YawFn>(raw) };
    source()
}

/// Take the session lock, ignoring poisoning.
///
/// Poisoning here would mean a panic inside the state machine, which has no allocation, no
/// indexing that is not bounds-checked by construction and no unwrap. If one ever happens, a
/// harness that then refuses to release the stick would be strictly worse than one that carries
/// on, so the recovery is to carry on.
fn session() -> std::sync::MutexGuard<'static, Session> {
    SESSION.lock().unwrap_or_else(|error| error.into_inner())
}

/// Ask the harness to do something, from inside this process.
///
/// The same entry point the command file goes through, so nothing can be asked for here that
/// could not be asked for from outside -- including the frame bounds.
pub fn request(command: Command) {
    session().accept(command, yaw());
}

/// Turn the camera by `degrees`, measured against the camera's own yaw.
///
/// Convenience over [`request`]; identical to `turn <degrees>` in the command file. Returns
/// immediately -- the turn runs over the following frames and reports its outcome to the log.
pub fn turn_degrees(degrees: f32) {
    request(Command::Turn {
        degrees,
        budget: command::DEFAULT_TURN_BUDGET_FRAMES,
    });
}

/// Blank every human input the engine reads, for `frames` frames.
///
/// Capped at [`command::MAX_BLOCK_FRAMES`]; see there for why there is no unbounded form.
pub fn block_human_input(frames: u32) {
    request(Command::Block {
        frames: frames.min(command::MAX_BLOCK_FRAMES),
    });
}

/// Stop blanking the player's input now.
pub fn unblock_human_input() {
    request(Command::Unblock);
}

/// Stop authoring anything. Does not lift a block.
pub fn release() {
    request(Command::Release);
}

/// Is the human's input currently being blanked?
#[must_use]
pub fn is_blocking() -> bool {
    BLOCKING.load(Ordering::Relaxed)
}

/// Camera motion, in degrees on one frame, that counts as somebody's hand on the controls while
/// the harness is blocking and authoring nothing.
///
/// Bigger than the camera's own follow-and-spring drift, because the point is to catch
/// interference rather than to report the engine breathing. The test is only applied while a
/// `block` is running -- outside one, a camera that moves while the harness is idle is simply a
/// player playing, which is not contamination and not news.
pub const FOREIGN_MOTION_DEGREES: f32 = 2.0;

/// The yaw the previous tick saw, as `f32` bits, and whether there was one.
#[cfg(windows)]
static LAST_TICK_YAW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
#[cfg(windows)]
static LAST_TICK_YAW_VALID: AtomicBool = AtomicBool::new(false);
/// Frames on which the camera moved while a block was in force and nothing was being authored.
static FOREIGN_MOTION_FRAMES: AtomicU64 = AtomicU64::new(0);

/// Frames since the harness last authored anything. Starts high, so a run that has not authored
/// yet gets no grace.
#[cfg(windows)]
static FRAMES_SINCE_AUTHORED: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(u32::MAX);

/// Frames after the harness stops authoring during which the camera may still be carrying its
/// push, and motion is not counted as foreign.
///
/// Measured on 2026-09-26: after `pad3` was released at full deflection the camera kept turning
/// about 4.4 degrees a frame, and the check logged that as contamination although nothing but
/// the harness had touched a control. The probe already waits this many frames for the same
/// reason before it reports.
#[cfg(windows)]
const AUTHORED_COAST_FRAMES: u32 = 15;

/// Advance the harness one frame: poll the command file, step the state machine, publish the
/// result for the detours to stamp.
///
/// **The clock is `Present`, not an input device.** `ds2-invasion-path`'s frame hook calls this
/// once per rendered frame, which runs whatever is plugged in, whatever has focus, and whether
/// or not anybody has touched a control for an hour. The previous version advanced from inside
/// whichever device poll fired first, and a live session showed what that costs: the game
/// stopped calling the elected poll and the harness went deaf mid-run.
#[cfg(windows)]
pub fn on_present_frame() {
    device::tick_devices();
    device::poll_command_file();
    let yaw_now = yaw();
    let frame = session().frame(yaw_now);
    frame.authored.publish();
    BLOCKING.store(frame.block, Ordering::Relaxed);
    detect_foreign_motion(yaw_now, &frame);
}

/// Notice the camera moving when, by the harness's own account, nothing should be moving it.
///
/// This is the experiment's integrity check. While a `block` is in force the human's input is
/// supposed to be blanked, so a camera that swings anyway means either that a hand reached an
/// input path this crate does not cover, or that a device the block relies on stopped being
/// polled. Either way the run is contaminated and the log has to say so, because a measurement
/// that cannot detect its own contamination is not evidence.
#[cfg(windows)]
fn detect_foreign_motion(yaw_now: Option<f32>, frame: &drive::Frame) {
    let Some(now) = yaw_now else {
        LAST_TICK_YAW_VALID.store(false, Ordering::Relaxed);
        return;
    };
    let had_previous = LAST_TICK_YAW_VALID.swap(true, Ordering::Relaxed);
    let previous = f32::from_bits(LAST_TICK_YAW.swap(now.to_bits(), Ordering::Relaxed));
    if !frame.authored.is_empty() {
        FRAMES_SINCE_AUTHORED.store(0, Ordering::Relaxed);
        return;
    }
    // Saturating, so the "never authored" marker stays at the top instead of wrapping to zero.
    let since_authored = FRAMES_SINCE_AUTHORED
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            Some(n.saturating_add(1))
        })
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    if !had_previous || !frame.block || since_authored <= AUTHORED_COAST_FRAMES {
        return;
    }
    let moved = turn::wrap_degrees(now - previous).abs();
    if moved < FOREIGN_MOTION_DEGREES {
        return;
    }
    let total = FOREIGN_MOTION_FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    // Only the first few, and then powers of ten: a contaminated run should say so loudly once,
    // not drown the log it is trying to make readable.
    if total <= 3 || total.is_power_of_two() {
        log::log(format_args!(
            "{LOG_PREFIX} CONTAMINATED: the camera's yaw moved {moved:.2} degrees while a block \
             was in force and the harness was authoring nothing. Either an input path this crate \
             does not cover reached the camera, or a device whose poll does the blanking has \
             stopped being called. {total} frames so far -- treat any measurement from this run \
             as suspect."
        ));
    }
}

/// Frames on which the camera moved while blocked and unauthored. Zero is the only clean answer.
#[must_use]
pub fn foreign_motion_frames() -> u64 {
    FOREIGN_MOTION_FRAMES.load(Ordering::Relaxed)
}

/// Detour the three `DLUID` device polls.
///
/// Call from the loader's post-Arxan callback, on the entry-point thread, with `DllMain` already
/// returned -- the same position every other hooking feature in this workspace installs from.
///
/// Returns how many of the three sites were hooked. A partial install is reported rather than
/// hidden: with only the pad hooked, a mouse-and-keyboard player's input is not blocked, and a
/// run that believed otherwise would be a contaminated experiment.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Must run after `neuter_arxan` (or after
/// `schedule_after_arxan`). All three targets were checked with `scripts/ds2-arxan-chain.py` and
/// are clean prologues rather than Arxan redirects, and the install re-reads the recorded
/// prologue bytes and refuses any site whose first five bytes are not the ones `ds2-rva`
/// records.
#[cfg(windows)]
pub unsafe fn install() -> usize {
    // SAFETY: every pointer here is one the game handed this detour, or is derived from it by an
    // offset this crate validated before installing. The callee's own contract asks for exactly
    // that live object, and reads inside it go through the fault-tolerant readers.
    unsafe { device::install() }
}

/// How many times each device poll has run, in the order pad, DirectInput mouse, keyboard,
/// Win32 mouse.
///
/// A zero here is the difference between "the harness pressed nothing" and "the harness was
/// never reached", which read identically from the outside and mean opposite things. A pad count
/// of zero on a keyboard-and-mouse session is expected; all four zero after a hooked install is
/// a detour that lost its prologue to something else. `status` logs these.
#[cfg(windows)]
#[must_use]
pub fn poll_counts() -> [u64; 4] {
    device::fire_counts()
}

/// Host build: there is nothing to hook, and saying so is better than a crate that silently
/// does nothing.
#[cfg(not(windows))]
pub fn install() -> usize {
    log::log(format_args!(
        "{LOG_PREFIX} install skipped: this is a host build, there is no game image to detour"
    ));
    0
}
