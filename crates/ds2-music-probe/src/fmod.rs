//! Typed calls into FMOD: through the game's own import slots where it imports the function, and
//! through `GetProcAddress` for the handful it never imports.
//!
//! Every call here is made on the game's sound thread, from inside one of the game's own FMOD
//! calls (see `install`), which is where the game makes them.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use ds2_game_base::mem::{safe_read_cstr, safe_read_usize};

use crate::playlist::TrackKey;

/// `FMOD_OK`.
pub(crate) const FMOD_OK: i32 = 0;

/// Longest event or bank name read back.
const NAME_MAX: usize = 256;

/// How deep to look for a channel under an event's channel group.
const GROUP_DEPTH: u32 = 3;

/// How many wave banks `Event::getInfo` is given room to describe.
const WAVEBANKS: usize = 4;

/// `Channel::setLoopCount` value for "loop forever", which is how the game's channels arrive.
pub(crate) const LOOP_FOREVER: i32 = -1;

// MSVC x64 member functions: `this` first, a `bool` in the low byte of its register.
type StartFn = unsafe extern "system" fn(usize) -> i32;
type BoolArgFn = unsafe extern "system" fn(usize, u8) -> i32;
type GetInfoFn = unsafe extern "system" fn(usize, *mut i32, *mut usize, *mut u8) -> i32;
type OutPointerFn = unsafe extern "system" fn(usize, *mut usize) -> i32;
type CountFn = unsafe extern "system" fn(usize, *mut i32) -> i32;
type IndexedFn = unsafe extern "system" fn(usize, i32, *mut usize) -> i32;
type TimeFn = unsafe extern "system" fn(usize, *mut u32, u32) -> i32;
type SetTimeFn = unsafe extern "system" fn(usize, u32, u32) -> i32;
type IntArgFn = unsafe extern "system" fn(usize, i32) -> i32;
type IsPlayingFn = unsafe extern "system" fn(usize, *mut u8) -> i32;
type BySystemIdFn = unsafe extern "system" fn(usize, u32, u32, *mut usize) -> i32;

/// The four fronted imports, as the Windows loader resolved them. Written by `install`.
pub(crate) static ORIGINAL_START: AtomicUsize = AtomicUsize::new(0);
pub(crate) static ORIGINAL_STOP: AtomicUsize = AtomicUsize::new(0);
pub(crate) static ORIGINAL_SET_PAUSED: AtomicUsize = AtomicUsize::new(0);
pub(crate) static ORIGINAL_GET_STATE: AtomicUsize = AtomicUsize::new(0);

static GET_INFO: AtomicUsize = AtomicUsize::new(0);
static GET_CHANNEL_GROUP: AtomicUsize = AtomicUsize::new(0);
static SET_MUTE: AtomicUsize = AtomicUsize::new(0);
static GET_EVENT_BY_SYSTEM_ID: AtomicUsize = AtomicUsize::new(0);
static GET_NUM_EVENTS: AtomicUsize = AtomicUsize::new(0);
static GROUP_NUM_CHANNELS: AtomicUsize = AtomicUsize::new(0);
static GROUP_GET_CHANNEL: AtomicUsize = AtomicUsize::new(0);
static GROUP_NUM_GROUPS: AtomicUsize = AtomicUsize::new(0);
static GROUP_GET_GROUP: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_CURRENT_SOUND: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_GET_POSITION: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_SET_POSITION: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_SET_LOOP_COUNT: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_IS_PLAYING: AtomicUsize = AtomicUsize::new(0);
static SOUND_GET_LENGTH: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_SET_MUTE: AtomicUsize = AtomicUsize::new(0);
static CHANNEL_GET_AUDIBILITY: AtomicUsize = AtomicUsize::new(0);
static MEMORY_GET_STATS: AtomicUsize = AtomicUsize::new(0);
static GET_3D_LISTENER: AtomicUsize = AtomicUsize::new(0);
static SET_3D_ATTRIBUTES: AtomicUsize = AtomicUsize::new(0);
static GET_PARENT_GROUP: AtomicUsize = AtomicUsize::new(0);

type ListenerFn = unsafe extern "system" fn(
    usize,
    i32,
    *mut [f32; 3],
    *mut [f32; 3],
    *mut [f32; 3],
    *mut [f32; 3],
) -> i32;
type Set3dFn =
    unsafe extern "system" fn(usize, *const [f32; 3], *const [f32; 3], *const [f32; 3]) -> i32;
type AudibilityFn = unsafe extern "system" fn(usize, *mut f32) -> i32;
type MemoryStatsFn = unsafe extern "C" fn(*mut i32, *mut i32, i32) -> i32;
type FreeEventDataFn = unsafe extern "system" fn(usize, usize, u8) -> i32;

/// Exports the game never imports, by their MSVC decorated names.
const FMODEX_EXPORTS: [(&str, &[u8], &AtomicUsize); 8] = [
    (
        "Channel::setMute",
        b"?setMute@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@_N@Z\0",
        &CHANNEL_SET_MUTE,
    ),
    (
        "Channel::getAudibility",
        b"?getAudibility@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAM@Z\0",
        &CHANNEL_GET_AUDIBILITY,
    ),
    (
        "FMOD_Memory_GetStats",
        b"FMOD_Memory_GetStats\0",
        &MEMORY_GET_STATS,
    ),
    (
        "ChannelGroup::getGroup",
        b"?getGroup@ChannelGroup@FMOD@@QEAA?AW4FMOD_RESULT@@HPEAPEAV12@@Z\0",
        &GROUP_GET_GROUP,
    ),
    (
        "Channel::getPosition",
        b"?getPosition@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAII@Z\0",
        &CHANNEL_GET_POSITION,
    ),
    (
        "Channel::setLoopCount",
        b"?setLoopCount@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@H@Z\0",
        &CHANNEL_SET_LOOP_COUNT,
    ),
    (
        "Channel::isPlaying",
        b"?isPlaying@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEA_N@Z\0",
        &CHANNEL_IS_PLAYING,
    ),
    (
        "Sound::getLength",
        b"?getLength@Sound@FMOD@@QEAA?AW4FMOD_RESULT@@PEAII@Z\0",
        &SOUND_GET_LENGTH,
    ),
];

const EVENT_EXPORTS: [(&str, &[u8], &AtomicUsize); 1] = [(
    "EventSystem::getNumEvents",
    b"?getNumEvents@EventSystem@FMOD@@QEAA?AW4FMOD_RESULT@@PEAH@Z\0",
    &GET_NUM_EVENTS,
)];

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
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

/// Resolve every function this module calls. Returns the names that could not be found.
///
/// # Safety
///
/// `base` must be the live game module base.
pub(crate) unsafe fn resolve(base: usize) -> Vec<&'static str> {
    let mut missing = Vec::new();
    let imports: [(&str, u32, &AtomicUsize); 12] = [
        (
            "EventSystem::get3DListenerAttributes",
            ds2_rva::FMOD_EVENT_SYSTEM_GET_3D_LISTENER_IAT,
            &GET_3D_LISTENER,
        ),
        (
            "Event::set3DAttributes",
            ds2_rva::FMOD_EVENT_SET_3D_ATTRIBUTES_IAT,
            &SET_3D_ATTRIBUTES,
        ),
        (
            "Event::getParentGroup",
            ds2_rva::FMOD_EVENT_GET_PARENT_GROUP_IAT,
            &GET_PARENT_GROUP,
        ),
        (
            "Event::getInfo",
            ds2_rva::FMOD_EVENT_GET_INFO_IAT,
            &GET_INFO,
        ),
        (
            "Event::getChannelGroup",
            ds2_rva::FMOD_EVENT_GET_CHANNEL_GROUP_IAT,
            &GET_CHANNEL_GROUP,
        ),
        (
            "Event::setMute",
            ds2_rva::FMOD_EVENT_SET_MUTE_IAT,
            &SET_MUTE,
        ),
        (
            "EventSystem::getEventBySystemID",
            ds2_rva::FMOD_EVENT_SYSTEM_GET_EVENT_BY_SYSTEM_ID_IAT,
            &GET_EVENT_BY_SYSTEM_ID,
        ),
        (
            "ChannelGroup::getNumChannels",
            ds2_rva::FMOD_CHANNEL_GROUP_GET_NUM_CHANNELS_IAT,
            &GROUP_NUM_CHANNELS,
        ),
        (
            "ChannelGroup::getChannel",
            ds2_rva::FMOD_CHANNEL_GROUP_GET_CHANNEL_IAT,
            &GROUP_GET_CHANNEL,
        ),
        (
            "ChannelGroup::getNumGroups",
            ds2_rva::FMOD_CHANNEL_GROUP_GET_NUM_GROUPS_IAT,
            &GROUP_NUM_GROUPS,
        ),
        (
            "Channel::getCurrentSound",
            ds2_rva::FMOD_CHANNEL_GET_CURRENT_SOUND_IAT,
            &CHANNEL_CURRENT_SOUND,
        ),
        (
            "Channel::setPosition",
            ds2_rva::FMOD_CHANNEL_SET_POSITION_IAT,
            &CHANNEL_SET_POSITION,
        ),
    ];
    for (name, rva, into) in imports {
        // SAFETY: fault-safe read of a pointer-sized import slot inside the mapped image.
        let value = unsafe { safe_read_usize(base + rva as usize) }.unwrap_or(0);
        into.store(value, Ordering::Release);
        if value == 0 {
            missing.push(name);
        }
    }
    for (dll, table) in [
        ("fmodex64.dll\0", &FMODEX_EXPORTS[..]),
        ("fmod_event64.dll\0", &EVENT_EXPORTS[..]),
    ] {
        let wide: Vec<u16> = dll.encode_utf16().collect();
        // SAFETY: a NUL-terminated wide string; both DLLs are static imports of the game, so they
        // are mapped already and nothing is loaded.
        let module = unsafe { GetModuleHandleW(wide.as_ptr()) };
        for (name, symbol, into) in table {
            let address = if module.is_null() {
                0
            } else {
                // SAFETY: `module` is a loaded DLL, and `symbol` is a NUL-terminated literal.
                unsafe { GetProcAddress(module, symbol.as_ptr()) as usize }
            };
            into.store(address, Ordering::Release);
            if address == 0 {
                missing.push(name);
            }
        }
    }
    missing
}

/// What `Event::getInfo` says about one event.
#[derive(Clone, Debug)]
pub(crate) struct EventInfo {
    pub(crate) name: String,
    pub(crate) position_ms: i32,
    pub(crate) length_ms: i32,
    pub(crate) project_id: u32,
    pub(crate) system_id: u32,
    /// The first wave bank the event plays from, when asked for and FMOD says.
    pub(crate) bank: Option<String>,
}

impl EventInfo {
    /// FMOD reports `lengthms = -1` for an event with looping sounds: region and boss music, not
    /// jingles.
    pub(crate) fn looping(&self) -> bool {
        self.length_ms == -1
    }

    /// The event's playlist key: its bank and name, or `p<project>/name` when the bank is unknown.
    pub(crate) fn key(&self) -> TrackKey {
        TrackKey::new(
            self.bank
                .clone()
                .unwrap_or_else(|| format!("p{}", self.project_id)),
            self.name.clone(),
        )
    }
}

fn read_i32(buffer: &[u8], offset: usize) -> i32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buffer[offset..offset + 4]);
    i32::from_le_bytes(word)
}

/// Name an event through `Event::getInfo`, and with `with_bank` the wave bank it plays from.
pub(crate) fn event_info(event: usize, with_bank: bool) -> Option<EventInfo> {
    let get_info: GetInfoFn = load(&GET_INFO)?;
    let mut name_ptr: usize = 0;
    let mut info = [0u8; ds2_rva::FMOD_EVENT_INFO_BUFFER_SIZE];
    let mut banks = vec![0u8; WAVEBANKS * ds2_rva::FMOD_WAVEBANK_INFO_STRIDE_BOUND];
    if with_bank {
        let count = ds2_rva::FMOD_EVENT_INFO_MAX_WAVEBANKS_OFFSET;
        info[count..count + 4].copy_from_slice(&(WAVEBANKS as i32).to_le_bytes());
        let array = ds2_rva::FMOD_EVENT_INFO_WAVEBANK_INFO_OFFSET;
        info[array..array + 8].copy_from_slice(&(banks.as_mut_ptr() as usize).to_le_bytes());
    }
    // SAFETY: the game's own `getInfo` import, on a handle the game or FMOD handed us, with
    // out-parameters this frame owns. FMOD validates the handle and answers an error for a stale
    // one. The info buffer is zeroed but for the wave bank array, which is sized so no entry
    // layout can overrun it.
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
    let bank = (with_bank && read_i32(&info, ds2_rva::FMOD_EVENT_INFO_MAX_WAVEBANKS_OFFSET) >= 1)
        .then(|| {
            let start = ds2_rva::FMOD_WAVEBANK_INFO_NAME_OFFSET;
            let raw = &banks[start..start + NAME_MAX];
            let end = raw.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
            String::from_utf8_lossy(&raw[..end]).into_owned()
        })
        .filter(|bank| !bank.is_empty());
    Some(EventInfo {
        name,
        position_ms: read_i32(&info, ds2_rva::FMOD_EVENT_INFO_POSITION_MS_OFFSET),
        length_ms: read_i32(&info, ds2_rva::FMOD_EVENT_INFO_LENGTH_MS_OFFSET),
        project_id: read_i32(&info, ds2_rva::FMOD_EVENT_INFO_PROJECT_ID_OFFSET) as u32,
        system_id: read_i32(&info, ds2_rva::FMOD_EVENT_INFO_SYSTEM_ID_OFFSET) as u32,
        bank,
    })
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

/// The channel an event is playing on, or `None` when it has none (not started, or played out).
pub(crate) fn channel_of(event: usize) -> Option<usize> {
    let get_group: OutPointerFn = load(&GET_CHANNEL_GROUP)?;
    let mut group = 0usize;
    // SAFETY: the game's own `Event::getChannelGroup` import, with an out-parameter this frame
    // owns. FMOD validates the event handle.
    if unsafe { get_group(event, &raw mut group) } != FMOD_OK || group == 0 {
        return None;
    }
    first_channel(group, GROUP_DEPTH)
}

/// The channel's position in ms.
pub(crate) fn position(channel: usize) -> Option<u32> {
    let get_position: TimeFn = load(&CHANNEL_GET_POSITION)?;
    let mut ms = 0u32;
    // SAFETY: `fmodex64!Channel::getPosition`; FMOD validates channel handles.
    (unsafe { get_position(channel, &raw mut ms, ds2_rva::FMOD_TIMEUNIT_MS) } == FMOD_OK)
        .then_some(ms)
}

/// Move the channel to `ms`. The FMOD result.
pub(crate) fn set_position(channel: usize, ms: u32) -> i32 {
    let Some(set_position) = load::<SetTimeFn>(&CHANNEL_SET_POSITION) else {
        return -1;
    };
    // SAFETY: the game's own `Channel::setPosition` import; FMOD validates channel handles.
    unsafe { set_position(channel, ms, ds2_rva::FMOD_TIMEUNIT_MS) }
}

/// `Channel::setLoopCount`: `0` plays to the end once, [`LOOP_FOREVER`] loops. The FMOD result.
pub(crate) fn set_loop_count(channel: usize, count: i32) -> i32 {
    let Some(set_loop_count) = load::<IntArgFn>(&CHANNEL_SET_LOOP_COUNT) else {
        return -1;
    };
    // SAFETY: `fmodex64!Channel::setLoopCount`; FMOD validates channel handles.
    unsafe { set_loop_count(channel, count) }
}

/// Whether the channel is still playing; the FMOD error when it cannot say (a played-out channel
/// answers `FMOD_ERR_INVALID_HANDLE`).
pub(crate) fn is_playing(channel: usize) -> Result<bool, i32> {
    let Some(is_playing) = load::<IsPlayingFn>(&CHANNEL_IS_PLAYING) else {
        return Err(-1);
    };
    let mut playing = 0u8;
    // SAFETY: `fmodex64!Channel::isPlaying`, with an out-parameter this frame owns.
    let rc = unsafe { is_playing(channel, &raw mut playing) };
    if rc == FMOD_OK {
        Ok(playing != 0)
    } else {
        Err(rc)
    }
}

/// The length of the sound on the channel, in ms.
pub(crate) fn sound_length(channel: usize) -> Option<u32> {
    let current_sound: OutPointerFn = load(&CHANNEL_CURRENT_SOUND)?;
    let get_length: TimeFn = load(&SOUND_GET_LENGTH)?;
    let mut sound = 0usize;
    // SAFETY: the game's own `Channel::getCurrentSound` import, then `fmodex64!Sound::getLength`
    // on the sound it returned, with out-parameters this frame owns.
    unsafe {
        if current_sound(channel, &raw mut sound) != FMOD_OK || sound == 0 {
            return None;
        }
        let mut ms = 0u32;
        (get_length(sound, &raw mut ms, ds2_rva::FMOD_TIMEUNIT_MS) == FMOD_OK).then_some(ms)
    }
}

/// `Event::setMute`. The FMOD result.
pub(crate) fn set_mute(event: usize, mute: bool) -> i32 {
    let Some(set_mute) = load::<BoolArgFn>(&SET_MUTE) else {
        return -1;
    };
    // SAFETY: the game's own `Event::setMute` import; FMOD validates the handle.
    unsafe { set_mute(event, u8::from(mute)) }
}

/// `Event::start`, straight to FMOD rather than through the fronted slot. The FMOD result.
pub(crate) fn start(event: usize) -> i32 {
    let Some(start) = load::<StartFn>(&ORIGINAL_START) else {
        return -1;
    };
    // SAFETY: the import the Windows loader resolved; FMOD validates the handle.
    unsafe { start(event) }
}

/// `Event::stop`, straight to FMOD. The FMOD result.
pub(crate) fn stop(event: usize, immediate: bool) -> i32 {
    let Some(stop) = load::<BoolArgFn>(&ORIGINAL_STOP) else {
        return -1;
    };
    // SAFETY: the import the Windows loader resolved; FMOD validates the handle.
    unsafe { stop(event, u8::from(immediate)) }
}

/// `Event::setPaused`, straight to FMOD. The FMOD result.
pub(crate) fn set_paused(event: usize, paused: bool) -> i32 {
    let Some(set_paused) = load::<BoolArgFn>(&ORIGINAL_SET_PAUSED) else {
        return -1;
    };
    // SAFETY: the import the Windows loader resolved; FMOD validates the handle.
    unsafe { set_paused(event, u8::from(paused)) }
}

/// The game's `FMOD::EventSystem*`, or `None` before the sound manager exists.
pub(crate) fn event_system() -> Option<usize> {
    let base = ds2_game_base::mem::game_module_base().ok()?;
    // SAFETY: fault-safe reads: the singleton slot in the image, then the manager's field FMOD
    // itself wrote the event system into.
    unsafe {
        let manager = safe_read_usize(base + ds2_rva::SOUND_MANAGER_SINGLETON as usize)?;
        if manager == 0 {
            return None;
        }
        safe_read_usize(manager + ds2_rva::SOUND_MANAGER_EVENT_SYSTEM_OFFSET).filter(|s| *s != 0)
    }
}

/// `EventSystem::getEventBySystemID`. The handle, or the FMOD error.
pub(crate) fn event_by_system_id(system: usize, id: u32, mode: u32) -> Result<usize, i32> {
    let Some(by_id) = load::<BySystemIdFn>(&GET_EVENT_BY_SYSTEM_ID) else {
        return Err(-1);
    };
    let mut event = 0usize;
    // SAFETY: the game's own import, on the event system the game created, with an out-parameter
    // this frame owns.
    let rc = unsafe { by_id(system, id, mode, &raw mut event) };
    if rc == FMOD_OK && event != 0 {
        Ok(event)
    } else {
        Err(rc)
    }
}

/// `EventSystem::getNumEvents`: events in every loaded project.
pub(crate) fn num_events(system: usize) -> Option<i32> {
    let get: CountFn = load(&GET_NUM_EVENTS)?;
    let mut count = 0i32;
    // SAFETY: `fmod_event64!EventSystem::getNumEvents` on the game's event system.
    (unsafe { get(system, &raw mut count) } == FMOD_OK).then_some(count)
}

/// `Channel::setMute`. The FMOD result.
///
/// This, and not `Event::setMute`, is how the game's own track is silenced: an event mute does not
/// come off the channel again (see `ds2_rva::FMOD_EVENT_GROUP_FREE_EVENT_DATA_SLOT`'s section).
pub(crate) fn channel_set_mute(channel: usize, mute: bool) -> i32 {
    let Some(set_mute) = load::<BoolArgFn>(&CHANNEL_SET_MUTE) else {
        return -1;
    };
    // SAFETY: `fmodex64!Channel::setMute`; FMOD validates channel handles.
    unsafe { set_mute(channel, u8::from(mute)) }
}

/// `Channel::getAudibility`: the volume the channel is actually heard at, after every mute, fade,
/// group volume and 3D attenuation. `None` when FMOD cannot say.
pub(crate) fn audibility(channel: usize) -> Option<f32> {
    let get: AudibilityFn = load(&CHANNEL_GET_AUDIBILITY)?;
    let mut value = 0.0f32;
    // SAFETY: `fmodex64!Channel::getAudibility`, with an out-parameter this frame owns.
    (unsafe { get(channel, &raw mut value) } == FMOD_OK).then_some(value)
}

/// `FMOD_Memory_GetStats`: bytes FMOD has allocated now, and the most it ever has.
pub(crate) fn memory_stats() -> Option<(i32, i32)> {
    let get: MemoryStatsFn = load(&MEMORY_GET_STATS)?;
    let (mut current, mut max) = (0i32, 0i32);
    // SAFETY: the `fmodex64` C export, with two out-parameters this frame owns; `blocking = 0`
    // takes no lock the game could be holding.
    (unsafe { get(&raw mut current, &raw mut max, 0) } == FMOD_OK).then_some((current, max))
}

/// Listener 0's position, which is where the game keeps its region music.
pub(crate) fn listener_position(system: usize) -> Option<[f32; 3]> {
    let get: ListenerFn = load(&GET_3D_LISTENER)?;
    let mut position = [0.0f32; 3];
    let (mut velocity, mut forward, mut up) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3]);
    // SAFETY: the game's own import on its event system, with four out-parameters this frame owns.
    let rc = unsafe {
        get(
            system,
            0,
            &raw mut position,
            &raw mut velocity,
            &raw mut forward,
            &raw mut up,
        )
    };
    (rc == FMOD_OK).then_some(position)
}

/// Put an event at `position`, still. The FMOD result; a 2D event answers an error, harmlessly.
pub(crate) fn set_3d_position(event: usize, position: [f32; 3]) -> i32 {
    let Some(set) = load::<Set3dFn>(&SET_3D_ATTRIBUTES) else {
        return -1;
    };
    let still = [0.0f32; 3];
    // SAFETY: the game's own import; two vectors this frame owns and a null orientation, which
    // FMOD documents as "leave it".
    unsafe {
        set(
            event,
            &raw const position,
            &raw const still,
            core::ptr::null(),
        )
    }
}

/// Free the memory a stopped instance of ours holds, through its event group's `freeEventData`.
/// The FMOD result of the free.
pub(crate) fn free_event_data(event: usize) -> i32 {
    let Some(get_group) = load::<OutPointerFn>(&GET_PARENT_GROUP) else {
        return -1;
    };
    let mut group = 0usize;
    // SAFETY: the game's own `Event::getParentGroup` import, with an out-parameter this frame
    // owns. FMOD validates the handle.
    if unsafe { get_group(event, &raw mut group) } != FMOD_OK || group == 0 {
        return -2;
    }
    // SAFETY: `group` is a live `EventGroupI` FMOD just returned; its vtable pointer is its first
    // field, and slot `FMOD_EVENT_GROUP_FREE_EVENT_DATA_SLOT` is `freeEventData`, read out of
    // `fmod_event64.dll`'s own vtable. The reads are fault-safe.
    let Some(free) = (unsafe {
        safe_read_usize(group)
            .and_then(|vtable| {
                safe_read_usize(vtable + ds2_rva::FMOD_EVENT_GROUP_FREE_EVENT_DATA_SLOT * 8)
            })
            .filter(|f| *f != 0)
    }) else {
        return -3;
    };
    // SAFETY: `free` is `EventGroupI::freeEventData(Event* event, bool waituntilready)`, called on
    // its own group with our stopped instance; not waiting, so the sound thread is never blocked.
    let free: FreeEventDataFn = unsafe { std::mem::transmute::<usize, FreeEventDataFn>(free) };
    // SAFETY: as above.
    unsafe { free(group, event, 0) }
}
