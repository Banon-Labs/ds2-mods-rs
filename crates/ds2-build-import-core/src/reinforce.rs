//! Each weapon's and armour piece's maximum reinforcement, from the game's regulation.
//!
//! `data/reinforce-max.tsv` is written by `scripts/ds2-reinforce-max.py --cost-gated`, which
//! decrypts `enc_regulation.bnd.dcx` and follows each item to its reinforce row -- `ItemParam` to
//! `WeaponParam` to `WeaponReinforceParam`, or to `ArmorParam` to `ArmorReinforceParam` -- and reads
//! that row's maximum level. The script's docstring gives the offset of every field and the
//! evidence for each; what each field means is inferred from the data, not read from a paramdef.
//! Nothing in it comes from a planner site or from an item's name.
//!
//! `--cost-gated` writes 0 for the six items whose maximum is set but whose upgrade-cost row prices
//! no level at all: the four Black Dragon pieces, the Key to the Embedded and the Binoculars. The
//! blacksmith cannot price a level for them, so they are taken as not reinforceable.
//!
//! A grant uses this so a build's weapons and armour arrive at the level the recommender ranked
//! them at: its attack and defense numbers are the planner's full-upgrade values.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The table, as the script wrote it: item id, max level, `weapon` or `armor`, after a header.
const TABLE: &str = include_str!("../data/reinforce-max.tsv");

fn table() -> &'static HashMap<i32, u8> {
    static PARSED: OnceLock<HashMap<i32, u8>> = OnceLock::new();
    PARSED.get_or_init(|| {
        TABLE
            .lines()
            .filter_map(|line| {
                let mut columns = line.split('\t');
                let id = columns.next()?.parse().ok()?;
                let max = columns.next()?.parse().ok()?;
                Some((id, max))
            })
            .collect()
    })
}

/// The highest reinforcement the game lets `item_id` reach.
///
/// 10 for most weapons, 5 for boss and twinkling-titanite weapons, armour its own, 0 for an item
/// that cannot be reinforced. `None` for an item the table does not carry, which is anything that
/// is not a weapon or armour.
pub fn max_reinforce(item_id: i32) -> Option<u8> {
    table().get(&item_id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id_for;

    fn max_of(name: &str) -> Option<u8> {
        max_reinforce(id_for(name).expect(name))
    }

    #[test]
    fn the_table_is_whole() {
        // 333 weapons and 435 armour pieces: every weapon and armour id in the catalogue.
        assert_eq!(table().len(), 768);
    }

    #[test]
    fn a_regular_weapon_goes_to_ten() {
        assert_eq!(max_of("Falchion"), Some(10));
        assert_eq!(max_of("Old Knight Hammer"), Some(10));
    }

    #[test]
    fn a_boss_weapon_goes_to_five() {
        assert_eq!(max_of("Moonlight Greatsword"), Some(5));
    }

    #[test]
    fn armour_has_its_own_maximum() {
        assert_eq!(max_of("Wanderer Manchettes"), Some(10));
        assert_eq!(max_reinforce(25_060_101), Some(5), "Agdayne's Black Robe");
    }

    #[test]
    fn armour_that_cannot_be_reinforced_stays_at_zero() {
        assert_eq!(max_of("Black Dragon Helm"), Some(0));
    }

    #[test]
    fn a_ring_is_not_reinforced() {
        assert_eq!(max_of("Chloranthy Ring"), None);
    }
}
