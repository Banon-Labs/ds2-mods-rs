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
    /// The item is not in the inventory, so there is no entry to equip.
    ///
    /// **Its own variant because the first version reused `NoCharacter` for it**, and the resulting
    /// log said "no live character -- a null in the player chain" eighteen times about a character
    /// that was very much alive. A wrong error costs more than no error: it sends the reader to the
    /// wrong half of the code.
    NotInInventory,
    /// The param tables could not be walked.
    NoParams,
    /// `PlayerLevelUpSoulsParam` is not among the loaded params.
    NoLevelCosts,
    /// A function's first bytes are not what `ds2-rva` recorded.
    PrologueMismatch,
    /// This character has no Estus Flask, so there is no level to raise.
    NoEstusFlask,
    /// The game's own property table does not know the key asked for.
    ///
    /// Its own variant because the answer is `0xFF` and passing that on would write the CHARGE
    /// count rather than a level -- see [`ds2_rva::ESTUS_PROPERTY_NOT_FOUND`].
    UnknownEstusProperty,
}

impl core::fmt::Display for GameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            GameError::Unresolved => "a module base or RVA would not resolve",
            GameError::NoCharacter => "no live character -- a null in the player chain",
            GameError::NotInInventory => "not in the inventory, so there is nothing to equip",
            GameError::NoParams => "the param tables could not be walked",
            GameError::NoLevelCosts => "PlayerLevelUpSoulsParam is not loaded",
            GameError::PrologueMismatch => "a function's prologue is not what ds2-rva recorded",
            GameError::NoEstusFlask => "this character has no Estus Flask",
            GameError::UnknownEstusProperty => {
                "the game's property table does not know that Estus property"
            }
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

/// All ELEVEN stats, raw, as [`set_all_stats`] wants them back.
///
/// [`read_stats`] returns the nine a level buys. This returns those plus the two at index 9 and 10
/// that nothing here understands, because the setter takes all eleven and the only safe thing to do
/// with a field you cannot name is hand it back unchanged.
fn read_all_stats(param: usize) -> Option<[u16; ds2_rva::PLAYER_PARAM_STAT_COUNT]> {
    let mut out = [0u16; ds2_rva::PLAYER_PARAM_STAT_COUNT];
    let base = ds2_rva::PLAYER_PARAM_STAT_OFFSETS[0];
    for (index, slot) in out.iter_mut().enumerate() {
        // SAFETY: as `read_stats` -- inside the block the game's getter returned, fault-safe read.
        *slot = unsafe { safe_read_u16(param + base + index * 2)? };
    }
    Some(out)
}

/// What one [`set_all_stats`] call actually changed.
///
/// Before and after for both the stats and the level, for the same reason [`SoulsAdded`] is a pair:
/// a call that returned is not a call that did anything, and the level in particular is computed by
/// the game rather than written by us, so it is the one number that proves the recompute ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct StatsSet {
    pub(crate) stats: ([u16; 9], [u16; 9]),
    pub(crate) level: (u32, u32),
    /// What the stats amount to afterwards, which is what the player's own menu shows them.
    pub(crate) effective: Option<[u16; 9]>,
}

impl StatsSet {
    /// Whether the character now has the stats that were asked for.
    pub(crate) fn stats_took(&self, wanted: &[u16; 9]) -> bool {
        &self.stats.1 == wanted
    }
}

/// **Set the nine stats, and let the game work out everything they imply.**
///
/// Calls [`ds2_rva::PLAYER_PARAM_SET_ALL_STATS`], which writes both stat copies, recomputes the
/// effective stats, the derived block (HP, stamina, equip load) and the SOUL LEVEL, then reapplies
/// the HP and stamina caps to the live character.
///
/// # Nothing here writes a soul level, because a soul level is not writable
///
/// The game derives it: `level = max(1, sum(nine stats) - 53)`. Writing one would be writing a
/// cached sum, and a cached sum that disagrees with its inputs is exactly the corrupt character
/// this crate exists to avoid. So the level is an OUTPUT here, read back to confirm the recompute
/// happened rather than passed in.
///
/// # The two stats it does not touch
///
/// Index 9 and 10 are read out of the character and written back unchanged. They are not in the
/// level sum and nothing in this repo knows what they are; a setter that takes all eleven and a
/// caller that knows nine means the other two are carried, not zeroed.
///
/// # Safety
///
/// Calls into the game. **Game thread only**, and only with a `param` from [`player_param`]. The
/// prologue is byte-checked first, so a moved or patched function refuses rather than executes
/// whatever is now at that address.
pub(crate) unsafe fn set_all_stats(param: usize, wanted: &[u16; 9]) -> Result<StatsSet, GameError> {
    let site = game_rva(ds2_rva::PLAYER_PARAM_SET_ALL_STATS).map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::PLAYER_PARAM_SET_ALL_STATS_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::PLAYER_PARAM_SET_ALL_STATS_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }

    let stats_before = read_stats(param).ok_or(GameError::NoCharacter)?;
    let level_before = read_soul_level(param).ok_or(GameError::NoCharacter)?;
    // START FROM THE CHARACTER'S OWN ELEVEN, so the two unnamed ones survive.
    let mut block = read_all_stats(param).ok_or(GameError::NoCharacter)?;
    block[..wanted.len()].copy_from_slice(wanted);

    // SAFETY: the prologue matched the bytes recorded for this function, and the signature is the
    // one the disassembly implements -- the param block in RCX, a pointer to eleven `u16` in RDX,
    // no return. `block` outlives the call and is 22 readable bytes, which is exactly what the
    // callee loads.
    unsafe {
        let set: unsafe extern "system" fn(usize, *const u16) = core::mem::transmute(site);
        set(param, block.as_ptr());
    }

    Ok(StatsSet {
        stats: (stats_before, read_stats(param).unwrap_or(stats_before)),
        level: (level_before, read_soul_level(param).unwrap_or(level_before)),
        effective: read_effective_stats(param),
    })
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
/// The character's stats as they currently AMOUNT TO -- base plus whatever modifies them.
///
/// # This replaced a "mirror check" that was reporting corruption that was not there
///
/// `+0x1E` was read as a second copy of the base stats, on the strength of
/// [`ds2_rva::PLAYER_PARAM_SET_ALL_STATS`] writing each `u16` to both offsets. It does -- and then
/// calls the recompute, which overwrites `+0x1E` with the EFFECTIVE stats. So a difference is
/// normal and this crate spent one in-game run announcing that the player's save had been tampered
/// with, twice, on a save that was fine.
///
/// Measured: a character written to `int=38 faith=38` read back `40`/`40` here, everything else
/// exact. See [`ds2_rva::PLAYER_PARAM_EFFECTIVE_STATS_OFFSET`].
///
/// Returned for the LOG, so a run says what the character ended up with rather than only what was
/// asked for. Nothing branches on it.
pub(crate) fn read_effective_stats(param: usize) -> Option<[u16; 9]> {
    let mut out = [0u16; 9];
    let base = ds2_rva::PLAYER_PARAM_EFFECTIVE_STATS_OFFSET;
    for (index, slot) in out.iter_mut().enumerate() {
        // SAFETY: as `read_stats` -- inside the block the game's getter returned, fault-safe read.
        *slot = unsafe { safe_read_u16(param + base + index * 2)? };
    }
    Some(out)
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
/// **Never lowers, by choice rather than by impossibility.** The engine DOES have a path that
/// assigns soul memory absolutely, including downwards -- see
/// [`ds2_rva::PLAYER_PARAM_RESTORE_FROM_RECORD`], which this crate deliberately does not call.
///
/// Nothing needs reducing. The invariant is `soul memory >= what the level cost`, so a build that
/// LOWERS a level leaves it satisfied with room to spare: a lifetime total larger than the current
/// level requires is an ordinary character who respecced or died, not an inconsistent one. A floor
/// already met is therefore `Ok(None)` -- nothing to do -- rather than a write of the exact value.
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
    /// **A MODE, not an unknown.** See [`ds2_rva::ITEM_SPAWN_MODE_NORMAL`]; `1` and `2` change
    /// what the grant does, and a build import wants `0`.
    pub(crate) mode: u32,
    pub(crate) item_id: i32,
    pub(crate) durability: i32,
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
    let mut satisfied = 0usize;
    for chunk in items.chunks(batch) {
        // SAFETY: as above; `chunk` is a live slice of exactly `chunk.len()` entries.
        let ok = unsafe { give(inventory, chunk.as_ptr(), chunk.len() as u32) };
        if ok {
            granted += chunk.len();
            continue;
        }
        // ASK THE GAME WHY. `false` is one bit of a word that names the reason, and reading it is
        // the difference between a mod that reports a mystery and one that reports "you already
        // have ninety-nine of those". See `ds2_rva::ITEM_GIVE_ERROR_OFFSET`.
        let ids: Vec<i32> = chunk.iter().map(|item| item.item_id).collect();
        let (code, reason) = give_error().unwrap_or((0, "the reason word could not be read"));
        if ds2_rva::ITEM_GIVE_ERROR_ALREADY_SATISFIED.contains(&code) {
            // NOT A FAILURE. The build wanted the character to have this and the character does.
            satisfied += chunk.len();
            crate::log_line(format_args!(
                "{} {ids:?} already satisfied: {reason}",
                crate::LOG_PREFIX
            ));
            continue;
        }
        crate::log_line(format_args!(
            "{} ItemGive refused {ids:?}: {reason} ({code:#010x})",
            crate::LOG_PREFIX
        ));
    }
    Ok(granted + satisfied)
}

/// The game's own reason for the last refusal, as `(code, words)`.
///
/// # The mod spent three runs guessing at something the game was already saying
///
/// `ItemGive` returns a bare `bool`, and its entire `false` path is
/// `test dword [inner+0x10138], 0x80000000; sete al`. The rest of that word names the reason. Three
/// consumables looked like an unexplained refusal worth a disassembly session; they were a
/// character already holding ninety-nine of each, and these four bytes said so at the time.
fn give_error() -> Option<(u32, &'static str)> {
    let inventory = item_inventory().ok()?;
    // ONE hop, not the two that reach the bag -- the error word lives on the object the grant
    // thunk's own `mov rcx,[rcx+0x10]` produces.
    let inner = hop(inventory, ds2_rva::ITEM_BAG_LIST_OFFSET)?;
    // SAFETY: inside the object the game's own pointer chain produced; the read is fault-safe.
    let code = unsafe { safe_read_u32(inner + ds2_rva::ITEM_GIVE_ERROR_OFFSET) }?;
    let reason = ds2_rva::ITEM_GIVE_ERRORS
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map_or("an unrecorded reason", |(_, words)| *words);
    Some((code, reason))
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

/// A slot to fill and the item to fill it with, as a build names them.
///
/// The slot is the INTERNAL index [`ds2_rva::ITEM_SET_EQUIP`] takes, already mapped through
/// [`ds2_rva::ITEM_SLOT_FLAT_TO_INTERNAL`]. Building one of these from a flat index by hand is the
/// mistake that puts every weapon in the wrong hand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct EquipRequest<'a> {
    pub(crate) internal_slot: u32,
    /// **Every id the build's name could mean**, most-preferred first.
    ///
    /// Usually one. It is a list because a name the catalogue gives several ids to is granted as
    /// one of them and may be STORED as another: `Estus Flask` names three ids, the grant took the
    /// lowest, and the entry that appeared in the bag matched none of them -- so an equip that
    /// searched only for the id it granted found nothing and reported the flask missing from an
    /// inventory it was sitting in. The build said "Estus Flask"; any Estus Flask answers it.
    pub(crate) item_ids: &'a [i32],
}

/// What one equip actually did, read back rather than assumed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct EquipOutcome {
    pub(crate) internal_slot: u32,
    /// The id actually put in the slot, from the candidates offered.
    pub(crate) wanted: i32,
    /// The item id the slot holds afterwards, or `None` if the slot is empty.
    pub(crate) landed: Option<i32>,
}

impl EquipOutcome {
    /// Whether the slot ended up holding what was asked for.
    pub(crate) fn took(&self) -> bool {
        self.landed == Some(self.wanted)
    }
}

/// The bag that holds the inventory entries.
///
/// **TWO hops of `+0x10` from `ItemInventory2`**, because the equip function splits them across its
/// thunk and its implementation -- see [`ds2_rva::ITEM_BAG_LIST_OFFSET`]. One hop lands on a live
/// object that is not the bag, so nothing faults and nothing matches; the first in-game run of the
/// equip path reported every single item as missing for exactly that reason.
fn bag_list() -> Result<usize, GameError> {
    let mut at = item_inventory()?;
    for _ in 0..ds2_rva::ITEM_BAG_LIST_HOPS {
        at = hop(at, ds2_rva::ITEM_BAG_LIST_OFFSET).ok_or(GameError::NoCharacter)?;
    }
    Ok(at)
}

/// The inventory entry for an item id, preferring one that is not already worn.
///
/// # There is no native "item id -> handle" lookup, so this scans
///
/// [`ds2_rva::ITEM_SET_EQUIP`] names an item by a POINTER TO ITS INVENTORY ENTRY, not by a param id
/// -- the same shape of trap `er-build-import` hit, where the equip took an inventory index that
/// only exists after the grant. DS2 has no function answering "which entry holds item N", so the
/// entry array is walked. 3840 entries of 0x28 bytes is a 153KB scan, once per item, on a frame
/// where the player has just pressed a menu row.
///
/// **It prefers an UNWORN copy.** A build naming the same item twice, or a re-run over a character
/// that already wears it, would otherwise resolve both positions to the one entry -- and since the
/// equip is a MOVE, filling the second slot would strip the first.
fn entry_for_item(bag: usize, item_id: i32) -> Option<usize> {
    let base = bag + ds2_rva::ITEM_ENTRY_ARRAY_OFFSET;
    let mut spare: Option<usize> = None;
    for index in 0..ds2_rva::ITEM_ENTRY_COUNT {
        let entry = base + index * ds2_rva::ITEM_ENTRY_STRIDE;
        // SAFETY: inside the bag the game's own pointer chain produced; the read is fault-safe and
        // reports an unmapped page rather than raising.
        let Some(id) = (unsafe { safe_read_u32(entry + ds2_rva::ITEM_ENTRY_ITEM_ID_OFFSET) })
        else {
            continue;
        };
        if id as i32 != item_id {
            continue;
        }
        // SAFETY: as above.
        let flags =
            unsafe { ds2_game_base::mem::safe_read_u8(entry + ds2_rva::ITEM_ENTRY_FLAGS_OFFSET) };
        if flags.is_some_and(|flags| flags & ds2_rva::ITEM_ENTRY_FLAG_EQUIPPED != 0) {
            // THE COPY ALREADY BEING WORN WINS. This preference used to be the other way round, to
            // stop two slots resolving to one entry -- and the cost of that was the import taking
            // the player's own equipped gear off and replacing it with a copy it had just minted.
            // The player's existing item is the one with their upgrades, their infusion and their
            // durability on it; a freshly granted duplicate is not an improvement on it.
            return Some(entry);
        }
        spare.get_or_insert(entry);
    }
    spare
}

/// Whether the character already holds this item at all.
///
/// Used to decide whether to grant it. **A build asking for a sword the player already owns is not
/// asking for a second sword** -- and the second one arrives without their reinforcement or their
/// infusion, so granting it and then equipping it is strictly worse than leaving them alone.
pub(crate) fn already_held(item_id: i32) -> bool {
    bag_list().is_ok_and(|bag| entry_for_item(bag, item_id).is_some())
}

/// What a FLAT slot currently holds, through the game's own accessor.
///
/// Takes the flat index because that is what the accessor takes -- it is NOT the internal index
/// [`ds2_rva::ITEM_SET_EQUIP`] uses, and passing one for the other reads the wrong slot.
///
/// # Safety
///
/// Calls into the game. Game thread only.
unsafe fn equipped_in_flat_slot(
    inventory: usize,
    flat_slot: i32,
) -> Result<Option<i32>, GameError> {
    let site = game_rva(ds2_rva::ITEM_INVENTORY_EQUIPPED_BY_FLAT_SLOT)
        .map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::ITEM_INVENTORY_EQUIPPED_BY_FLAT_SLOT_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::ITEM_INVENTORY_EQUIPPED_BY_FLAT_SLOT_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }
    // SAFETY: the prologue matched, and the signature is the one the thunk implements -- the
    // inventory in RCX, a slot index in EDX, an entry pointer or null back in RAX.
    let entry = unsafe {
        let get: unsafe extern "system" fn(usize, i32) -> usize = core::mem::transmute(site);
        get(inventory, flat_slot)
    };
    if entry == 0 {
        return Ok(None);
    }
    // SAFETY: a pointer the game returned; the read is fault-safe regardless.
    Ok(unsafe { safe_read_u32(entry + ds2_rva::ITEM_ENTRY_ITEM_ID_OFFSET) }.map(|id| id as i32))
}

/// **Recompute the attunement slot count from the character's current attunement.**
///
/// Writing the stats does not do this -- see [`ds2_rva::ITEM_BAG_RECALC_ATTUNEMENT`]. Without it a
/// character written to attunement 30 keeps whatever count it had before, which for a blank
/// character is zero, and every spell in the build is dropped as over budget.
///
/// # Safety
///
/// Calls into the game. **Game thread only.** The prologue is byte-checked first.
pub(crate) unsafe fn recalc_attunement_slots() -> Result<u8, GameError> {
    let bag = bag_list()?;
    let site = game_rva(ds2_rva::ITEM_BAG_RECALC_ATTUNEMENT).map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::ITEM_BAG_RECALC_ATTUNEMENT_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::ITEM_BAG_RECALC_ATTUNEMENT_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }
    // SAFETY: the prologue matched, and the signature is the one the disassembly implements -- the
    // bag in RCX, nothing else, no return.
    unsafe {
        let recalc: unsafe extern "system" fn(usize) = core::mem::transmute(site);
        recalc(bag);
    }
    attunement_slots()
}

/// How many attunement slots the character has RIGHT NOW.
///
/// Read AFTER the stats are written, or it answers for the old attunement. See
/// [`ds2_rva::ITEM_BAG_ATTUNEMENT_SLOTS_OFFSET`] -- it is a budget spells spend, not a count of
/// positions, so this is an upper bound on how many can be attuned rather than the number.
pub(crate) fn attunement_slots() -> Result<u8, GameError> {
    let bag = bag_list()?;
    // SAFETY: inside the bag the game's own pointer chain produced; fault-safe read.
    unsafe { ds2_game_base::mem::safe_read_u8(bag + ds2_rva::ITEM_BAG_ATTUNEMENT_SLOTS_OFFSET) }
        .ok_or(GameError::NoCharacter)
}

/// The FLAT index for an internal one, by inverting [`ds2_rva::ITEM_SLOT_FLAT_TO_INTERNAL`].
///
/// Needed only because the write takes the internal index and the read-back takes the flat one.
fn flat_slot_for(internal: u32) -> Option<i32> {
    ds2_rva::ITEM_SLOT_FLAT_TO_INTERNAL
        .iter()
        .position(|mapped| *mapped >= 0 && *mapped as u32 == internal)
        .map(|flat| flat as i32)
}

/// **Put one item in one slot, through the game's own equip function, and read back what happened.**
///
/// # Safety
///
/// Calls into the game. **Game thread only.** The prologue is byte-checked first.
pub(crate) unsafe fn equip(request: EquipRequest<'_>) -> Result<EquipOutcome, GameError> {
    let inventory = item_inventory()?;
    let bag = bag_list()?;

    // ALREADY WEARING IT? THEN DO NOTHING. Not merely an optimisation:
    //
    // * equipping is a MOVE, which strips the item from wherever it sits before writing the slot,
    //   so "re-equip what is already there" is a strip and a write for no gain;
    // * on a SPELL slot the strip compacts the attunement array downward and re-notifies every
    //   slot after it, so re-attuning in place disturbs spells the build never mentioned;
    // * and the player's own copy is the one with their upgrades on it. Touching it can only lose.
    let flat = flat_slot_for(request.internal_slot);
    if let Some(flat) = flat
        // SAFETY: game thread, per this function's contract.
        && let Ok(Some(current)) = unsafe { equipped_in_flat_slot(inventory, flat) }
        && request.item_ids.contains(&current)
    {
        return Ok(EquipOutcome {
            internal_slot: request.internal_slot,
            wanted: current,
            landed: Some(current),
        });
    }
    let site = game_rva(ds2_rva::ITEM_SET_EQUIP).map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::ITEM_SET_EQUIP_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::ITEM_SET_EQUIP_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }
    // The first candidate that is actually in the bag. See `EquipRequest::item_ids`.
    let (item_id, entry) = request
        .item_ids
        .iter()
        .find_map(|id| entry_for_item(bag, *id).map(|entry| (*id, entry)))
        .ok_or(GameError::NotInInventory)?;

    // SAFETY: the prologue matched, and the signature is the one the disassembled thunk implements
    // -- the inventory in RCX, an internal slot in EDX, an entry pointer in R8, no return. `entry`
    // is a live inventory entry the scan just read out of the game's own array.
    unsafe {
        let set: unsafe extern "system" fn(usize, u32, usize) = core::mem::transmute(site);
        set(inventory, request.internal_slot, entry);
    }

    // READ IT BACK, because this function fails silently in two ways: a category mismatch returns
    // having done nothing, and attuning past capacity unequips the slot instead. Neither says so.
    // SAFETY: game thread, per this function's contract.
    let landed = match flat {
        Some(flat) => unsafe { equipped_in_flat_slot(inventory, flat) }?,
        None => None,
    };
    Ok(EquipOutcome {
        internal_slot: request.internal_slot,
        wanted: item_id,
        landed,
    })
}

/// The covenant id a build's name means, or `None` if nothing is named.
///
/// Compared with [`ds2_build_import_core::normalise`], because the planner writes
/// `Brotherhood_of_Blood` and [`ds2_rva::COVENANT_NAMES`] holds `Brotherhood of Blood`.
pub(crate) fn covenant_id(name: &str) -> Option<u8> {
    let wanted = ds2_build_import_core::normalise(name);
    if wanted.is_empty() || wanted == "nocovenant" {
        return None;
    }
    ds2_rva::COVENANT_NAMES
        .iter()
        .position(|candidate| ds2_build_import_core::normalise(candidate) == wanted)
        .map(|id| id as u8)
}

/// `PlayerCtrl`, the receiver the covenant setter takes.
fn player_ctrl() -> Result<usize, GameError> {
    let manager = game_manager().ok_or(GameError::Unresolved)?;
    hop(manager, ds2_rva::PLAYER_CTRL_OFFSET).ok_or(GameError::NoCharacter)
}

/// What one covenant change did, read back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CovenantSet {
    pub(crate) before: u8,
    pub(crate) after: u8,
    /// Whether this character had already discovered the covenant BEFORE the call.
    ///
    /// The interesting case is `false`, because the discovered flag is the part that cannot be
    /// undone -- see [`ds2_rva::PLAYER_PARAM_COVENANT_DISCOVERED_BASE`].
    pub(crate) already_discovered: bool,
}

/// **Join a covenant, through the game's own path, WITHOUT announcing it to the session.**
///
/// # This one cannot be undone, and it is the second such thing in this crate
///
/// The setter marks the covenant permanently discovered on this character
/// ([`ds2_rva::PLAYER_PARAM_COVENANT_DISCOVERED_BASE`]) and nothing clears that flag -- leaving the
/// covenant later changes which one is current and leaves the mark. `already_discovered` is read
/// BEFORE the call so the log can say whether this press is what set it.
///
/// `announce` is passed FALSE. A true one runs the server sync and, in an online session, tells
/// everyone in it that this character changed covenant. A build import is not a covenant the player
/// walked up to and joined, and has no business saying so on the network.
///
/// # Safety
///
/// Calls into the game. **Game thread only.** The prologue is byte-checked first.
pub(crate) unsafe fn set_covenant(id: u8) -> Result<CovenantSet, GameError> {
    let ctrl = player_ctrl()?;
    let param = player_param()?;
    let site = game_rva(ds2_rva::PLAYER_CTRL_SET_COVENANT).map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::PLAYER_CTRL_SET_COVENANT_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::PLAYER_CTRL_SET_COVENANT_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }
    let read_current = || {
        // SAFETY: inside the block the game's own getter returned; fault-safe read.
        unsafe { ds2_game_base::mem::safe_read_u8(param + ds2_rva::PLAYER_PARAM_COVENANT_OFFSET) }
    };
    let before = read_current().ok_or(GameError::NoCharacter)?;
    // SAFETY: as above.
    let already_discovered = unsafe {
        ds2_game_base::mem::safe_read_u8(
            param + ds2_rva::PLAYER_PARAM_COVENANT_DISCOVERED_BASE + usize::from(id),
        )
    }
    .is_some_and(|flag| flag != 0);

    // SAFETY: the prologue matched, and the signature is the one the disassembly implements --
    // PlayerCtrl in RCX, the id in EDX, an announce flag in R8B, no return.
    unsafe {
        let set: unsafe extern "system" fn(usize, i32, bool) = core::mem::transmute(site);
        set(ctrl, i32::from(id), false);
    }

    Ok(CovenantSet {
        before,
        after: read_current().unwrap_or(before),
        already_discovered,
    })
}

/// `fn(u32 key) -> u8` -- the game's property-key lookup.
type EstusPropertyIndexFn = unsafe extern "system" fn(u32) -> u8;

/// `fn(ItemInventory2*, const u8* property) -> u8` -- a level, or a maxed-out flag.
///
/// The `bool`-returning members of this family are typed `u8` deliberately. A Rust `bool` holding
/// anything but `0` or `1` is undefined behaviour, and "the callee only ever `sete`s" is a promise
/// about someone else's code; comparing a byte costs nothing and cannot be wrong.
type EstusPropertyFn = unsafe extern "system" fn(usize, *const u8) -> u8;

/// `fn(ItemInventory2*) -> u8` -- the charge count.
type EstusChargesFn = unsafe extern "system" fn(usize) -> u8;

/// `fn(ItemInventory2*, const u8* property, i32 level) -> u8`.
type EstusSetFn = unsafe extern "system" fn(usize, *const u8, i32) -> u8;

/// `fn(ItemInventory2*)` -- the refill.
type EstusRefillFn = unsafe extern "system" fn(usize);

/// Resolve one of the Estus thunks, byte-checking [`ds2_rva::ESTUS_THUNK_PROLOGUE`] first.
///
/// All five share one prologue because they are the same two-hop shape, so they share one check.
fn estus_site(rva: u32) -> Result<usize, GameError> {
    let site = game_rva(rva).map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::ESTUS_THUNK_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::ESTUS_THUNK_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }
    Ok(site)
}

/// **The table index for one Estus property, asked of the game rather than assumed.**
///
/// The mapping is the identity on this build, so this could have been `key as u8`. It is a call
/// because the answer is what the setter INDEXES WITH, and the one wrong answer -- `0xFF` for a key
/// the table does not hold -- is sign-extended by the setter into `entry + 0x24`, the charge count.
/// A hardcoded index cannot notice that; this can, and refuses.
fn estus_property(key: u32) -> Result<u8, GameError> {
    let site = game_rva(ds2_rva::ESTUS_PROPERTY_INDEX).map_err(|_| GameError::Unresolved)?;
    let mut prologue = [0u8; ds2_rva::ESTUS_PROPERTY_INDEX_PROLOGUE.len()];
    // SAFETY: a resolved RVA in the loaded image; the read is fault-safe.
    if !unsafe { ds2_game_base::mem::read_bytes(site, &mut prologue) }
        || prologue != ds2_rva::ESTUS_PROPERTY_INDEX_PROLOGUE
    {
        return Err(GameError::PrologueMismatch);
    }
    // SAFETY: the prologue matched, and the disassembly is six instructions over a static table --
    // it takes a key in ECX, touches no game state at all, and returns a byte in AL.
    let index = unsafe {
        let lookup: EstusPropertyIndexFn = core::mem::transmute(site);
        lookup(key)
    };
    if index == ds2_rva::ESTUS_PROPERTY_NOT_FOUND {
        return Err(GameError::UnknownEstusProperty);
    }
    Ok(index)
}

/// What one [`max_estus`] call did, as before-and-after pairs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct EstusMaxed {
    /// The uses level -- what Estus Flask Shards buy.
    pub(crate) uses: (u8, u8),
    /// The effect level -- what Sublime Bone Dust buys.
    pub(crate) effect: (u8, u8),
    /// Charges in the flask right now.
    pub(crate) charges: (u8, u8),
    /// The game's own answer to "is the uses level finished", read after the call.
    pub(crate) uses_at_max: bool,
    /// The game's own answer for the effect level.
    pub(crate) effect_at_max: bool,
}

impl EstusMaxed {
    /// Whether anything moved. A flask already at maximum is a success that changed nothing.
    pub(crate) fn changed(&self) -> bool {
        self.uses.0 != self.uses.1
            || self.effect.0 != self.effect.1
            || self.charges.0 != self.charges.1
    }
}

/// **Take the Estus Flask to the maximum this game allows, through the game's own upgrade path.**
///
/// # This is the Emerald Herald's own function, without the Emerald Herald
///
/// Her script command is `get(level); set(level + 1)` on exactly these functions and nothing else;
/// the conversation is what triggers it, not what performs it. So there is no dialogue to open, no
/// shard to hold and none to spend -- the setter never looks at the inventory for one. Asking for
/// the maximum directly is the same operation eleven presses of her menu would perform, minus the
/// eleven presses.
///
/// # It asks for more than the maximum on purpose
///
/// [`ds2_rva::ESTUS_LEVEL_ASK`] is deliberately larger than any shipped maximum, because the setter
/// clamps into `EstusFlaskMaxReinforceParam`'s own range before it writes. Naming `12` here would
/// have turned the game's maximum into this mod's maximum on any install whose regulation raised it.
///
/// # Safety
///
/// Calls into the game. **Game thread only**, and only with a character loaded. Every prologue is
/// byte-checked first.
pub(crate) unsafe fn max_estus() -> Result<EstusMaxed, GameError> {
    let inventory = item_inventory()?;
    let uses = estus_property(ds2_rva::ESTUS_PROPERTY_USES)?;
    let effect = estus_property(ds2_rva::ESTUS_PROPERTY_EFFECT)?;

    // SAFETY for this block: every site's prologue is checked by `estus_site`, and each signature is
    // the one its disassembled thunk implements -- the inventory in RCX, where there is a property
    // a pointer to it in RDX, a level in R8D. `uses` and `effect` are locals that outlive the calls.
    let level: EstusPropertyFn =
        unsafe { core::mem::transmute(estus_site(ds2_rva::ESTUS_GET_LEVEL)?) };
    let charges: EstusChargesFn =
        unsafe { core::mem::transmute(estus_site(ds2_rva::ESTUS_GET_CHARGES)?) };
    let at_max: EstusPropertyFn =
        unsafe { core::mem::transmute(estus_site(ds2_rva::ESTUS_IS_MAX)?) };
    let set: EstusSetFn = unsafe { core::mem::transmute(estus_site(ds2_rva::ESTUS_SET_PROPERTY)?) };
    let refill: EstusRefillFn = unsafe { core::mem::transmute(estus_site(ds2_rva::ESTUS_REFILL)?) };

    // NO FLASK MEANS NO UPGRADE, and the game says so in the level itself: the getter answers zero
    // when the flask's slot is unbound, and the game's own add path seeds a new flask at level one.
    // Checking here rather than after the write keeps the caller's "grant one first" path honest.
    // SAFETY: as above.
    let uses_before = unsafe { level(inventory, &uses) };
    if uses_before == 0 {
        return Err(GameError::NoEstusFlask);
    }
    // SAFETY: as above.
    let (effect_before, charges_before) =
        unsafe { (level(inventory, &effect), charges(inventory)) };

    // SAFETY: as above. The setter's `bool` is deliberately dropped -- `false` from it means "the
    // level was already that", which is not a failure, and the read-back below is the real answer.
    unsafe {
        set(inventory, &uses, ds2_rva::ESTUS_LEVEL_ASK);
        set(inventory, &effect, ds2_rva::ESTUS_LEVEL_ASK);
        // The uses write already moved the charge count by the difference. This makes the result
        // the same whether or not the player had been drinking -- it is the bonfire's own refill.
        refill(inventory);
    }

    // SAFETY: as above.
    unsafe {
        Ok(EstusMaxed {
            uses: (uses_before, level(inventory, &uses)),
            effect: (effect_before, level(inventory, &effect)),
            charges: (charges_before, charges(inventory)),
            uses_at_max: at_max(inventory, &uses) != 0,
            effect_at_max: at_max(inventory, &effect) != 0,
        })
    }
}
