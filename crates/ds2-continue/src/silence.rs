//! Hold the master channel group at zero for the length of the shortcut, then give it back.
//!
//! # Why this lever and not one of the four that failed
//!
//! Every earlier attempt looked for the frontend to call something named. It never does: a sweep
//! of all `e8` in the title's address range against `AppSoundManager`'s found zero hits, and the
//! FMOD symbols the project wanted -- `getMasterChannelGroup`, `setMute`, `setPaused` -- looked
//! like dead name-table strings.
//!
//! They are not dead. They are *imports*, and MSVC does not call an import directly; it calls a
//! `jmp qword ptr [rip+N]` thunk that jumps through the IAT. A scan for `call [rip+N]` into the
//! FMOD slots finds nothing at all, which is exactly the result that got recorded as "no xrefs".
//! Scanning for the thunks first turns up thirteen, and behind them the whole API:
//! `Event::start` (6 call sites), `Event::setPaused` (7), `System::getMasterChannelGroup` (5),
//! `ChannelGroup::setVolume` (1).
//!
//! That last one is the useful number. **One** call site in the entire image sets a channel
//! group's volume, and it is `MOFmodSoundManager`'s command drain applying the master volume to
//! the master group. There is no second volume path to fight with.
//!
//! # What is actually being asserted
//!
//! Nothing here is inferred from a name. `[`[`ds2_rva::SOUND_MANAGER_SINGLETON`]`]` is the one
//! global written by the sound manager's lazy constructor; the vtable of the object it allocates
//! carries an RTTI descriptor reading `.?AVMOFmodSoundManager@DLMO@@`; and
//! [`ds2_rva::SOUND_MANAGER_MASTER_GROUP_OFFSET`] is written by FMOD itself, because the game
//! passes `&this->_0x9f8` straight to `System::getMasterChannelGroup` as the out-parameter. The
//! field holds a master channel group because FMOD put it there.
//!
//! Muting the master group is FMOD's own semantics for "silence everything": every channel and
//! every sub-group the game creates mixes through it. Button sounds and BGM go together, which is
//! what was asked for.
//!
//! # Restoring to the game's number, not to `1.0`
//!
//! [`ds2_rva::SOUND_MANAGER_MASTER_VOLUME_OFFSET`] is where the drain parks the volume it is about
//! to apply -- it stores the incoming float there and reloads it three instructions later to hand
//! to `setVolume`. Restoring from that field gives back exactly what the player configured.
//! Writing `1.0` instead would quietly overwrite the options menu, which is a worse bug than the
//! noise this is meant to remove.
//!
//! If that field is not a usable volume when the time comes -- zero because nothing has applied
//! one yet, or not finite -- the restore falls back to `1.0`, FMOD's own default for a master
//! group, and says so in the log rather than silently leaving the game mute.
//!
//! # There is no frame pump, and assuming one cost a run
//!
//! The first version of this hooked `MOFmodSoundManager::v0` (`0x1409dfef0`) and re-asserted zero
//! after it every frame. `v0` contains the image's only `FMOD::EventSystem::update` call, which
//! FMOD documents as a once-a-frame requirement, so it looked exactly like the frame pump.
//!
//! **It is not.** Measured over a 57-second run, the detour fired essentially once -- during
//! process teardown. The mute was therefore never asserted while the game was playing, and the
//! restore requested at `StartIngame` (t=5.68s) was not performed until t=56.9s, one line above
//! `ds2-offline: detach`. Containing a per-frame API call does not make a function per-frame.
//!
//! So nothing here waits for a pump. It hangs on three functions that are each known to run:
//!
//! * **`v6`, audio init** -- it must run or there is no master group, and the run proved there is
//!   one. It is also the function that *writes* the master group, so on its return the group
//!   exists and nothing has played yet. That is where the mute goes.
//! * **`v2`, the command drain** -- the only code in the image that can change a channel group's
//!   volume. Re-asserting zero after it means the mute only has to out-run one function.
//! * **`FeSubStateTitleStartIngame::v1`** -- observed firing at t=5.68s in that same run.
//!
//! The restore is performed inline rather than deferred to another thread. FMOD Ex takes its own
//! locks, so a `setVolume` from the frontend thread is legal; deferring it to a thread that turned
//! out not to be running is what broke the first attempt.
//!
//! # If the shortcut never finishes
//!
//! Muting starts at audio init and the primary release is `StartIngame`. A shortcut that dies before
//! reaching it would otherwise leave the game silent, so the character list's two non-load
//! outcomes -- backed out to the top menu, refused by the ownership gate -- release it too, as
//! does every path that abandons the autoload. Each arm and release is logged.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// `FMOD::ChannelGroup::setVolume(this, volume)`.
///
/// MSVC `__thiscall` on x64 is the same register assignment as the Windows C ABI for a leading
/// pointer argument, and a `float` in argument position 2 travels in `xmm1` either way, so
/// `extern "system"` describes it exactly. The return is an `FMOD_RESULT`; `0` is `FMOD_OK`.
type SetVolumeFn = unsafe extern "system" fn(*mut c_void, f32) -> u32;

/// `MOFmodSoundManager::v6` (init) and `::v2` (the command drain). One argument each, `this`.
type SoundManagerFn = unsafe extern "system" fn(*mut u8);

/// `FeSubStateTitleStartIngame::v1`, the substate's `enter`. One argument, `this`.
type StartIngameEnterFn = unsafe extern "system" fn(*mut u8);

pub(crate) static SOUND_INIT_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static SOUND_DRAIN_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static START_INGAME_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);

/// Non-zero while the master group is being held at zero every frame.
static ARMED: AtomicU32 = AtomicU32::new(0);

/// Set once the restore has actually been applied, so a second release is a no-op rather than a
/// second `setVolume` with a stale number.
static RESTORED: AtomicU32 = AtomicU32::new(0);

/// The loaded image base, published by `install`.
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);

/// Whether `[continue] silence` asked for this at all.
static ENABLED: AtomicU32 = AtomicU32::new(0);

/// Turn the suppression on. Call before [`crate::install`]; it does nothing on its own until the
/// pump's detour is installed.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(u32::from(enabled), Ordering::Release);
}

/// Whether suppression was requested.
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire) != 0
}

pub(crate) fn set_module_base(base: usize) {
    MODULE_BASE.store(base, Ordering::Release);
}

/// Begin holding the master group at zero, and apply it now.
///
/// # Safety
///
/// Must run in the game process with the image mapped and the sound manager constructed -- which
/// is why the only caller is the detour on audio init, after the original has returned.
unsafe fn arm() {
    if !enabled() || ARMED.swap(1, Ordering::AcqRel) == 1 {
        return;
    }
    let applied = unsafe { apply(0.0) };
    log(format_args!("{LOG_PREFIX} silence armed applied={applied}"));
}

/// Write `volume` to the master channel group. Returns what FMOD said, or `None` if the manager,
/// the group or the import slot could not be resolved.
///
/// # Safety
///
/// The caller must be running in the game process with the image mapped at [`MODULE_BASE`].
unsafe fn apply(volume: f32) -> ApplyResult {
    let Some(manager) = (unsafe { sound_manager() }) else {
        return ApplyResult::NoManager;
    };
    let Some(group) = (unsafe { master_group(manager) }) else {
        return ApplyResult::NoGroup;
    };
    let Some(set_volume) = (unsafe { set_volume_fn() }) else {
        return ApplyResult::NoImport;
    };
    // SAFETY: `group` is FMOD's own master channel group and `set_volume` its own exported method.
    // This is the identical call the game makes at 0x1409e0c96.
    ApplyResult::Fmod(unsafe { set_volume(group, volume) })
}

/// What [`apply`] managed to do, so a failure names the step that failed instead of vanishing.
enum ApplyResult {
    NoManager,
    NoGroup,
    NoImport,
    Fmod(u32),
}

impl std::fmt::Display for ApplyResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoManager => f.write_str("no-manager"),
            Self::NoGroup => f.write_str("no-master-group"),
            Self::NoImport => f.write_str("no-import-slot"),
            Self::Fmod(result) => write!(f, "fmod_result={result}"),
        }
    }
}

/// Give the master volume back, now, on the calling thread.
///
/// Safe to call any number of times and from any path; the first one wins. FMOD Ex takes its own
/// locks, so this does not need to be marshalled onto the audio thread -- and the first version of
/// this module deferred it to a thread that turned out not to be running, which is exactly the
/// mistake being avoided.
///
/// `reason` is logged so a mute that outlives the shortcut can be traced to the path that should
/// have released it and did not.
pub(crate) fn release(reason: &str) {
    if !enabled() || ARMED.load(Ordering::Acquire) == 0 {
        return;
    }
    if RESTORED.swap(1, Ordering::AcqRel) == 1 {
        return;
    }

    // The volume the game itself last applied, not 1.0 -- writing 1.0 would silently overwrite
    // whatever the player set in the options menu.
    // SAFETY: called from a game thread with the image mapped; every dereference inside is
    // null-checked.
    let (volume, source) = unsafe {
        match sound_manager() {
            // SAFETY: `manager` is live and this offset is the float the drain applies to the
            // master group; reading it is the same load the game makes at 0x1409e0c87.
            Some(manager) => {
                let stored = manager
                    .add(ds2_rva::SOUND_MANAGER_MASTER_VOLUME_OFFSET)
                    .cast::<f32>()
                    .read();
                // A master group whose volume was never applied reads back as the zero its
                // allocation was handed out with. Restoring that would leave the game silent while
                // claiming to have given the audio back, so fall through to FMOD's own default for
                // a master group and say which was used.
                if stored.is_finite() && stored > 0.0 {
                    (stored, "game")
                } else {
                    (1.0_f32, "default")
                }
            }
            None => (1.0_f32, "default"),
        }
    };
    // SAFETY: as above.
    let applied = unsafe { apply(volume) };
    ARMED.store(0, Ordering::Release);
    log(format_args!(
        "{LOG_PREFIX} silence restored by={reason} volume={volume} source={source} {applied}"
    ));
}

/// Resolve `[SOUND_MANAGER_SINGLETON]`, the live `MOFmodSoundManager`.
///
/// # Safety
///
/// The caller must be running in the game process with the image mapped at [`MODULE_BASE`].
unsafe fn sound_manager() -> Option<*mut u8> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return None;
    }
    // SAFETY: the RVA names a pointer-sized global in the mapped image; this is the same load the
    // game's own accessor at 0x1409ddbc0 makes.
    let slot = (base + ds2_rva::SOUND_MANAGER_SINGLETON as usize) as *const *mut u8;
    let manager = unsafe { slot.read() };
    (!manager.is_null()).then_some(manager)
}

/// Resolve `ChannelGroup::setVolume` out of the import table.
///
/// # Safety
///
/// As [`sound_manager`]. Reads one IAT slot, which the loader has already filled by the time any
/// game code runs.
unsafe fn set_volume_fn() -> Option<SetVolumeFn> {
    let base = MODULE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return None;
    }
    // SAFETY: the RVA names the import slot for fmodex64.dll's setVolume; the loader wrote a
    // resolved function pointer there before the entry point ran.
    let slot = (base + ds2_rva::FMOD_CHANNEL_GROUP_SET_VOLUME_IAT as usize) as *const usize;
    let raw = unsafe { slot.read() };
    if raw == 0 {
        return None;
    }
    // SAFETY: the slot holds fmodex64.dll's exported `ChannelGroup::setVolume`, whose signature
    // `SetVolumeFn` describes.
    Some(unsafe { std::mem::transmute::<usize, SetVolumeFn>(raw) })
}

/// The master `FMOD::ChannelGroup*`, or `None` before the sound manager's init has run.
///
/// # Safety
///
/// As [`sound_manager`].
unsafe fn master_group(manager: *mut u8) -> Option<*mut c_void> {
    // SAFETY: `manager` is the live sound manager; FMOD wrote this field itself, as the
    // out-parameter of `System::getMasterChannelGroup`.
    let group = unsafe {
        manager
            .add(ds2_rva::SOUND_MANAGER_MASTER_GROUP_OFFSET)
            .cast::<*mut c_void>()
            .read()
    };
    (!group.is_null()).then_some(group)
}

/// `MOFmodSoundManager::v6`, audio init. The earliest moment a master group exists.
///
/// After the original, not before: the original is what asks FMOD for the group and stores it, so
/// there is nothing to mute until it has returned.
unsafe extern "system" fn detour_sound_init(this: *mut u8) {
    let trampoline = SOUND_INIT_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook's copy of this site's original prologue, with the vtable's signature.
        let original = unsafe { std::mem::transmute::<usize, SoundManagerFn>(trampoline) };
        unsafe { original(this) };
    }
    // SAFETY: the original returned, so the master group is stored and the manager is live.
    unsafe { arm() };
}

/// `MOFmodSoundManager::v2`, the command drain. Re-assert zero after the only function that could
/// have undone it.
///
/// Cheap because it is rare: the drain applies a volume only when a volume command was queued, so
/// this is not a per-frame write.
unsafe extern "system" fn detour_sound_drain(this: *mut u8) {
    let trampoline = SOUND_DRAIN_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook's copy of this site's original prologue, with the vtable's signature.
        let original = unsafe { std::mem::transmute::<usize, SoundManagerFn>(trampoline) };
        unsafe { original(this) };
    }
    if ARMED.load(Ordering::Acquire) == 0 {
        return;
    }
    // SAFETY: the original just returned on the sound manager, so it and the group are live.
    unsafe { apply(0.0) };
}

/// `FeSubStateTitleStartIngame::v1`. The shortcut is over; take the volume back, then let the
/// substate do its own work with the audio already restored.
unsafe extern "system" fn detour_start_ingame(this: *mut u8) {
    release("start-ingame");
    let trampoline = START_INGAME_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline != 0 {
        // SAFETY: MinHook's copy of this site's original prologue, with the vtable's signature.
        let original = unsafe { std::mem::transmute::<usize, StartIngameEnterFn>(trampoline) };
        unsafe { original(this) };
    }
}

/// The two detours this module contributes, as `(name, rva, detour, trampoline)`.
///
/// Returned rather than installed here so `install` keeps one loop, one log format and one count
/// for every site the crate patches.
pub(crate) fn sites() -> [(&'static str, u32, *mut c_void, &'static AtomicUsize); 3] {
    [
        (
            "sound-init",
            ds2_rva::SOUND_MANAGER_INIT,
            detour_sound_init as *mut c_void,
            &SOUND_INIT_TRAMPOLINE,
        ),
        (
            "sound-drain",
            ds2_rva::SOUND_MANAGER_COMMAND_DRAIN,
            detour_sound_drain as *mut c_void,
            &SOUND_DRAIN_TRAMPOLINE,
        ),
        (
            "start-ingame-enter",
            ds2_rva::FE_SUBSTATE_START_INGAME_ENTER,
            detour_start_ingame as *mut c_void,
            &START_INGAME_TRAMPOLINE,
        ),
    ]
}
