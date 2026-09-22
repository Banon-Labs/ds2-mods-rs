//! The three detours, and the three writes they make.
//!
//! # Why a detour per device rather than one hook on the mapper
//!
//! The mapper is where a value becomes "camera X" rather than "axis 3", which sounds like the
//! better place to write. It is not, for this job: the mapper's binding is the thing being
//! measured, and a harness that wrote past it could never tell you whether the binding is what
//! you thought. Writing the DEVICE keeps the whole of the engine's own interpretation -- the
//! deadzone, the sensitivity setting, the Y inversion, the mapper itself -- in the loop, so what
//! the game does with an injected stick is what it would do with a real one.
//!
//! # Why the write happens after the original
//!
//! Each poll rebuilds its device's fields from the API it just called. A value written first is
//! overwritten by the original; a value written after is the one the mapper reads that frame.
//!
//! # Which detour is the frame
//!
//! Whichever of the three fires FIRST claims ownership, and from then on only that one advances
//! the state machine. That is what makes the tick exactly one per frame regardless of how many
//! devices exist: a mouse-and-keyboard player has no `PadDevice` polling, a pad player's mouse
//! still polls, and neither case needs a special path.

use core::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use ds2_game_base::mem::{game_module_base, read_bytes};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::authored::Authored;
use crate::command::SequenceGate;
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

/// Which device owns the frame tick. `NO_OWNER` until the first detour fires.
const NO_OWNER: u32 = u32::MAX;
static TICK_OWNER: AtomicU32 = AtomicU32::new(NO_OWNER);

/// Frames observed, for the command poll's throttle.
static FRAME: AtomicU64 = AtomicU64::new(0);

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
const MOUSE: usize = 1;
const KEYBOARD: usize = 2;

const SITES: [Site; 3] = [
    Site {
        name: "pad",
        rva: ds2_rva::PAD_DEVICE_POLL,
        prologue: ds2_rva::PAD_DEVICE_POLL_PROLOGUE,
    },
    Site {
        name: "mouse",
        rva: ds2_rva::MOUSE_DEVICE_POLL,
        prologue: ds2_rva::MOUSE_DEVICE_POLL_PROLOGUE,
    },
    Site {
        name: "keyboard",
        rva: ds2_rva::KEYBOARD_DEVICE_POLL,
        prologue: ds2_rva::KEYBOARD_DEVICE_POLL_PROLOGUE,
    },
];

/// Trampolines back to the originals, published before each site is patched so a detour that
/// fires immediately cannot read a zero and silently skip the game's own poll.
static TRAMPOLINES: [AtomicUsize; SITES.len()] = [const { AtomicUsize::new(0) }; SITES.len()];

/// How many times each detour has fired. Reported by `status`, and the difference between "the
/// harness pressed nothing" and "the harness was never reached".
static FIRED: [AtomicU64; SITES.len()] = [const { AtomicU64::new(0) }; SITES.len()];

/// Every one of the three polls is `bool poll(this)` with `this` in RCX: they return in `al`
/// (the pad's success path is `mov al,1`, the mouse and keyboard return 0 or 1) and read no
/// other argument register. The detours hand back whatever the original returned, unchanged.
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
        // the one all three overrides implement.
        let original: PollFn = unsafe { std::mem::transmute::<usize, PollFn>(trampoline) };
        unsafe { original(this) }
    };

    // Elect a frame owner on the first poll of any device, then tick only on that one.
    let owner = TICK_OWNER.load(Ordering::Relaxed);
    if owner == NO_OWNER {
        // A relaxed CAS: two devices racing here is not possible (the engine polls them in
        // sequence on one thread), and if it somehow were, either winner is a correct answer.
        TICK_OWNER.store(index as u32, Ordering::Relaxed);
        harness_log!(
            "tick owner is the {} poll -- the state machine advances once per {} poll",
            SITES[index].name,
            SITES[index].name
        );
        FRAME.store(0, Ordering::Relaxed);
        crate::on_frame();
    } else if owner == index as u32 {
        FRAME.fetch_add(1, Ordering::Relaxed);
        crate::on_frame();
    }

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
            MOUSE => write_mouse(this, blocking, &authored),
            KEYBOARD => write_keyboard(this, blocking),
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

/// Blank the player's mouse and stamp an authored delta.
///
/// # Safety
///
/// `this` is a live `DLUID::MouseDevice` whose poll has just run.
unsafe fn write_mouse(this: *mut u8, blocking: bool, authored: &Authored) {
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
    if let Some([dx, dy]) = authored.mouse {
        unsafe {
            put_f32(this, ds2_rva::MOUSE_DEVICE_DELTA_X_OFFSET, dx);
            put_f32(this, ds2_rva::MOUSE_DEVICE_DELTA_Y_OFFSET, dy);
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

// One detour per site rather than a shared body: MinHook hands a detour no way to learn which
// site it was reached from, so the index has to be baked into the function.
unsafe extern "system" fn detour_pad(this: *mut u8) -> u64 {
    unsafe { poll(PAD, this) }
}
unsafe extern "system" fn detour_mouse(this: *mut u8) -> u64 {
    unsafe { poll(MOUSE, this) }
}
unsafe extern "system" fn detour_keyboard(this: *mut u8) -> u64 {
    unsafe { poll(KEYBOARD, this) }
}

const DETOURS: [PollFn; SITES.len()] = [detour_pad, detour_mouse, detour_keyboard];

/// Where the command file is, or `None` if the game directory cannot be resolved.
fn command_path() -> Option<PathBuf> {
    ds2_game_base::log::game_directory_path().map(|dir| dir.join(COMMAND_FILE_NAME))
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
        Ok(command) => crate::request(command),
        Err(error) => harness_log!("command REJECTED {line:?}: {error}"),
    }
}

/// Detour all three device polls. See [`crate::install`] for the contract.
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

/// How many times each detour has fired, for `status` and for the run report.
pub(crate) fn fire_counts() -> [u64; SITES.len()] {
    [
        FIRED[PAD].load(Ordering::Relaxed),
        FIRED[MOUSE].load(Ordering::Relaxed),
        FIRED[KEYBOARD].load(Ordering::Relaxed),
    ]
}
