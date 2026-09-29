//! The two detours: the clamp on the game's weapon update, and the per-frame check on the net
//! session update that decides when to push our weapons through it again.

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

use crate::LOG_PREFIX;

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

fn key_name() -> String {
    KEY_BINDING
        .load()
        .filter(|chord| chord.vk != 0)
        .map_or_else(|| "no key".to_string(), chord_name)
}

/// Bind the on/off key, or unbind it with `None`. Takes effect on the next frame.
pub fn set_key(chord: Option<Chord>) {
    let before = key_name();
    match chord {
        Some(chord) => KEY_BINDING.store(chord),
        None => KEY_BINDING.store(Chord {
            modifiers: 0,
            vk: 0,
            dik: None,
        }),
    }
    let after = key_name();
    if before != after {
        log(format_args!("{LOG_PREFIX} key {before} -> {after}"));
    }
}
use crate::policy::{self, Action, Equipped, Held, Ledger, RemoteWeapons, Slot, Tracker, Watch};

/// `CHR_WEAPON_UPDATE(PlayerCtrl*, WeaponUpdateRequest*)`.
type WeaponUpdate = unsafe extern "system" fn(usize, *mut u8);

/// `SAVE_DATA_ITEM_INVENTORY_WRITE(this, stream*, enabled)`.
type SaveWrite = unsafe extern "system" fn(usize, usize, u32) -> usize;

/// Trampoline back to the real inventory save writer.
static SAVE_WRITE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Every inventory weapon the cap lowered, with its real level. The tick sweeps it and the save
/// writer reads it, so it is behind a lock; neither holds it across a call into the game.
static LEDGER: Mutex<Ledger> = Mutex::new(Ledger::new());

/// The equipped slots as the last resweep left them. Game thread only.
static WATCH: Mutex<Watch> = Mutex::new(Watch::new());

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

/// Trampoline back to the real weapon update. Also what the push calls, so the push runs the
/// game's code and not our detour's clamp twice.
static WEAPON_UPDATE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Whether the feature is on. Flipped by the key; starts on, because `[weapon_sync] enabled`
/// already asked for it.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// The key that flips [`ENABLED`]. Unset means unbound.
static KEY_BINDING: AtomicChord = AtomicChord::unset();

/// Whether the key was down at the last poll, so a hold is one press.
static WAS_DOWN: AtomicBool = AtomicBool::new(false);

/// A press the key thread saw and the game-thread tick has not acted on yet.
static PRESSED: AtomicBool = AtomicBool::new(false);

/// `GameManagerImp`'s address (the global that holds the pointer), resolved at install.
static GAME_MANAGER: AtomicUsize = AtomicUsize::new(0);

/// `PlayerCtrl`'s vtable, resolved at install. A roster entry is a player only if it has this.
static PLAYER_CTRL_VTABLE: AtomicUsize = AtomicUsize::new(0);

/// The cap the clamp applies: [`NO_CAP`], or a level `0..=10`.
static CURRENT_CAP: AtomicU32 = AtomicU32::new(NO_CAP);
const NO_CAP: u32 = u32::MAX;

/// A remote player at this level, for testing solo. [`NO_CAP`] when not configured.
static TEST_CAP: AtomicU32 = AtomicU32::new(NO_CAP);

/// The local `PlayerCtrl` the last check saw, so a new world gets one line of what it was built
/// with. That line is the evidence that a load came back with real levels.
static LAST_PLAYER: AtomicUsize = AtomicUsize::new(0);

/// Frames since the last check, and the first-tick flag.
static FRAME: AtomicU32 = AtomicU32::new(0);
static FIRST_TICK: AtomicBool = AtomicBool::new(false);

/// How often the roster is read. Four times a second is well inside how long a peer's packet 61
/// takes to matter and keeps the fault-safe reads off every frame.
const CHECK_EVERY_FRAMES: u32 = 15;

/// The tracker. Only the game thread touches it (the tick detour), so the lock is uncontended.
static TRACKER: Mutex<Tracker> = Mutex::new(Tracker::new());

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

/// Whether the feature is on right now, for the HUD.
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const c_void, module: *mut c_void, flags: u32) -> i32;
}

/// `SND_ASYNC | SND_NODEFAULT | SND_MEMORY`, the flags `ds2-voice-chat` plays its clips with:
/// return at once, never fall back to the system beep, and read the WAV from the pointer. A new
/// clip cuts off one still playing.
const PLAY_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004;

/// Say which way the key just went. `true` when `winmm` accepted the clip.
fn announce(on: bool) -> bool {
    let clip = crate::clip(on);
    // SAFETY: `clip` is a `'static` WAV compiled into this DLL, so it outlives the async play.
    unsafe { PlaySoundW(clip.as_ptr().cast(), core::ptr::null_mut(), PLAY_FLAGS) != 0 }
}

/// What [`install`] managed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Whether both detours are live. When this is `false` nothing was patched.
    pub installed: bool,
}

/// Change the pretend remote player's level while the game runs, or remove it with `None`.
///
/// The next check (within a quarter second) picks it up exactly as it would a real player
/// arriving, changing weapons or leaving, so removing it exercises the in-world restore.
pub fn set_test_cap(test_cap: Option<u8>) {
    let previous = TEST_CAP.swap(cap_to_atomic(test_cap), Ordering::AcqRel);
    if previous != cap_to_atomic(test_cap) {
        log(format_args!(
            "{LOG_PREFIX} test_cap {} -> {}",
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
fn checked_site(rva: u32, expected: &[u8], name: &str) -> Option<usize> {
    let site = match game_rva(rva) {
        Ok(site) => site,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} not installed reason=no-module-base what={name} -- {error}"
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
            "{LOG_PREFIX} not installed reason=prologue what={name} va=0x{site:016x} read={read} \
             saw={found:02x?} want={expected:02x?} -- that address is not {name} on this build, so \
             nothing was patched"
        ));
        return None;
    }
    Some(site)
}

/// Hook both sites. Nothing is patched unless both prologues match and both hooks are created.
///
/// `test_cap` stands in for a remote player at that level, so the cap can be seen working with
/// nobody else in the world. `key` turns the feature on and off in game; `None` leaves it unbound.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Call once, from the loader's post-Arxan
/// install position.
pub unsafe fn install(test_cap: Option<u8>, key: Option<Chord>) -> Outcome {
    if let Some(chord) = key {
        KEY_BINDING.store(chord);
    }
    let refused = Outcome { installed: false };
    let Some(weapon_site) = checked_site(
        ds2_rva::CHR_WEAPON_UPDATE,
        &ds2_rva::CHR_WEAPON_UPDATE_PROLOGUE,
        "CHR_WEAPON_UPDATE",
    ) else {
        return refused;
    };
    // The cap writes the inventory, so it is never installed without the hook that keeps the real
    // levels in the save.
    let Some(save_site) = checked_site(
        ds2_rva::SAVE_DATA_ITEM_INVENTORY_WRITE,
        &ds2_rva::SAVE_DATA_ITEM_INVENTORY_WRITE_PROLOGUE,
        "SAVE_DATA_ITEM_INVENTORY_WRITE",
    ) else {
        return refused;
    };
    let (Ok(manager), Ok(vtable)) = (
        game_rva(ds2_rva::GAME_MANAGER_IMP),
        game_rva(ds2_rva::PLAYER_CTRL_VTABLE),
    ) else {
        log(format_args!(
            "{LOG_PREFIX} not installed reason=no-module-base"
        ));
        return refused;
    };
    GAME_MANAGER.store(manager, Ordering::Release);
    PLAYER_CTRL_VTABLE.store(vtable, Ordering::Release);
    TEST_CAP.store(cap_to_atomic(test_cap), Ordering::Release);

    // SAFETY: MinHook's own initialiser, no arguments; ALREADY_INITIALIZED means another crate in
    // this DLL got there first.
    let status = unsafe { MH_Initialize() };
    if status != MH_STATUS::MH_OK && status != MH_STATUS::MH_ERROR_ALREADY_INITIALIZED {
        log(format_args!(
            "{LOG_PREFIX} not installed: MH_Initialize said {status:?}"
        ));
        return refused;
    }
    // The save fix goes in first: until the tick lowers something it changes nothing, and nothing
    // may be lowered before it is live.
    // SAFETY: the site matched its recorded prologue, and the detour has the same ABI.
    let save = match unsafe {
        MhHook::new(save_site as *mut c_void, save_write_detour as *mut c_void)
    } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} not installed: MH_CreateHook on SAVE_DATA_ITEM_INVENTORY_WRITE said \
                 {status:?}"
            ));
            return refused;
        }
    };
    SAVE_WRITE_ORIGINAL.store(save.trampoline() as usize, Ordering::Release);
    // SAFETY: an address `MhHook::new` accepted above.
    let status = unsafe { MH_EnableHook(save_site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} not installed: MH_EnableHook on SAVE_DATA_ITEM_INVENTORY_WRITE said \
             {status:?}"
        ));
        return refused;
    }
    // The clamp goes in next and the tick last. The clamp does nothing until the tick has set a
    // cap, so if the tick cannot register, the weapon update runs exactly as the game's own.
    // SAFETY: the site matched its recorded prologue, and the detour has the same ABI.
    let weapon = match unsafe {
        MhHook::new(
            weapon_site as *mut c_void,
            weapon_update_detour as *mut c_void,
        )
    } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} not installed: MH_CreateHook on CHR_WEAPON_UPDATE said {status:?}"
            ));
            return refused;
        }
    };
    // Published before the site is patched, so a detour that fires at once has somewhere to go.
    WEAPON_UPDATE_ORIGINAL.store(weapon.trampoline() as usize, Ordering::Release);
    // SAFETY: an address `MhHook::new` accepted above.
    let status = unsafe { MH_EnableHook(weapon_site as *mut c_void) };
    if status != MH_STATUS::MH_OK {
        log(format_args!(
            "{LOG_PREFIX} not installed: MH_EnableHook on CHR_WEAPON_UPDATE said {status:?}"
        ));
        return refused;
    }
    // The net session update is shared with `ds2-voice-chat`; `ds2-net-tick` owns its one detour
    // and runs this after the original.
    // SAFETY: the loader's post-Arxan install position, which is this function's own contract.
    let tick_site = match unsafe { ds2_net_tick::register(ds2_net_tick::When::After, tick) } {
        Ok(site) => site,
        Err(error) => {
            log(format_args!(
                "{LOG_PREFIX} not installed: no tick -- {error}. The clamp is in but never has a \
                 cap, so weapon levels are never changed"
            ));
            return refused;
        }
    };
    std::thread::spawn(poll_key);
    let key = key_name();
    log(format_args!(
        "{LOG_PREFIX} installed weapon-update=0x{weapon_site:016x} \
         save-write=0x{save_site:016x} tick=0x{tick_site:016x} (shared) key={key} test_cap={} -- \
         while another player is in the world, every weapon in the inventory above their highest \
         weapon level is lowered to it, equipped or not; the save always gets the real levels",
        show(test_cap)
    ));
    // After the detours, and never a reason to refuse: the swords only show the switch.
    crate::hud::install();
    Outcome { installed: true }
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

/// Every weapon and shield in the bag's entry array, pack and box alike, or `None` when the array
/// could not be read in one piece. A partial read is never answered, because the ledger takes an
/// entry missing from the answer to be gone.
///
/// An entry counts when its type carries an infusion (weapons and shields), its item id is set,
/// and its handle is its own index, which is what makes the index the save block's index too.
fn held_weapons(bag: usize) -> Option<Vec<Held>> {
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
        let kind = entry[ds2_rva::ITEM_ENTRY_TYPE_OFFSET];
        let Ok(index) = u16::try_from(index) else {
            break;
        };
        if item == 0 || item == u32::MAX || handle != index || kind >= ITEM_TYPE_WEAPON_BELOW {
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

/// Item types below this are weapons and shields: the types that carry an infusion.
const ITEM_TYPE_WEAPON_BELOW: u8 = ds2_rva::ITEM_TYPE_HAS_INFUSION_BELOW;

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

/// One of our weapons as the inventory holds it: the real level.
#[derive(Clone, Copy, Debug)]
struct InventoryWeapon {
    item: u32,
    /// The entry's handle, which is its index in the bag and the ledger's key.
    handle: u16,
    durability_bits: u32,
    level: u8,
    infusion: u8,
}

/// Inventory weapon slot `slot`, read the way `0x1401b66a0` reads it. `None` for an empty slot.
fn inventory_weapon(bag: usize, slot: usize) -> Option<InventoryWeapon> {
    let entry = read_ptr(bag + ds2_rva::ITEM_BAG_EQUIPPED_ENTRIES_OFFSET + slot * 8)?;
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
    Some(InventoryWeapon {
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

/// A character's six weapon records, `(item id, level byte)`, indexed by character-side slot.
fn weapon_records(character: usize) -> Option<RemoteWeapons> {
    let asm = read_ptr(character + ds2_rva::CHARACTER_CTRL_CHR_ASM_CTRL_OFFSET)?;
    let table = read_ptr(asm + ds2_rva::CHR_ASM_CTRL_RECORD_TABLE_OFFSET)?;
    let mut raw = [0u8; ds2_rva::WEAPON_SLOT_COUNT * ds2_rva::EQUIP_RECORD_STRIDE];
    // SAFETY: one fault-safe bulk read; nothing is interpreted unless it all came back.
    if !unsafe { read_bytes(table + ds2_rva::EQUIP_RECORD_ARRAY_OFFSET, &mut raw) } {
        return None;
    }
    let mut out = [(0u32, 0u8); 6];
    for (slot, record) in out.iter_mut().enumerate() {
        let base = slot * ds2_rva::EQUIP_RECORD_STRIDE;
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

/// A character's live weapon levels, `ChrAsmEquip +0x70` per weapon entry.
fn live_levels(character: usize) -> Option<[u8; 6]> {
    let asm = read_ptr(character + ds2_rva::CHARACTER_CTRL_CHR_ASM_CTRL_OFFSET)?;
    let equip = read_ptr(asm + ds2_rva::CHR_ASM_CTRL_EQUIP_OFFSET)?;
    let mut out = [0u8; 6];
    for (n, level) in out.iter_mut().enumerate() {
        // SAFETY: fault-safe read.
        *level = unsafe {
            safe_read_u8(
                equip
                    + n * ds2_rva::CHR_ASM_EQUIP_WEAPON_STRIDE
                    + ds2_rva::CHR_ASM_EQUIP_WEAPON_LEVEL_OFFSET,
            )?
        };
    }
    Some(out)
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

/// Every other person's weapon records.
fn remotes(local: usize) -> Vec<RemoteWeapons> {
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
        if let Some(records) = weapon_records(character) {
            out.push(records);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The clamp.
// ---------------------------------------------------------------------------------------------

/// The weapon update detour: lower the request's level to the cap, then run the game's code.
///
/// Only for the local character (the only caller passes it, and this checks), and only for the
/// six weapon slots. Never panics across the boundary.
unsafe extern "system" fn weapon_update_detour(player: usize, request: *mut u8) {
    let _ = std::panic::catch_unwind(|| clamp_request(player, request as usize));
    let raw = WEAPON_UPDATE_ORIGINAL.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: MinHook's trampoline for this exact function and ABI.
        let original: WeaponUpdate = unsafe { std::mem::transmute::<usize, WeaponUpdate>(raw) };
        // SAFETY: forwarding the game's own arguments.
        unsafe { original(player, request) };
    }
}

fn clamp_request(player: usize, request: usize) {
    let Some(cap) = cap_from_atomic(CURRENT_CAP.load(Ordering::Acquire)) else {
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
    if slot as usize >= ds2_rva::WEAPON_SLOT_COUNT {
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
        "{LOG_PREFIX} clamped the game's own weapon update slot={slot} item={item} +{level} -> \
         +{lowered}"
    ));
}

// ---------------------------------------------------------------------------------------------
// The save.
// ---------------------------------------------------------------------------------------------

/// How many times the inventory save writer has run, for the log.
static SAVES: AtomicU32 = AtomicU32::new(0);

/// The inventory save writer's detour: before the game streams the save block, every record the
/// cap lowered gets its real level back, so no save ever keeps a lowered level. The entries the
/// pause menu and the equip path read stay lowered: the block is a separate copy, and it is the
/// only thing the writer streams.
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
    let ledger = LEDGER.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(manager) = inventory_manager() else {
        log(format_args!(
            "{LOG_PREFIX} save #{save}: the inventory is being written but its manager is \
             unreadable; lowered={}",
            ledger.len()
        ));
        return;
    };
    let (fixed, already) = fix_save_block(manager, &ledger);
    log(format_args!(
        "{LOG_PREFIX} save #{save}: the inventory is being written with lowered={} weapons; their \
         save records carry the real level (fixed={fixed} already-real={already})",
        ledger.len()
    ));
}

// ---------------------------------------------------------------------------------------------
// The whole inventory, and the equipped slots.
// ---------------------------------------------------------------------------------------------

/// One pass of the ledger over every weapon in the inventory at `cap`, and the writes it asks
/// for. A restore also puts the real level in that entry's save record, in case the game copied
/// the lowered one there while it was lowered.
fn sweep_inventory(cap: Option<u8>) {
    let (Some(manager), Some(bag)) = (inventory_manager(), bag()) else {
        return;
    };
    let Some(held) = held_weapons(bag) else {
        return;
    };
    let mut ledger = LEDGER.lock().unwrap_or_else(|poison| poison.into_inner());
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
        "{LOG_PREFIX} inventory cap={} changed={} lowered={lowered} restored={restored} \
         failed={failed} weapons={} still-lowered={} [{}{}]",
        show(cap),
        lowered + restored,
        held.len(),
        ledger.len(),
        sample.join(" "),
        if writes.len() > SAMPLE { " ..." } else { "" }
    ));
}

/// How many weapons the inventory holds at each level, `+0:12 +10:40`, so two loads of the same
/// character can be compared: before a capped session and after it has saved and reloaded.
fn level_census(bag: usize) -> String {
    let Some(held) = held_weapons(bag) else {
        return "unreadable".to_string();
    };
    let mut counts = [0usize; 16];
    for weapon in &held {
        counts[usize::from(weapon.level & 0x0f)] += 1;
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

/// The six equipped slots in inventory slot order: the inventory entry, and the two copies the
/// character carries.
fn equipped(bag: usize, player: usize) -> Equipped {
    let records = weapon_records(player);
    let live = live_levels(player);
    let mut out = [Slot::default(); 6];
    for (slot, out) in out.iter_mut().enumerate() {
        let chr_slot = ds2_rva::WEAPON_INTERNAL_TO_CHR_SLOT[slot] as usize;
        out.inventory = inventory_weapon(bag, slot).map(|weapon| (weapon.item, weapon.level));
        out.record = records.map(|records| (records[chr_slot].0, records[chr_slot].1 & 0x0f));
        out.live = live.map(|live| live[live_index(chr_slot)]);
    }
    out
}

fn show_slots(slots: &Equipped) -> String {
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

/// How often [`poll_key`] reads the keyboard: a little faster than a frame, so no press is missed.
const KEY_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// Read the key on a thread of our own and hand fresh presses to the tick through [`PRESSED`].
///
/// Not on the game thread. The first build read `GetAsyncKeyState` inside the net session update
/// tick, and the one Frida attach that hooked `GetAsyncKeyState` in that build froze the game
/// before it reported attached. Whether the call site caused that is not proven. The game thread
/// has no need to call into user32 for a key either way.
fn poll_key() {
    loop {
        std::thread::sleep(KEY_POLL);
        let down = game_has_focus() && KEY_BINDING.load().is_some_and(chord_down);
        if !WAS_DOWN.swap(down, Ordering::Relaxed) && down {
            PRESSED.store(true, Ordering::Release);
        }
    }
}

/// Registered with `ds2-net-tick` to run after the net session update, every frame on the game
/// thread. Whatever the update holds, it has let go of by now.
///
/// A press handed over by [`poll_key`] flips the feature and forces a check in the same frame, so
/// turning it off restores at once and turning it on caps at once.
fn tick(_session: usize) {
    let pressed = PRESSED.swap(false, Ordering::AcqRel);
    if pressed {
        let on = !ENABLED.fetch_xor(true, Ordering::AcqRel);
        let key = key_name();
        if on {
            log(format_args!(
                "{LOG_PREFIX} TOGGLED ON by {key} -- weapons are capped again whenever another \
                 player is in the world"
            ));
        } else {
            log(format_args!(
                "{LOG_PREFIX} TOGGLED OFF by {key} -- real weapon levels are restored now and \
                 nothing is capped until {key} is pressed again"
            ));
        }
        let spoken = announce(on);
        log(format_args!(
            "{LOG_PREFIX} announce {} played={spoken}",
            if on { "on" } else { "off" }
        ));
        crate::hud::toggled(on);
    }
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    if pressed || frame.is_multiple_of(CHECK_EVERY_FRAMES) {
        check();
    }
}

fn check() {
    let local = local_player().unwrap_or(0);
    if LAST_PLAYER.swap(local, Ordering::AcqRel) != local
        && local != 0
        && let Some(bag) = bag()
    {
        log(format_args!(
            "{LOG_PREFIX} world player=0x{local:x} built with: {} inventory weapons by level: {}",
            copies(bag, local),
            level_census(bag)
        ));
    }
    let remotes = if local == 0 {
        Vec::new()
    } else {
        remotes(local)
    };
    let test_cap = cap_from_atomic(TEST_CAP.load(Ordering::Acquire));
    let enabled = ENABLED.load(Ordering::Acquire);
    let cap = policy::effective(enabled, policy::cap(&remotes, test_cap));
    if !FIRST_TICK.swap(true, Ordering::AcqRel) {
        log(format_args!(
            "{LOG_PREFIX} tick live player=0x{local:x} people={} cap={}",
            remotes.len(),
            show(cap)
        ));
    }
    let (action, previous) = {
        let mut tracker = TRACKER.lock().unwrap_or_else(|poison| poison.into_inner());
        let previous = tracker.applied();
        (tracker.step(local, cap), previous)
    };
    let cap = if local == 0 { None } else { cap };
    CURRENT_CAP.store(cap_to_atomic(cap), Ordering::Release);
    // Every weapon in the inventory first, so whatever is equipped next, by anything, is already
    // at the cap, and the push below reads capped entries.
    sweep_inventory(cap);
    let redrive = matches!(action, Action::Redrive { .. });
    if local != 0
        && !redrive
        && let Some(bag) = bag()
    {
        resweep_if_changed(bag, local, cap);
    }
    if let Action::Redrive { cap } = action {
        let highest: Vec<String> = remotes
            .iter()
            .map(|records| show(policy::remote_highest(records)))
            .collect();
        log(format_args!(
            "{LOG_PREFIX} cap {} -> {} enabled={enabled} people={} their-highest=[{}] test_cap={} \
             player=0x{local:x}",
            show(previous),
            show(cap),
            remotes.len(),
            highest.join(","),
            show(test_cap)
        ));
        push(local, cap);
    }
}

/// The cap moved: push every equipped weapon, log all three copies of every slot, and remember
/// the slots as settled so the watch asks again only when they change.
fn push(player: usize, cap: Option<u8>) {
    let Some(bag) = bag() else {
        log(format_args!("{LOG_PREFIX} push skipped: no inventory bag"));
        return;
    };
    let pushed = push_slots(bag, player, cap);
    let verb = if cap.is_some() { "CAPPED" } else { "RESTORED" };
    log(format_args!(
        "{LOG_PREFIX} {verb} cap={} pushed=[{}] after: {}",
        show(cap),
        pushed.join(" "),
        copies(bag, player)
    ));
    let mut watch = WATCH.lock().unwrap_or_else(|poison| poison.into_inner());
    if cap.is_some() {
        watch.settle(equipped(bag, player));
    } else {
        let _ = watch.changed(&equipped(bag, player), None);
    }
}

/// The equipped slots changed while a cap is on, for whatever reason: resweep all six, and log
/// one line with the slots before, now, what was pushed and what the character carries after.
fn resweep_if_changed(bag: usize, player: usize, cap: Option<u8>) {
    let now = equipped(bag, player);
    let mut watch = WATCH.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(before) = watch.changed(&now, cap) else {
        return;
    };
    let slots = policy::resweep(&now);
    let pushed = if slots.is_empty() {
        Vec::new()
    } else {
        push_slots(bag, player, cap)
    };
    let after = equipped(bag, player);
    watch.settle(after);
    let over = after.iter().any(|slot| {
        let limit = cap.unwrap_or(ds2_rva::WEAPON_LEVEL_MAX);
        slot.inventory.is_some()
            && (slot.record.is_some_and(|(_, level)| level > limit)
                || slot.live.is_some_and(|level| level > limit))
    });
    log(format_args!(
        "{LOG_PREFIX} equipped changed under cap={}: before=[{}] now=[{}] resweep slots={slots:?} \
         pushed=[{}] after=[{}] all-within-cap={}",
        show(cap),
        before.map_or_else(|| "first look".to_string(), |before| show_slots(&before)),
        show_slots(&now),
        pushed.join(" "),
        show_slots(&after),
        !over
    ));
}

/// Push every equipped weapon whose character copies differ from `clamp(inventory level, cap)`
/// through the game's weapon update, all six slots checked. Answers what was pushed.
fn push_slots(bag: usize, player: usize, cap: Option<u8>) -> Vec<String> {
    let raw = WEAPON_UPDATE_ORIGINAL.load(Ordering::Acquire);
    if raw == 0 {
        return Vec::new();
    }
    // SAFETY: MinHook's trampoline for this exact function and ABI. Calling the trampoline rather
    // than the patched entry runs the game's code only; the clamp has already been applied to the
    // request by `policy::clamp` below.
    let update: WeaponUpdate = unsafe { std::mem::transmute::<usize, WeaponUpdate>(raw) };
    let records = weapon_records(player);
    let live = live_levels(player);
    let mut pushed = Vec::new();
    for slot in 0..ds2_rva::WEAPON_SLOT_COUNT {
        let Some(weapon) = inventory_weapon(bag, slot) else {
            continue;
        };
        let level = policy::clamp(weapon.level, cap);
        let chr_slot = ds2_rva::WEAPON_INTERNAL_TO_CHR_SLOT[slot];
        let carried = records.map(|records| records[chr_slot as usize]);
        let live_level = live.map(|live| live[live_index(chr_slot as usize)]);
        if carried == Some((weapon.item, level)) && live_level == Some(level) {
            continue;
        }
        // Two calls, not one, and the first is what makes the second land. The record table's
        // writer (0x1403463d0) keeps a new record only when the item id or the u16 at +0x0C
        // differs from the old one, and the level sits in the byte after that u16: a level-only
        // change with the same item is computed and thrown away. Measured: one push left the
        // record at +10 while the live state took +3. The packet 61 receiver on every peer
        // writes through the same function, so a peer would keep our old level too. Fists first
        // changes the id; the real item at the new level then lands, here and on every peer.
        for (item, item_level, infusion) in [
            (ds2_rva::FISTS_ITEM_ID, 0, 0),
            (weapon.item, level, weapon.infusion),
        ] {
            let mut request = Request([0u8; ds2_rva::WEAPON_UPDATE_REQUEST_SIZE]);
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_SLOT_OFFSET..][..4]
                .copy_from_slice(&chr_slot.to_le_bytes());
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_ITEM_OFFSET..][..4]
                .copy_from_slice(&item.to_le_bytes());
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_DURABILITY_OFFSET..][..4]
                .copy_from_slice(&weapon.durability_bits.to_le_bytes());
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_LEVEL_OFFSET] = item_level;
            request.0[ds2_rva::WEAPON_UPDATE_REQUEST_INFUSION_OFFSET] = infusion;
            // SAFETY: game thread, after the net session update returned; `player` is the local
            // PlayerCtrl read this frame, and the request is laid out exactly as `0x1401b66a0`
            // lays out the one it passes (ds2-rva CHR_WEAPON_UPDATE).
            unsafe { update(player, request.0.as_mut_ptr()) };
        }
        let real = LEDGER
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .real(weapon.handle, weapon.item)
            .unwrap_or(weapon.level);
        pushed.push(format!("s{slot}:{}+{real}->+{level}", weapon.item));
    }
    pushed
}

/// All three copies of every weapon slot's level, inventory slot order.
fn copies(bag: usize, player: usize) -> String {
    let inventory: Vec<String> = (0..ds2_rva::WEAPON_SLOT_COUNT)
        .map(|slot| {
            inventory_weapon(bag, slot).map_or_else(|| "-".to_string(), |w| format!("+{}", w.level))
        })
        .collect();
    let records: Vec<String> = weapon_records(player).map_or_else(Vec::new, |records| {
        ds2_rva::WEAPON_INTERNAL_TO_CHR_SLOT
            .iter()
            .map(|chr| format!("+{}", records[*chr as usize].1 & 0x0f))
            .collect()
    });
    let live: Vec<String> = live_levels(player).map_or_else(Vec::new, |live| {
        ds2_rva::WEAPON_INTERNAL_TO_CHR_SLOT
            .iter()
            .map(|chr| format!("+{}", live[live_index(*chr as usize)]))
            .collect()
    });
    format!(
        "inventory=[{}] records=[{}] live=[{}] (inventory slot order)",
        inventory.join(","),
        records.join(","),
        live.join(",")
    )
}

/// The request block, aligned the way the game's stack copy is.
#[repr(C, align(8))]
struct Request([u8; ds2_rva::WEAPON_UPDATE_REQUEST_SIZE]);
