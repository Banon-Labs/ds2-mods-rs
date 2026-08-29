//! Turning a planner's item names into the ids the game's own grant function takes.
//!
//! # The game takes ids, and the planner gives names
//!
//! `ItemGive` wants an `i32` `ItemParam` row id per item. soulsplanner gives display names with
//! underscores for spaces -- `Greatsword_of_the_Forlorn`, `Caithas_Chime`, `Black_Hood`. The join
//! between the two lives in `data/items.tsv`, extracted by `scripts/ds2-item-ids.py`, and that file
//! says out loud that it is SECOND-HAND: a community Cheat Engine table's join rather than a read
//! of the game's own `ItemParam` plus its FMG names.
//!
//! # Two traps that a substring match walks straight into
//!
//! **`Black_Hood` matches two items.** The catalogue holds both `Black Hood` (`27680100`) and
//! `Leydia Black Hood` (`25090100`). A contains-style match picks whichever comes first and is
//! wrong roughly half the time, silently. So matching here is EXACT, after normalisation, and an
//! ambiguous name is an error rather than a coin flip.
//!
//! **`Caithas_Chime` is spelled `Caitha's Chime` in the catalogue.** The planner drops the
//! apostrophe; the game keeps it. So normalisation cannot be "underscores become spaces" -- it has
//! to discard everything that is not a letter or a digit, on both sides. `caithaschime` then equals
//! `caithaschime`, while `blackhood` still differs from `leydiablackhood`.
//!
//! # The empty slots are named, not omitted
//!
//! A build's lists are fixed length with the gaps spelled out: `No_Spell`, `No_Item`, `No_Ring`,
//! `No_Infusion`, `Bare_Fists`. Those are not items and must never be looked up -- `Bare_Fists` in
//! particular would fail to resolve and read as a broken catalogue rather than as an empty hand.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The id/name join. See the file's own header for its provenance.
const CATALOGUE: &str = include_str!("../data/items.tsv");

/// Names the planner uses for an empty slot rather than omitting it.
///
/// Compared after [`normalise`], so the underscores here are decoration.
///
/// **The EMPTY STRING is in here**, and it is the one that was missing. A real build came back with
/// six slots named `""` rather than `No_Ring`, and each one was reported as `no item called ""` --
/// six lines of alarm about a character wearing nothing in six places it was not wearing anything.
/// The planner uses both spellings and neither is a problem.
const EMPTY_SLOTS: [&str; 7] = [
    "",
    "nospell",
    "noitem",
    "noring",
    "noinfusion",
    "barefists",
    "none",
];

/// Why a name did not become an id.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ItemError {
    /// The name is a placeholder for an empty slot, not an item.
    EmptySlot,
    /// No catalogue entry has this name.
    Unknown { name: String },
    /// More than one id carries this name. **Never resolved by picking one.**
    Ambiguous { name: String, ids: Vec<i32> },
}

impl core::fmt::Display for ItemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ItemError::EmptySlot => write!(f, "an empty slot, not an item"),
            ItemError::Unknown { name } => write!(f, "no item called {name:?}"),
            ItemError::Ambiguous { name, ids } => {
                write!(f, "{} items are called {name:?}: {ids:?}", ids.len())
            }
        }
    }
}

/// Everything but letters and digits, discarded, and the rest lowercased.
///
/// Aggressive on purpose -- see the module docs. The planner and the catalogue disagree about
/// spaces, underscores and apostrophes, and agree about nothing else that matters.
pub fn normalise(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// The parsed catalogue: normalised name -> every id carrying it.
fn catalogue() -> &'static HashMap<String, Vec<i32>> {
    static PARSED: OnceLock<HashMap<String, Vec<i32>>> = OnceLock::new();
    PARSED.get_or_init(|| {
        let mut out: HashMap<String, Vec<i32>> = HashMap::new();
        for line in CATALOGUE.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let Some((id, name)) = line.split_once('\t') else {
                continue;
            };
            let Ok(id) = id.trim().parse::<i32>() else {
                continue;
            };
            out.entry(normalise(name)).or_default().push(id);
        }
        out
    })
}

/// Whether a planner name means "this slot is empty".
pub fn is_empty_slot(name: &str) -> bool {
    let normalised = normalise(name);
    EMPTY_SLOTS.contains(&normalised.as_str())
}

/// The `ItemParam` row id for a planner name.
pub fn id_for(name: &str) -> Result<i32, ItemError> {
    if is_empty_slot(name) {
        return Err(ItemError::EmptySlot);
    }
    let key = normalise(name);
    match catalogue().get(&key).map(Vec::as_slice) {
        None | Some([]) => Err(ItemError::Unknown {
            name: name.to_owned(),
        }),
        Some([only]) => Ok(*only),
        Some(many) => Err(ItemError::Ambiguous {
            name: name.to_owned(),
            ids: many.to_vec(),
        }),
    }
}

/// How many items the catalogue knows.
pub fn catalogue_size() -> usize {
    catalogue().values().map(Vec::len).sum()
}

/// A weapon infusion, as the game numbers them.
///
/// Read off the `ItemUpgrade` dropdown that all three community tables carry identically. **The
/// dropdown's NAME is a trap** -- it is called `ItemUpgrade` and it lists infusions, not upgrade
/// levels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Infusion {
    None = 0,
    Fire = 1,
    Magic = 2,
    Lightning = 3,
    Dark = 4,
    Poison = 5,
    Bleed = 6,
    Raw = 7,
    Enchanted = 8,
    Mundane = 9,
}

impl Infusion {
    /// The infusion a planner names, or `None` if it is not one.
    ///
    /// `No_Infusion` comes back as [`Infusion::None`] rather than as an error: an uninfused weapon
    /// is a weapon, and the byte the game wants for it is `0`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match normalise(name).as_str() {
            "none" | "noinfusion" | "normal" => Infusion::None,
            "fire" => Infusion::Fire,
            "magic" => Infusion::Magic,
            "lightning" => Infusion::Lightning,
            "dark" => Infusion::Dark,
            "poison" => Infusion::Poison,
            "bleed" => Infusion::Bleed,
            "raw" => Infusion::Raw,
            "enchanted" => Infusion::Enchanted,
            "mundane" => Infusion::Mundane,
            _ => return None,
        })
    }

    /// The byte the game's spawn struct carries.
    pub const fn byte(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalogue loads and is the size the extractor reported.
    #[test]
    fn the_catalogue_is_populated() {
        assert_eq!(catalogue_size(), 1236);
    }

    /// EVERY ITEM IN BUILD 253 RESOLVES.
    ///
    /// Names taken verbatim from what `scripts/ds2-soulsplanner.py 253` returns, so this is the
    /// planner's spelling rather than a tidied version of it. If the join or the normalisation
    /// breaks, this is the test that says so in the planner's own words.
    #[test]
    fn every_item_in_a_real_build_resolves() {
        for (name, expected) in [
            // Armour
            ("Black_Hood", 27680100),
            ("Armor_of_the_Forlorn", 26940101),
            ("Shadow_Gauntlets", 22230102),
            ("Leggings_of_the_Forlorn", 26940103),
            // Weapons, including the one whose apostrophe the planner drops
            ("Caithas_Chime", 4090000),
            ("Greatsword_of_the_Forlorn", 1997000),
            // Rings
            ("Ring_of_Binding", 40410000),
            ("Flynns_Ring", 41100000),
            ("Crest_of_Blood", 40730000),
            ("Third_Dragon_Ring", 40040002),
            // Spells
            ("Dark_Weapon", 34060000),
            ("Resonant_Soul", 35030000),
        ] {
            assert_eq!(id_for(name), Ok(expected), "{name}");
        }
    }

    /// `Black_Hood` is NOT `Leydia Black Hood`, which is what a substring match would give.
    #[test]
    fn an_exact_name_does_not_match_a_longer_one() {
        assert_eq!(id_for("Black_Hood"), Ok(27680100));
        assert_eq!(id_for("Leydia_Black_Hood"), Ok(25090100));
        assert_ne!(id_for("Black_Hood"), id_for("Leydia_Black_Hood"));
    }

    /// The planner drops apostrophes the catalogue keeps.
    #[test]
    fn punctuation_does_not_have_to_agree() {
        assert_eq!(normalise("Caithas_Chime"), normalise("Caitha's Chime"));
        assert_eq!(id_for("Caithas_Chime"), id_for("Caitha's Chime"));
    }

    /// **A SLOT NAMED BY NOTHING IS AN EMPTY SLOT.** Real builds use `""` as well as `No_Ring`.
    ///
    /// Six of these came back from build 1 and each was announced as a missing item, which reads as
    /// a broken catalogue rather than as an empty hand -- the same failure `Bare_Fists` was already
    /// guarded against, in a spelling nobody had seen yet.
    #[test]
    fn a_slot_named_by_nothing_is_empty() {
        for nothing in ["", " ", "_", "   _  "] {
            assert!(is_empty_slot(nothing), "{nothing:?}");
            assert_eq!(id_for(nothing), Err(ItemError::EmptySlot), "{nothing:?}");
        }
    }

    /// An empty slot is refused as an empty slot, not as a missing item.
    ///
    /// The distinction is the whole point: `Bare_Fists` would otherwise fail to resolve and read as
    /// a broken catalogue rather than as an empty hand.
    #[test]
    fn the_placeholders_are_not_looked_up() {
        for placeholder in [
            "No_Spell",
            "No_Item",
            "No_Ring",
            "No_Infusion",
            "Bare_Fists",
        ] {
            assert!(is_empty_slot(placeholder), "{placeholder}");
            assert_eq!(
                id_for(placeholder),
                Err(ItemError::EmptySlot),
                "{placeholder}"
            );
        }
    }

    /// A name nothing carries says so, with the name in it.
    #[test]
    fn an_unknown_name_names_itself() {
        let error = id_for("Sword_Of_Nothing_At_All").expect_err("not an item");
        assert!(error.to_string().contains("Sword_Of_Nothing_At_All"));
    }

    /// A colliding name is refused with every candidate, never silently resolved.
    ///
    /// The catalogue has eight duplicate display names. Which ones is not the point -- the point is
    /// that picking one would be a coin flip the caller never sees.
    #[test]
    fn a_colliding_name_refuses_and_lists_the_candidates() {
        let colliding: Vec<_> = catalogue()
            .iter()
            .filter(|(_, ids)| ids.len() > 1)
            .collect();
        assert!(
            !colliding.is_empty(),
            "the catalogue should have collisions"
        );
        for (name, ids) in colliding {
            match id_for(name) {
                Err(ItemError::Ambiguous { ids: reported, .. }) => {
                    assert_eq!(reported.len(), ids.len());
                }
                other => panic!("{name} resolved to {other:?} instead of refusing"),
            }
        }
    }

    /// The infusions a build names map to the bytes the game wants.
    #[test]
    fn the_builds_infusions_map_to_the_games_bytes() {
        assert_eq!(Infusion::from_name("Dark").map(Infusion::byte), Some(4));
        assert_eq!(Infusion::from_name("Bleed").map(Infusion::byte), Some(6));
        assert_eq!(Infusion::from_name("Mundane").map(Infusion::byte), Some(9));
        // An uninfused weapon is a weapon, and its byte is zero.
        assert_eq!(Infusion::from_name("No_Infusion"), Some(Infusion::None));
        assert_eq!(Infusion::None.byte(), 0);
        // A weapon name in the infusion column is not an infusion.
        assert_eq!(Infusion::from_name("Greatsword_of_the_Forlorn"), None);
    }
}
