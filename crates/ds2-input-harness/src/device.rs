//! The four detours, and the writes they make.
//!
//! # Why a detour per device rather than one hook on the mapper
//!
//! The mapper is where a value becomes "camera X" rather than "axis 3", which sounds like the
//! better place to write. It is not, for this job: the mapper's binding is the thing being
//! measured, and a harness that wrote past it could never tell you whether the binding is what
//! you thought. Writing the DEVICE keeps the whole of the engine's own interpretation -- the
//! deadzone, the sensitivity setting, the Y inversion, the mapper itself -- in the loop, so what
//! the game does with an injected input is what it would do with a real one.
//!
//! # Why the write happens after the original
//!
//! Each poll rebuilds its device's fields from the API it just called. A value written first is
//! overwritten by the original; a value written after is the one the consumer reads that frame.
//!
//! # The mouse is two devices, and only one of them is the camera
//!
//! `DLUID::MouseDevice` reads DirectInput and is a real device the engine really polls. It is
//! NOT what turns the camera: measured live, thirty frames of authored delta in it moved the
//! published yaw by exactly zero. `WindowsMouseDevice` is -- it reads `GetCursorPos`, and
//! `parseCameraInput` differences two successive values of what it stores. `ds2-rva` carries the
//! whole chain. So the DirectInput mouse is blanked (it feeds something, and a block that left
//! it alone would be a half-block) and never authored, and the Win32 mouse is both.
//!
//! # The virtual cursor
//!
//! The camera's mouse input is a DIFFERENCE of absolute positions, so authoring a constant
//! position produces one frame of motion and then stillness. This module therefore keeps a
//! virtual cursor, adds the authored delta to it each frame, and publishes it.
//!
//! **Engagement is one-way on purpose.** Once a virtual position has been published, reverting
//! to the real one would hand the consumer a single frame whose difference is the whole gap
//! between them -- a spin, not a stop. So once engaged the module stays engaged, and while it is
//! not driving it advances the virtual cursor by exactly the real cursor's own delta. That is
//! behaviourally identical to vanilla: the player's hand moves the camera by the same amount it
//! always did. It is also what makes blocking work, because blocking is simply the same loop
//! with the real delta left out.
//!
//! # Which detour is the frame
//!
//! Whichever of the four fires FIRST claims ownership, and from then on only that one advances
//! the state machine. That is what makes the tick exactly one per frame regardless of how many
//! devices exist: a mouse-and-keyboard player has no pad polling, and neither case needs a
//! special path.

use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, read_bytes};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::authored::Authored;
use crate::command::{Command, SequenceGate};
use crate::log::harness_log;

/// The file an agent outside the process writes. Sits beside the game executable, where every
/// other file this repo reads or writes at runtime lives.
const COMMAND_FILE_NAME: &str = "ds2-input-harness-cmd.txt";

/// How often the command file is read, in frames.
///
/// A file read every frame is a syscall per frame for a path that is idle almost always. Twelve
/// frames is a fifth of a second at 60fps and costs the game thread essentially nothing -- the
/// same number, for the same reason, that `er-input-harness`'s command loop settled on.
const COMMAND_POLL_INTERVAL_FRAMES: u64 = 12;

/// Frames observed, for the command poll's throttle. Advanced by the `Present` tick, never by a
/// device poll.
static FRAME: AtomicU64 = AtomicU64::new(0);

/// How many consecutive ticks a device poll must go without firing before it is called gone.
///
/// Half a second at 60fps. Long enough that a frame in which the engine simply skipped a poll is
/// not an event; short enough that a controller pulled out of its socket is reported while the
/// run that cares is still going.
const DEVICE_SILENT_TICKS: u64 = 30;

/// The poll counts the previous tick saw, so a device that stops being polled can be noticed.
static LAST_COUNTS: [AtomicU64; SITES.len()] = [const { AtomicU64::new(0) }; SITES.len()];
/// Consecutive ticks each poll has been silent for.
static SILENT_TICKS: [AtomicU64; SITES.len()] = [const { AtomicU64::new(0) }; SITES.len()];
/// Whether each poll has been reported as gone, so it is said once rather than every frame.
static REPORTED_GONE: [AtomicBool; SITES.len()] = [const { AtomicBool::new(false) }; SITES.len()];

/// One hookable device poll.
struct Site {
    /// What to call it in a log line.
    name: &'static str,
    /// Where it is, as an RVA.
    rva: u32,
    /// The five bytes that must be there, or the site is refused.
    prologue: [u8; 5],
}

/// Index into [`SITES`], [`TRAMPOLINES`] and [`FIRED`]. Also the value [`TICK_OWNER`] holds.
const PAD: usize = 0;
const DINPUT_MOUSE: usize = 1;
const KEYBOARD: usize = 2;
const WINDOWS_MOUSE: usize = 3;

const SITES: [Site; 4] = [
    Site {
        name: "pad",
        rva: ds2_rva::PAD_DEVICE_POLL,
        prologue: ds2_rva::PAD_DEVICE_POLL_PROLOGUE,
    },
    Site {
        name: "dinput-mouse",
        rva: ds2_rva::MOUSE_DEVICE_POLL,
        prologue: ds2_rva::MOUSE_DEVICE_POLL_PROLOGUE,
    },
    Site {
        name: "keyboard",
        rva: ds2_rva::KEYBOARD_DEVICE_POLL,
        prologue: ds2_rva::KEYBOARD_DEVICE_POLL_PROLOGUE,
    },
    Site {
        name: "windows-mouse",
        rva: ds2_rva::WINDOWS_MOUSE_DEVICE_POLL,
        prologue: ds2_rva::WINDOWS_MOUSE_DEVICE_POLL_PROLOGUE,
    },
];

/// Trampolines back to the originals, published before each site is patched so a detour that
/// fires immediately cannot read a zero and silently skip the game's own poll.
static TRAMPOLINES: [AtomicUsize; SITES.len()] = [const { AtomicUsize::new(0) }; SITES.len()];

/// How many times each detour has fired. Reported by `status`, and the difference between "the
/// harness pressed nothing" and "the harness was never reached" -- two things that read
/// identically from the outside and mean opposite things.
static FIRED: [AtomicU64; SITES.len()] = [const { AtomicU64::new(0) }; SITES.len()];

/// Whether the virtual cursor is being published. One-way; see the module docs.
static CURSOR_ENGAGED: AtomicBool = AtomicBool::new(false);
/// The virtual cursor, as `f32` bits so a fractional push accumulates instead of rounding to
/// nothing every frame.
static VIRTUAL_X: AtomicU32 = AtomicU32::new(0);
static VIRTUAL_Y: AtomicU32 = AtomicU32::new(0);
/// The real cursor position the previous poll stored, so the human's own delta can be measured.
static REAL_PREV_X: AtomicI32 = AtomicI32::new(0);
static REAL_PREV_Y: AtomicI32 = AtomicI32::new(0);

/// Every one of the polls is `void/bool poll(this)` with `this` in RCX: they return in `al` (or
/// not at all) and read no other argument register. The detours hand back whatever the original
/// returned, unchanged.
type PollFn = unsafe extern "system" fn(*mut u8) -> u64;

/// Run the original, then let the harness have its say.
///
/// # Safety
///
/// `this` is the device the game is polling, `index` a valid index into [`SITES`]. Every write
/// below is at an offset `ds2-rva` recorded from the poll's own disassembly, so it lands inside
/// the object the original has just finished writing.
unsafe fn poll(index: usize, this: *mut u8) -> u64 {
    FIRED[index].fetch_add(1, Ordering::Relaxed);

    let trampoline = TRAMPOLINES[index].load(Ordering::Acquire);
    let result = if trampoline == 0 {
        0
    } else {
        // SAFETY: MinHook published this trampoline for exactly this site, and the signature is
        // the one all the overrides implement.
        let original: PollFn = unsafe { std::mem::transmute::<usize, PollFn>(trampoline) };
        unsafe { original(this) }
    };

    // NO TICK HERE. These detours are write-only: they stamp whatever the state machine last
    // published and nothing else. The clock is `Present`, through `ds2-invasion-path`'s frame
    // hook -- see `crate::on_present_frame`. It used to be right here, electing whichever poll
    // fired first as the frame owner, and a live session showed exactly what is wrong with that:
    // the game stopped calling the elected poll and the harness went deaf, three fresh commands
    // producing no log line at all while the process was alive. A device can be unplugged, lose
    // focus, or simply stop being polled; a rendered frame cannot.
    if this.is_null() {
        return result;
    }
    let blocking = crate::is_blocking();
    let authored = Authored::current();
    // SAFETY: `this` is the live device object the original has just written, and every offset
    // is one that same function wrote.
    unsafe {
        match index {
            PAD => write_pad(this, blocking, &authored),
            DINPUT_MOUSE => write_dinput_mouse(this, blocking),
            KEYBOARD => write_keyboard(this, blocking),
            WINDOWS_MOUSE => write_windows_mouse(this, blocking, &authored),
            _ => {}
        }
    }
    result
}

/// Store an `f32` at `base + offset`.
///
/// # Safety
///
/// `base + offset` must be inside the live device object.
unsafe fn put_f32(base: *mut u8, offset: usize, value: f32) {
    unsafe { base.add(offset).cast::<f32>().write_unaligned(value) };
}

/// Read an `i32` at `base + offset`.
///
/// # Safety
///
/// `base + offset` must be inside the live device object.
unsafe fn get_i32(base: *mut u8, offset: usize) -> i32 {
    unsafe { base.add(offset).cast::<i32>().read_unaligned() }
}

/// Store an `i32` at `base + offset`.
///
/// # Safety
///
/// `base + offset` must be inside the live device object.
unsafe fn put_i32(base: *mut u8, offset: usize, value: i32) {
    unsafe { base.add(offset).cast::<i32>().write_unaligned(value) };
}

/// Zero `len` bytes at `base + offset`.
///
/// # Safety
///
/// The whole range must be inside the live device object.
unsafe fn zero(base: *mut u8, offset: usize, len: usize) {
    unsafe { base.add(offset).write_bytes(0, len) };
}

/// Blank the player's pad and stamp whatever the harness is authoring.
///
/// # Safety
///
/// `this` is a live `DLUID::PadDevice` whose poll has just run.
unsafe fn write_pad(this: *mut u8, blocking: bool, authored: &Authored) {
    if blocking {
        // The six normalised axes, the button word and both triggers: everything the poll
        // itself writes, so everything downstream can see.
        unsafe {
            zero(
                this,
                ds2_rva::PAD_DEVICE_AXES_OFFSET,
                ds2_rva::PAD_DEVICE_AXIS_COUNT * size_of::<f32>(),
            );
            zero(this, ds2_rva::PAD_DEVICE_BUTTONS_OFFSET, size_of::<u16>());
            zero(
                this,
                ds2_rva::PAD_DEVICE_LEFT_TRIGGER_OFFSET,
                2 * size_of::<f32>(),
            );
            // And the raw `DIJOYSTATE` the joystick arm filled. The poll copies the AXES out of
            // it but leaves its button bytes where they are, so a DirectInput pad's buttons live
            // only here -- blanking the normalised fields alone would leave them pressed.
            zero(
                this,
                ds2_rva::PAD_DEVICE_DIJOYSTATE_OFFSET,
                ds2_rva::PAD_DEVICE_DIJOYSTATE_BYTES,
            );
        }
    }
    for (index, value) in authored.axes.iter().enumerate() {
        if let Some(value) = value {
            unsafe {
                put_f32(
                    this,
                    ds2_rva::PAD_DEVICE_AXES_OFFSET + index * size_of::<f32>(),
                    *value,
                );
            }
        }
    }
    if let Some(mask) = authored.buttons {
        unsafe {
            this.add(ds2_rva::PAD_DEVICE_BUTTONS_OFFSET)
                .cast::<u16>()
                .write_unaligned(mask);
        }
    }
    if let Some(value) = authored.triggers[0] {
        unsafe { put_f32(this, ds2_rva::PAD_DEVICE_LEFT_TRIGGER_OFFSET, value) };
    }
    if let Some(value) = authored.triggers[1] {
        unsafe { put_f32(this, ds2_rva::PAD_DEVICE_RIGHT_TRIGGER_OFFSET, value) };
    }
}

/// Blank the player's DirectInput mouse.
///
/// **Blanked but never authored.** This device is not the camera -- see the module docs -- so
/// writing a delta into it would be pressing a button nothing is listening to. It is still real
/// input reaching the engine, so a `block` that skipped it would be a half-block.
///
/// # Safety
///
/// `this` is a live `DLUID::MouseDevice` whose poll has just run.
unsafe fn write_dinput_mouse(this: *mut u8, blocking: bool) {
    if blocking {
        unsafe {
            // The raw `DIMOUSESTATE2` first -- its `rgbButtons` are not copied out by the poll,
            // so the buttons live only there.
            zero(
                this,
                ds2_rva::MOUSE_DEVICE_RAW_STATE_OFFSET,
                ds2_rva::MOUSE_DEVICE_RAW_STATE_BYTES,
            );
            // Then the three floats the poll derived: X, Y and the wheel, contiguous.
            zero(
                this,
                ds2_rva::MOUSE_DEVICE_DELTA_X_OFFSET,
                3 * size_of::<f32>(),
            );
        }
    }
}

/// Blank the player's keyboard.
///
/// There is no authored counterpart: the harness has no reason to press a key, and a vocabulary
/// it does not need is a surface it should not have. The block is the whole of the keyboard's
/// involvement.
///
/// # Safety
///
/// `this` is a live `DLUID::KeyboardDevice` whose poll has just run.
unsafe fn write_keyboard(this: *mut u8, blocking: bool) {
    if blocking {
        unsafe {
            zero(
                this,
                ds2_rva::KEYBOARD_DEVICE_DIK_TABLE_OFFSET,
                ds2_rva::KEYBOARD_DEVICE_DIK_TABLE_BYTES,
            );
        }
    }
}

/// Publish the virtual cursor, which is what actually turns the camera.
///
/// # Safety
///
/// `this` is a live `WindowsMouseDevice` whose poll has just run, so the clamped position at
/// `+0x08` is the value `FUN_140b0d0e0` is about to read.
unsafe fn write_windows_mouse(this: *mut u8, blocking: bool, authored: &Authored) {
    let real_x = unsafe { get_i32(this, ds2_rva::WINDOWS_MOUSE_DEVICE_POSITION_OFFSET) };
    let real_y = unsafe {
        get_i32(
            this,
            ds2_rva::WINDOWS_MOUSE_DEVICE_POSITION_OFFSET + size_of::<i32>(),
        )
    };

    if blocking {
        // Wheel, buttons and the button-edge word. `FUN_140b0d0e0` reads all three, and they
        // are the whole of this device's non-positional contribution.
        unsafe {
            zero(
                this,
                ds2_rva::WINDOWS_MOUSE_DEVICE_WHEEL_OFFSET,
                ds2_rva::WINDOWS_MOUSE_DEVICE_BUTTON_EDGE_OFFSET + size_of::<u32>()
                    - ds2_rva::WINDOWS_MOUSE_DEVICE_WHEEL_OFFSET,
            );
        }
    }

    // Nothing to do, and nothing has been published yet: leave the device exactly as the game
    // wrote it. A session that never drives the camera is byte-for-byte vanilla here.
    if !CURSOR_ENGAGED.load(Ordering::Relaxed) {
        if authored.mouse.is_none() && !blocking {
            REAL_PREV_X.store(real_x, Ordering::Relaxed);
            REAL_PREV_Y.store(real_y, Ordering::Relaxed);
            return;
        }
        // First engagement starts exactly where the real cursor is, so the first published
        // difference is only what was asked for.
        CURSOR_ENGAGED.store(true, Ordering::Relaxed);
        VIRTUAL_X.store((real_x as f32).to_bits(), Ordering::Relaxed);
        VIRTUAL_Y.store((real_y as f32).to_bits(), Ordering::Relaxed);
        REAL_PREV_X.store(real_x, Ordering::Relaxed);
        REAL_PREV_Y.store(real_y, Ordering::Relaxed);
        harness_log!(
            "virtual cursor engaged at ({real_x}, {real_y}) -- from here the camera reads this \
             module's position, and keeps doing so for the rest of the session (reverting would \
             hand the consumer one frame's difference equal to the whole gap, which is a spin)"
        );
    }

    let mut x = f32::from_bits(VIRTUAL_X.load(Ordering::Relaxed));
    let mut y = f32::from_bits(VIRTUAL_Y.load(Ordering::Relaxed));

    // The human's own motion, passed through unchanged -- unless it is being blocked, which is
    // the whole of what blocking mouse-look means.
    if !blocking {
        x += (real_x - REAL_PREV_X.load(Ordering::Relaxed)) as f32;
        y += (real_y - REAL_PREV_Y.load(Ordering::Relaxed)) as f32;
    }
    if let Some([dx, dy]) = authored.mouse {
        x += dx;
        y += dy;
    }

    REAL_PREV_X.store(real_x, Ordering::Relaxed);
    REAL_PREV_Y.store(real_y, Ordering::Relaxed);
    VIRTUAL_X.store(x.to_bits(), Ordering::Relaxed);
    VIRTUAL_Y.store(y.to_bits(), Ordering::Relaxed);

    // SAFETY: the same qword the original just wrote, at the offset it wrote it to. Nothing
    // re-clamps after this store, which is what lets an authored turn run past the client rect.
    unsafe {
        put_i32(
            this,
            ds2_rva::WINDOWS_MOUSE_DEVICE_POSITION_OFFSET,
            x as i32,
        );
        put_i32(
            this,
            ds2_rva::WINDOWS_MOUSE_DEVICE_POSITION_OFFSET + size_of::<i32>(),
            y as i32,
        );
    }
}

// One detour per site rather than a shared body: MinHook hands a detour no way to learn which
// site it was reached from, so the index has to be baked into the function.
unsafe extern "system" fn detour_pad(this: *mut u8) -> u64 {
    unsafe { poll(PAD, this) }
}
unsafe extern "system" fn detour_dinput_mouse(this: *mut u8) -> u64 {
    unsafe { poll(DINPUT_MOUSE, this) }
}
unsafe extern "system" fn detour_keyboard(this: *mut u8) -> u64 {
    unsafe { poll(KEYBOARD, this) }
}
unsafe extern "system" fn detour_windows_mouse(this: *mut u8) -> u64 {
    unsafe { poll(WINDOWS_MOUSE, this) }
}

const DETOURS: [PollFn; SITES.len()] = [
    detour_pad,
    detour_dinput_mouse,
    detour_keyboard,
    detour_windows_mouse,
];

/// Where the command file is, or `None` if the game directory cannot be resolved.
fn command_path() -> Option<PathBuf> {
    ds2_game_base::log::game_directory_path().map(|dir| dir.join(COMMAND_FILE_NAME))
}

/// Advance the frame counter and report any device that has stopped being polled.
///
/// Called once per `Present`. A device going away mid-run is not an error -- the user is going
/// to unplug a controller on purpose -- but it IS a change in the conditions of whatever
/// experiment is running, and an experiment that cannot notice its own conditions changing
/// produces data nobody should trust.
pub(crate) fn tick_devices() {
    FRAME.fetch_add(1, Ordering::Relaxed);
    for (index, site) in SITES.iter().enumerate() {
        let now = FIRED[index].load(Ordering::Relaxed);
        if now != LAST_COUNTS[index].swap(now, Ordering::Relaxed) {
            SILENT_TICKS[index].store(0, Ordering::Relaxed);
            if REPORTED_GONE[index].swap(false, Ordering::Relaxed) {
                harness_log!(
                    "CONTAMINATION NOTE: the {} poll is being called again -- a device came back",
                    site.name
                );
            }
            continue;
        }
        // A poll that has NEVER fired is not a device that went away; it is a device this
        // session does not have. Only a poll that was running and stopped is worth a line.
        if now == 0 {
            continue;
        }
        let silent = SILENT_TICKS[index].fetch_add(1, Ordering::Relaxed) + 1;
        if silent == DEVICE_SILENT_TICKS && !REPORTED_GONE[index].swap(true, Ordering::Relaxed) {
            harness_log!(
                "CONTAMINATION NOTE: the {} poll has not been called for {DEVICE_SILENT_TICKS} \
                 frames after {now} calls -- that device has gone away. Nothing the harness \
                 drives depends on it (the clock is Present), but its input is no longer being \
                 blanked either, because there is nothing left to blank.",
                site.name
            );
        }
    }
}

/// Read the command file, if this frame is one of the ones that does, and act on anything new.
pub(crate) fn poll_command_file() {
    if !FRAME
        .load(Ordering::Relaxed)
        .is_multiple_of(COMMAND_POLL_INTERVAL_FRAMES)
    {
        return;
    }
    static GATE: std::sync::Mutex<SequenceGate> = std::sync::Mutex::new(SequenceGate::NEW);
    let Some(path) = command_path() else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        // A missing file is the normal state -- most sessions never write one -- so this is not
        // logged. A file that exists but cannot be read looks the same from here; the next
        // successful read reports whatever it finds.
        return;
    };
    let mut gate = GATE.lock().unwrap_or_else(|error| error.into_inner());
    let Some(line) = gate.take(&contents) else {
        return;
    };
    drop(gate);
    match crate::command::parse(line) {
        Ok(command) => {
            crate::request(command);
            // The counts belong with `status` because they answer the question a negative result
            // cannot: a device whose poll has fired zero times was never reached, and writing
            // into it proved nothing. Not having this is why "the DirectInput mouse does not
            // move the camera" took a live run to distinguish from "the hook never ran".
            if matches!(command, Command::Status) {
                let counts = fire_counts();
                harness_log!(
                    "status: polls pad={} dinput-mouse={} keyboard={} windows-mouse={} \
                     cursor-engaged={}",
                    counts[PAD],
                    counts[DINPUT_MOUSE],
                    counts[KEYBOARD],
                    counts[WINDOWS_MOUSE],
                    CURSOR_ENGAGED.load(Ordering::Relaxed)
                );
            }
        }
        Err(error) => harness_log!("command REJECTED {line:?}: {error}"),
    }
}

/// Detour every device poll. See [`crate::install`] for the contract.
///
/// # Safety
///
/// Patches executable memory in the loaded game image; must run after the Arxan callback.
pub(crate) unsafe fn install() -> usize {
    let base = match game_module_base() {
        Ok(base) => base,
        Err(error) => {
            harness_log!("install-failed stage=module-base error={error}");
            return 0;
        }
    };

    // MinHook is statically linked into whichever DLL contains this crate, so nothing else
    // shares this instance and ALREADY_INITIALIZED can only mean this ran twice.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        harness_log!("install-failed stage=MH_Initialize status={status:?}");
        return 0;
    }

    let mut installed = 0;
    for (index, site) in SITES.iter().enumerate() {
        let address = base + site.rva as usize;

        // REFUSE A SITE WHOSE FIRST FIVE BYTES ARE NOT THE ONES RECORDED. The addresses here
        // are anchored to one build id, and a Steam update that moves them would otherwise be
        // discovered as a detour on whatever now lives at that offset -- which on the input path
        // is a crash at best. The read is fault-safe so an unmapped page reports rather than
        // faults.
        let mut found = [0u8; 5];
        // SAFETY: `read_bytes` reports an unmapped page rather than faulting.
        if !unsafe { read_bytes(address, &mut found) } {
            harness_log!(
                "hook-refused site={} va=0x{address:016x} reason=unreadable",
                site.name
            );
            continue;
        }
        if found != site.prologue {
            let name = site.name;
            let expected = site.prologue;
            let build = ds2_rva::BUILD_ID;
            harness_log!(
                "hook-refused site={name} va=0x{address:016x} reason=prologue-moved \
                 expected={expected:02x?} found={found:02x?} (ds2-rva is anchored to build \
                 {build})"
            );
            continue;
        }

        let hook =
            match unsafe { MhHook::new(address as *mut c_void, DETOURS[index] as *mut c_void) } {
                Ok(hook) => hook,
                Err(status) => {
                    harness_log!(
                        "hook-failed site={} va=0x{address:016x} stage=MH_CreateHook \
                         status={status:?}",
                        site.name
                    );
                    continue;
                }
            };
        // Published BEFORE the site is patched, so a detour cannot observe a zero and silently
        // skip the game's own poll -- which on this path would be the input stopping entirely.
        TRAMPOLINES[index].store(hook.trampoline() as usize, Ordering::Release);
        let status = unsafe { MH_EnableHook(address as *mut c_void) };
        if status != MH_STATUS::MH_OK {
            harness_log!(
                "hook-failed site={} va=0x{address:016x} stage=MH_EnableHook status={status:?}",
                site.name
            );
            continue;
        }
        installed += 1;
        harness_log!(
            "hooked site={} rva=0x{:08x} va=0x{address:016x}",
            site.name,
            site.rva
        );
    }

    let path = command_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<game directory not resolved>".to_owned());
    harness_log!(
        "install installed={installed}/{} commands={path}",
        SITES.len()
    );
    if installed != SITES.len() {
        harness_log!(
            "PARTIAL install: a device whose poll is not hooked is one whose input is NEITHER \
             blocked NOR authorable. Treat any measurement taken now as contaminated."
        );
    }
    installed
}

/// How many times each detour has fired, in [`SITES`] order.
pub(crate) fn fire_counts() -> [u64; SITES.len()] {
    [
        FIRED[PAD].load(Ordering::Relaxed),
        FIRED[DINPUT_MOUSE].load(Ordering::Relaxed),
        FIRED[KEYBOARD].load(Ordering::Relaxed),
        FIRED[WINDOWS_MOUSE].load(Ordering::Relaxed),
    ]
}
