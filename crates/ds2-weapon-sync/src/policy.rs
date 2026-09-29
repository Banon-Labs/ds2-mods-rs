//! The decisions, with no game in them: what the cap is, what a level becomes under it, and when
//! the character's weapons have to be pushed through the game's weapon update again.
//!
//! Everything here is arithmetic over numbers the Windows half reads, so it builds and is tested
//! on the machine the code is written on.

/// The highest level the packet 61 receiver accepts. A byte above it is not a level.
pub const WEAPON_LEVEL_MAX: u8 = ds2_rva::WEAPON_LEVEL_MAX;

/// One remote player's six weapon records, as `(item id, level byte)`.
pub type RemoteWeapons = [(u32, u8); 6];

/// The highest weapon level any one remote player has equipped, or `None` when this player's
/// records say nothing yet.
///
/// A record with item id `0` or `u32::MAX` is unset, not a +0 weapon. A player whose six records
/// are all unset is left out of the cap rather than counted as +0: the join data has not been
/// traced (docs/DS2-WEAPON-LEVEL-SYNC.md, Q2), and reading "not arrived yet" as "+0" would lower
/// every weapon to +0 for however long it takes to arrive. Fists count, at their own level `0`,
/// because an empty hand is a real answer.
///
/// Level bytes are masked to the low nibble (the receiver stores the nibble) and anything above
/// [`WEAPON_LEVEL_MAX`] is ignored.
pub fn remote_highest(records: &RemoteWeapons) -> Option<u8> {
    records
        .iter()
        .filter(|(item, _)| *item != 0 && *item != u32::MAX)
        .map(|(_, level)| level & 0x0f)
        .filter(|level| *level <= WEAPON_LEVEL_MAX)
        .max()
}

/// The cap: the highest level among every other player, or `None` when nobody else is in the
/// world (or nobody else's records have arrived).
///
/// `test_cap` stands in for a remote player at that level, so the mechanism can be exercised solo.
/// It takes part in the maximum like any other player: it is not an override.
pub fn cap(remotes: &[RemoteWeapons], test_cap: Option<u8>) -> Option<u8> {
    remotes
        .iter()
        .filter_map(remote_highest)
        .chain(test_cap.map(|level| level.min(WEAPON_LEVEL_MAX)))
        .max()
}

/// The cap after the on/off key: switched off, there is never a cap, whoever is in the world. The
/// tracker then sees the cap go away and restores, exactly as it does when the last player leaves.
pub fn effective(enabled: bool, cap: Option<u8>) -> Option<u8> {
    if enabled { cap } else { None }
}

/// What one of our levels becomes. Never above the real level, so the cap only ever lowers.
pub fn clamp(real: u8, cap: Option<u8>) -> u8 {
    match cap {
        Some(cap) if real > cap => cap,
        _ => real,
    }
}

/// What the per-frame check asks the Windows half to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing changed.
    Nothing,
    /// Push every equipped weapon through the game's weapon update again, each at
    /// `clamp(real, cap)`. `cap == None` is the restore.
    Redrive {
        /// The cap to apply, or `None` to restore the real levels.
        cap: Option<u8>,
    },
}

/// Remembers which cap the character's weapons were last pushed at, and for which character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracker {
    /// The `PlayerCtrl` the last push was for. `0` when there is no character.
    player: usize,
    /// The cap the last push applied. `None` means real levels.
    applied: Option<u8>,
}

impl Tracker {
    /// A tracker that has pushed nothing.
    pub const fn new() -> Self {
        Self {
            player: 0,
            applied: None,
        }
    }

    /// The cap the character currently carries, as far as this tracker knows.
    pub const fn applied(&self) -> Option<u8> {
        self.applied
    }

    /// One check. `player` is the local `PlayerCtrl` (`0` when there is none) and `cap` is what
    /// [`cap`] answered this frame.
    ///
    /// * No character: forget everything and do nothing. The character is gone (title, load
    ///   screen), and the next one is built from the inventory, which the cap never wrote.
    /// * A different character than last time: a new world. The rebuild went through the weapon
    ///   update, which clamps at whatever cap was current, so push once whenever either the old
    ///   push or the new cap is a cap. Real levels with no cap need nothing.
    /// * The same character, a different cap (lower, higher or gone): push at the new one.
    pub fn step(&mut self, player: usize, cap: Option<u8>) -> Action {
        if player == 0 {
            *self = Self::new();
            return Action::Nothing;
        }
        if player != self.player {
            let stale = self.applied.is_some() || cap.is_some();
            self.player = player;
            self.applied = cap;
            return if stale {
                Action::Redrive { cap }
            } else {
                Action::Nothing
            };
        }
        if cap == self.applied {
            return Action::Nothing;
        }
        self.applied = cap;
        Action::Redrive { cap }
    }
}

// -------------------------------------------------------------------------------------------------
// The whole inventory.
// -------------------------------------------------------------------------------------------------

/// One weapon or shield in the inventory's entry array, as a check read it.
///
/// The entry's index is also its handle and its record's index in the save block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    /// The entry's index in the bag's array.
    pub index: u16,
    /// `ItemParam` row id.
    pub item: u32,
    /// The level the entry holds now, low nibble.
    pub level: u8,
}

/// One level the check has to write into an inventory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Write {
    /// The entry's index.
    pub index: u16,
    /// The item the entry held when it was read. The writer checks it again before writing.
    pub item: u32,
    /// The level the entry holds.
    pub from: u8,
    /// The level it is to hold.
    pub to: u8,
}

impl Write {
    /// Whether this write puts a real level back. The cap only lowers, so a raise is a restore.
    pub const fn restores(&self) -> bool {
        self.to > self.from
    }
}

/// What the ledger remembers about one entry it lowered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Kept {
    item: u32,
    real: u8,
    written: u8,
}

/// Every inventory entry the cap has lowered, with its real level, so the restore and the save
/// know what the entry really is.
///
/// Keyed by the entry's index, and trusted only while the entry still holds the same item at the
/// level this ledger wrote. Anything else means the game wrote the entry itself since -- a
/// different weapon in the slot, a reload from the save, an upgrade -- and what the game wrote is
/// the real level from then on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ledger {
    kept: std::collections::BTreeMap<u16, Kept>,
}

impl Ledger {
    /// A ledger that has lowered nothing.
    pub const fn new() -> Self {
        Self {
            kept: std::collections::BTreeMap::new(),
        }
    }

    /// Whether nothing in the inventory is lowered.
    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    /// How many entries are lowered.
    pub fn len(&self) -> usize {
        self.kept.len()
    }

    /// The real level of entry `index`, if the ledger lowered it and it still holds `item`.
    pub fn real(&self, index: u16, item: u32) -> Option<u8> {
        self.kept
            .get(&index)
            .filter(|kept| kept.item == item)
            .map(|kept| kept.real)
    }

    /// One pass over every weapon in the inventory: lower each one above `cap` to it, put back
    /// every one the ledger lowered that `cap` no longer lowers (all of them when `cap` is
    /// `None`), and forget entries that are gone. Idempotent: a second pass over what the first
    /// one left writes nothing.
    ///
    /// `held` must be a complete read of the array. An entry missing from it is taken to be gone
    /// (dropped, sold, replaced), so a failed read must not be passed in as an empty one.
    pub fn sweep(&mut self, held: &[Held], cap: Option<u8>) -> Vec<Write> {
        let mut writes = Vec::new();
        let mut next = std::collections::BTreeMap::new();
        for entry in held {
            let real = match self.kept.get(&entry.index) {
                Some(kept) if kept.item == entry.item && kept.written == entry.level => kept.real,
                _ => entry.level,
            };
            let target = clamp(real, cap);
            if target != entry.level {
                writes.push(Write {
                    index: entry.index,
                    item: entry.item,
                    from: entry.level,
                    to: target,
                });
            }
            if target != real {
                next.insert(
                    entry.index,
                    Kept {
                        item: entry.item,
                        real,
                        written: target,
                    },
                );
            }
        }
        self.kept = next;
        writes
    }

    /// The level the save must write for block record `index`, when it holds `item` at `level`:
    /// the real one, if the ledger lowered that entry and the record does not already carry it.
    pub fn saved_level(&self, index: u16, item: u32, level: u8) -> Option<u8> {
        self.real(index, item).filter(|real| *real != level)
    }

    /// Every entry the ledger lowered, as `(index, item, real level)`.
    pub fn lowered(&self) -> impl Iterator<Item = (u16, u32, u8)> + '_ {
        self.kept
            .iter()
            .map(|(index, kept)| (*index, kept.item, kept.real))
    }
}

// -------------------------------------------------------------------------------------------------
// The equipped slots.
// -------------------------------------------------------------------------------------------------

/// One equipped weapon slot, in inventory slot order: what the inventory entry says, and the two
/// copies the character carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    /// The equipped inventory entry's `(item, level)`, `None` when the slot is empty.
    pub inventory: Option<(u32, u8)>,
    /// The equipment record's `(item, level)`.
    pub record: Option<(u32, u8)>,
    /// The live weapon state's level.
    pub live: Option<u8>,
}

impl Slot {
    /// Whether the character carries exactly what the inventory entry says. An empty slot is
    /// never pushed: the game puts Fists there itself, and there is nothing to cap.
    pub fn carried(&self) -> bool {
        match self.inventory {
            None => true,
            Some((item, level)) => self.record == Some((item, level)) && self.live == Some(level),
        }
    }
}

/// The six equipped weapon slots at one check.
pub type Equipped = [Slot; 6];

/// The slots a resweep pushes: every slot whose character copies do not match its inventory
/// entry, over all six, whichever one changed.
pub fn resweep(equipped: &Equipped) -> Vec<usize> {
    (0..equipped.len())
        .filter(|slot| !equipped[*slot].carried())
        .collect()
}

/// Watches the equipped slots while a cap is on, and says when they changed since the last
/// resweep, for any reason: a swap from the inventory, an unequip, the game's own weapon update,
/// a load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    settled: Option<Equipped>,
}

impl Watch {
    /// A watch that has seen nothing.
    pub const fn new() -> Self {
        Self { settled: None }
    }

    /// One check. With a cap on, answers the slots as they were after the last resweep when
    /// `now` differs from them, which is the caller's cue to resweep and then [`Watch::settle`].
    /// The first check under a cap answers `None` for the before, and still asks for a resweep.
    /// With no cap it forgets, so the next cap starts from a fresh look.
    pub fn changed(&mut self, now: &Equipped, cap: Option<u8>) -> Option<Option<Equipped>> {
        if cap.is_none() {
            self.settled = None;
            return None;
        }
        match self.settled {
            Some(settled) if settled == *now => None,
            before => Some(before),
        }
    }

    /// Remember what the slots look like after a resweep, so only a later change asks again. A
    /// slot the resweep could not bring into line is not retried until something changes, so a
    /// copy that refuses a level is one push, not a push every check.
    pub fn settle(&mut self, after: Equipped) {
        self.settled = Some(after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAGGER: u32 = 1_000_000;
    const SHIELD: u32 = 2_750_000;
    const FISTS: u32 = 3_400_000;

    fn player(levels: [(u32, u8); 6]) -> RemoteWeapons {
        levels
    }

    #[test]
    fn the_cap_is_the_highest_level_any_other_player_has() {
        // The user's example: host +0, phantom +9 -> 9.
        let host = player([
            (DAGGER, 0),
            (FISTS, 0),
            (FISTS, 0),
            (SHIELD, 0),
            (FISTS, 0),
            (FISTS, 0),
        ]);
        let phantom = player([
            (DAGGER, 9),
            (FISTS, 0),
            (FISTS, 0),
            (SHIELD, 3),
            (FISTS, 0),
            (FISTS, 0),
        ]);
        assert_eq!(cap(&[host, phantom], None), Some(9));
        assert_eq!(cap(&[host], None), Some(0));
    }

    #[test]
    fn nobody_else_means_no_cap() {
        assert_eq!(cap(&[], None), None);
    }

    #[test]
    fn a_player_whose_records_have_not_arrived_is_not_a_plus_zero_player() {
        let unset = player([(0, 0), (u32::MAX, 0), (0, 0), (0, 0), (0, 0), (0, 0)]);
        assert_eq!(remote_highest(&unset), None);
        assert_eq!(cap(&[unset], None), None);
        let phantom = player([(DAGGER, 5), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)]);
        assert_eq!(cap(&[unset, phantom], None), Some(5));
    }

    #[test]
    fn empty_hands_count_at_zero() {
        let unarmed = player([(FISTS, 0); 6]);
        assert_eq!(remote_highest(&unarmed), Some(0));
    }

    #[test]
    fn level_bytes_are_nibbles_and_out_of_range_ones_are_ignored() {
        // Infusion lives in the next byte, but a high nibble here must still not raise the cap.
        let odd = player([
            (DAGGER, 0x35),
            (SHIELD, 0x0c),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
        ]);
        assert_eq!(remote_highest(&odd), Some(5));
    }

    #[test]
    fn the_test_cap_is_one_more_player_not_an_override() {
        assert_eq!(cap(&[], Some(3)), Some(3));
        let phantom = player([(DAGGER, 7), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)]);
        assert_eq!(cap(&[phantom], Some(3)), Some(7));
        assert_eq!(cap(&[], Some(200)), Some(WEAPON_LEVEL_MAX));
    }

    #[test]
    fn clamp_only_ever_lowers() {
        assert_eq!(clamp(10, Some(9)), 9);
        assert_eq!(clamp(3, Some(9)), 3, "a +3 dagger stays +3 under a +9 cap");
        assert_eq!(clamp(9, Some(9)), 9);
        assert_eq!(clamp(10, Some(0)), 0);
        assert_eq!(clamp(10, None), 10);
        for real in 0..=WEAPON_LEVEL_MAX {
            for cap in 0..=WEAPON_LEVEL_MAX {
                assert!(clamp(real, Some(cap)) <= real);
            }
        }
    }

    #[test]
    fn a_cap_arriving_pushes_once_and_a_steady_cap_pushes_nothing() {
        let mut t = Tracker::new();
        assert_eq!(
            t.step(0x100, None),
            Action::Nothing,
            "alone in our own world"
        );
        assert_eq!(t.step(0x100, Some(9)), Action::Redrive { cap: Some(9) });
        assert_eq!(t.step(0x100, Some(9)), Action::Nothing);
        assert_eq!(t.applied(), Some(9));
    }

    #[test]
    fn a_cap_that_moves_either_way_pushes_again() {
        let mut t = Tracker::new();
        t.step(0x100, Some(0));
        assert_eq!(t.step(0x100, Some(5)), Action::Redrive { cap: Some(5) });
        assert_eq!(t.step(0x100, Some(2)), Action::Redrive { cap: Some(2) });
    }

    #[test]
    fn the_encounter_ending_in_the_same_world_restores() {
        // A phantom leaves our world while we host: no reload, so the restore is ours to do.
        let mut t = Tracker::new();
        t.step(0x100, Some(3));
        assert_eq!(t.step(0x100, None), Action::Redrive { cap: None });
        assert_eq!(t.step(0x100, None), Action::Nothing);
    }

    #[test]
    fn losing_the_character_forgets_and_pushes_nothing() {
        // Quit to title, or the load screen after dying: there is no character to push.
        let mut t = Tracker::new();
        t.step(0x100, Some(3));
        assert_eq!(t.step(0, Some(3)), Action::Nothing);
        assert_eq!(t.applied(), None);
    }

    #[test]
    fn a_new_world_after_a_capped_one_restores_even_with_no_gap() {
        // An invader sent home: the PlayerCtrl changes between two checks with no null frame in
        // between, and the rebuild may have gone through the clamp at the old cap.
        let mut t = Tracker::new();
        t.step(0x100, Some(3));
        assert_eq!(t.step(0x200, None), Action::Redrive { cap: None });
        assert_eq!(t.step(0x200, None), Action::Nothing);
    }

    #[test]
    fn a_new_world_that_is_already_capped_pushes_at_its_cap() {
        let mut t = Tracker::new();
        assert_eq!(t.step(0x200, Some(4)), Action::Redrive { cap: Some(4) });
    }

    #[test]
    fn switching_off_while_capped_restores_and_switching_on_caps_again() {
        let mut t = Tracker::new();
        let remote = Some(3);
        assert_eq!(
            t.step(0x100, effective(true, remote)),
            Action::Redrive { cap: Some(3) }
        );
        assert_eq!(
            t.step(0x100, effective(false, remote)),
            Action::Redrive { cap: None },
            "off restores even with the other player still here"
        );
        assert_eq!(t.step(0x100, effective(false, remote)), Action::Nothing);
        assert_eq!(
            t.step(0x100, effective(true, remote)),
            Action::Redrive { cap: Some(3) }
        );
    }

    #[test]
    fn switching_on_alone_changes_nothing() {
        let mut t = Tracker::new();
        assert_eq!(t.step(0x100, effective(false, None)), Action::Nothing);
        assert_eq!(t.step(0x100, effective(true, None)), Action::Nothing);
    }

    #[test]
    fn a_new_uncapped_world_after_an_uncapped_one_needs_nothing() {
        let mut t = Tracker::new();
        t.step(0x100, None);
        assert_eq!(t.step(0x200, None), Action::Nothing);
    }

    // ---------------------------------------------------------------------------------------------
    // The whole inventory and the equipped slots, against a pretend game.
    // ---------------------------------------------------------------------------------------------

    use std::collections::BTreeMap;

    /// A game with an inventory, a save block, six weapon slots, and the two ways a slot can
    /// change: through the game's weapon update (which our detour clamps), and a path that skips
    /// it and carries the entry's real level, the worst case the resweep has to catch.
    struct Game {
        /// Entry index -> (item, level the entry holds now).
        bag: BTreeMap<u16, (u32, u8)>,
        /// Entry index -> the real level, as the test knows it.
        truth: BTreeMap<u16, u8>,
        /// The save block's record per entry: (item, level).
        block: BTreeMap<u16, (u32, u8)>,
        /// Which entry each inventory weapon slot holds.
        equipped: [Option<u16>; 6],
        /// What the character carries per slot: (item, record level, live level).
        carried: [Option<(u32, u8, u8)>; 6],
        /// The cap the weapon update detour applies.
        detour_cap: Option<u8>,
        /// Whether the game copies an entry's level into its save record whenever it equips it,
        /// which would carry a lowered level into the save if the save were not fixed.
        leaks_into_block: bool,
        ledger: Ledger,
        watch: Watch,
        tracker: Tracker,
        resweeps: usize,
    }

    const PLAYER: usize = 0x100;

    impl Game {
        fn new() -> Self {
            Self {
                bag: BTreeMap::new(),
                truth: BTreeMap::new(),
                block: BTreeMap::new(),
                equipped: [None; 6],
                carried: [None; 6],
                detour_cap: None,
                leaks_into_block: true,
                ledger: Ledger::new(),
                watch: Watch::new(),
                tracker: Tracker::new(),
                resweeps: 0,
            }
        }

        fn pick_up(&mut self, index: u16, item: u32, level: u8) {
            self.bag.insert(index, (item, level));
            self.truth.insert(index, level);
            self.block.insert(index, (item, level));
        }

        /// The game equips `index` into `slot` through its own weapon update.
        fn equip(&mut self, slot: usize, index: u16) {
            let (item, level) = self.bag[&index];
            let level = clamp(level, self.detour_cap);
            self.equipped[slot] = Some(index);
            self.carried[slot] = Some((item, level, level));
            if self.leaks_into_block {
                self.block.insert(index, (item, self.bag[&index].1));
            }
        }

        /// Something equips `index` into `slot` without the weapon update, at its real level.
        fn equip_around_the_update(&mut self, slot: usize, index: u16) {
            let (item, _) = self.bag[&index];
            let real = self.truth[&index];
            self.equipped[slot] = Some(index);
            self.carried[slot] = Some((item, real, real));
        }

        fn unequip(&mut self, slot: usize) {
            self.equipped[slot] = None;
            self.carried[slot] = None;
        }

        fn slots(&self) -> Equipped {
            let mut out = [Slot::default(); 6];
            for (slot, out) in out.iter_mut().enumerate() {
                out.inventory = self.equipped[slot].map(|index| self.bag[&index]);
                out.record = self.carried[slot].map(|(item, level, _)| (item, level));
                out.live = self.carried[slot].map(|(_, _, live)| live);
            }
            out
        }

        /// One check, in the order the tick runs it.
        fn check(&mut self, cap: Option<u8>) {
            let redrive = self.tracker.step(PLAYER, cap) != Action::Nothing;
            self.detour_cap = cap;
            let held: Vec<Held> = self
                .bag
                .iter()
                .map(|(index, (item, level))| Held {
                    index: *index,
                    item: *item,
                    level: *level,
                })
                .collect();
            for write in self.ledger.sweep(&held, cap) {
                let entry = self.bag.get_mut(&write.index).unwrap();
                assert_eq!(entry.0, write.item);
                entry.1 = write.to;
                if write.restores() {
                    self.block.insert(write.index, (write.item, write.to));
                }
            }
            let now = self.slots();
            let watched = self.watch.changed(&now, cap).is_some();
            if redrive || watched {
                self.resweeps += 1;
                for slot in resweep(&now) {
                    let (item, level) = now[slot].inventory.unwrap();
                    self.carried[slot] = Some((item, level, level));
                }
                let after = self.slots();
                self.watch.settle(after);
            }
        }

        /// The save: what `SaveDataItemInventory2` streams, after the detour fixed the block.
        fn save(&mut self) -> BTreeMap<u16, (u32, u8)> {
            for (index, record) in &mut self.block {
                if let Some(real) = self.ledger.saved_level(*index, record.0, record.1) {
                    record.1 = real;
                }
            }
            self.block.clone()
        }

        fn assert_capped(&self, cap: u8, step: &str) {
            for (index, (_, level)) in &self.bag {
                assert!(
                    *level <= cap,
                    "{step}: inventory entry {index} at +{level} over +{cap}"
                );
            }
            for (slot, carried) in self.carried.iter().enumerate() {
                if let Some((_, record, live)) = carried {
                    assert!(
                        *record <= cap && *live <= cap,
                        "{step}: slot {slot} carries +{record}/+{live} over +{cap}"
                    );
                }
            }
        }

        fn assert_real(&self, step: &str) {
            for (index, (_, level)) in &self.bag {
                assert_eq!(*level, self.truth[index], "{step}: inventory entry {index}");
            }
            for slot in 0..6 {
                if let Some(index) = self.equipped[slot] {
                    let (item, record, live) = self.carried[slot].unwrap();
                    let real = self.truth[&index];
                    assert_eq!(
                        (item, record, live),
                        (self.bag[&index].0, real, real),
                        "{step}: slot {slot}"
                    );
                }
            }
        }

        fn assert_saved_real(saved: &BTreeMap<u16, (u32, u8)>, truth: &BTreeMap<u16, u8>) {
            for (index, (_, level)) in saved {
                assert_eq!(*level, truth[index], "saved record {index}");
            }
        }
    }

    const CLUB: u32 = 1_500_000;
    const LONGSWORD: u32 = 1_140_000;
    const STAFF: u32 = 3_800_000;
    const RAPIER: u32 = 1_010_000;

    #[test]
    fn every_weapon_is_capped_through_every_swap_and_every_one_comes_back() {
        let mut g = Game::new();
        g.pick_up(0, DAGGER, 10);
        g.pick_up(1, SHIELD, 5);
        g.pick_up(2, CLUB, 3);
        g.pick_up(3, LONGSWORD, 10);
        g.pick_up(4, STAFF, 7); // in the pack, never equipped before the cap
        g.pick_up(5, RAPIER, 10); // put away in the box
        g.pick_up(6, FISTS - 1, 0); // already at +0: never touched
        for (slot, index) in [(0, 0), (1, 1), (2, 2), (3, 3)] {
            g.equip(slot, index);
        }
        g.check(None);
        g.assert_real("alone");
        assert_eq!(g.save(), g.block, "nothing lowered, nothing to fix");

        let cap = Some(0);
        g.check(cap);
        g.assert_capped(0, "cap on");
        assert_eq!(
            g.ledger.len(),
            6,
            "every weapon above +0 is lowered, equipped or not"
        );

        g.equip(1, 4);
        g.check(cap);
        g.assert_capped(0, "swap slot 1 to a pack weapon");

        g.equip_around_the_update(3, 5);
        g.assert_eq_before_check_is_over_cap(0);
        g.check(cap);
        g.assert_capped(0, "swap slot 3 around the weapon update");

        g.unequip(0);
        g.check(cap);
        g.assert_capped(0, "unequip slot 0");

        g.equip_around_the_update(2, 0);
        g.check(cap);
        g.assert_capped(0, "swap slot 2 to the weapon slot 0 held");

        g.pick_up(7, DAGGER + 1, 9);
        g.check(cap);
        g.assert_capped(0, "a weapon picked up mid-cap");

        let saved = g.save();
        Game::assert_saved_real(&saved, &g.truth);
        g.assert_capped(0, "a save does not lift the cap in the game");

        g.check(None);
        g.assert_real("cap off");
        assert!(g.ledger.is_empty());
        Game::assert_saved_real(&g.save(), &g.truth);
        assert!(g.resweeps >= 6, "every change was resweept: {}", g.resweeps);

        g.check(None);
        g.assert_real("steady after the restore");
    }

    impl Game {
        /// The worst case really did carry a level over the cap before the check caught it.
        fn assert_eq_before_check_is_over_cap(&self, cap: u8) {
            assert!(
                self.carried
                    .iter()
                    .flatten()
                    .any(|(_, record, _)| *record > cap),
                "the scripted swap should have carried a level over the cap"
            );
        }
    }

    #[test]
    fn a_sweep_twice_writes_nothing_the_second_time() {
        let mut ledger = Ledger::new();
        let mut held = vec![
            Held {
                index: 0,
                item: DAGGER,
                level: 10,
            },
            Held {
                index: 1,
                item: SHIELD,
                level: 2,
            },
        ];
        let writes = ledger.sweep(&held, Some(3));
        assert_eq!(
            writes,
            vec![Write {
                index: 0,
                item: DAGGER,
                from: 10,
                to: 3
            }]
        );
        held[0].level = 3;
        assert!(ledger.sweep(&held, Some(3)).is_empty());
        assert_eq!(ledger.real(0, DAGGER), Some(10));
        let back = ledger.sweep(&held, None);
        assert_eq!(
            back,
            vec![Write {
                index: 0,
                item: DAGGER,
                from: 3,
                to: 10
            }]
        );
        assert!(back[0].restores());
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_level_the_game_wrote_itself_is_the_real_one_from_then_on() {
        // A reload rebuilds the entry from the save at its real level while the cap is still on.
        let mut ledger = Ledger::new();
        let at = |level| {
            [Held {
                index: 9,
                item: DAGGER,
                level,
            }]
        };
        ledger.sweep(&at(10), Some(3));
        let writes = ledger.sweep(&at(8), Some(3));
        assert_eq!(writes[0].to, 3);
        assert_eq!(ledger.real(9, DAGGER), Some(8));
        assert_eq!(ledger.sweep(&at(3), None)[0].to, 8);
    }

    #[test]
    fn a_different_weapon_in_a_lowered_entry_is_not_given_the_old_ones_level() {
        let mut ledger = Ledger::new();
        ledger.sweep(
            &[Held {
                index: 4,
                item: DAGGER,
                level: 10,
            }],
            Some(0),
        );
        let replaced = [Held {
            index: 4,
            item: SHIELD,
            level: 0,
        }];
        assert!(ledger.sweep(&replaced, None).is_empty());
        assert_eq!(ledger.real(4, DAGGER), None);
        assert_eq!(ledger.saved_level(4, SHIELD, 0), None);
    }

    #[test]
    fn a_dropped_weapon_is_forgotten() {
        let mut ledger = Ledger::new();
        ledger.sweep(
            &[Held {
                index: 4,
                item: DAGGER,
                level: 10,
            }],
            Some(0),
        );
        assert!(ledger.sweep(&[], Some(0)).is_empty());
        assert!(ledger.is_empty());
    }

    #[test]
    fn the_save_gets_the_real_level_only_for_a_record_that_lost_it() {
        let mut ledger = Ledger::new();
        ledger.sweep(
            &[Held {
                index: 2,
                item: DAGGER,
                level: 10,
            }],
            Some(1),
        );
        assert_eq!(ledger.saved_level(2, DAGGER, 1), Some(10));
        assert_eq!(ledger.saved_level(2, DAGGER, 10), None);
        assert_eq!(ledger.saved_level(2, SHIELD, 1), None);
        assert_eq!(ledger.lowered().collect::<Vec<_>>(), vec![(2, DAGGER, 10)]);
    }

    #[test]
    fn a_resweep_covers_every_slot_not_just_the_one_that_changed() {
        let mut slots = [Slot::default(); 6];
        slots[0] = Slot {
            inventory: Some((DAGGER, 3)),
            record: Some((DAGGER, 10)),
            live: Some(10),
        };
        slots[3] = Slot {
            inventory: Some((SHIELD, 0)),
            record: Some((SHIELD, 0)),
            live: Some(5),
        };
        slots[4] = Slot {
            inventory: Some((CLUB, 2)),
            record: Some((CLUB, 2)),
            live: Some(2),
        };
        assert_eq!(resweep(&slots), vec![0, 3]);
    }

    #[test]
    fn the_watch_asks_once_per_change_and_never_without_a_cap() {
        let mut watch = Watch::new();
        let mut slots = [Slot::default(); 6];
        assert_eq!(watch.changed(&slots, None), None);
        assert_eq!(watch.changed(&slots, Some(0)), Some(None));
        watch.settle(slots);
        assert_eq!(watch.changed(&slots, Some(0)), None);
        let before = slots;
        slots[1].inventory = Some((DAGGER, 0));
        assert_eq!(watch.changed(&slots, Some(0)), Some(Some(before)));
        watch.settle(slots);
        assert_eq!(watch.changed(&slots, Some(0)), None);
        assert_eq!(watch.changed(&slots, None), None);
        assert_eq!(watch.changed(&slots, Some(0)), Some(None));
    }
}
