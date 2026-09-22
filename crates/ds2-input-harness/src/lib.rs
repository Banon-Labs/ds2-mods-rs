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
//! * **Three `DLUID::*Device` classes own the reads**, each overriding vtable slot 23:
//!   `PadDevice` (`0x140f05540`), `MouseDevice` (`0x140f074a0`), `KeyboardDevice`
//!   (`0x140f06dd0`).
//! * **The mouse is `IDirectInputDevice8::GetDeviceState(0x14, ...)`** -- a `DIMOUSESTATE2`,
//!   converted to floats on the device object. That is the mouse-look path, whole.
//! * **The keyboard is `GetDeviceState(0x100, ...)`** -- the 256-byte DIK table.
//! * **The pad has three backends** -- `XInputGetState`, `GetDeviceState(0x50, ...)` for a
//!   DirectInput joystick, and a third HID-shaped one -- **and all three normalise into the same
//!   six floats on the device object.**
//!
//! That convergence is what makes this crate small. The engine downstream of these three
//! objects (the `DLUI`/`DLUID` mapper) reads the DEVICE, never the API, so one write per device
//! after its own poll has run covers every backend a player might have plugged in, and a value
//! written there is indistinguishable from one the hardware produced.
//!
//! **After, not before.** Each poll rewrites its device's fields from scratch, so a value
//! written ahead of the original is simply overwritten. Same edge, same fix, as
//! `er-input-harness`'s `pad_inject`.
//!
//! # What this crate does NOT claim
//!
//! It does not claim that pad axis 3 is the camera. That binding lives in the `DLUI` mapper,
//! it is affected by the player's own settings, and it was not established statically. What is
//! here instead is a way to MEASURE it: `probe` holds each axis in turn and reports what the
//! camera's yaw did, and `turn` is a closed loop on that same yaw which reports `NO RESPONSE`
//! rather than success when the axis it drives moves nothing. See [`drive`] and [`turn`].
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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub use crate::command::Command;
pub use crate::log::{LOG_PREFIX, LogFn, set_logger};

use crate::drive::Session;

/// The harness's whole state. A `Mutex` rather than atomics because this is a state machine and
/// not a value, and because every one of its users is the game thread -- the three device
/// detours are the only callers, they run in sequence within one frame, and the critical section
/// is a few dozen arithmetic instructions with no allocation and nothing that can panic.
static SESSION: Mutex<Session> = Mutex::new(Session::new());

/// Whether the detours should blank what the hardware produced. Published out of [`SESSION`]
/// once per frame so the two detours that are not the frame owner do not need the lock.
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

/// Advance the harness one frame: poll the command file, step the state machine, publish the
/// result for the detours to stamp.
///
/// Called by exactly one of the three device detours per frame -- see [`device`] for how that
/// one is chosen -- so the state machine advances once per frame however many devices exist.
#[cfg(windows)]
fn on_frame() {
    device::poll_command_file();
    let frame = session().frame(yaw());
    frame.authored.publish();
    BLOCKING.store(frame.block, Ordering::Relaxed);
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
    unsafe { device::install() }
}

/// How many times each of the three device polls has run, in the order pad, mouse, keyboard.
///
/// A zero here is the difference between "the harness pressed nothing" and "the harness was
/// never reached", which read identically from the outside and mean opposite things. A pad
/// count of zero on a keyboard-and-mouse session is expected; all three zero after a hooked
/// install is a detour that lost its prologue to something else.
#[cfg(windows)]
#[must_use]
pub fn poll_counts() -> [u64; 3] {
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
