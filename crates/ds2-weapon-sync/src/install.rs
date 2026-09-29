//! The detours: the clamp on the game's weapon update and on its armour update, the fix on the
//! inventory save writer, and the per-frame check on the net session update that decides when to
//! push our equipment through the updates again.
//!
//! Two features run on this machinery, each a [`Feature`] with its own switch, key, cap, ledger,
//! tracker and watch: weapon sync (`ds2-weapon-sync:` in the log) and armour sync
//! (`ds2-armor-sync:`). Either can be installed without the other. What they share is only what
//! the game has once: the save writer (one detour fixes the records both ledgers lowered), the net
//! session tick (one registration checks both), and the thread that polls the keys.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use ds2_game_base::mem::{
    game_rva, read_bytes, safe_read_u8, safe_read_u16, safe_read_u32, safe_read_usize,
};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};
use ds2_hotkey_config::chord_name;
use ds2_hotkey_config::keys::{Chord, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_SHIFT};
use ds2_hotkey_config::live::AtomicChord;

use crate::policy::{self, Action, Equipped, Held, Kind, Ledger, Remote, Slot, Tracker, Watch};

unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
    fn GetCurrentProcessId() -> u32;
}

/// `VK_CONTROL`, `VK_MENU`, `VK_SHIFT` -- the three modifiers a [`Chord`] can carry.
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_SHIFT: i32 = 0x10;

/// Whether the foreground window belongs to this process, so a key typed into another window is
/// not a press. Same check as `ds2-voice-chat`'s.
fn game_has_focus() -> bool {
    // SAFETY: a plain Win32 call with no arguments.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return false;
    }
    let mut owner = 0u32;
    // SAFETY: `foreground` is a handle Win32 just returned and `owner` is a live `u32`.
    unsafe { GetWindowThreadProcessId(foreground, &raw mut owner) };
    // SAFETY: a plain Win32 call with no arguments.
    owner != 0 && owner == unsafe { GetCurrentProcessId() }
}

fn vk_down(vk: i32) -> bool {
    // SAFETY: a plain Win32 call taking an integer. Only the high bit ("down now") is used.
    unsafe { GetAsyncKeyState(vk) < 0 }
}

fn chord_down(chord: Chord) -> bool {
    if chord.vk == 0 {
        return false;
    }
    for (bit, vk) in [
        (MODIFIER_CTRL, VK_CONTROL),
        (MODIFIER_ALT, VK_MENU),
        (MODIFIER_SHIFT, VK_SHIFT),
    ] {
        if chord.modifiers & bit != 0 && !vk_down(vk) {
            return false;
        }
    }
    i32::try_from(chord.vk).is_ok_and(vk_down)
}

// ---------------------------------------------------------------------------------------------
// The two features.
// ---------------------------------------------------------------------------------------------

/// One feature's switches and state. Everything a press, a cap or a restore touches is here, so
/// the two features never share a flag.
pub(crate) struct Feature {
    kind: Kind,
    /// What every line of this feature begins with.
    pub(crate) prefix: &'static str,
    /// "weapon sync" or "armor sync", for the log lines that say what was toggled.
    name: &'static str,
    /// What the feature lowers, plural, for the log.
    things: &'static str,
    /// Whether [`install`] put this feature in.
    installed: AtomicBool,
    /// Flipped by the key; starts on, because the config's `enabled` already asked for it.
    enabled: AtomicBool,
    /// The key that flips [`Feature::enabled`]. Unset means unbound.
    key: AtomicChord,
    /// Whether the key was down at the last poll, so a hold is one press.
    was_down: AtomicBool,
    /// A press the key thread saw and the game-thread tick has not acted on yet.
    pressed: AtomicBool,
    /// The cap the clamp applies: [`NO_CAP`], or a level.
    current_cap: AtomicU32,
    /// A remote player at this level, for testing solo. [`NO_CAP`] when not configured.
    test_cap: AtomicU32,
    /// Trampoline back to the game's update for this feature's items. Also what the push calls,
    /// so the push runs the game's code and not our clamp twice.
    update_original: AtomicUsize,
    /// Every inventory entry this feature lowered, with its real level. The tick sweeps it and the
    /// save writer reads it, so it is behind a lock; neither holds it across a call into the game.
    ledger: Mutex<Ledger>,
    /// The equipped slots as the last resweep left them. Game thread only.
    watch: Mutex<Watch>,
    /// Which cap the equipment was last pushed at. Game thread only.
    tracker: Mutex<Tracker>,
    /// The local `PlayerCtrl` the last check saw, so a new world gets one line of what it was
    /// built with.
    last_player: AtomicUsize,
    /// Whether the first check has been logged.
    first_tick: AtomicBool,
}

impl Feature {
    const fn new(
        kind: Kind,
        prefix: &'static str,
        name: &'static str,
        things: &'static str,
    ) -> Self {
        Self {
            kind,
            prefix,
            name,
            things,
            installed: AtomicBool::new(false),
            enabled: AtomicBool::new(true),
            key: AtomicChord::unset(),
            was_down: AtomicBool::new(false),
            pressed: AtomicBool::new(false),
            current_cap: AtomicU32::new(NO_CAP),
            test_cap: AtomicU32::new(NO_CAP),
            update_original: AtomicUsize::new(0),
            ledger: Mutex::new(Ledger::new()),
            watch: Mutex::new(Watch::new()),
            tracker: Mutex::new(Tracker::new()),
            last_player: AtomicUsize::new(0),
            first_tick: AtomicBool::new(false),
        }
    }

    fn installed(&self) -> bool {
        self.installed.load(Ordering::Acquire)
    }

    fn key_name(&self) -> String {
        self.key
            .load()
            .filter(|chord| chord.vk != 0)
            .map_or_else(|| "no key".to_string(), chord_name)
    }

    /// The slot count, and for slot `i` (`0..slots`): the bag's equip slot, the character-side
    /// slot the update takes, and the equipment record's index.
    fn inventory_slot(&self, slot: usize) -> usize {
        match self.kind {
            Kind::Weapon => slot,
            Kind::Armor => ds2_rva::ARMOR_INVENTORY_SLOT_FIRST + slot,
        }
    }

    fn request_slot(&self, slot: usize) -> i32 {
        match self.kind {
            Kind::Weapon => ds2_rva::WEAPON_INTERNAL_TO_CHR_SLOT[slot],
            // `0x1401b66a0` passes `DAT_1410c44a8[slot - 6]` = 0, 1, 2, 3.
            Kind::Armor => i32::try_from(slot).unwrap_or(0),
        }
    }

    fn record_index(&self, slot: usize) -> usize {
        match self.kind {
            Kind::Weapon => ds2_rva::WEAPON_INTERNAL_TO_CHR_SLOT[slot] as usize,
            Kind::Armor => ds2_rva::ARMOR_RECORD_FIRST + slot,
        }
    }

    /// Where the live level of slot `slot` sits in `ChrAsmEquip`.
    fn live_level_offset(&self, slot: usize) -> usize {
        match self.kind {
            Kind::Weapon => {
                live_index(self.record_index(slot)) * ds2_rva::CHR_ASM_EQUIP_WEAPON_STRIDE
                    + ds2_rva::CHR_ASM_EQUIP_WEAPON_LEVEL_OFFSET
            }
            Kind::Armor => {
                ds2_rva::CHR_ASM_EQUIP_ARMOR_OFFSET
                    + slot * ds2_rva::CHR_ASM_EQUIP_ARMOR_STRIDE
                    + ds2_rva::CHR_ASM_EQUIP_ARMOR_LEVEL_OFFSET
            }
        }
    }

    /// The id the update and the record carry for an inventory item: the item id for a weapon,
    /// the ArmorParam id for armour (`ItemParam +0x18`, which is the item id minus
    /// [`ds2_rva::ARMOR_PARAM_ID_FROM_ITEM_ID`] for every armour row).
    fn request_item(&self, item: u32) -> u32 {
        match self.kind {
            Kind::Weapon => item,
            Kind::Armor => item.wrapping_sub(ds2_rva::ARMOR_PARAM_ID_FROM_ITEM_ID),
        }
    }

    /// A record's id as the inventory item it came from, so the two compare.
    fn record_item(&self, id: u32) -> u32 {
        match self.kind {
            Kind::Weapon => id,
            Kind::Armor if id == 0 || id == u32::MAX => id,
            Kind::Armor => id.wrapping_add(ds2_rva::ARMOR_PARAM_ID_FROM_ITEM_ID),
        }
    }

    /// What the first of the two pushes puts in a slot so the second is a change of item, which
    /// the record writer keeps: Fists for a weapon, "no piece" for armour.
    fn placeholder(&self) -> u32 {
        match self.kind {
            Kind::Weapon => ds2_rva::FISTS_ITEM_ID,
            Kind::Armor => ds2_rva::ARMOR_EMPTY_ITEM_ID,
        }
    }
}

/// Weapon sync.
pub(crate) static WEAPONS: Feature =
    Feature::new(Kind::Weapon, crate::LOG_PREFIX, "weapon sync", "weapons");

/// Armour sync.
pub(crate) static ARMOR: Feature =
    Feature::new(Kind::Armor, crate::ARMOR_LOG_PREFIX, "armor sync", "armor pieces");

pub(crate) fn feature(kind: Kind) -> &'static Feature {
    match kind {
        Kind::Weapon => &WEAPONS,
        Kind::Armor => &ARMOR,
    }
}

const FEATURES: [&Feature; 2] = [&WEAPONS, &ARMOR];

/// Bind a feature's on/off key, or unbind it with `None`. Takes effect on the next frame.
pub fn set_key(kind: Kind, chord: Option<Chord>) {
    let f = feature(kind);
    let before = f.key_name();
    match chord {
        Some(chord) => f.key.store(chord),
        None => f.key.store(Chord {
            modifiers: 0,
            vk: 0,
            dik: None,
        }),
    }
    let after = f.key_name();
    if before != after {
        log(format_args!("{} key {before} -> {after}", f.prefix));
    }
}

/// `CHR_WEAPON_UPDATE` / `CHR_ARMOR_UPDATE`: `(PlayerCtrl*, UpdateRequest*)`.
type EquipUpdate = unsafe extern "system" fn(usize, *mut u8);

/// `SAVE_DATA_ITEM_INVENTORY_WRITE(this, stream*, enabled)`.
type SaveWrite = unsafe extern "system" fn(usize, usize, u32) -> usize;

/// Trampoline back to the real inventory save writer.
static SAVE_WRITE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" {
    fn WriteProcessMemory(
        process: isize,
        address: *mut c_void,
        buffer: *const c_void,
        size: usize,
        written: *mut usize,
    ) -> i32;
}

/// `GetCurrentProcess()`'s constant pseudo handle.
const CURRENT_PROCESS: isize = -1;

/// Write one byte, answering `false` instead of faulting on an unmapped or read-only page.
fn safe_write_u8(address: usize, value: u8) -> bool {
    let mut written = 0usize;
    // SAFETY: the source is a local byte; `WriteProcessMemory` checks the destination in the
    // kernel and answers FALSE for a page it cannot write.
    let ok = unsafe {
        WriteProcessMemory(
            CURRENT_PROCESS,
            address as *mut c_void,
            (&raw const value).cast(),
            1,
            &raw mut written,
        )
    };
    ok != 0 && written == 1
}

/// `GameManagerImp`'s address (the global that holds the pointer), resolved at install.
static GAME_MANAGER: AtomicUsize = AtomicUsize::new(0);

/// `PlayerCtrl`'s vtable, resolved at install. A roster entry is a player only if it has this.
static PLAYER_CTRL_VTABLE: AtomicUsize = AtomicUsize::new(0);

const NO_CAP: u32 = u32::MAX;

/// Frames since the last check.
static FRAME: AtomicU32 = AtomicU32::new(0);

/// How often the roster is read. Four times a second is well inside how long a peer's packet 61
/// or 62 takes to matter and keeps the fault-safe reads off every frame.
const CHECK_EVERY_FRAMES: u32 = 15;

/// A log sink, installed by the loader so this crate writes into the same file as everything else.
/// Stored as a `usize` because a `fn` pointer is not an `Atomic` type.
static LOGGER: AtomicUsize = AtomicUsize::new(0);

/// Signature of the sink. Matches the loader's own logging entry point.
pub type LogFn = fn(std::fmt::Arguments<'_>);

/// Point this crate's logging at the loader's log file. Call before [`install`].
pub fn set_logger(logger: LogFn) {
    LOGGER.store(logger as usize, Ordering::Release);
}

/// The sink [`set_logger`] stored, so the HUD can hand `ds2-overlay` the same one.
pub(crate) fn logger() -> Option<LogFn> {
    let raw = LOGGER.load(Ordering::Acquire);
    // SAFETY: a nonzero `raw` is only ever a `LogFn` stored by `set_logger` above.
    (raw != 0).then(|| unsafe { std::mem::transmute::<usize, LogFn>(raw) })
}

pub(crate) fn log(args: std::fmt::Arguments<'_>) {
    if let Some(logger) = logger() {
        logger(args);
    }
}

/// Whether a feature is installed and on right now, for the HUD.
pub(crate) fn enabled(kind: Kind) -> bool {
    let f = feature(kind);
    f.installed() && f.enabled.load(Ordering::Acquire)
}

#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const c_void, module: *mut c_void, flags: u32) -> i32;
}

/// `SND_ASYNC | SND_NODEFAULT | SND_MEMORY`, the flags `ds2-voice-chat` plays its clips with:
/// return at once, never fall back to the system beep, and read the WAV from the pointer. A new
/// clip cuts off one still playing.
const PLAY_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004;

/// Say which way the key just went, in the feature's own words. `true` when `winmm` accepted it.
fn announce(kind: Kind, on: bool) -> bool {
    let clip = crate::clip_for(kind, on);
    // SAFETY: `clip` is a `'static` WAV compiled into this DLL, so it outlives the async play.
    unsafe { PlaySoundW(clip.as_ptr().cast(), core::ptr::null_mut(), PLAY_FLAGS) != 0 }
}

/// One feature's configuration, as the loader read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// A pretend remote player at this level, so the cap can be seen working alone.
    pub test_cap: Option<u8>,
    /// The on/off key. `None` leaves it unbound.
    pub key: Option<Chord>,
}

/// What [`install`] managed to do, per feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Weapon sync is live. `false` when it was not asked for or could not go in.
    pub weapons: bool,
    /// Armour sync is live.
    pub armor: bool,
}

/// Change a feature's pretend remote player while the game runs, or remove it with `None`.
///
/// The next check (within a quarter second) picks it up exactly as it would a real player
/// arriving, changing equipment or leaving, so removing it exercises the in-world restore.
pub fn set_test_cap(kind: Kind, test_cap: Option<u8>) {
    let f = feature(kind);
    let previous = f.test_cap.swap(cap_to_atomic(test_cap), Ordering::AcqRel);
    if previous != cap_to_atomic(test_cap) {
        log(format_args!(
            "{} test_cap {} -> {}",
            f.prefix,
            show(cap_from_atomic(previous)),
            show(test_cap)
        ));
    }
}

fn cap_from_atomic(raw: u32) -> Option<u8> {
    u8::try_from(raw).ok()
}

fn cap_to_atomic(cap: Option<u8>) -> u32 {
    cap.map_or(NO_CAP, u32::from)
}

fn show(cap: Option<u8>) -> String {
    cap.map_or_else(|| "none".to_string(), |level| format!("+{level}"))
}

/// The resolved address of `rva` if it holds `expected`, logged either way it fails.
fn checked_site(prefix: &str, rva: u32, expected: &[u8], name: &str) -> Option<usize> {
    let site = match game_rva(rva) {
        Ok(site) => site,
        Err(error) => {
            log(format_args!(
                "{prefix} not installed reason=no-module-base what={name} -- {error}"
            ));
            return None;
        }
    };
    let mut found = vec![0u8; expected.len()];
    // SAFETY: a resolved RVA in the loaded image; `read_bytes` reports an unmapped page rather
    // than faulting.
    let read = unsafe { read_bytes(site, &mut found) };
    if !read || found != expected {
        log(format_args!(
            "{prefix} not installed reason=prologue what={name} va=0x{site:016x} read={read} \
             saw={found:02x?} want={expected:02x?} -- that address is not {name} on this build, so \
             nothing was patched"
        ));
        return None;
    }
    Some(site)
}

/// Create and enable one MinHook detour, logging under `prefix` if it fails.
fn hook(prefix: &str, site: usize, detour: *mut c_void, name: &str) -> Option<usize> {
    // SAFETY: the caller checked the site's recorded prologue, and the detour has the same ABI.
    let created = unsafe { MhHook::new(site as *mut c_void, detour) };
    let hook = match created {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{prefix} not installed: MH_CreateHook on {name} said {status:?}"
            ));
            return None;
        }
    };
    let trampoline = hook.trampoline() as usize;
    Some(trampoline)
}

fn enable(prefix: &str, site: usize, name: &str) -> bool {
    // SAFETY: an address `MhHook::new` accepted.
    let status = unsafe { MH_EnableHook(site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{prefix} not installed: MH_EnableHook on {name} said {status:?}"
        ));
        return false;
    }
    true
}

/// The update detour's site, its prologue, its detour and its name, per feature.
fn update_site(kind: Kind) -> (u32, &'static [u8], *mut c_void, &'static str) {
    match kind {
        Kind::Weapon => (
            ds2_rva::CHR_WEAPON_UPDATE,
            &ds2_rva::CHR_WEAPON_UPDATE_PROLOGUE,
            weapon_update_detour as *mut c_void,
            "CHR_WEAPON_UPDATE",
        ),
        Kind::Armor => (
            ds2_rva::CHR_ARMOR_UPDATE,
            &ds2_rva::CHR_ARMOR_UPDATE_PROLOGUE,
            armor_update_detour as *mut c_void,
            "CHR_ARMOR_UPDATE",
        ),
    }
}

/// Install weapon sync, armour sync, or both. Each goes in only if its settings are given, and a
/// feature that cannot go in is logged and left out without taking the other with it.
///
/// Both lower inventory entries, so neither is installed without the save writer's fix, which is
/// put in first and shared. The clamp on each feature's update goes in before the tick; the tick
/// is registered once for whichever features are in.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Call once, from the loader's post-Arxan
/// install position.
pub unsafe fn install(weapons: Option<Settings>, armor: Option<Settings>) -> Outcome {
    let none = Outcome {
        weapons: false,
        armor: false,
    };
    let wanted: Vec<(&'static Feature, Settings)> = [(&WEAPONS, weapons), (&ARMOR, armor)]
        .into_iter()
        .filter_map(|(f, settings)| settings.map(|s| (f, s)))
        .collect();
    if wanted.is_empty() {
        return none;
    }
    let refuse_all = |why: &str| {
        for (f, _) in &wanted {
            log(format_args!("{} not installed: {why}", f.prefix));
        }
        none
    };
    let prefix = wanted[0].0.prefix;
    let Some(save_site) = checked_site(
        prefix,
        ds2_rva::SAVE_DATA_ITEM_INVENTORY_WRITE,
        &ds2_rva::SAVE_DATA_ITEM_INVENTORY_WRITE_PROLOGUE,
        "SAVE_DATA_ITEM_INVENTORY_WRITE",
    ) else {
        return refuse_all("the save writer is not where it should be");
    };
    let (Ok(manager), Ok(vtable)) = (
        game_rva(ds2_rva::GAME_MANAGER_IMP),
        game_rva(ds2_rva::PLAYER_CTRL_VTABLE),
    ) else {
        return refuse_all("reason=no-module-base");
    };
    GAME_MANAGER.store(manager, Ordering::Release);
    PLAYER_CTRL_VTABLE.store(vtable, Ordering::Release);

    // SAFETY: MinHook's own initialiser, no arguments; ALREADY_INITIALIZED means another crate in
    // this DLL got there first.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        return refuse_all(&format!("MH_Initialize said {status:?}"));
    }
    // The save fix goes in first: until a tick lowers something it changes nothing, and nothing
    // may be lowered before it is live.
    let Some(save_trampoline) = hook(
        prefix,
        save_site,
        save_write_detour as *mut c_void,
        "SAVE_DATA_ITEM_INVENTORY_WRITE",
    ) else {
        return refuse_all("no save writer hook");
    };
    SAVE_WRITE_ORIGINAL.store(save_trampoline, Ordering::Release);
    if !enable(prefix, save_site, "SAVE_DATA_ITEM_INVENTORY_WRITE") {
        return refuse_all("the save writer hook would not enable");
    }

    // Each feature's clamp. It does nothing until the tick sets a cap, so a feature whose tick
    // never runs leaves its update exactly as the game's own.
    let mut live = Vec::new();
    for (f, settings) in &wanted {
        let (rva, prologue, detour, name) = update_site(f.kind);
        let Some(site) = checked_site(f.prefix, rva, prologue, name) else {
            continue;
        };
        let Some(trampoline) = hook(f.prefix, site, detour, name) else {
            continue;
        };
        // Published before the site is patched, so a detour that fires at once has somewhere to go.
        f.update_original.store(trampoline, Ordering::Release);
        if !enable(f.prefix, site, name) {
            continue;
        }
        if let Some(chord) = settings.key {
            f.key.store(chord);
        }
        f.test_cap
            .store(cap_to_atomic(settings.test_cap), Ordering::Release);
        live.push((*f, site, settings.test_cap));
    }
    if live.is_empty() {
        return none;
    }
    // The net session update is shared with `ds2-voice-chat`; `ds2-net-tick` owns its one detour
    // and runs this after the original.
    // SAFETY: the loader's post-Arxan install position, which is this function's own contract.
    let tick_site = match unsafe { ds2_net_tick::register(ds2_net_tick::When::After, tick) } {
        Ok(site) => site,
        Err(error) => {
            for (f, _, _) in &live {
                log(format_args!(
                    "{} not installed: no tick -- {error}. The clamp is in but never has a cap, so \
                     no level is ever changed",
                    f.prefix
                ));
            }
            return none;
        }
    };
    for (f, site, test_cap) in &live {
        f.installed.store(true, Ordering::Release);
        log(format_args!(
            "{} installed update=0x{site:016x} save-write=0x{save_site:016x} (shared) \
             tick=0x{tick_site:016x} (shared) key={} test_cap={} -- while another player is in \
             the world, every one of our {} in the inventory above the highest level any of them \
             has equipped is lowered to it, equipped or not; the save always gets the real levels",
            f.prefix,
            f.key_name(),
            show(*test_cap),
            f.things
        ));
    }
    std::thread::spawn(poll_keys);
    // After the detours, and never a reason to refuse: the tiles only show the switches.
    for (f, _, _) in &live {
        crate::hud::install(f.kind);
    }
    Outcome {
        weapons: WEAPONS.installed(),
        armor: ARMOR.installed(),
    }
}

// ---------------------------------------------------------------------------------------------
// Reading the game.
// ---------------------------------------------------------------------------------------------

fn non_null(pointer: usize) -> Option<usize> {
    (pointer != 0).then_some(pointer)
}

fn read_ptr(address: usize) -> Option<usize> {
    // SAFETY: fault-safe read; an unmapped or freed address answers None.
    non_null(unsafe { safe_read_usize(address)? })
}

fn local_player() -> Option<usize> {
    let manager = read_ptr(GAME_MANAGER.load(Ordering::Acquire))?;
    read_ptr(manager + ds2_rva::PLAYER_CTRL_OFFSET)
}

/// The inventory's manager, `[ItemInventory2 + 0x10]`: the first bag hop, and the object whose
/// `+0x30` is the save block.
fn inventory_manager() -> Option<usize> {
    let manager = read_ptr(GAME_MANAGER.load(Ordering::Acquire))?;
    let data = read_ptr(manager + ds2_rva::GAME_DATA_MANAGER_OFFSET)?;
    let inventory = read_ptr(data + ds2_rva::ITEM_INVENTORY_OFFSET)?;
    read_ptr(inventory + ds2_rva::ITEM_BAG_LIST_OFFSET)
}

/// The bag: `[[[GameManagerImp + 0xA8] + 0x10] + 0x10] + 0x10`, two bag hops.
fn bag() -> Option<usize> {
    let mut at = inventory_manager()?;
    for _ in 1..ds2_rva::ITEM_BAG_LIST_HOPS {
        at = read_ptr(at + ds2_rva::ITEM_BAG_LIST_OFFSET)?;
    }
    Some(at)
}

fn entry_address(bag: usize, index: u16) -> usize {
    bag + ds2_rva::ITEM_ENTRY_ARRAY_OFFSET + usize::from(index) * ds2_rva::ITEM_ENTRY_STRIDE
}

/// Every entry in the bag's array that belongs to `kind` (weapons and shields, or armour), pack
/// and box alike, or `None` when the array could not be read in one piece. A partial read is never
/// answered, because the ledger takes an entry missing from the answer to be gone.
///
/// An entry counts when its type is the feature's, its item id is set, and its handle is its own
/// index, which is what makes the index the save block's index too.
fn held(bag: usize, kind: Kind) -> Option<Vec<Held>> {
    let mut raw = vec![0u8; ds2_rva::ITEM_ENTRY_COUNT * ds2_rva::ITEM_ENTRY_STRIDE];
    // SAFETY: one fault-safe bulk read; nothing is interpreted unless it all came back.
    if !unsafe { read_bytes(bag + ds2_rva::ITEM_ENTRY_ARRAY_OFFSET, &mut raw) } {
        return None;
    }
    let mut out = Vec::new();
    let (entries, _) = raw.as_chunks::<{ ds2_rva::ITEM_ENTRY_STRIDE }>();
    for (index, entry) in entries.iter().enumerate() {
        let id = ds2_rva::ITEM_ENTRY_ITEM_ID_OFFSET;
        let item = u32::from_le_bytes([entry[id], entry[id + 1], entry[id + 2], entry[id + 3]]);
        let handle = u16::from_le_bytes([
            entry[ds2_rva::ITEM_ENTRY_HANDLE_OFFSET],
            entry[ds2_rva::ITEM_ENTRY_HANDLE_OFFSET + 1],
        ]);
        let item_type = entry[ds2_rva::ITEM_ENTRY_TYPE_OFFSET];
        let Ok(index) = u16::try_from(index) else {
            break;
        };
        if item == 0 || item == u32::MAX || handle != index || !kind.holds(item_type) {
            continue;
        }
        out.push(Held {
            index,
            item,
            level: entry[ds2_rva::ITEM_ENTRY_LEVEL_OFFSET] & 0x0f,
        });
    }
    Some(out)
}

/// Put `to` in the low nibble of a level byte at `address` if the entry still holds `item` at
/// `from`. `item_at` is where that entry or record keeps its item id.
fn write_level(item_at: usize, level_at: usize, item: u32, from: Option<u8>, to: u8) -> bool {
    // SAFETY: fault-safe reads.
    let (Some(found), Some(byte)) = (unsafe { safe_read_u32(item_at) }, unsafe {
        safe_read_u8(level_at)
    }) else {
        return false;
    };
    if found != item || from.is_some_and(|from| byte & 0x0f != from) {
        return false;
    }
    safe_write_u8(level_at, (byte & 0xf0) | (to & 0x0f))
}

/// The save block record for entry `index`: where its item id and its level byte are.
fn save_record(manager: usize, index: u16) -> (usize, usize) {
    let record = manager
        + ds2_rva::ITEM_INVENTORY_SAVE_BLOCK_OFFSET
        + usize::from(index) * ds2_rva::ITEM_SAVE_RECORD_STRIDE;
    (
        record + ds2_rva::ITEM_SAVE_RECORD_ITEM_OFFSET,
        record + ds2_rva::ITEM_SAVE_RECORD_LEVEL_OFFSET,
    )
}

/// Give every save record the ledger lowered its real level back. Answers `(fixed, already real)`.
fn fix_save_block(manager: usize, ledger: &Ledger) -> (usize, usize) {
    let (mut fixed, mut already) = (0, 0);
    for (index, item, real) in ledger.lowered() {
        let (item_at, level_at) = save_record(manager, index);
        // SAFETY: fault-safe reads.
        let (Some(found), Some(level)) = (unsafe { safe_read_u32(item_at) }, unsafe {
            safe_read_u8(level_at)
        }) else {
            continue;
        };
        match ledger.saved_level(index, found, level & 0x0f) {
            Some(real_level) if write_level(item_at, level_at, item, None, real_level) => {
                fixed += 1;
            }
            None if found == item && level & 0x0f == real => already += 1,
            _ => {}
        }
    }
    (fixed, already)
}

/// One equipped item as the inventory holds it: the real level.
#[derive(Clone, Copy, Debug)]
struct InventoryItem {
    item: u32,
    /// The entry's handle, which is its index in the bag and the ledger's key.
    handle: u16,
    durability_bits: u32,
    level: u8,
    infusion: u8,
}

/// The inventory entry in the bag's equip slot `inventory_slot`, read the way `0x1401b66a0` reads
/// it. `None` for an empty slot.
fn inventory_item(bag: usize, inventory_slot: usize) -> Option<InventoryItem> {
    let entry = read_ptr(bag + ds2_rva::ITEM_BAG_EQUIPPED_ENTRIES_OFFSET + inventory_slot * 8)?;
    // SAFETY: fault-safe reads of an entry the bag points at.
    let (kind, item, handle, durability_bits, level, infusion) = unsafe {
        (
            safe_read_u8(entry + ds2_rva::ITEM_ENTRY_TYPE_OFFSET)?,
            safe_read_u32(entry + ds2_rva::ITEM_ENTRY_ITEM_ID_OFFSET)?,
            safe_read_u16(entry + ds2_rva::ITEM_ENTRY_HANDLE_OFFSET)?,
            safe_read_u32(entry + ds2_rva::ITEM_ENTRY_DURABILITY_OFFSET)?,
            safe_read_u8(entry + ds2_rva::ITEM_ENTRY_LEVEL_OFFSET)?,
            safe_read_u8(entry + ds2_rva::ITEM_ENTRY_INFUSION_OFFSET)?,
        )
    };
    Some(InventoryItem {
        item,
        handle,
        durability_bits,
        level: if kind < ds2_rva::ITEM_TYPE_HAS_LEVEL_BELOW {
            level & 0x0f
        } else {
            0
        },
        infusion: if kind < ds2_rva::ITEM_TYPE_HAS_INFUSION_BELOW {
            infusion & 0x0f
        } else {
            0
        },
    })
}

/// How many equipment records the weapon and armour features read: weapons 0..5, armour 6..9.
const RECORDS_READ: usize = ds2_rva::ARMOR_RECORD_FIRST + ds2_rva::ARMOR_SLOT_COUNT;

/// A character's first ten equipment records, `(item id, level byte)`, by record index.
fn records(character: usize) -> Option<[(u32, u8); RECORDS_READ]> {
    let asm = read_ptr(character + ds2_rva::CHARACTER_CTRL_CHR_ASM_CTRL_OFFSET)?;
    let table = read_ptr(asm + ds2_rva::CHR_ASM_CTRL_RECORD_TABLE_OFFSET)?;
    let mut raw = [0u8; RECORDS_READ * ds2_rva::EQUIP_RECORD_STRIDE];
    // SAFETY: one fault-safe bulk read; nothing is interpreted unless it all came back.
    if !unsafe { read_bytes(table + ds2_rva::EQUIP_RECORD_ARRAY_OFFSET, &mut raw) } {
        return None;
    }
    let mut out = [(0u32, 0u8); RECORDS_READ];
    for (index, record) in out.iter_mut().enumerate() {
        let base = index * ds2_rva::EQUIP_RECORD_STRIDE;
        let item = &raw[base + ds2_rva::EQUIP_RECORD_ITEM_OFFSET..][..4];
        *record = (
            u32::from_le_bytes([item[0], item[1], item[2], item[3]]),
            raw[base + ds2_rva::EQUIP_RECORD_LEVEL_OFFSET],
        );
    }
    Some(out)
}

/// The live weapon entry for character-side slot `chr_slot`.
///
/// Records alternate hands (`r = index * 2 + hand`), live entries group them
/// (`n = hand * 3 + index`). Read live: record 0 (right hand 1, the Dagger) was live entry 0 and
/// record 1 (left hand 1, the shield) was live entry 3.
const fn live_index(chr_slot: usize) -> usize {
    (chr_slot % 2) * 3 + chr_slot / 2
}

/// A character's live levels for one feature's slots, inventory slot order.
fn live_levels(f: &Feature, character: usize) -> Option<Vec<u8>> {
    let asm = read_ptr(character + ds2_rva::CHARACTER_CTRL_CHR_ASM_CTRL_OFFSET)?;
    let equip = read_ptr(asm + ds2_rva::CHR_ASM_CTRL_EQUIP_OFFSET)?;
    (0..f.kind.slots())
        // SAFETY: fault-safe read.
        .map(|slot| unsafe { safe_read_u8(equip + f.live_level_offset(slot)) })
        .collect()
}

const NAME_LIMIT: usize = 64;
const NETWORK_PLAYER_PREFIX: &str = "NetworkPlayer";

/// Whether this `PlayerCtrl` is a person in our session: the remote-player factory's name, and
/// not a bloodstain replay's phantom param (bd `ds2-playerctrl-is-not-a-person-2026-09-24`; the
/// same test `ds2-invasion-path`'s census makes).
fn is_person(character: usize) -> bool {
    let string = character + ds2_rva::CHARACTER_CTRL_NAME_OFFSET;
    let (Some(length), Some(capacity)) = (
        // SAFETY: fault-safe reads.
        unsafe { safe_read_usize(string + ds2_rva::WSTRING_LEN_OFFSET) },
        // SAFETY: fault-safe reads.
        unsafe { safe_read_usize(string + ds2_rva::WSTRING_CAPACITY_OFFSET) },
    ) else {
        return false;
    };
    if length < NETWORK_PLAYER_PREFIX.len() || length > capacity || length > NAME_LIMIT {
        return false;
    }
    let characters = if capacity > ds2_rva::WSTRING_SSO_MAX {
        match read_ptr(string) {
            Some(pointer) => pointer,
            None => return false,
        }
    } else {
        string
    };
    for (index, want) in NETWORK_PLAYER_PREFIX.encode_utf16().enumerate() {
        // SAFETY: fault-safe read inside the declared length.
        if unsafe { safe_read_u16(characters + index * 2) } != Some(want) {
            return false;
        }
    }
    let Some(block) = read_ptr(character + ds2_rva::CHARACTER_CTRL_PHANTOM_BLOCK_OFFSET) else {
        return false;
    };
    // SAFETY: fault-safe read.
    match unsafe { safe_read_u8(block + ds2_rva::PHANTOM_BLOCK_PHANTOM_PARAM_OFFSET) } {
        Some(param) => !ds2_rva::REPLAY_PHANTOM_PARAM_IDS.contains(&param),
        None => false,
    }
}

/// Most roster entries one check walks, so a torn begin/end pair cannot become a long scan.
const ROSTER_LIMIT: usize = 512;

/// Every other person's weapon and armour records: records 0..5 (packet 61 writes them) and
/// 6..9 (packet 62, the receiver `0x140162150` case `0x3e`, writes them through the same
/// `0x1403463d0` at index `piece + 6`).
fn remotes(local: usize) -> Vec<Remote> {
    let mut out = Vec::new();
    let Some(manager) = read_ptr(GAME_MANAGER.load(Ordering::Acquire)) else {
        return out;
    };
    let Some(characters) = read_ptr(manager + ds2_rva::GAME_MANAGER_CHARACTER_MANAGER_OFFSET)
    else {
        return out;
    };
    let (Some(begin), Some(end)) = (
        read_ptr(characters + ds2_rva::CHARACTER_MANAGER_ENTITY_BEGIN_OFFSET),
        read_ptr(characters + ds2_rva::CHARACTER_MANAGER_ENTITY_END_OFFSET),
    ) else {
        return out;
    };
    let count = end.saturating_sub(begin) / 8;
    let vtable = PLAYER_CTRL_VTABLE.load(Ordering::Acquire);
    for index in 0..count.min(ROSTER_LIMIT) {
        let Some(character) = read_ptr(begin + index * 8) else {
            continue;
        };
        if character == local {
            continue;
        }
        // SAFETY: fault-safe read.
        if unsafe { safe_read_usize(character) } != Some(vtable) || !is_person(character) {
            continue;
        }
        if let Some(records) = records(character) {
            let mut remote = Remote {
                weapons: [(0, 0); 6],
                armor: [(0, 0); 4],
            };
            remote
                .weapons
                .copy_from_slice(&records[..ds2_rva::WEAPON_SLOT_COUNT]);
            remote
                .armor
                .copy_from_slice(&records[ds2_rva::ARMOR_RECORD_FIRST..]);
            out.push(remote);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The clamps.
// ---------------------------------------------------------------------------------------------

/// The weapon update detour: lower the request's level to weapon sync's cap, then run the game's
/// code. Never panics across the boundary.
unsafe extern "system" fn weapon_update_detour(player: usize, request: *mut u8) {
    let _ = std::panic::catch_unwind(|| clamp_request(&WEAPONS, player, request as usize));
    forward(&WEAPONS, player, request);
}

/// The armour update detour, the same for armour sync's cap.
unsafe extern "system" fn armor_update_detour(player: usize, request: *mut u8) {
    let _ = std::panic::catch_unwind(|| clamp_request(&ARMOR, player, request as usize));
    forward(&ARMOR, player, request);
}

fn forward(f: &Feature, player: usize, request: *mut u8) {
    let raw = f.update_original.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: MinHook's trampoline for this exact function and ABI.
        let original: EquipUpdate = unsafe { std::mem::transmute::<usize, EquipUpdate>(raw) };
        // SAFETY: forwarding the game's own arguments.
        unsafe { original(player, request) };
    }
}

/// Only for the local character (the only caller passes it, and this checks), and only for the
/// feature's own slots.
fn clamp_request(f: &Feature, player: usize, request: usize) {
    let Some(cap) = cap_from_atomic(f.current_cap.load(Ordering::Acquire)) else {
        return;
    };
    if request == 0 || local_player() != Some(player) {
        return;
    }
    // SAFETY: fault-safe reads of the request the game passed.
    let (Some(slot), Some(level), Some(item)) = (
        unsafe { safe_read_u32(request + ds2_rva::WEAPON_UPDATE_REQUEST_SLOT_OFFSET) },
        // SAFETY: as above.
        unsafe { safe_read_u8(request + ds2_rva::WEAPON_UPDATE_REQUEST_LEVEL_OFFSET) },
        // SAFETY: as above.
        unsafe { safe_read_u32(request + ds2_rva::WEAPON_UPDATE_REQUEST_ITEM_OFFSET) },
    ) else {
        return;
    };
    if slot as usize >= f.kind.slots() {
        return;
    }
    let lowered = policy::clamp(level, Some(cap));
    if lowered == level {
        return;
    }
    // SAFETY: `request` is the game's own request block (on its caller's stack, or on ours for a
    // push), just read successfully at this offset. It is the argument this call consumes, not
    // game state that outlives it.
    unsafe { *((request + ds2_rva::WEAPON_UPDATE_REQUEST_LEVEL_OFFSET) as *mut u8) = lowered };
    log(format_args!(
        "{} clamped the game's own update slot={slot} item={item} +{level} -> +{lowered}",
        f.prefix
    ));
}

// ---------------------------------------------------------------------------------------------
// The save.
// ---------------------------------------------------------------------------------------------

/// How many times the inventory save writer has run, for the log.
static SAVES: AtomicU32 = AtomicU32::new(0);

/// The inventory save writer's detour: before the game streams the save block, every record
/// either feature lowered gets its real level back, so no save ever keeps a lowered level. The
/// entries the pause menu and the equip path read stay lowered: the block is a separate copy, and
/// it is the only thing the writer streams.
unsafe extern "system" fn save_write_detour(this: usize, stream: usize, enabled: u32) -> usize {
    if enabled != 0 {
        let _ = std::panic::catch_unwind(before_save);
    }
    let raw = SAVE_WRITE_ORIGINAL.load(Ordering::Acquire);
    if raw == 0 {
        return 0;
    }
    // SAFETY: MinHook's trampoline for this exact function and ABI.
    let original: SaveWrite = unsafe { std::mem::transmute::<usize, SaveWrite>(raw) };
    // SAFETY: forwarding the game's own arguments.
    unsafe { original(this, stream, enabled) }
}

fn before_save() {
    let save = SAVES.fetch_add(1, Ordering::Relaxed) + 1;
    let manager = inventory_manager();
    for f in FEATURES.iter().filter(|f| f.installed()) {
        let ledger = f.ledger.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(manager) = manager else {
            log(format_args!(
                "{} save #{save}: the inventory is being written but its manager is unreadable; \
                 lowered={}",
                f.prefix,
                ledger.len()
            ));
            continue;
        };
        let (fixed, already) = fix_save_block(manager, &ledger);
        log(format_args!(
            "{} save #{save}: the inventory is being written with lowered={} {}; their save \
             records carry the real level (fixed={fixed} already-real={already})",
            f.prefix,
            ledger.len(),
            f.things
        ));
    }
}

// ---------------------------------------------------------------------------------------------
// The whole inventory, and the equipped slots.
// ---------------------------------------------------------------------------------------------

/// One pass of a feature's ledger over its items in the inventory at `cap`, and the writes it
/// asks for. A restore also puts the real level in that entry's save record, in case the game
/// copied the lowered one there while it was lowered.
fn sweep_inventory(f: &Feature, cap: Option<u8>) {
    let (Some(manager), Some(bag)) = (inventory_manager(), bag()) else {
        return;
    };
    let Some(held) = held(bag, f.kind) else {
        return;
    };
    let mut ledger = f.ledger.lock().unwrap_or_else(|poison| poison.into_inner());
    if cap.is_none() && ledger.is_empty() {
        return;
    }
    let writes = ledger.sweep(&held, cap);
    if writes.is_empty() {
        return;
    }
    let (mut lowered, mut restored, mut failed) = (0usize, 0usize, 0usize);
    let mut sample = Vec::new();
    for write in &writes {
        let entry = entry_address(bag, write.index);
        let wrote = write_level(
            entry + ds2_rva::ITEM_ENTRY_ITEM_ID_OFFSET,
            entry + ds2_rva::ITEM_ENTRY_LEVEL_OFFSET,
            write.item,
            Some(write.from),
            write.to,
        );
        if !wrote {
            failed += 1;
            continue;
        }
        if write.restores() {
            restored += 1;
            let (item_at, level_at) = save_record(manager, write.index);
            write_level(item_at, level_at, write.item, None, write.to);
        } else {
            lowered += 1;
        }
        if sample.len() < SAMPLE {
            sample.push(format!(
                "#{}:{}+{}->+{}",
                write.index, write.item, write.from, write.to
            ));
        }
    }
    log(format_args!(
        "{} inventory cap={} changed={} lowered={lowered} restored={restored} failed={failed} \
         {}={} still-lowered={} [{}{}]",
        f.prefix,
        show(cap),
        lowered + restored,
        f.things.replace(' ', "-"),
        held.len(),
        ledger.len(),
        sample.join(" "),
        if writes.len() > SAMPLE { " ..." } else { "" }
    ));
}

/// How many of a feature's items the inventory holds at each level, `+0:12 +10:40`, so two loads
/// of the same character can be compared: before a capped session and after it has saved and
/// reloaded.
fn level_census(bag: usize, kind: Kind) -> String {
    let Some(held) = held(bag, kind) else {
        return "unreadable".to_string();
    };
    let mut counts = [0usize; 16];
    for item in &held {
        counts[usize::from(item.level & 0x0f)] += 1;
    }
    let parts: Vec<String> = counts
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(level, count)| format!("+{level}:{count}"))
        .collect();
    format!("total={} [{}]", held.len(), parts.join(" "))
}

/// How many entries an inventory line names before it says "...".
const SAMPLE: usize = 8;

/// A feature's equipped slots in inventory slot order: the inventory entry, and the two copies
/// the character carries (the record's id given back as the item id it came from).
fn equipped(f: &Feature, bag: usize, player: usize) -> Equipped {
    let records = records(player);
    let live = live_levels(f, player);
    (0..f.kind.slots())
        .map(|slot| Slot {
            inventory: inventory_item(bag, f.inventory_slot(slot))
                .map(|item| (item.item, item.level)),
            record: records.map(|records| {
                let (id, level) = records[f.record_index(slot)];
                (f.record_item(id), level & 0x0f)
            }),
            live: live.as_ref().map(|live| live[slot]),
        })
        .collect()
}

fn show_slots(slots: &[Slot]) -> String {
    slots
        .iter()
        .enumerate()
        .map(|(slot, s)| {
            let inventory = s.inventory.map_or_else(
                || "-".to_string(),
                |(item, level)| format!("{item}+{level}"),
            );
            let record = s.record.map_or_else(
                || "?".to_string(),
                |(item, level)| format!("{item}+{level}"),
            );
            let live = s
                .live
                .map_or_else(|| "?".to_string(), |level| format!("+{level}"));
            format!("s{slot}:{inventory}/rec {record}/live {live}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------------------------
// The tick, and the push.
// ---------------------------------------------------------------------------------------------

/// How often [`poll_keys`] reads the keyboard: a little faster than a frame, so no press is missed.
const KEY_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// Read both features' keys on a thread of our own and hand fresh presses to the tick through
/// each feature's `pressed`.
///
/// Not on the game thread. The first build read `GetAsyncKeyState` inside the net session update
/// tick, and the one Frida attach that hooked `GetAsyncKeyState` in that build froze the game
/// before it reported attached. Whether the call site caused that is not proven. The game thread
/// has no need to call into user32 for a key either way.
fn poll_keys() {
    loop {
        std::thread::sleep(KEY_POLL);
        let focus = game_has_focus();
        for f in FEATURES.iter().filter(|f| f.installed()) {
            let down = focus && f.key.load().is_some_and(chord_down);
            if !f.was_down.swap(down, Ordering::Relaxed) && down {
                f.pressed.store(true, Ordering::Release);
            }
        }
    }
}

/// A press handed over by [`poll_keys`]: flip the feature, say so, and show it.
fn toggle(f: &Feature) {
    let on = !f.enabled.fetch_xor(true, Ordering::AcqRel);
    let key = f.key_name();
    if on {
        log(format_args!(
            "{} TOGGLED ON by {key} -- {} are capped again whenever another player is in the world",
            f.prefix, f.things
        ));
    } else {
        log(format_args!(
            "{} TOGGLED OFF by {key} -- real {} levels are restored now and nothing is capped \
             until {key} is pressed again",
            f.prefix, f.things
        ));
    }
    let spoken = announce(f.kind, on);
    log(format_args!(
        "{} announce {} played={spoken} ({})",
        f.prefix,
        if on { "on" } else { "off" },
        f.name
    ));
    crate::hud::toggled(f.kind, on);
}

/// Registered with `ds2-net-tick` to run after the net session update, every frame on the game
/// thread. Whatever the update holds, it has let go of by now.
///
/// A press flips its feature and forces a check in the same frame, so turning it off restores at
/// once and turning it on caps at once. The other feature is checked on its own schedule.
fn tick(_session: usize) {
    let mut forced = [false; 2];
    for (index, f) in FEATURES.iter().enumerate() {
        if f.installed() && f.pressed.swap(false, Ordering::AcqRel) {
            toggle(f);
            forced[index] = true;
        }
    }
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    let scheduled = frame.is_multiple_of(CHECK_EVERY_FRAMES);
    if !scheduled && !forced.contains(&true) {
        return;
    }
    let local = local_player().unwrap_or(0);
    let remotes = if local == 0 {
        Vec::new()
    } else {
        remotes(local)
    };
    for (index, f) in FEATURES.iter().enumerate() {
        if f.installed() && (scheduled || forced[index]) {
            check(f, local, &remotes);
        }
    }
}

fn check(f: &Feature, local: usize, remotes: &[Remote]) {
    if f.last_player.swap(local, Ordering::AcqRel) != local
        && local != 0
        && let Some(bag) = bag()
    {
        log(format_args!(
            "{} world player=0x{local:x} built with: {} inventory {} by level: {}",
            f.prefix,
            copies(f, bag, local),
            f.things,
            level_census(bag, f.kind)
        ));
    }
    let test_cap = cap_from_atomic(f.test_cap.load(Ordering::Acquire));
    let enabled = f.enabled.load(Ordering::Acquire);
    let cap = policy::effective(enabled, policy::cap_for(f.kind, remotes, test_cap));
    if !f.first_tick.swap(true, Ordering::AcqRel) {
        log(format_args!(
            "{} tick live player=0x{local:x} people={} cap={}",
            f.prefix,
            remotes.len(),
            show(cap)
        ));
    }
    let (action, previous) = {
        let mut tracker = f.tracker.lock().unwrap_or_else(|poison| poison.into_inner());
        let previous = tracker.applied();
        (tracker.step(local, cap), previous)
    };
    let cap = if local == 0 { None } else { cap };
    f.current_cap.store(cap_to_atomic(cap), Ordering::Release);
    // Every item of this feature in the inventory first, so whatever is equipped next, by
    // anything, is already at the cap, and the push below reads capped entries.
    sweep_inventory(f, cap);
    let redrive = matches!(action, Action::Redrive { .. });
    if local != 0
        && !redrive
        && let Some(bag) = bag()
    {
        resweep_if_changed(f, bag, local, cap);
    }
    if let Action::Redrive { cap } = action {
        let highest: Vec<String> = remotes
            .iter()
            .map(|remote| {
                show(match f.kind {
                    Kind::Weapon => policy::remote_highest(&remote.weapons),
                    Kind::Armor => policy::remote_armor_highest(&remote.armor, &remote.weapons),
                })
            })
            .collect();
        log(format_args!(
            "{} cap {} -> {} enabled={enabled} people={} their-highest=[{}] test_cap={} \
             player=0x{local:x}",
            f.prefix,
            show(previous),
            show(cap),
            remotes.len(),
            highest.join(","),
            show(test_cap)
        ));
        push(f, local, cap);
    }
}

/// The cap moved: push every equipped slot, log all three copies of every slot, and remember the
/// slots as settled so the watch asks again only when they change.
fn push(f: &Feature, player: usize, cap: Option<u8>) {
    let Some(bag) = bag() else {
        log(format_args!("{} push skipped: no inventory bag", f.prefix));
        return;
    };
    let pushed = push_slots(f, bag, player, cap);
    let verb = if cap.is_some() { "CAPPED" } else { "RESTORED" };
    log(format_args!(
        "{} {verb} cap={} pushed=[{}] after: {}",
        f.prefix,
        show(cap),
        pushed.join(" "),
        copies(f, bag, player)
    ));
    let mut watch = f.watch.lock().unwrap_or_else(|poison| poison.into_inner());
    if cap.is_some() {
        watch.settle(equipped(f, bag, player));
    } else {
        let _ = watch.changed(&equipped(f, bag, player), None);
    }
}

/// The equipped slots changed while a cap is on, for whatever reason: resweep all of them, and
/// log one line with the slots before, now, what was pushed and what the character carries after.
fn resweep_if_changed(f: &Feature, bag: usize, player: usize, cap: Option<u8>) {
    let now = equipped(f, bag, player);
    let mut watch = f.watch.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(before) = watch.changed(&now, cap) else {
        return;
    };
    let slots = policy::resweep(&now);
    let pushed = if slots.is_empty() {
        Vec::new()
    } else {
        push_slots(f, bag, player, cap)
    };
    let after = equipped(f, bag, player);
    let limit = cap.unwrap_or(f.kind.level_max());
    let over = after.iter().any(|slot| {
        slot.inventory.is_some()
            && (slot.record.is_some_and(|(_, level)| level > limit)
                || slot.live.is_some_and(|level| level > limit))
    });
    log(format_args!(
        "{} equipped changed under cap={}: before=[{}] now=[{}] resweep slots={slots:?} \
         pushed=[{}] after=[{}] all-within-cap={}",
        f.prefix,
        show(cap),
        before.map_or_else(|| "first look".to_string(), |before| show_slots(&before)),
        show_slots(&now),
        pushed.join(" "),
        show_slots(&after),
        !over
    ));
    watch.settle(after);
}

/// Push every equipped slot whose character copies differ from `clamp(inventory level, cap)`
/// through the game's own update for this feature, all slots checked. Answers what was pushed.
fn push_slots(f: &Feature, bag: usize, player: usize, cap: Option<u8>) -> Vec<String> {
    let raw = f.update_original.load(Ordering::Acquire);
    if raw == 0 {
        return Vec::new();
    }
    // SAFETY: MinHook's trampoline for this exact function and ABI. Calling the trampoline rather
    // than the patched entry runs the game's code only; the clamp has already been applied to the
    // request by `policy::clamp` below.
    let update: EquipUpdate = unsafe { std::mem::transmute::<usize, EquipUpdate>(raw) };
    let now = equipped(f, bag, player);
    let mut pushed = Vec::new();
    for slot in 0..f.kind.slots() {
        let Some(item) = inventory_item(bag, f.inventory_slot(slot)) else {
            continue;
        };
        let level = policy::clamp(item.level, cap);
        if now[slot].record == Some((item.item, level)) && now[slot].live == Some(level) {
            continue;
        }
        // Two calls, not one, and the first is what makes the second land. The record table's
        // writer (0x1403463d0) keeps a new record only when the item id or the u16 at +0x0C
        // differs from the old one, and the level sits in the byte after that u16: a level-only
        // change with the same item is computed and thrown away. Measured for weapons: one push
        // left the record at +10 while the live state took +3; the armour update writes through
        // the same function. The packet 61/62 receiver on every peer writes through it too, so a
        // peer would keep our old level as well. A placeholder first (Fists, or no armour piece,
        // the value the game itself sends for an empty slot) changes the id; the real item at the
        // new level then lands, here and on every peer.
        for (id, item_level, infusion) in [
            (f.placeholder(), 0, 0),
            (f.request_item(item.item), level, item.infusion),
        ] {
            let mut request = Request([0u8; ds2_rva::WEAPON_UPDATE_REQUEST_SIZE]);
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_SLOT_OFFSET..][..4]
                .copy_from_slice(&f.request_slot(slot).to_le_bytes());
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_ITEM_OFFSET..][..4]
                .copy_from_slice(&id.to_le_bytes());
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_DURABILITY_OFFSET..][..4]
                .copy_from_slice(&item.durability_bits.to_le_bytes());
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_LEVEL_OFFSET] = item_level;
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_INFUSION_OFFSET] = infusion;
            // SAFETY: game thread, after the net session update returned; `player` is the local
            // PlayerCtrl read this frame, and the request is laid out exactly as `0x1401b66a0`
            // lays out the one it passes (ds2-rva CHR_WEAPON_UPDATE, CHR_ARMOR_UPDATE).
            unsafe { update(player, request.0.as_mut_ptr()) };
        }
        let real = f
            .ledger
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .real(item.handle, item.item)
            .unwrap_or(item.level);
        pushed.push(format!("s{slot}:{}+{real}->+{level}", item.item));
    }
    pushed
}

/// All three copies of every slot's level, inventory slot order.
fn copies(f: &Feature, bag: usize, player: usize) -> String {
    let slots = equipped(f, bag, player);
    let column = |pick: &dyn Fn(&Slot) -> Option<u8>| {
        slots
            .iter()
            .map(|slot| pick(slot).map_or_else(|| "-".to_string(), |level| format!("+{level}")))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "inventory=[{}] records=[{}] live=[{}] (inventory slot order)",
        column(&|slot| slot.inventory.map(|(_, level)| level)),
        column(&|slot| slot.record.map(|(_, level)| level)),
        column(&|slot| slot.live),
    )
}

/// The request block, aligned the way the game's stack copy is.
#[repr(C, align(8))]
struct Request([u8; ds2_rva::WEAPON_UPDATE_REQUEST_SIZE]);
