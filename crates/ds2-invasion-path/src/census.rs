//! Who else is in this session, and where they are standing.
//!
//! # How a player is told from a rat
//!
//! By the class of the object, which in a binary carrying 5271 RTTI type descriptors is a fact
//! rather than an inference. DARK SOULS II builds remote players out of the same `PlayerCtrl`
//! class as the local one -- two factories allocate `0x4a0` bytes and call the same constructor,
//! differing only in the name they format (`L"Player_%06u"` against `L"NetworkPlayer_%06u"` /
//! `L"GhostPlayer_%06u"`). So a player is exactly an object whose first eight bytes are
//! `PlayerCtrl`'s vtable, and everything else in the map is not.
//!
//! That is worth contrasting with what the Elden Ring crate had to do. There the roster is read
//! from a set the engine documents as holding players, with a **widening sweep** behind it for
//! when that set comes back empty, and two different trust rules depending on which of the two
//! found the character -- because the test available there is a `chr_type` byte, and an
//! uncatalogued type looks the same as a mistake. A live run there drew a permanent arrow at a
//! single `ChrType 7` sitting among 582 map NPCs. A vtable comparison cannot make that error:
//! either the object is that class or it is not.
//!
//! # Everything refuses rather than faults
//!
//! Every hop is null-checked and every read goes through `ds2-game-base`'s fault-safe readers,
//! which report an unmapped page instead of raising. This walks a container the game is
//! simultaneously mutating, in someone's live invasion; the correct response to a pointer that
//! has gone stale between two reads is to return a shorter roster this frame, not to take the
//! session down.

use ds2_game_base::mem::{game_rva, safe_read_f32, safe_read_usize};

/// One other player in the session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Player {
    /// The `PlayerCtrl` address. Used as the identity a colour is bound to -- see
    /// `crate::routes::Palette` for why that is weaker than an engine id and why it is enough.
    pub(crate) ctrl: usize,
    /// World position, metres, `y` up.
    pub(crate) position: [f32; 3],
    /// Straight-line distance from the local player, in **3D**.
    pub(crate) distance: f32,
}

/// What one roster read saw, including what it rejected.
///
/// The counts are not decoration. `remotes = 0` with a small `characters` is "you are alone";
/// `remotes = 0` with a large one is a bug in this crate, and the difference is invisible without
/// both numbers. The Elden Ring crate learned that from a log line it could not interpret.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Census {
    /// Entries walked in the roster.
    pub(crate) characters: usize,
    /// Of those, objects whose vtable is `PlayerCtrl`'s.
    pub(crate) players: usize,
    /// Of those, ones that are not the local player.
    pub(crate) remotes: usize,
    /// Entries skipped because a read failed or a pointer was null. A non-zero value here is the
    /// difference between "there was nobody" and "the walk gave up part way".
    pub(crate) skipped: usize,
}

/// The local player's `PlayerCtrl`, plus the roster bounds, or `None` before there is a world.
///
/// Returns `(player_ctrl, begin, end)`, all read off the same `GameManagerImp`, so there is no
/// window in which the roster is from one frame and the local player from another.
///
/// # The local player comes from `GameManagerImp`, not from `CharacterManager`
///
/// The Ghidra project labels `CharacterManager+0x50` `player_ctrl`, and that label is wrong --
/// `0x14035b6c5` stores `[manager + index*8 + 0x50] = character` with the index read out of the
/// character itself, which is a registry and not a pointer. Reading it as one answers with
/// whatever is in slot 0, which is usually a real character, which is precisely why the mislabel
/// survives being tried.
///
/// So this takes the hop `PLAYER_PARAM_GET` takes: [`ds2_rva::PLAYER_CTRL_OFFSET`] on
/// `GameManagerImp`, established from an eight-instruction function the repo already verified for
/// `ds2-build-import`. A field name in a curated type is a lead; a two-hop accessor whose whole
/// body you have read is a fact.
fn world() -> Option<(usize, usize, usize)> {
    let manager_address = game_rva(ds2_rva::GAME_MANAGER_IMP).ok()?;
    // SAFETY: a resolved RVA inside the loaded image; the reader reports an unmapped page rather
    // than faulting, which is the case at the very start of boot before the global is written.
    let game_manager = non_null(unsafe { safe_read_usize(manager_address)? })?;
    let characters = non_null(unsafe {
        safe_read_usize(game_manager + ds2_rva::GAME_MANAGER_CHARACTER_MANAGER_OFFSET)?
    })?;
    // Null at the title screen, which is an answer rather than a failure: there is no character
    // to draw from until someone loads one.
    let player = non_null(unsafe { safe_read_usize(game_manager + ds2_rva::PLAYER_CTRL_OFFSET)? })?;
    let begin =
        unsafe { safe_read_usize(characters + ds2_rva::CHARACTER_MANAGER_ENTITY_BEGIN_OFFSET)? };
    let end =
        unsafe { safe_read_usize(characters + ds2_rva::CHARACTER_MANAGER_ENTITY_END_OFFSET)? };
    Some((player, begin, end))
}

/// `Some(pointer)` unless it is null.
const fn non_null(pointer: usize) -> Option<usize> {
    if pointer == 0 { None } else { Some(pointer) }
}

/// Read a character's world position, refusing anything that is not three real numbers.
///
/// The offset is the one the engine's own accessor uses -- slot `+0x148` of the `CharacterCtrl`
/// vtable is a function whose entire body copies sixteen bytes from `this+0x90`.
///
/// This reads the field rather than calling that function, which is a deliberate departure from
/// bd `ds2-call-the-games-own-functions`. The rule exists for state the engine maintains
/// invariants over -- inventory slots, spell attunement, souls -- where a hand-built write
/// corrupts something nobody catalogued. Here the "function" is four `mov`s with no branch, no
/// invariant and no side effect, and it is virtual: calling it would mean reading the vtable,
/// indexing it, and calling through a pointer into a module the game may have relocated, to do
/// the copy this does directly. That is more ways to be wrong, not fewer.
fn position(character: usize) -> Option<[f32; 3]> {
    let base = character + ds2_rva::CHARACTER_CTRL_POSITION_OFFSET;
    let mut out = [0.0f32; 3];
    for (index, slot) in out.iter_mut().enumerate() {
        // SAFETY: `character` came out of the engine's own roster and is null-checked; the read
        // is fault-safe regardless of whether the object has since been freed.
        *slot = unsafe { safe_read_f32(base + index * 4)? };
        if !slot.is_finite() {
            return None;
        }
    }
    Some(out)
}

/// Where the local player is, or `None` before there is one.
pub(crate) fn local_position() -> Option<[f32; 3]> {
    let (player, _, _) = world()?;
    position(player)
}

/// Most roster entries one pass will walk.
///
/// The roster is a begin/end pointer pair read out of live memory, so its length is a
/// subtraction of two numbers that were briefly inconsistent if the game resized the container
/// between the two reads. A torn pair can describe a span of gigabytes. Walking it would be a
/// multi-second stall inside a frame; refusing it costs one skipped pass.
///
/// DARK SOULS II's largest maps hold a few hundred characters -- the Elden Ring crate's own logs
/// record 584 in one sweep -- so this is roughly an order of magnitude of headroom over anything
/// real.
pub(crate) const MAX_ROSTER: usize = 8192;

/// One slot of the roster, as [`walk_roster`] found it.
enum Entry {
    /// The slot's pointer or the object's vtable could not be read. Counts as skipped: the
    /// container was resized under the walk, or the object has been freed since.
    Unreadable,
    /// A null slot. An ordinary hole in the roster, not a failure, and not counted as one.
    Empty,
    /// A live object and its primary vtable.
    Object { ctrl: usize, vtable: usize },
}

/// Walk the roster's begin/end span, handing each slot to `visit`. `false` means the span itself
/// was refused and nothing was walked.
///
/// Shared by [`remotes`] and [`nearest_npc`] so that the part that is dangerous to get wrong --
/// the bounds -- exists once. The part that differs between them is a vtable comparison, which
/// is the safe part.
fn walk_roster(begin: usize, end: usize, mut visit: impl FnMut(Entry)) -> bool {
    // A begin/end pair, not a pointer and a count -- three independent iteration sites in the
    // image read it that way. A pair that is inverted, misaligned or absurdly long is a torn
    // read rather than a roster, and is refused outright.
    if begin == 0 || end < begin || !(end - begin).is_multiple_of(core::mem::size_of::<usize>()) {
        return false;
    }
    let count = (end - begin) / core::mem::size_of::<usize>();
    if count > MAX_ROSTER {
        return false;
    }
    for index in 0..count {
        // SAFETY: `begin` is the engine's own array base and `index` is inside the span it
        // declared; the read is fault-safe if the container was resized under us.
        let Some(character) = (unsafe { safe_read_usize(begin + index * 8) }) else {
            visit(Entry::Unreadable);
            continue;
        };
        let Some(character) = non_null(character) else {
            visit(Entry::Empty);
            continue;
        };
        // SAFETY: as above -- the object's first word is its vtable, and a freed object reads as
        // a refusal rather than a fault.
        let Some(vtable) = (unsafe { safe_read_usize(character) }) else {
            visit(Entry::Unreadable);
            continue;
        };
        visit(Entry::Object {
            ctrl: character,
            vtable,
        });
    }
    true
}

/// Every other player in the session, nearest first.
///
/// `max` caps the returned list, but the whole roster is still walked and counted, so the
/// [`Census`] reports what was there rather than what was kept.
pub(crate) fn remotes(max: usize) -> Option<(Vec<Player>, Census)> {
    let (local, begin, end) = world()?;
    let local_position = position(local)?;
    let player_vtable = game_rva(ds2_rva::PLAYER_CTRL_VTABLE).ok()?;

    let mut census = Census::default();
    let mut found: Vec<Player> = Vec::new();

    walk_roster(begin, end, |entry| {
        census.characters += 1;
        let Entry::Object { ctrl, vtable } = entry else {
            if matches!(entry, Entry::Unreadable) {
                census.skipped += 1;
            }
            return;
        };
        if vtable != player_vtable {
            return;
        }
        census.players += 1;
        if ctrl == local {
            return;
        }
        census.remotes += 1;
        let Some(position) = position(ctrl) else {
            census.skipped += 1;
            return;
        };
        let distance = crate::geometry::length(crate::geometry::sub(position, local_position));
        if !distance.is_finite() {
            census.skipped += 1;
            return;
        }
        found.push(Player {
            ctrl,
            position,
            distance,
        });
    });

    // Nearest first, so a `max` that truncates keeps the players worth pointing at. `total_cmp`
    // rather than `partial_cmp`: every distance here is already finite, and a comparator that
    // can return `None` needs an arm that cannot happen, which is a worse thing to read.
    found.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    found.truncate(max);
    Some((found, census))
}

/// The nearest non-player character in the map, or `None` when there is none.
///
/// # Why this exists, and why it is not a feature
///
/// A solo player has `remotes=0` forever, so every line the route and the trail can write is an
/// install line and none of them is an execution line. This gives the self-check a real target:
/// an NPC is a live object at a real world position, standing on the navmesh, so routing to one
/// runs the whole chain -- snap, request, poll, decode, space, spawn -- in the state the game is
/// actually in while this is being developed.
///
/// # Telling an NPC from a player is the same comparison, with the other constant
///
/// [`ds2_rva::CHARACTER_CTRL_VTABLE`] against [`ds2_rva::PLAYER_CTRL_VTABLE`], and the position
/// comes from the same [`ds2_rva::CHARACTER_CTRL_POSITION_OFFSET`] -- that offset's own
/// documentation records that `PlayerCtrl` inherits the accessor unchanged, so one offset serves
/// both. An exact vtable match, not a subclass test: a `CharacterCtrl` SUBCLASS that is not this
/// class will not match, and that is the conservative direction. It means some NPCs are invisible
/// to this rather than some non-characters being mistaken for one.
///
/// Returns a [`Player`] because everything downstream -- the route request, the colour slot, the
/// drawn line -- only needs a pointer, a position and a distance, and inventing a second
/// near-identical struct would mean two of every function that touches one.
///
/// # `latched` is what keeps the experiment still
///
/// Pass the address the self-check picked last time and this re-finds THAT object and reports
/// where it is now, falling back to a fresh nearest pick only once it has left the roster.
///
/// Without the latch the target is whichever NPC is nearest this frame, and two NPCs milling
/// around each other would swap the route's destination several times a second -- re-planning
/// constantly, re-laying the trail, and producing a log in which no two lines are about the same
/// thing. A latched target also means a walking NPC stays the target while it walks, which is
/// the case worth watching: it is the only way to see the trail re-lay itself.
///
/// The latch is re-found by WALKING the roster rather than by dereferencing the stored address.
/// An address alone is freed memory the moment the NPC despawns, and "is this pointer still in
/// the engine's own list" is a question only the list can answer.
pub(crate) fn self_check_target(latched: Option<usize>) -> Option<Player> {
    let (local, begin, end) = world()?;
    let local_position = position(local)?;
    let character_vtable = game_rva(ds2_rva::CHARACTER_CTRL_VTABLE).ok()?;

    let mut held: Option<Player> = None;
    let mut best: Option<Player> = None;
    walk_roster(begin, end, |entry| {
        let Entry::Object { ctrl, vtable } = entry else {
            return;
        };
        if vtable != character_vtable || ctrl == local {
            return;
        }
        let Some(position) = position(ctrl) else {
            return;
        };
        let distance = crate::geometry::length(crate::geometry::sub(position, local_position));
        if !distance.is_finite() {
            return;
        }
        let candidate = Player {
            ctrl,
            position,
            distance,
        };
        if Some(ctrl) == latched {
            held = Some(candidate);
        }
        if best.as_ref().is_none_or(|kept| distance < kept.distance) {
            best = Some(candidate);
        }
    });
    held.or(best)
}
