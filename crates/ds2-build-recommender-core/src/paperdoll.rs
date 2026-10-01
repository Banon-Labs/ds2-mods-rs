//! A generated build laid out as the game's Equipment page lays out a character: every slot, what
//! the build puts in it, and which slots Apply leaves alone.
//!
//! The slots are the page's own: three right-hand, three left-hand, four rings, head, chest, hands
//! and legs, ten belt slots, two of arrows and two of bolts (`ds2_rva::FE_EQUIP_EMPTY_ART` says how
//! the page was read). They are grouped and placed as the design draws them -- the "Generated build
//! as a paperdoll" board -- which follows the page: the right hand over the left with the rings
//! beside them, the armour under both, and the belt beside the ammunition at the bottom.
//!
//! What a slot holds is read off [`to_import`], the build Apply hands to `ds2-build-import`, so the
//! paperdoll cannot show a slot filled that Apply leaves alone, or the other way round. Apply equips
//! only what a build names -- `ds2_build_import_core::equip::plan` skips every empty-slot name,
//! `Naked` among them -- so a slot the build leaves empty keeps whatever the character has there.

use ds2_build_import_core::{Build, Infusion, is_empty_slot};

use crate::backend::{GeneratedBuild, to_import};
use crate::weapons;

/// A weapon, ring or armour slot's side, in the design's pixels.
pub const BIG: f32 = 112.0;
/// Between two weapon, ring or armour slots.
pub const BIG_GAP: f32 = 12.0;
/// A belt, arrow or bolt slot's side.
pub const SMALL: f32 = 88.0;
/// Between two belt, arrow or bolt slots.
pub const SMALL_GAP: f32 = 10.0;
/// Between two groups side by side: a hand and its rings, the belt and the ammunition.
pub const GROUP_GAP: f32 = 40.0;
/// Three hand slots across.
const HANDS_W: f32 = 3.0 * BIG + 2.0 * BIG_GAP;
/// Five belt slots across.
const BELT_W: f32 = 5.0 * SMALL + 4.0 * SMALL_GAP;
/// The grid's width: its widest row, the belt with two ammunition slots beside it.
pub const GRID_W: f32 = BELT_W + GROUP_GAP + 2.0 * SMALL + SMALL_GAP;

/// One slot of the Equipment page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// A right-hand weapon slot, `0..3`.
    RightHand(u8),
    /// A left-hand weapon slot, `0..3`.
    LeftHand(u8),
    /// A ring slot, `0..4`.
    Ring(u8),
    /// The head.
    Head,
    /// The chest.
    Chest,
    /// The hands.
    Hands,
    /// The legs.
    Legs,
    /// A belt (quick item) slot, `0..10`.
    Item(u8),
    /// An arrow slot, `0..2`.
    Arrows(u8),
    /// A bolt slot, `0..2`.
    Bolts(u8),
}

impl Slot {
    /// Where a generated build's weapon goes, and where the paperdoll opens: right hand 1, the slot
    /// [`to_import`] puts it in.
    pub const PRIMARY: Slot = Slot::RightHand(0);

    /// What the panel calls it: `Right weapon 1`, `Ring 3`, `Quick item 10`.
    pub fn label(self) -> String {
        match self {
            Slot::RightHand(i) => format!("Right weapon {}", i + 1),
            Slot::LeftHand(i) => format!("Left weapon {}", i + 1),
            Slot::Ring(i) => format!("Ring {}", i + 1),
            Slot::Head => "Head".to_owned(),
            Slot::Chest => "Chest".to_owned(),
            Slot::Hands => "Hands".to_owned(),
            Slot::Legs => "Legs".to_owned(),
            Slot::Item(i) => format!("Quick item {}", i + 1),
            Slot::Arrows(i) => format!("Arrows {}", i + 1),
            Slot::Bolts(i) => format!("Bolts {}", i + 1),
        }
    }

    /// Its side in the design's pixels: [`BIG`] for a weapon, ring or armour slot, [`SMALL`] for
    /// the belt and the ammunition.
    pub const fn side(self) -> f32 {
        match self {
            Slot::Item(_) | Slot::Arrows(_) | Slot::Bolts(_) => SMALL,
            _ => BIG,
        }
    }

    /// Its left edge in the design's pixels, from the grid's left.
    pub fn x(self) -> f32 {
        let big = BIG + BIG_GAP;
        let small = SMALL + SMALL_GAP;
        match self {
            Slot::RightHand(i) | Slot::LeftHand(i) => f32::from(i) * big,
            Slot::Ring(i) => HANDS_W + GROUP_GAP + f32::from(i % 2) * big,
            Slot::Head => 0.0,
            Slot::Chest => big,
            Slot::Hands => 2.0 * big,
            Slot::Legs => 3.0 * big,
            Slot::Item(i) => f32::from(i % 5) * small,
            Slot::Arrows(i) | Slot::Bolts(i) => BELT_W + GROUP_GAP + f32::from(i) * small,
        }
    }

    /// How far its centre is from the grid's left: what Up and Down line slots up by.
    pub fn centre_x(self) -> f32 {
        self.x() + self.side() * 0.5
    }

    /// The heading the design prints over the group this slot starts, if it starts one. The second
    /// pair of rings starts no group: it sits under the first, under the same heading.
    pub const fn group(self) -> Option<&'static str> {
        match self {
            Slot::RightHand(0) => Some("Right hand"),
            Slot::LeftHand(0) => Some("Left hand"),
            Slot::Ring(0) => Some("Rings"),
            Slot::Head => Some("Armor"),
            Slot::Item(0) => Some("Quick items"),
            Slot::Arrows(0) => Some("Arrows"),
            Slot::Bolts(0) => Some("Bolts"),
            _ => None,
        }
    }
}

/// The slots in the rows the D-pad walks, top to bottom: each band of the grid left to right, the
/// belt's two rows each with the ammunition beside it.
pub const ROWS: [&[Slot]; 5] = [
    &[
        Slot::RightHand(0),
        Slot::RightHand(1),
        Slot::RightHand(2),
        Slot::Ring(0),
        Slot::Ring(1),
    ],
    &[
        Slot::LeftHand(0),
        Slot::LeftHand(1),
        Slot::LeftHand(2),
        Slot::Ring(2),
        Slot::Ring(3),
    ],
    &[Slot::Head, Slot::Chest, Slot::Hands, Slot::Legs],
    &[
        Slot::Item(0),
        Slot::Item(1),
        Slot::Item(2),
        Slot::Item(3),
        Slot::Item(4),
        Slot::Arrows(0),
        Slot::Arrows(1),
    ],
    &[
        Slot::Item(5),
        Slot::Item(6),
        Slot::Item(7),
        Slot::Item(8),
        Slot::Item(9),
        Slot::Bolts(0),
        Slot::Bolts(1),
    ],
];

/// What a slot holds once Apply has run.
#[derive(Clone, Debug, PartialEq)]
pub enum Held {
    /// An item the build puts here, which Apply grants if the character lacks it and equips.
    Item(Piece),
    /// An armour slot the build leaves bare to stay under its load. Apply equips nothing there, so
    /// whatever the character wears in it stays on -- with a weight the build's numbers leave out.
    Bare,
    /// A slot the build says nothing about. Apply leaves it as it is.
    Kept,
}

/// An item in a slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Its name as the panel prints it, `Sanctum Crossbow`.
    pub name: String,
    /// Its infusion, for a weapon.
    pub infusion: Option<Infusion>,
    /// The item id its icon is filed under; `None` when the catalogue has no single item by that
    /// name.
    pub item: Option<u32>,
}

/// A generated build, every slot of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Paperdoll {
    held: Vec<(Slot, Held)>,
}

impl Paperdoll {
    /// Lay `build` out as Apply equips it.
    pub fn of(build: &GeneratedBuild) -> Self {
        let (import, _) = to_import(build);
        let held = ROWS
            .iter()
            .flat_map(|row| row.iter().copied())
            .map(|slot| (slot, read(&import, slot)))
            .collect();
        Self { held }
    }

    /// What `slot` holds.
    pub fn held(&self, slot: Slot) -> &Held {
        self.held
            .iter()
            .find(|(at, _)| *at == slot)
            .map_or(&Held::Kept, |(_, held)| held)
    }

    /// Every slot with what it holds, row by row.
    pub fn slots(&self) -> &[(Slot, Held)] {
        &self.held
    }
}

/// What `slot` holds in the build Apply is handed.
fn read(import: &Build, slot: Slot) -> Held {
    match slot {
        // `LH1, RH1, LH2, RH2, LH3, RH3`, each name followed by its infusion.
        Slot::RightHand(i) => weapon(&import.weapons, 2 * usize::from(i) + 1),
        Slot::LeftHand(i) => weapon(&import.weapons, 2 * usize::from(i)),
        Slot::Ring(i) => worn(import.rings.get(usize::from(i)), Held::Kept),
        Slot::Head => worn(import.armor.first(), Held::Bare),
        Slot::Chest => worn(import.armor.get(1), Held::Bare),
        Slot::Hands => worn(import.armor.get(2), Held::Bare),
        Slot::Legs => worn(import.armor.get(3), Held::Bare),
        // A generated build names no quick item and no ammunition.
        Slot::Item(_) | Slot::Arrows(_) | Slot::Bolts(_) => Held::Kept,
    }
}

/// The weapon at `position` of a build's `name, infusion` list.
fn weapon(list: &[String], position: usize) -> Held {
    let Some(key) = list.get(2 * position).filter(|key| !is_empty_slot(key)) else {
        return Held::Kept;
    };
    let infusion = list
        .get(2 * position + 1)
        .map_or(Some(Infusion::None), |text| {
            "NMFLDPBREU"
                .chars()
                .filter_map(weapons::infusion_for_code)
                .find(|infusion| weapons::planner_name(*infusion) == text)
        });
    Held::Item(Piece {
        name: weapons::by_key(key).map_or_else(|| key.clone(), |row| row.name.to_owned()),
        infusion,
        item: weapons::item_id(key),
    })
}

/// A ring or armour piece by name, or `empty` where the build names none.
fn worn(name: Option<&String>, empty: Held) -> Held {
    match name {
        Some(name) if !is_empty_slot(name) => Held::Item(Piece {
            name: name.clone(),
            infusion: None,
            item: weapons::item_id(name),
        }),
        _ => empty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{StubBackend, generate};
    use crate::model::PanelState;

    fn stub(allow_naked: bool) -> GeneratedBuild {
        let state = PanelState {
            weapon: Some("Moonlight_Greatsword"),
            allow_naked,
            ..PanelState::default()
        };
        generate(&StubBackend, &state, None).expect("the stub build is valid")
    }

    fn every_slot() -> Vec<Slot> {
        ROWS.iter().flat_map(|row| row.iter().copied()).collect()
    }

    /// The page's slots, each once: three in each hand, four rings, four armour, ten belt slots,
    /// two of each ammunition.
    #[test]
    fn the_rows_hold_every_slot_of_the_page_once() {
        let slots = every_slot();
        for (index, slot) in slots.iter().enumerate() {
            assert!(!slots[..index].contains(slot), "{slot:?} twice");
        }
        let count = |pick: fn(&Slot) -> bool| slots.iter().filter(|slot| pick(slot)).count();
        assert_eq!(count(|s| matches!(s, Slot::RightHand(i) if *i < 3)), 3);
        assert_eq!(count(|s| matches!(s, Slot::LeftHand(i) if *i < 3)), 3);
        assert_eq!(count(|s| matches!(s, Slot::Ring(i) if *i < 4)), 4);
        assert_eq!(
            count(|s| matches!(s, Slot::Head | Slot::Chest | Slot::Hands | Slot::Legs)),
            4
        );
        assert_eq!(count(|s| matches!(s, Slot::Item(i) if *i < 10)), 10);
        assert_eq!(count(|s| matches!(s, Slot::Arrows(i) if *i < 2)), 2);
        assert_eq!(count(|s| matches!(s, Slot::Bolts(i) if *i < 2)), 2);
        assert_eq!(slots.len(), 28);
    }

    /// Along each row the slots run left to right without touching, and the widest row ends at
    /// the grid's edge.
    #[test]
    fn a_rows_slots_run_left_to_right_inside_the_grid() {
        for row in ROWS {
            for pair in row.windows(2) {
                let (left, right) = (pair[0], pair[1]);
                assert!(
                    left.x() + left.side() < right.x(),
                    "{left:?} runs into {right:?}"
                );
            }
            let last = row[row.len() - 1];
            assert!(
                last.x() + last.side() <= GRID_W,
                "{last:?} is past the grid"
            );
        }
        assert_eq!(Slot::Bolts(1).x() + SMALL, GRID_W);
        // The design's numbers: the rings start 40 past the hands, the ammunition 40 past the belt.
        assert_eq!(Slot::Ring(0).x(), 400.0);
        assert_eq!(Slot::Arrows(0).x(), 520.0);
    }

    /// The stub build as Apply leaves it: its weapon in right hand 1, its three rings, its one
    /// piece of armour with the rest bare, and every other slot as the character has it.
    #[test]
    fn the_stub_build_fills_what_apply_equips_and_keeps_the_rest() {
        let doll = Paperdoll::of(&stub(false));
        let Held::Item(weapon) = doll.held(Slot::PRIMARY) else {
            panic!("right hand 1 holds {:?}", doll.held(Slot::PRIMARY));
        };
        assert_eq!(weapon.name, "Moonlight Greatsword");
        assert_eq!(weapon.infusion, Some(Infusion::None));
        assert!(
            weapon.item.is_some(),
            "the Moonlight Greatsword has an item id"
        );
        for slot in [
            Slot::RightHand(1),
            Slot::RightHand(2),
            Slot::LeftHand(0),
            Slot::LeftHand(1),
            Slot::LeftHand(2),
            Slot::Ring(3),
            Slot::Item(0),
            Slot::Arrows(1),
            Slot::Bolts(0),
        ] {
            assert_eq!(doll.held(slot), &Held::Kept, "{slot:?}");
        }
        let names: Vec<&str> = (0..3)
            .map(|i| match doll.held(Slot::Ring(i)) {
                Held::Item(ring) => ring.name.as_str(),
                other => panic!("ring {i} holds {other:?}"),
            })
            .collect();
        assert_eq!(
            names,
            ["Dexterity Ring", "Ring of Knowledge", "Strength Ring"]
        );
        match doll.held(Slot::Head) {
            Held::Item(head) => {
                assert_eq!(head.name, "Desert Sorceress Hood");
                assert!(head.item.is_some(), "the hood has an item id");
            }
            other => panic!("the head holds {other:?}"),
        }
        for slot in [Slot::Chest, Slot::Hands, Slot::Legs] {
            assert_eq!(doll.held(slot), &Held::Bare, "{slot:?}");
        }
    }

    /// A build asked to wear no armour leaves all four slots bare, not kept: the build means bare.
    #[test]
    fn a_build_without_armour_is_bare_in_all_four_slots() {
        let doll = Paperdoll::of(&stub(true));
        for slot in [Slot::Head, Slot::Chest, Slot::Hands, Slot::Legs] {
            assert_eq!(doll.held(slot), &Held::Bare, "{slot:?}");
        }
    }

    /// Every item the stub build puts in a slot has an icon id, so none draws as a blank box.
    #[test]
    fn every_piece_of_the_stub_build_has_an_item_id() {
        let doll = Paperdoll::of(&stub(false));
        for (slot, held) in doll.slots() {
            if let Held::Item(piece) = held {
                assert!(piece.item.is_some(), "{slot:?} {} has no id", piece.name);
            }
        }
    }
}
