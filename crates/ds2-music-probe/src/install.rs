//! The four import-slot detours, the FMOD calls that name an event and read its channel, and the
//! log lines.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use ds2_game_base::mem::{safe_read_cstr, safe_read_u32, safe_read_usize};

use crate::{LOG_FIRST_ANY_EVENTS, LOG_PREFIX, SAMPLE_EVERY_MS, is_music_name};

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

unsafe extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, new: u32, old: *mut u32) -> i32;
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetCurrentThreadId() -> u32;
}

const PAGE_READWRITE: u32 = 0x04;

/// `FMOD_OK`.
const FMOD_OK: i32 = 0;

/// Returned by a detour whose original was never published, which the publish-before-write order
/// in [`patch_import`] rules out. `FMOD_ERR_INTERNAL` would be a lie about FMOD; any non-zero
/// result is a failure to the game's callers, which is the honest answer.
const NOT_CALLED: i32 = -1;

/// Longest event or sound name read back. Every name seen is ten characters.
const NAME_MAX: usize = 128;

/// How deep to look for a channel under an event's channel group.
const GROUP_DEPTH: u32 = 3;

// FMOD signatures. All are MSVC x64 member functions, so `this` is the first integer argument and
// a `bool` travels in the low byte of its register.
type StartFn = unsafe extern "system" fn(usize) -> i32;
type BoolArgFn = unsafe extern "system" fn(usize, u8) -> i32;
type GetStateFn = unsafe extern "system" fn(usize, *mut u32) -> i32;
type GetInfoFn = unsafe extern "system" fn(usize, *mut i32, *mut usize, *mut u8) -> i32;
type OutPointerFn = unsafe extern "system" fn(usize, *mut usize) -> i32;
type CountFn = unsafe extern "system" fn(usize, *mut i32) -> i32;
type IndexedFn = unsafe extern "system" fn(usize, i32, *mut usize) -> i32;
type TimeFn = unsafe extern "system" fn(usize, *mut u32, u32) -> i32;
type SoundNameFn = unsafe extern "system" fn(usize, *mut u8, i32) -> i32;

static ORIGINAL_START: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_STOP: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SET_PAUSED: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_GET_STATE: AtomicUsize = AtomicUsize::new(0);

static GET_INFO: AtomicUsize = AtomicUsize::new(0);
static GET_CHANNEL_GROUP: AtomicUsize = AtomicUsize::new(0);
static GROUP_NUM_CHANNELS: AtomicUsize = AtomicUsize::new(0);
static GROUP_GET_CHANNEL: AtomicUsize = AtomicUsize::new(0);
static GROUP_NUM_GROUPS: AtomicUsize = AtomicUsize::new(0);
static GROUP_GET_GROUP: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_CURRENT_SOUND: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_GET_POSITION: AtomicUsize = AtomicUsize::new(0);
static SOUND_GET_LENGTH: AtomicUsize = AtomicUsize::new(0);
static SOUND_GET_NAME: AtomicUsize = AtomicUsize::new(0);

/// The `fmodex64.dll` exports the game never imports, by their MSVC decorated names.
const EXPORT_GROUP_GET_GROUP: &[u8] =
    b"?getGroup@ChannelGroup@FMOD@@QEAA?AW4FMOD_RESULT@@HPEAPEAV12@@Z\0";
const EXPORT_CHANNEL_GET_POSITION: &[u8] =
    b"?getPosition@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAII@Z\0";
const EXPORT_SOUND_GET_LENGTH: &[u8] = b"?getLength@Sound@FMOD@@QEAA?AW4FMOD_RESULT@@PEAII@Z\0";
const EXPORT_SOUND_GET_NAME: &[u8] = b"?getName@Sound@FMOD@@QEAA?AW4FMOD_RESULT@@PEADH@Z\0";

/// The music event the game started most recently and has not stopped, or `0`.
static CURRENT_MUSIC: AtomicUsize = AtomicUsize::new(0);
/// Events of any name logged so far; see [`LOG_FIRST_ANY_EVENTS`].
static ANY_LOGGED: AtomicU32 = AtomicU32::new(0);
/// When the last position sample was taken, in ms since [`origin`].
static LAST_SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
static CLOCK_REPORTED: AtomicBool = AtomicBool::new(false);

fn origin() -> &'static Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now)
}

fn now_ms() -> u64 {
    origin().elapsed().as_millis() as u64
}

fn load<F: Copy>(slot: &AtomicUsize) -> Option<F> {
    let raw = slot.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    // SAFETY: every slot here is only ever stored with a function address of the type `F` its
    // callers name -- an import the Windows loader resolved or a `GetProcAddress` result -- and
    // `F` is always a plain `extern "system" fn` pointer, the size of a `usize`.
    Some(unsafe { std::mem::transmute_copy::<usize, F>(&raw) })
}

/// What `Event::getInfo` says about one event.
struct EventInfo {
    name: String,
    position_ms: i32,
    length_ms: i32,
    project_id: u32,
    system_id: u32,
}

/// Name an event through `Event::getInfo`. `None` if the call failed or there is no function.
fn describe(event: usize) -> Option<EventInfo> {
    let get_info: GetInfoFn = load(&GET_INFO)?;
    let mut name_ptr: usize = 0;
    let mut info = [0u8; ds2_rva::FMOD_EVENT_INFO_BUFFER_SIZE];
    // SAFETY: the game's own `getInfo` import, called with an event handle the game itself just
    // passed to an FMOD event function, and two out-parameters this frame owns. FMOD validates the
    // handle and answers an error for a stale one. The info buffer is zeroed, which is what its
    // input fields require.
    let rc = unsafe {
        get_info(
            event,
            core::ptr::null_mut(),
            &raw mut name_ptr,
            info.as_mut_ptr(),
        )
    };
    if rc != FMOD_OK {
        return None;
    }
    // SAFETY: a fault-safe bounded read of the string FMOD pointed at.
    let name = unsafe { safe_read_cstr(name_ptr, NAME_MAX) }.map_or_else(
        || "?".to_owned(),
        |b| String::from_utf8_lossy(&b).into_owned(),
    );
    let word = |offset: usize| -> [u8; 4] {
        let mut out = [0u8; 4];
        out.copy_from_slice(&info[offset..offset + 4]);
        out
    };
    Some(EventInfo {
        name,
        position_ms: i32::from_le_bytes(word(ds2_rva::FMOD_EVENT_INFO_POSITION_MS_OFFSET)),
        length_ms: i32::from_le_bytes(word(ds2_rva::FMOD_EVENT_INFO_LENGTH_MS_OFFSET)),
        project_id: u32::from_le_bytes(word(ds2_rva::FMOD_EVENT_INFO_PROJECT_ID_OFFSET)),
        system_id: u32::from_le_bytes(word(ds2_rva::FMOD_EVENT_INFO_SYSTEM_ID_OFFSET)),
    })
}

/// The map index the player stands in, `map=<n>`, `map=none` for the sentinel, `map=-` when the
/// chain does not resolve (title screen, before `GameManagerImp` exists).
fn map_index() -> String {
    let Ok(base) = ds2_game_base::mem::game_module_base() else {
        return "-".to_owned();
    };
    // SAFETY: fault-safe reads at every hop. `GameManagerImp -> +0x38 MapManager -> +0x170` is the
    // chain `ds2-invasion-path` reads; see the two constants for the evidence.
    let index = unsafe {
        safe_read_usize(base + ds2_rva::GAME_MANAGER_IMP as usize)
            .filter(|gm| *gm != 0)
            .and_then(|gm| safe_read_usize(gm + ds2_rva::GAME_MANAGER_MAP_MANAGER_OFFSET))
            .filter(|mm| *mm != 0)
            .and_then(|mm| safe_read_u32(mm + ds2_rva::MAP_MANAGER_PLAYER_MAP_INDEX_OFFSET))
    };
    match index {
        None => "-".to_owned(),
        Some(ds2_rva::MAP_INDEX_NONE) => "none".to_owned(),
        Some(i) => i.to_string(),
    }
}

/// Log an event call if it is music, the current music event, or one of the first few events.
fn report(kind: &str, event: usize, extra: std::fmt::Arguments<'_>) -> Option<EventInfo> {
    let info = describe(event);
    let music = info.as_ref().is_some_and(|i| is_music_name(&i.name));
    let current = event != 0 && CURRENT_MUSIC.load(Ordering::Relaxed) == event;
    if !music && !current && ANY_LOGGED.fetch_add(1, Ordering::Relaxed) >= LOG_FIRST_ANY_EVENTS {
        return info;
    }
    match &info {
        Some(i) => log(format_args!(
            "{LOG_PREFIX} {kind} name={} handle=0x{event:x} music={music} map={} project={} \
             system={} event-pos={}ms event-len={}ms{extra}",
            i.name,
            map_index(),
            i.project_id,
            i.system_id,
            i.position_ms,
            i.length_ms
        )),
        None => log(format_args!(
            "{LOG_PREFIX} {kind} name=? handle=0x{event:x} getInfo-failed map={}{extra}",
            map_index()
        )),
    }
    info
}

/// The first channel under a channel group, looking into subgroups up to [`GROUP_DEPTH`].
fn first_channel(group: usize, depth: u32) -> Option<usize> {
    let num_channels: CountFn = load(&GROUP_NUM_CHANNELS)?;
    let get_channel: IndexedFn = load(&GROUP_GET_CHANNEL)?;
    let mut count = 0i32;
    // SAFETY: the game's own imports, on a channel group FMOD handed back for a live event this
    // same call, with out-parameters this frame owns.
    if unsafe { num_channels(group, &raw mut count) } == FMOD_OK && count > 0 {
        let mut channel = 0usize;
        // SAFETY: as above.
        if unsafe { get_channel(group, 0, &raw mut channel) } == FMOD_OK && channel != 0 {
            return Some(channel);
        }
    }
    if depth == 0 {
        return None;
    }
    let num_groups: CountFn = load(&GROUP_NUM_GROUPS)?;
    let get_group: IndexedFn = load(&GROUP_GET_GROUP)?;
    let mut groups = 0i32;
    // SAFETY: as above.
    if unsafe { num_groups(group, &raw mut groups) } != FMOD_OK {
        return None;
    }
    for index in 0..groups {
        let mut child = 0usize;
        // SAFETY: as above; `index` is below the count FMOD just returned.
        if unsafe { get_group(group, index, &raw mut child) } == FMOD_OK
            && child != 0
            && let Some(channel) = first_channel(child, depth - 1)
        {
            return Some(channel);
        }
    }
    None
}

/// One position line for the current music event, at most every [`SAMPLE_EVERY_MS`].
fn maybe_sample() {
    let event = CURRENT_MUSIC.load(Ordering::Relaxed);
    if event == 0 {
        return;
    }
    let now = now_ms();
    let last = LAST_SAMPLE_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < SAMPLE_EVERY_MS
        || LAST_SAMPLE_MS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    let Some(info) = describe(event) else {
        log(format_args!(
            "{LOG_PREFIX} sample handle=0x{event:x} getInfo-failed -- the event is gone"
        ));
        CURRENT_MUSIC.store(0, Ordering::Relaxed);
        return;
    };
    let mut group = 0usize;
    let group_ok = load::<OutPointerFn>(&GET_CHANNEL_GROUP).is_some_and(|get_group| {
        // SAFETY: the game's own `Event::getChannelGroup` import on an event `getInfo` just
        // accepted, with an out-parameter this frame owns.
        unsafe { get_group(event, &raw mut group) == FMOD_OK && group != 0 }
    });
    let channel = if group_ok {
        first_channel(group, GROUP_DEPTH)
    } else {
        None
    };
    let mut position = u32::MAX;
    let mut length = u32::MAX;
    let mut sound_name = String::from("-");
    if let Some(channel) = channel {
        if let Some(get_position) = load::<TimeFn>(&CHANNEL_GET_POSITION) {
            // SAFETY: `fmodex64!Channel::getPosition` on a channel FMOD just returned; FMOD
            // validates channel handles.
            unsafe { get_position(channel, &raw mut position, ds2_rva::FMOD_TIMEUNIT_MS) };
        }
        let mut sound = 0usize;
        if let Some(current_sound) = load::<OutPointerFn>(&CHANNEL_CURRENT_SOUND)
            // SAFETY: as above, for the game's own `Channel::getCurrentSound` import.
            && unsafe { current_sound(channel, &raw mut sound) } == FMOD_OK
            && sound != 0
        {
            if let Some(get_length) = load::<TimeFn>(&SOUND_GET_LENGTH) {
                // SAFETY: `fmodex64!Sound::getLength` on the sound FMOD just returned.
                unsafe { get_length(sound, &raw mut length, ds2_rva::FMOD_TIMEUNIT_MS) };
            }
            if let Some(get_name) = load::<SoundNameFn>(&SOUND_GET_NAME) {
                let mut buffer = [0u8; NAME_MAX];
                // SAFETY: `fmodex64!Sound::getName` into a buffer this frame owns, its length
                // passed so FMOD truncates rather than overruns.
                if unsafe { get_name(sound, buffer.as_mut_ptr(), NAME_MAX as i32) } == FMOD_OK {
                    let end = buffer.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
                    sound_name = String::from_utf8_lossy(&buffer[..end]).into_owned();
                }
            }
        }
    }
    let ms = |v: u32| {
        if v == u32::MAX {
            "-".to_owned()
        } else {
            format!("{v}ms")
        }
    };
    log(format_args!(
        "{LOG_PREFIX} sample name={} handle=0x{event:x} event-pos={}ms event-len={}ms \
         group=0x{group:x} channel=0x{:x} channel-pos={} sound={sound_name} sound-len={} map={}",
        info.name,
        info.position_ms,
        info.length_ms,
        channel.unwrap_or(0),
        ms(position),
        ms(length),
        map_index()
    ));
}

unsafe extern "system" fn detour_start(event: usize) -> i32 {
    let Some(original) = load::<StartFn>(&ORIGINAL_START) else {
        return NOT_CALLED;
    };
    // SAFETY: the import the Windows loader resolved, with the game's own argument.
    let rc = unsafe { original(event) };
    let info = report("start", event, format_args!(" rc={rc}"));
    claim(event, info.as_ref());
    maybe_sample();
    rc
}

/// Make `event` the one [`maybe_sample`] follows, if it is the background track.
///
/// Only a looping music event (`lengthms = -1`) takes over from one that is already followed. The
/// first run showed why: at the title the looping theme `m000000002` starts, then the two-second
/// jingle `m000000013` starts and stops, and following the latest start meant the jingle's stop
/// left nothing followed while the theme played on, so no position line was ever written.
fn claim(event: usize, info: Option<&EventInfo>) {
    let Some(info) = info else {
        return;
    };
    if !is_music_name(&info.name) {
        return;
    }
    let looping = info.length_ms == LENGTH_LOOPING;
    if looping || CURRENT_MUSIC.load(Ordering::Relaxed) == 0 {
        CURRENT_MUSIC.store(event, Ordering::Relaxed);
        LAST_SAMPLE_MS.store(now_ms(), Ordering::Relaxed);
    }
}

/// `FMOD_EVENT_INFO::lengthms` for an event with looping sounds.
const LENGTH_LOOPING: i32 = -1;

unsafe extern "system" fn detour_stop(event: usize, immediate: u8) -> i32 {
    let Some(original) = load::<BoolArgFn>(&ORIGINAL_STOP) else {
        return NOT_CALLED;
    };
    // Named before the call: a stopped event may no longer answer `getInfo`.
    report("stop", event, format_args!(" immediate={}", immediate != 0));
    if CURRENT_MUSIC.load(Ordering::Relaxed) == event {
        CURRENT_MUSIC.store(0, Ordering::Relaxed);
    }
    // SAFETY: the import the Windows loader resolved, with the game's own arguments.
    unsafe { original(event, immediate) }
}

unsafe extern "system" fn detour_set_paused(event: usize, paused: u8) -> i32 {
    let Some(original) = load::<BoolArgFn>(&ORIGINAL_SET_PAUSED) else {
        return NOT_CALLED;
    };
    // SAFETY: the import the Windows loader resolved, with the game's own arguments.
    let rc = unsafe { original(event, paused) };
    let info = report(
        "setPaused",
        event,
        format_args!(" paused={} rc={rc}", paused != 0),
    );
    if paused == 0 {
        claim(event, info.as_ref());
    }
    maybe_sample();
    rc
}

unsafe extern "system" fn detour_get_state(event: usize, state: *mut u32) -> i32 {
    let Some(original) = load::<GetStateFn>(&ORIGINAL_GET_STATE) else {
        return NOT_CALLED;
    };
    // SAFETY: the import the Windows loader resolved, with the game's own arguments.
    let rc = unsafe { original(event, state) };
    if !CLOCK_REPORTED.swap(true, Ordering::Relaxed) {
        // SAFETY: takes nothing and cannot fail.
        let tid = unsafe { GetCurrentThreadId() };
        log(format_args!(
            "{LOG_PREFIX} clock getState first call tid={tid}"
        ));
    }
    maybe_sample();
    rc
}

/// Point one import slot at `replacement`, publishing the original first. False, logged, on
/// failure.
///
/// # Safety
///
/// `base` must be the live game module base and `rva` a pointer-sized import slot in its `.idata`.
unsafe fn patch_import(
    base: usize,
    rva: u32,
    replacement: usize,
    original: &AtomicUsize,
    name: &str,
) -> bool {
    let slot = (base + rva as usize) as *mut usize;
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
            "{LOG_PREFIX} import-hook-failed import={name} stage=VirtualProtect slot=0x{:016x}",
            slot as usize
        ));
        return false;
    }
    // SAFETY: the slot is now writable and holds the resolved import. The original is published
    // before the slot changes, so a thread that reaches the detour at once finds it.
    unsafe {
        original.store(slot.read(), Ordering::Release);
        slot.write(replacement);
        let mut restored = 0u32;
        VirtualProtect(
            slot.cast::<c_void>(),
            std::mem::size_of::<usize>(),
            old_protect,
            &raw mut restored,
        );
    }
    log(format_args!(
        "{LOG_PREFIX} hooked import={name} slot=0x{:016x} original=0x{:016x}",
        slot as usize,
        original.load(Ordering::Acquire)
    ));
    true
}

/// Copy the address in one of the game's import slots into `into`, without changing the slot.
///
/// # Safety
///
/// `base` must be the live game module base.
unsafe fn read_import(base: usize, rva: u32, into: &AtomicUsize) -> bool {
    // SAFETY: fault-safe read of a slot inside the mapped image.
    let value = unsafe { safe_read_usize(base + rva as usize) }.unwrap_or(0);
    into.store(value, Ordering::Release);
    value != 0
}

/// Look up one `fmodex64.dll` export by its decorated name into `into`.
fn read_export(module: *mut c_void, name: &[u8], into: &AtomicUsize) -> bool {
    if module.is_null() {
        return false;
    }
    // SAFETY: `module` is the loaded `fmodex64.dll`, and `name` is a NUL-terminated literal.
    let address = unsafe { GetProcAddress(module, name.as_ptr()) } as usize;
    into.store(address, Ordering::Release);
    address != 0
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Whether the `Event::start` slot, the one the feature cannot do without, was fronted.
    pub installed: bool,
    /// Slots fronted, of four.
    pub hooked: u32,
}

/// Front the four FMOD event import slots and resolve the functions the log lines call.
///
/// # Safety
///
/// Writes four pointer-sized import slots in the game image. Call once, from the loader's
/// post-Arxan callback, before the game has started any sound.
pub unsafe fn install() -> Outcome {
    let _ = origin();
    let Ok(base) = ds2_game_base::mem::game_module_base() else {
        log(format_args!(
            "{LOG_PREFIX} no game module base; nothing hooked"
        ));
        return Outcome {
            installed: false,
            hooked: 0,
        };
    };
    // SAFETY: each RVA is the named import slot in `ds2-rva`; nothing is written.
    let imports = unsafe {
        [
            (
                "getInfo",
                read_import(base, ds2_rva::FMOD_EVENT_GET_INFO_IAT, &GET_INFO),
            ),
            (
                "getChannelGroup",
                read_import(
                    base,
                    ds2_rva::FMOD_EVENT_GET_CHANNEL_GROUP_IAT,
                    &GET_CHANNEL_GROUP,
                ),
            ),
            (
                "getNumChannels",
                read_import(
                    base,
                    ds2_rva::FMOD_CHANNEL_GROUP_GET_NUM_CHANNELS_IAT,
                    &GROUP_NUM_CHANNELS,
                ),
            ),
            (
                "getChannel",
                read_import(
                    base,
                    ds2_rva::FMOD_CHANNEL_GROUP_GET_CHANNEL_IAT,
                    &GROUP_GET_CHANNEL,
                ),
            ),
            (
                "getNumGroups",
                read_import(
                    base,
                    ds2_rva::FMOD_CHANNEL_GROUP_GET_NUM_GROUPS_IAT,
                    &GROUP_NUM_GROUPS,
                ),
            ),
            (
                "getCurrentSound",
                read_import(
                    base,
                    ds2_rva::FMOD_CHANNEL_GET_CURRENT_SOUND_IAT,
                    &CHANNEL_CURRENT_SOUND,
                ),
            ),
        ]
    };
    let name: Vec<u16> = "fmodex64.dll\0".encode_utf16().collect();
    // SAFETY: a NUL-terminated wide string; `fmodex64.dll` is a static import of the game, so it
    // is mapped already and nothing is loaded.
    let fmodex = unsafe { GetModuleHandleW(name.as_ptr()) };
    let exports = [
        (
            "getGroup",
            read_export(fmodex, EXPORT_GROUP_GET_GROUP, &GROUP_GET_GROUP),
        ),
        (
            "getPosition",
            read_export(fmodex, EXPORT_CHANNEL_GET_POSITION, &CHANNEL_GET_POSITION),
        ),
        (
            "getLength",
            read_export(fmodex, EXPORT_SOUND_GET_LENGTH, &SOUND_GET_LENGTH),
        ),
        (
            "getName",
            read_export(fmodex, EXPORT_SOUND_GET_NAME, &SOUND_GET_NAME),
        ),
    ];
    let missing: Vec<&str> = imports
        .iter()
        .chain(exports.iter())
        .filter(|(_, ok)| !ok)
        .map(|(name, _)| *name)
        .collect();
    log(format_args!(
        "{LOG_PREFIX} resolved fmodex64=0x{:x} missing={missing:?}",
        fmodex as usize
    ));
    if GET_INFO.load(Ordering::Acquire) == 0 {
        log(format_args!(
            "{LOG_PREFIX} Event::getInfo unresolved; no event could be named, nothing hooked"
        ));
        return Outcome {
            installed: false,
            hooked: 0,
        };
    }

    let start: StartFn = detour_start;
    let stop: BoolArgFn = detour_stop;
    let set_paused: BoolArgFn = detour_set_paused;
    let get_state: GetStateFn = detour_get_state;
    // SAFETY: each RVA is the named import slot, read out of the image's import descriptors.
    let results = unsafe {
        [
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_START_IAT,
                start as usize,
                &ORIGINAL_START,
                "fmod_event64!Event::start",
            ),
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_STOP_IAT,
                stop as usize,
                &ORIGINAL_STOP,
                "fmod_event64!Event::stop",
            ),
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_SET_PAUSED_IAT,
                set_paused as usize,
                &ORIGINAL_SET_PAUSED,
                "fmod_event64!Event::setPaused",
            ),
            patch_import(
                base,
                ds2_rva::FMOD_EVENT_GET_STATE_IAT,
                get_state as usize,
                &ORIGINAL_GET_STATE,
                "fmod_event64!Event::getState",
            ),
        ]
    };
    Outcome {
        installed: results[0],
        hooked: results.iter().filter(|ok| **ok).count() as u32,
    }
}
