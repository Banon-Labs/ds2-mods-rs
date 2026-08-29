//! Which slot each thing a build names belongs in.
//!
//! # The planner's lists are POSITIONAL, and that is provable rather than assumed
//!
//! Every list soulsplanner emits is in the order its own form draws, and the form's field ids say
//! so. Read off a real page:
//!
//! ```text
//! spell-1 … spell-14   head chest hands legs
//! lh1 lh1-infusion rh1 rh1-infusion lh2 lh2-infusion rh2 rh2-infusion lh3 lh3-infusion rh3 rh3-infusion
//! ring-1 … ring-4      item-1 … item-10
//! ```
//!
//! Two things fall straight out of that:
//!
//! * **The weapon list is `LH1, RH1, LH2, RH2, LH3, RH3`**, interleaved with its infusions -- which
//!   is exactly the game's own flat weapon order. Guessing this was tempting and wrong-looking:
//!   across eight real builds position 0 held a shield or a catalyst six times and a weapon twice,
//!   which is suggestive and is not evidence. The field ids are.
//! * **The counts match the game's exactly** -- fourteen spell fields against fourteen attunement
//!   positions, ten item fields against ten hotbar slots. A planner that modelled slots differently
//!   would not line up like that.
//!
//! # Where the numbers are NOT
//!
//! This module deliberately holds no slot indices. A [`SlotKind`] and a position within it is all
//! the planner can tell us; turning that into the index the game's equip function takes needs the
//! flat-to-internal table, which lives in `ds2-rva` beside the disassembly that produced it. Copying
//! those numbers here would be a second place for them to drift, and the hand swap they encode is
//! the single easiest thing in this feature to get silently wrong.

use crate::items::is_empty_slot;
use crate::saved_build::Build;

/// A family of equipment slots, in the game's own grouping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotKind {
    /// Six positions, alternating hands: `LH1, RH1, LH2, RH2, LH3, RH3`.
    Weapon,
    /// Four: head, chest, hands, legs.
    Armour,
    /// Four.
    Ring,
    /// Fourteen attunement positions -- but the character's capacity is usually smaller, and it is
    /// a budget rather than a count. The runtime half has to ask the game.
    Spell,
    /// Ten consumable quick slots.
    Hotbar,
}

impl SlotKind {
    /// How many positions this family has in DARK SOULS II.
    pub const fn positions(self) -> usize {
        match self {
            SlotKind::Weapon => 6,
            SlotKind::Armour | SlotKind::Ring => 4,
            SlotKind::Spell => 14,
            SlotKind::Hotbar => 10,
        }
    }

    /// What to call it in a log line.
    pub const fn describe(self) -> &'static str {
        match self {
            SlotKind::Weapon => "weapon",
            SlotKind::Armour => "armour",
            SlotKind::Ring => "ring",
            SlotKind::Spell => "spell",
            SlotKind::Hotbar => "hotbar",
        }
    }
}

/// One thing a build wants in one place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlannedSlot {
    pub kind: SlotKind,
    /// Which position within the family, counting from zero.
    pub position: usize,
    /// The item's name, as the planner spells it.
    pub name: String,
    /// The infusion, for a weapon that names one.
    pub infusion: Option<String>,
}

/// Everything a build wants worn, held, attuned or quick-slotted, in the order it named them.
///
/// Empty slots are DROPPED rather than emitted as a request to equip nothing. Clearing a slot the
/// player already filled is not something a build import was asked to do -- a build that names no
/// helmet is a build that is silent about helmets, not one demanding a bare head.
pub fn plan(build: &Build) -> Vec<PlannedSlot> {
    let mut out = Vec::new();

    // WEAPONS ARE PAIRS. The list is `name, infusion, name, infusion, ...`, so position N is at
    // index 2N. Walking it flat would put `Dark` and `Bleed` in weapon slots.
    for (position, pair) in build.weapons.chunks(2).enumerate() {
        let [name, infusion] = pair else { continue };
        if is_empty_slot(name) || position >= SlotKind::Weapon.positions() {
            continue;
        }
        out.push(PlannedSlot {
            kind: SlotKind::Weapon,
            position,
            name: name.clone(),
            infusion: Some(infusion.clone()).filter(|text| !is_empty_slot(text)),
        });
    }

    for (kind, names) in [
        (SlotKind::Armour, &build.armor),
        (SlotKind::Ring, &build.rings),
        (SlotKind::Spell, &build.spells),
        (SlotKind::Hotbar, &build.items),
    ] {
        for (position, name) in names.iter().enumerate() {
            if is_empty_slot(name) || position >= kind.positions() {
                continue;
            }
            out.push(PlannedSlot {
                kind,
                position,
                name: name.clone(),
                infusion: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saved_build::Stats;

    fn build_with(weapons: &[&str], armor: &[&str], spells: &[&str]) -> Build {
        Build {
            id: 1,
            class: "cleric".into(),
            gender: 0,
            covenant: "No_Covenant".into(),
            grip: 0,
            armor: armor.iter().map(|s| (*s).into()).collect(),
            weapons: weapons.iter().map(|s| (*s).into()).collect(),
            rings: Vec::new(),
            spells: spells.iter().map(|s| (*s).into()).collect(),
            items: Vec::new(),
            stats: Stats::default(),
        }
    }

    /// **THE WEAPON LIST IS PAIRS, AND POSITION N IS AT INDEX 2N.**
    ///
    /// Taken from build 1's real weapons list. Reading it flat would make `Fume_Sword` position 1
    /// instead of the infusion of position 0, and put every later weapon in the wrong hand.
    #[test]
    fn weapons_are_name_infusion_pairs() {
        let build = build_with(
            &[
                "Buckler",
                "No_Infusion",
                "Fume_Sword",
                "Dark",
                "Black_Witchs_Staff",
                "No_Infusion",
                "Bare_Fists",
                "No_Infusion",
            ],
            &[],
            &[],
        );
        let planned = plan(&build);
        assert_eq!(planned.len(), 3, "Bare_Fists is not a weapon");
        assert_eq!(planned[0].position, 0);
        assert_eq!(planned[0].name, "Buckler");
        // No_Infusion is an empty slot, so it becomes no infusion rather than the string.
        assert_eq!(planned[0].infusion, None);
        assert_eq!(planned[1].position, 1);
        assert_eq!(planned[1].name, "Fume_Sword");
        assert_eq!(planned[1].infusion.as_deref(), Some("Dark"));
        assert_eq!(planned[2].position, 2);
    }

    /// An empty slot is silence about that slot, not an instruction to strip it.
    #[test]
    fn empty_slots_are_dropped_rather_than_cleared() {
        let build = build_with(&[], &["Black_Hood", "Naked", "", "No_Item"], &[]);
        let planned = plan(&build);
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].name, "Black_Hood");
        assert_eq!(planned[0].position, 0, "head is position zero");
    }

    /// A list longer than the game has positions is truncated, not wrapped around.
    ///
    /// Wrapping would write the fifteenth spell into the first attunement position -- a build's own
    /// data quietly overwriting itself.
    #[test]
    fn a_list_longer_than_the_slots_is_truncated() {
        let spells: Vec<&str> = vec!["Resonant_Soul"; 20];
        let build = build_with(&[], &[], &spells);
        let planned = plan(&build);
        assert_eq!(planned.len(), SlotKind::Spell.positions());
        assert_eq!(planned.last().expect("some").position, 13);
    }

    /// The counts are the game's, and the planner's form has exactly as many fields.
    #[test]
    fn the_slot_counts_are_the_games() {
        assert_eq!(SlotKind::Weapon.positions(), 6);
        assert_eq!(SlotKind::Armour.positions(), 4);
        assert_eq!(SlotKind::Ring.positions(), 4);
        // Fourteen `spell-N` fields on the page, fourteen attunement positions in the game.
        assert_eq!(SlotKind::Spell.positions(), 14);
        // Ten `item-N` fields, ten hotbar slots.
        assert_eq!(SlotKind::Hotbar.positions(), 10);
    }
}
