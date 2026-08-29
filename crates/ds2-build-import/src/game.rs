//! Reading the live character, and granting items through the game's own function.
//!
//! # Nothing here builds game state by hand
//!
//! Items are granted by calling [`ds2_rva::ITEM_GIVE`], not by writing inventory records. A slot the
//! game did not build is a slot whose invariants nobody maintained -- the linked-list links at
//! `+0x00`/`+0x08`, the handle at `+0x1C` that the discard path consumes, the equip category, the
//! counters the UI reads. The engine's own function maintains whatever it maintains, including the
//! parts nobody discovered.
//!
//! The same principle governs reads where the game offers an accessor: [`player_param`] calls
//! [`ds2_rva::PLAYER_PARAM_GET`] rather than walking two pointers, because that function already
//! null-checks both hops and getting a null check wrong is the failure that crashes someone else's
//! game.
//!
//! # The soul costs come from the game, not from a file
//!
//! [`soul_costs`] walks the param tables the game has already loaded and reads
//! `PlayerLevelUpSoulsParam` out of them. The alternative -- decrypting `enc_regulation.bnd.dcx` and
//! shipping the numbers -- is worse in two ways that matter: a shipped copy can drift from the build
//! actually running, and it would silently contradict any param mod the player has installed.
//!
//! # Everything refuses rather than faults
//!
//! Every pointer hop goes through the fault-safe readers and every one is checked for null. This
//! code runs while a player is standing in a pause menu; a wrong pointer here is their crash, in
//! their game, with our name on it.

use ds2_build_import_core::SoulCosts;
use ds2_game_base::mem::{game_rva, safe_read_u16, safe_read_u32, safe_read_usize};

/// What the game could not tell us.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GameError {
    /// A module base or RVA would not resolve.
    Unresolved,
    /// A pointer in the chain was null -- no character, or the title screen.
    NoCharacter,
    /// The param tables could not be walked.
    NoParams,
    /// `PlayerLevelUpSoulsParam` is not among the loaded params.
    NoLevelCosts,
    /// A function's first bytes are not what `ds2-rva` recorded.
    PrologueMismatch,
}

impl core::fmt::Display for GameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            GameError::Unresolved => "a module base or RVA would not resolve",
            GameError::NoCharacter => "no live character -- a null in the player chain",
            GameError::NoParams => "the param tables could not be walked",
            GameError::NoLevelCosts => "PlayerLevelUpSoulsParam is not loaded",
            GameError::PrologueMismatch => "a function's prologue is not what ds2-rva recorded",
        };
        f.write_str(text)
    }
}

/// `Some(pointer)` unless it is null.
const fn non_null(pointer: usize) -> Option<usize> {
    if pointer == 0 { None } else { Some(pointer) }
}

/// One fault-safe hop: read a pointer at `at + offset` and refuse a null.
fn hop(at: usize, offset: usize) -> Option<usize> {
    // SAFETY: `safe_read_usize` reports an unmapped page rather than faulting, and every caller
    // below has already established `at` as a pointer the game itself stores.
    non_null(unsafe { safe_read_usize(at + offset)? })
}

/// `GameManagerImp`, or `None` before the game has one.
fn game_manager() -> Option<usize> {
    let address = game_rva(ds2_rva::GAME_MANAGER_IMP).ok()?;
    // SAFETY: a resolved RVA inside the loaded image.
    non_null(unsafe { safe_read_usize(address)? })
}

/// `PlayerParam`, by calling the game's own null-guarded getter.
///
/// The getter returns NULL at whichever hop is null -- which is the case at the title screen, where
/// `PlayerCtrl` does not exist -- rather than faulting, so a null return is an answer.
pub(crate) fn player_param() -> Result<usize, GameError> {
    let getter = game_rva(ds2_rva::PLAYER_PARAM_GET).map_err(|_| GameError::Unresolved)?;
    // SAFETY: the target is a `.pdata` function start recorded in `ds2-rva` and verified byte for
    // byte -- eight instructions, two null checks, a `ret`. It takes no arguments and touches no
    // state, so there is nothing for a caller to get wrong beyond calling it at all.
    let param: usize = unsafe {
        let get: unsafe extern "system" fn() -> usize = core::mem::transmute(getter);
        get()
    };
    non_null(param).ok_or(GameError::NoCharacter)
}

/// The nine stats, **in the game's storage order**.
///
/// Not the planner's order -- see [`ds2_rva::PLAYER_PARAM_STAT_OFFSETS`]. Adaptability is last here
/// and seventh there, and reading one as the other swaps three stats silently.
pub(crate) fn read_stats(param: usize) -> Option<[u16; 9]> {
    let mut out = [0u16; 9];
    for (slot, offset) in out.iter_mut().zip(ds2_rva::PLAYER_PARAM_STAT_OFFSETS) {
        // SAFETY: `param` came from the game's own getter, and the offsets are inside the block it
        // returns; the read is fault-safe regardless.
        *slot = unsafe { safe_read_u16(param + offset)? };
    }
    Some(out)
}

/// The character's soul level.
pub(crate) fn read_soul_level(param: usize) -> Option<u32> {
    // SAFETY: as `read_stats`.
    unsafe { safe_read_u32(param + ds2_rva::PLAYER_PARAM_SOUL_LEVEL_OFFSET) }
}

/// Soul memory, as the game stores it -- **two fields**, returned together.
///
/// They are returned as a pair rather than as one number so a caller cannot forget the second one
/// exists. See [`ds2_rva::PLAYER_PARAM_SOUL_MEMORY_OFFSETS`].
pub(crate) fn read_soul_memory(param: usize) -> Option<[u32; 2]> {
    let mut out = [0u32; 2];
    for (slot, offset) in out
        .iter_mut()
        .zip(ds2_rva::PLAYER_PARAM_SOUL_MEMORY_OFFSETS)
    {
        // SAFETY: as `read_stats`.
        *slot = unsafe { safe_read_u32(param + offset)? };
    }
    Some(out)
}

/// The address of a loaded param table, by name.
///
/// Walks the master index the game builds when it loads the regulation. The name in the index
/// carries its extension (`"PlayerLevelUpSoulsParam.param"`), so the comparison is against the stem.
fn find_param(name: &str) -> Result<usize, GameError> {
    let manager = game_manager().ok_or(GameError::NoCharacter)?;
    let mut anchor = manager;
    for offset in ds2_rva::PARAM_ANCHOR_OFFSETS {
        anchor = hop(anchor, offset).ok_or(GameError::NoParams)?;
    }
    // The table sits BELOW the anchor, which is a measured relationship rather than a field offset.
    let table = anchor
        .checked_sub(ds2_rva::PARAM_TABLE_FROM_ANCHOR)
        .ok_or(GameError::NoParams)?;
    let index_end = anchor
        .checked_sub(ds2_rva::PARAM_INDEX_END_FROM_ANCHOR)
        .ok_or(GameError::NoParams)?;
    let index_start = table + ds2_rva::PARAM_INDEX_START_OFFSET;
    if index_end <= index_start {
        return Err(GameError::NoParams);
    }

    let count = (index_end - index_start) / ds2_rva::PARAM_INDEX_STRIDE;
    // A sane bound. The game loads a few hundred params; a count in the millions means the anchor
    // walk landed somewhere else and the loop below would read the whole address space.
    if count == 0 || count > 4096 {
        return Err(GameError::NoParams);
    }
    for index in 0..count {
        let entry = index_start + index * ds2_rva::PARAM_INDEX_STRIDE;
        // SAFETY: inside the bounded index the game itself built; fault-safe regardless.
        let (data, name_at) = unsafe {
            (
                safe_read_u32(entry + ds2_rva::PARAM_INDEX_DATA_OFFSET),
                safe_read_u32(entry + ds2_rva::PARAM_INDEX_NAME_OFFSET),
            )
        };
        let (Some(data), Some(name_at)) = (data, name_at) else {
            continue;
        };
        // SAFETY: the name is a NUL-terminated ASCII string inside the table; the reader is bounded
        // and walks page by page.
        let Some(bytes) =
            (unsafe { ds2_game_base::mem::safe_read_cstr(table + name_at as usize, 64) })
        else {
            continue;
        };
        let Ok(text) = core::str::from_utf8(&bytes) else {
            continue;
        };
        if text.split('.').next() == Some(name) {
            return Ok(table + data as usize);
        }
    }
    Err(GameError::NoLevelCosts)
}

/// The souls each level costs, read out of the game's own loaded params.
///
/// Row id is the level being LEFT, which is the same convention
/// [`ds2_build_import_core::SoulCosts`] uses -- so `costs[n]` is the cost of going from `n` to
/// `n + 1`, and a row for level `n` lands at index `n - 1`.
pub(crate) fn soul_costs() -> Result<SoulCosts, GameError> {
    let param = find_param(ds2_rva::PARAM_PLAYER_LEVEL_UP_SOULS)?;
    // SAFETY: `param` is a table address the game's own index pointed at.
    let rows = unsafe { safe_read_u16(param + ds2_rva::PARAM_ROW_COUNT_OFFSET) }
        .ok_or(GameError::NoParams)? as usize;
    if rows == 0 {
        return Err(GameError::NoLevelCosts);
    }

    // Indexed by level, so a param whose rows are out of order still lands correctly.
    let mut costs = vec![0u64; rows];
    let mut seen = 0usize;
    for index in 0..rows {
        let entry = param + ds2_rva::PARAM_ROW_INDEX_OFFSET + index * ds2_rva::PARAM_ROW_STRIDE;
        // SAFETY: inside the row index the param declares the length of.
        let (id, at) = unsafe {
            (
                safe_read_u32(entry + ds2_rva::PARAM_ROW_ID_OFFSET),
                safe_read_u32(entry + ds2_rva::PARAM_ROW_DATA_OFFSET),
            )
        };
        let (Some(id), Some(at)) = (id, at) else {
            continue;
        };
        // SAFETY: the row's own data, at the offset its index entry declares.
        let Some(cost) = (unsafe {
            safe_read_u32(param + at as usize + ds2_rva::PLAYER_LEVEL_UP_SOULS_COST_OFFSET)
        }) else {
            continue;
        };
        // Row id IS the level, and level 1 is the first cost.
        if let Some(slot) = id
            .checked_sub(1)
            .and_then(|slot| costs.get_mut(slot as usize))
        {
            *slot = u64::from(cost);
            seen += 1;
        }
    }
    if seen == 0 {
        return Err(GameError::NoLevelCosts);
    }
    SoulCosts::new(costs).map_err(|_| GameError::NoLevelCosts)
}

/// Whether the two copies of the stat block agree.
///
/// `PlayerParam` stores the eleven stats at `+0x08` and AGAIN at
/// [`ds2_rva::PLAYER_PARAM_STAT_MIRROR_OFFSET`]. Every legitimate path writes both; a tool that
/// pokes one writes one. So a disagreement is positive evidence that something has written this
/// character's stats by hand -- ours or somebody else's -- and it costs 22 bytes of comparison.
///
/// `None` when either copy could not be read.
pub(crate) fn stat_mirror_agrees(param: usize) -> Option<bool> {
    let mut primary = [0u8; ds2_rva::PLAYER_PARAM_STAT_BLOCK_SIZE];
    let mut mirror = [0u8; ds2_rva::PLAYER_PARAM_STAT_BLOCK_SIZE];
    // SAFETY: both ranges are inside the block the game's own getter returned; the reads are
    // fault-safe and report an unmapped page rather than raising.
    let read = unsafe {
        ds2_game_base::mem::read_bytes(param + ds2_rva::PLAYER_PARAM_STAT_OFFSETS[0], &mut primary)
            && ds2_game_base::mem::read_bytes(
                param + ds2_rva::PLAYER_PARAM_STAT_MIRROR_OFFSET,
                &mut mirror,
            )
    };
    read.then(|| primary == mirror)
}

/// What one [`add_souls`] call actually changed.
///
/// Every field is a BEFORE/AFTER pair because the call can silently do nothing in three different
/// ways -- a status flag on the player vetoes the whole function, and each counter has its own skip
/// byte -- so "it returned" is not evidence that anything moved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SoulsAdded {
    pub(crate) held: (u32, u32),
    pub(crate) memory: ([u32; 2], [u32; 2]),
    /// The per-counter skip bytes as they read at the time, for the log.
    pub(crate) guards: [u8; 3],
}

impl SoulsAdded {
    /// Whether both soul-memory counters moved.
    pub(crate) fn memory_moved(&self) -> bool {
        self.memory.1[0] > self.memory.0[0] && self.memory.1[1] > self.memory.0[1]
    }
}

/// Raise souls through the game's own [`ds2_rva::PLAYER_PARAM_ADD_SOULS`], and check it landed.
///
/// **One call raises souls held AND both soul-memory counters**, and fires the change notifications
/// a hand-written triple of stores would not. That is the whole reason to call it: soul memory is
/// exactly the field this crate must never write by hand, because the game treats it as monotonic
/// and derives matchmaking from it.
///
/// # Safety
///
/// Calls into the game. **Game thread only**, with a character loaded.
pub(crate) unsafe fn add_souls(param: usize, amount: u32) -> Result<SoulsAdded, GameError> {
    let site = game_rva(ds2_rva::PLAYER_PARAM_ADD_SOULS).map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::PLAYER_PARAM_ADD_SOULS_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::PLAYER_PARAM_ADD_SOULS_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }

    let held_before = read_souls_held(param).ok_or(GameError::NoCharacter)?;
    let memory_before = read_soul_memory(param).ok_or(GameError::NoCharacter)?;
    let mut guards = [0u8; 3];
    for (slot, offset) in guards
        .iter_mut()
        .zip(ds2_rva::PLAYER_PARAM_SOUL_COUNTER_GUARDS)
    {
        // SAFETY: inside the block the getter returned.
        *slot = unsafe { ds2_game_base::mem::safe_read_u8(param + offset) }.unwrap_or(0);
    }

    // SAFETY: the prologue matched the bytes recorded for this function, and the signature is the
    // one the disassembly implements -- the param block in RCX, an unsigned amount in EDX, no
    // return. It only reads `param`'s own fields and its own arguments.
    unsafe {
        let add: unsafe extern "system" fn(usize, u32) = core::mem::transmute(site);
        add(param, amount);
    }

    Ok(SoulsAdded {
        held: (held_before, read_souls_held(param).unwrap_or(held_before)),
        memory: (
            memory_before,
            read_soul_memory(param).unwrap_or(memory_before),
        ),
        guards,
    })
}

/// Souls currently held.
pub(crate) fn read_souls_held(param: usize) -> Option<u32> {
    // SAFETY: as `read_stats`.
    unsafe { safe_read_u32(param + ds2_rva::PLAYER_PARAM_SOULS_HELD_OFFSET) }
}

/// Raise soul memory to at least `floor`, through the game, and say what happened.
///
/// **Never lowers.** Soul memory is monotonic in this game and the engine offers no path that
/// reduces it; a character who has played past this level keeps their own larger number. So a floor
/// already met is `Ok(None)` -- nothing to do -- rather than a write of the exact value.
///
/// The amount added is the SHORTFALL, and it also raises souls held by the same amount, which is
/// the game's own coupling rather than a side effect worth avoiding: a character who legitimately
/// reached that soul memory did hold those souls.
///
/// # Safety
///
/// As [`add_souls`]: game thread, character loaded.
pub(crate) unsafe fn raise_soul_memory_to(
    param: usize,
    floor: u64,
) -> Result<Option<SoulsAdded>, GameError> {
    let memory = read_soul_memory(param).ok_or(GameError::NoCharacter)?;
    let lowest = u64::from(memory[0].min(memory[1]));
    if lowest >= floor {
        return Ok(None);
    }
    // The counters saturate at the game's own cap, so asking for more than that is asking for a
    // silent clamp; ask for exactly what the cap allows instead.
    let shortfall = (floor - lowest).min(u64::from(ds2_rva::PLAYER_PARAM_SOULS_CAP)) as u32;
    // SAFETY: forwarded to the caller.
    unsafe { add_souls(param, shortfall) }.map(Some)
}

/// One entry of the array [`ds2_rva::ITEM_GIVE`] takes.
///
/// `#[repr(C)]` and asserted to be [`ds2_rva::ITEM_SPAWN_SIZE`] at compile time, because the callee
/// strides by that and a padding surprise would hand it every field shifted.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ItemSpawn {
    /// Both community writers store `0` here and nothing observed reads it.
    pub(crate) unknown: u32,
    pub(crate) item_id: i32,
    pub(crate) durability: f32,
    pub(crate) quantity: u16,
    pub(crate) reinforce: u8,
    pub(crate) infusion: u8,
}

const _: () = assert!(core::mem::size_of::<ItemSpawn>() == ds2_rva::ITEM_SPAWN_SIZE);

/// `fn(inventory, *const ItemSpawn, count) -> bool`.
type ItemGiveFn = unsafe extern "system" fn(usize, *const ItemSpawn, u32) -> bool;

/// The inventory object [`ds2_rva::ITEM_GIVE`] is called on.
fn item_inventory() -> Result<usize, GameError> {
    let manager = game_manager().ok_or(GameError::NoCharacter)?;
    let data = hop(manager, ds2_rva::GAME_DATA_MANAGER_OFFSET).ok_or(GameError::NoCharacter)?;
    hop(data, ds2_rva::ITEM_INVENTORY_OFFSET).ok_or(GameError::NoCharacter)
}

/// Grant items, in batches, through the game's own function.
///
/// Returns how many of `items` were accepted. **The return value of each call is checked** -- it is
/// a bool in `AL` and `false` means that batch granted nothing. Neither community implementation
/// checks it, which is why their failures are silent.
///
/// # Safety
///
/// Calls into the game. **Game thread only**, and only with a character loaded. Both community
/// implementations call this from a fresh remote thread with no game state at all, so the function
/// itself depends on nothing but its arguments -- but a caller on the game's own thread is not
/// racing the game's inventory reads, which is the point.
pub(crate) unsafe fn give_items(items: &[ItemSpawn], batch: usize) -> Result<usize, GameError> {
    if items.is_empty() {
        return Ok(0);
    }
    let batch = batch.clamp(1, ds2_rva::ITEM_GIVE_MAX_PER_CALL);
    let inventory = item_inventory()?;
    let site = game_rva(ds2_rva::ITEM_GIVE).map_err(|_| GameError::Unresolved)?;

    // THE BYTES BEFORE THE CALL. An RVA is a number, and on a build this table was not read from it
    // points into the middle of something else that would accept a call and do anything at all.
    let mut prologue = [0u8; ds2_rva::ITEM_GIVE_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; `read_bytes` faults safely.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::ITEM_GIVE_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }

    // SAFETY: the site's first bytes are the ones recorded for it, and the signature is the one the
    // disassembled thunk implements -- inventory in RCX, the array in RDX, the count in R8D, a bool
    // back in AL. The slice outlives the call and the callee only reads it.
    let give: ItemGiveFn = unsafe { core::mem::transmute(site) };
    let mut granted = 0usize;
    for chunk in items.chunks(batch) {
        // SAFETY: as above; `chunk` is a live slice of exactly `chunk.len()` entries.
        let ok = unsafe { give(inventory, chunk.as_ptr(), chunk.len() as u32) };
        if ok {
            granted += chunk.len();
        }
    }
    Ok(granted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The struct the game strides over is the size the game strides by.
    #[test]
    fn the_spawn_entry_is_the_size_the_callee_assumes() {
        assert_eq!(core::mem::size_of::<ItemSpawn>(), 0x10);
        assert_eq!(core::mem::size_of::<ItemSpawn>(), ds2_rva::ITEM_SPAWN_SIZE);
        // And the fields land where the two community writers put them.
        let spawn = ItemSpawn::default();
        let base = &spawn as *const _ as usize;
        assert_eq!(&spawn.item_id as *const _ as usize - base, 0x04);
        assert_eq!(&spawn.durability as *const _ as usize - base, 0x08);
        assert_eq!(&spawn.quantity as *const _ as usize - base, 0x0C);
        assert_eq!(&spawn.reinforce as *const _ as usize - base, 0x0E);
        assert_eq!(&spawn.infusion as *const _ as usize - base, 0x0F);
    }

    /// The batch size is clamped to what the engine's own gate accepts.
    ///
    /// The gate is `lea eax,[rsi-1]; cmp eax,0x1f; ja` -- so `0` is refused by the engine and
    /// anything past 32 is too. Clamping here means a caller's mistake is a smaller batch rather
    /// than a call the engine rejects wholesale.
    #[test]
    fn the_batch_is_clamped_to_the_engines_gate() {
        assert_eq!(ds2_rva::ITEM_GIVE_MAX_PER_CALL, 32);
        assert_eq!(0usize.clamp(1, ds2_rva::ITEM_GIVE_MAX_PER_CALL), 1);
        assert_eq!(999usize.clamp(1, ds2_rva::ITEM_GIVE_MAX_PER_CALL), 32);
        assert_eq!(8usize.clamp(1, ds2_rva::ITEM_GIVE_MAX_PER_CALL), 8);
    }

    /// Every failure says something different, so a log line is a diagnosis.
    #[test]
    fn each_failure_reads_differently() {
        let all = [
            GameError::Unresolved,
            GameError::NoCharacter,
            GameError::NoParams,
            GameError::NoLevelCosts,
            GameError::PrologueMismatch,
        ];
        for (index, one) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(one.to_string(), other.to_string());
            }
        }
    }

    /// The stat offsets are nine distinct `u16` slots, two bytes apart, in one run.
    #[test]
    fn the_stats_are_a_contiguous_block() {
        let offsets = ds2_rva::PLAYER_PARAM_STAT_OFFSETS;
        assert_eq!(offsets.len(), 9);
        for pair in offsets.windows(2) {
            assert_eq!(pair[1] - pair[0], 2, "a u16 apart");
        }
        assert_eq!(ds2_rva::PLAYER_PARAM_STAT_NAMES.len(), offsets.len());
        // Adaptability is LAST in the game's order, which is the trap this pins.
        assert_eq!(ds2_rva::PLAYER_PARAM_STAT_NAMES[8], "adaptability");
        assert_ne!(ds2_rva::PLAYER_PARAM_STAT_NAMES[6], "adaptability");
    }
}
