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
}
