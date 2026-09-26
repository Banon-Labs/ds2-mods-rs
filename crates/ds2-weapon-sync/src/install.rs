//! The two detours: the clamp on the game's weapon update, and the per-frame check on the net
//! session update that decides when to push our weapons through it again.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use ds2_game_base::mem::{
    game_rva, read_bytes, safe_read_u8, safe_read_u16, safe_read_u32, safe_read_usize,
};
use ds2_hook::{MH_EnableHook, MH_Initialize, MH_STATUS, MhHook};

use crate::LOG_PREFIX;
use crate::policy::{self, Action, RemoteWeapons, Tracker};

/// `CHR_WEAPON_UPDATE(PlayerCtrl*, WeaponUpdateRequest*)`.
type WeaponUpdate = unsafe extern "system" fn(usize, *mut u8);

/// `NET_SESSION_UPDATE(this, f32 delta)`. The delta is a float in `xmm1`; declaring it as an
/// integer would compile and hand the original whatever that register held.
type NetSessionUpdate = unsafe extern "system" fn(usize, f32);

/// Trampoline back to the real weapon update. Also what the push calls, so the push runs the
/// game's code and not our detour's clamp twice.
static WEAPON_UPDATE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Trampoline back to the real net session update.
static TICK_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

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

fn log(args: std::fmt::Arguments<'_>) {
    let raw = LOGGER.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: `raw` is only ever a `LogFn` stored by `set_logger` above.
        let logger: LogFn = unsafe { std::mem::transmute::<usize, LogFn>(raw) };
        logger(args);
    }
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
/// nobody else in the world.
///
/// # Safety
///
/// Patches executable memory in the loaded game image. Call once, from the loader's post-Arxan
/// install position.
pub unsafe fn install(test_cap: Option<u8>) -> Outcome {
    let refused = Outcome { installed: false };
    let Some(weapon_site) = checked_site(
        ds2_rva::CHR_WEAPON_UPDATE,
        &ds2_rva::CHR_WEAPON_UPDATE_PROLOGUE,
        "CHR_WEAPON_UPDATE",
    ) else {
        return refused;
    };
    let Some(tick_site) = checked_site(
        ds2_rva::NET_SESSION_UPDATE,
        &ds2_rva::NET_SESSION_UPDATE_PROLOGUE,
        "NET_SESSION_UPDATE",
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
    // Both hooks are created before either is enabled, so a failure on the second leaves the first
    // created but never live: the clamp without the tick would lower levels and never restore them.
    //
    // The tick goes first because it is the one that can collide: `ds2-voice-chat` detours the same
    // net session update. MinHook binds one detour per address, and the union in `ds2-hook` cannot
    // carry this signature (the delta is a float in xmm1).
    // SAFETY: the site matched its recorded prologue, and the detour has the same ABI.
    let tick = match unsafe { MhHook::new(tick_site as *mut c_void, tick_detour as *mut c_void) } {
        Ok(hook) => hook,
        Err(status) => {
            log(format_args!(
                "{LOG_PREFIX} not installed: MH_CreateHook on NET_SESSION_UPDATE said {status:?} \
                 -- if that is MH_ERROR_ALREADY_CREATED, [voice_chat] owns this site; turn one of \
                 them off"
            ));
            return refused;
        }
    };
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
    // Published before the sites are patched, so a detour that fires at once has somewhere to go.
    TICK_ORIGINAL.store(tick.trampoline() as usize, Ordering::Release);
    WEAPON_UPDATE_ORIGINAL.store(weapon.trampoline() as usize, Ordering::Release);
    for (site, name) in [
        (weapon_site, "CHR_WEAPON_UPDATE"),
        (tick_site, "NET_SESSION_UPDATE"),
    ] {
        // SAFETY: an address `MhHook::new` accepted above.
        let status = unsafe { MH_EnableHook(site as *mut c_void) };
        if status != MH_STATUS::MH_OK {
            log(format_args!(
                "{LOG_PREFIX} not installed: MH_EnableHook on {name} said {status:?}"
            ));
            return refused;
        }
    }
    log(format_args!(
        "{LOG_PREFIX} installed weapon-update=0x{weapon_site:016x} tick=0x{tick_site:016x} \
         test_cap={} -- while another player is in the world, our weapons above their highest \
         weapon level are lowered to it; the inventory (what the save keeps) is never written",
        show(test_cap)
    ));
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

/// The bag: `[[[GameManagerImp + 0xA8] + 0x10] + 0x10] + 0x10`, two bag hops.
fn bag() -> Option<usize> {
    let manager = read_ptr(GAME_MANAGER.load(Ordering::Acquire))?;
    let data = read_ptr(manager + ds2_rva::GAME_DATA_MANAGER_OFFSET)?;
    let mut at = read_ptr(data + ds2_rva::ITEM_INVENTORY_OFFSET)?;
    for _ in 0..ds2_rva::ITEM_BAG_LIST_HOPS {
        at = read_ptr(at + ds2_rva::ITEM_BAG_LIST_OFFSET)?;
    }
    Some(at)
}

/// One of our weapons as the inventory holds it: the real level.
#[derive(Clone, Copy, Debug)]
struct InventoryWeapon {
    item: u32,
    durability_bits: u32,
    level: u8,
    infusion: u8,
}

/// Inventory weapon slot `slot`, read the way `0x1401b66a0` reads it. `None` for an empty slot.
fn inventory_weapon(bag: usize, slot: usize) -> Option<InventoryWeapon> {
    let entry = read_ptr(bag + ds2_rva::ITEM_BAG_EQUIPPED_ENTRIES_OFFSET + slot * 8)?;
    // SAFETY: fault-safe reads of an entry the bag points at.
    let (kind, item, durability_bits, level, infusion) = unsafe {
        (
            safe_read_u8(entry + ds2_rva::ITEM_ENTRY_TYPE_OFFSET)?,
            safe_read_u32(entry + ds2_rva::ITEM_ENTRY_ITEM_ID_OFFSET)?,
            safe_read_u32(entry + ds2_rva::ITEM_ENTRY_DURABILITY_OFFSET)?,
            safe_read_u8(entry + ds2_rva::ITEM_ENTRY_LEVEL_OFFSET)?,
            safe_read_u8(entry + ds2_rva::ITEM_ENTRY_INFUSION_OFFSET)?,
        )
    };
    Some(InventoryWeapon {
        item,
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
// The tick, and the push.
// ---------------------------------------------------------------------------------------------

unsafe extern "system" fn tick_detour(this: usize, delta: f32) {
    let raw = TICK_ORIGINAL.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: MinHook's trampoline for this exact function and ABI.
        let original: NetSessionUpdate =
            unsafe { std::mem::transmute::<usize, NetSessionUpdate>(raw) };
        // SAFETY: forwarding the game's own arguments.
        unsafe { original(this, delta) };
    }
    // After the original: whatever the session update holds, it has let go of by now.
    let _ = std::panic::catch_unwind(check);
}

fn check() {
    if !FRAME
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(CHECK_EVERY_FRAMES)
    {
        return;
    }
    let local = local_player().unwrap_or(0);
    if LAST_PLAYER.swap(local, Ordering::AcqRel) != local
        && local != 0
        && let Some(bag) = bag()
    {
        log(format_args!(
            "{LOG_PREFIX} world player=0x{local:x} built with: {}",
            copies(bag, local)
        ));
    }
    let remotes = if local == 0 {
        Vec::new()
    } else {
        remotes(local)
    };
    let test_cap = cap_from_atomic(TEST_CAP.load(Ordering::Acquire));
    let cap = policy::cap(&remotes, test_cap);
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
    CURRENT_CAP.store(
        cap_to_atomic(if local == 0 { None } else { cap }),
        Ordering::Release,
    );
    if let Action::Redrive { cap } = action {
        let highest: Vec<String> = remotes
            .iter()
            .map(|records| show(policy::remote_highest(records)))
            .collect();
        log(format_args!(
            "{LOG_PREFIX} cap {} -> {} people={} their-highest=[{}] test_cap={} player=0x{local:x}",
            show(previous),
            show(cap),
            remotes.len(),
            highest.join(","),
            show(test_cap)
        ));
        push(local, cap);
    }
}

/// Push every equipped weapon through the game's weapon update at `clamp(real, cap)`, then read
/// all three copies back and log them, so the line says what the character carries and that the
/// inventory did not move.
fn push(player: usize, cap: Option<u8>) {
    let raw = WEAPON_UPDATE_ORIGINAL.load(Ordering::Acquire);
    let Some(bag) = bag() else {
        log(format_args!("{LOG_PREFIX} push skipped: no inventory bag"));
        return;
    };
    if raw == 0 {
        return;
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
        pushed.push(format!(
            "s{slot}:{}+{}->+{level}",
            weapon.item, weapon.level
        ));
    }
    let verb = if cap.is_some() { "CAPPED" } else { "RESTORED" };
    log(format_args!(
        "{LOG_PREFIX} {verb} cap={} pushed=[{}] after: {}",
        show(cap),
        pushed.join(" "),
        copies(bag, player)
    ));
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
