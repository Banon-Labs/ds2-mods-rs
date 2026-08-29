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
const EMPTY_SLOTS: [&str; 8] = [
    "",
    // An armour slot the planner draws as wearing nothing. Found by sweeping forty real builds
    // (`scripts/ds2-catalogue-sweep.py`), not in the game -- which is the point of that tool.
    "naked",
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
    /// Every id carrying this name is one the catalogue's author flagged unsafe to spawn.
    UnsafeToSpawn { name: String, ids: Vec<i32> },
}

impl core::fmt::Display for ItemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ItemError::EmptySlot => write!(f, "an empty slot, not an item"),
            ItemError::Unknown { name } => write!(f, "no item called {name:?}"),
            ItemError::Ambiguous { name, ids } => {
                write!(f, "{} items are called {name:?}: {ids:?}", ids.len())
            }
            ItemError::UnsafeToSpawn { name, ids } => write!(
                f,
                "{name:?} is flagged UNSAFE to spawn in the catalogue ({ids:?})"
            ),
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

/// The suffix the catalogue's author appends to an item they warn against spawning.
///
/// Sixteen rows carry it: the gestures, the Darksign, the Crushed Eye Orb, the dragon stones, two
/// broken weapons and one of the four Estus Flasks. They are not items in the sense the grant
/// function means -- a gesture is not inventory -- and the author flagged them by hand.
const UNSAFE_SUFFIX: &str = " (UNSAFE)";

/// Rows the GAME will never put in an inventory, whatever the catalogue calls them.
///
/// # `ItemParam + 0x52` bit 3 is "may exist in an inventory", and 46 rows lack it
///
/// The add path checks it -- `test byte [rax+0x52],0x8; je return` -- and returns having added
/// NOTHING while `ItemGive` still answers `true`. A caller that trusts the return value believes it
/// granted something that does not exist.
///
/// # The three Estus Flask rows here are NOT upgrade states
///
/// That was this comment's own first answer and it was wrong. DARK SOULS II has exactly one Estus
/// Flask, `60155000`, and its upgrade level is a BYTE ON THE INVENTORY ENTRY -- the held id never
/// changes. See `ds2_rva::ESTUS_SET_PROPERTY`. `60155010/20/30` are rows of `EstusFlaskLvDataParam`
/// (`60155000 + (row - 1)`), reachable only at effect levels 11, 21 and 31 in a game that caps the
/// effect level at 6, so nothing in a shipped game ever names them.
///
/// The conclusion the wrong reading reached was still the right one: the real flask is `60155000`,
/// the row this catalogue marks `(UNSAFE)`, which is why [`WRONGLY_FLAGGED`] exists. Granting the
/// flask by name used to pick the lowest of the other three and silently achieve nothing.
///
/// This list is short because it is only what has been PROVEN from the params. The authoritative
/// test is the bit itself, and reading it at runtime would retire this list entirely.
const NOT_INVENTORY_ITEMS: [i32; 3] = [60155010, 60155020, 60155030];

/// Rows the catalogue flags `(UNSAFE)` that the game's own params say are ordinary items.
///
/// `60155000` is the real Estus Flask: `ItemParam + 0x52 = 0x0d` -- may exist in an inventory,
/// unique, maximum stack one. The table's author flagged it, and taking that at face value removed
/// the only Estus Flask a character can actually hold.
///
/// A character who already has a flask will still be refused, by the game, with "already held and
/// it is not stackable" -- which is the correct answer and is reported as satisfied rather than
/// failed.
const WRONGLY_FLAGGED: [i32; 1] = [60155000];

/// One catalogue row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Entry {
    id: i32,
    /// The author marked this row [`UNSAFE_SUFFIX`].
    unsafe_to_spawn: bool,
}

/// The parsed catalogue: normalised name -> every row carrying it.
///
/// # Two annotations, treated oppositely, and the difference is the whole reason this is not a
/// plain split
///
/// Twenty-three names end in a parenthetical. `(UNSAFE)` is stripped before the key is built, so
/// `Black_Separation_Crystal` FINDS its row and is refused with a reason. `(Recolor)` is left in,
/// because all seven recoloured weapons have a real unannotated twin -- `Longsword` `1220000`
/// beside `Longsword (Recolor)` `5600000` -- so that annotation is what tells them apart, and
/// stripping it would turn seven clean lookups into seven coin flips.
///
/// Verified by reading the catalogue: every `(Recolor)` has a twin, and `Darksign` has none.
fn catalogue() -> &'static HashMap<String, Vec<Entry>> {
    static PARSED: OnceLock<HashMap<String, Vec<Entry>>> = OnceLock::new();
    PARSED.get_or_init(|| {
        let mut out: HashMap<String, Vec<Entry>> = HashMap::new();
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
            // A row the game will never place in an inventory is not a row this can offer.
            if NOT_INVENTORY_ITEMS.contains(&id) {
                continue;
            }
            let unsafe_to_spawn = name.ends_with(UNSAFE_SUFFIX) && !WRONGLY_FLAGGED.contains(&id);
            let name = name.strip_suffix(UNSAFE_SUFFIX).unwrap_or(name);
            out.entry(normalise(name)).or_default().push(Entry {
                id,
                unsafe_to_spawn,
            });
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
///
/// A name that resolves ONLY to rows the catalogue flags unsafe is refused as
/// [`ItemError::UnsafeToSpawn`] rather than granted. Where safe and unsafe rows share a name --
/// `Estus Flask` has three of the first and one of the second -- the unsafe ones are dropped and
/// the rest answer normally.
pub fn id_for(name: &str) -> Result<i32, ItemError> {
    if is_empty_slot(name) {
        return Err(ItemError::EmptySlot);
    }
    let key = normalise(name);
    let Some(entries) = catalogue().get(&key).filter(|rows| !rows.is_empty()) else {
        return Err(ItemError::Unknown {
            name: name.to_owned(),
        });
    };
    let safe: Vec<i32> = entries
        .iter()
        .filter(|entry| !entry.unsafe_to_spawn)
        .map(|entry| entry.id)
        .collect();
    match safe.as_slice() {
        // Every row carrying this name is flagged. Refusing NAMES THE REASON -- "no item called
        // Black_Separation_Crystal" is false and sends the reader looking for a missing row.
        [] => Err(ItemError::UnsafeToSpawn {
            name: name.to_owned(),
            ids: entries.iter().map(|entry| entry.id).collect(),
        }),
        [only] => Ok(*only),
        _ => Err(ItemError::Ambiguous {
            name: name.to_owned(),
            ids: safe,
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

    /// The catalogue loads, and holds the extractor's rows minus the ones the game refuses to keep.
    ///
    /// The extractor reports 1236. Three are dropped here as [`NOT_INVENTORY_ITEMS`] -- the Estus
    /// Flask upgrade states, which no inventory can ever hold. The subtraction is written out so
    /// that a change to either number has to be deliberate.
    #[test]
    fn the_catalogue_is_populated() {
        const EXTRACTED: usize = 1236;
        assert_eq!(catalogue_size(), EXTRACTED - NOT_INVENTORY_ITEMS.len());
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

    /// A name carried by several SPAWNABLE ids is refused with every candidate, never resolved.
    ///
    /// The qualifier is load-bearing. Stripping the `(UNSAFE)` suffix creates name collisions that
    /// did not exist before -- `Dragon Torso Stone` now names both `60406000` and the flagged
    /// `60406010` -- and those are NOT ambiguous, because only one of them is a thing this will
    /// ever grant. Ambiguity is about the choices actually on offer.
    #[test]
    fn a_colliding_name_refuses_and_lists_the_candidates() {
        let colliding: Vec<_> = catalogue()
            .iter()
            .map(|(name, rows)| {
                let spawnable = rows.iter().filter(|row| !row.unsafe_to_spawn).count();
                (name, spawnable)
            })
            .filter(|(_, spawnable)| *spawnable > 1)
            .collect();
        assert!(
            !colliding.is_empty(),
            "the catalogue should have collisions"
        );
        for (name, spawnable) in colliding {
            match id_for(name) {
                Err(ItemError::Ambiguous { ids: reported, .. }) => {
                    assert_eq!(reported.len(), spawnable, "{name}");
                }
                other => panic!("{name} resolved to {other:?} instead of refusing"),
            }
        }
    }

    /// **AN ITEM THE CATALOGUE WARNS ABOUT IS REFUSED WITH THAT REASON, NOT AS A MISSING ROW.**
    ///
    /// Found by `scripts/ds2-catalogue-sweep.py`: a real build named `Black_Separation_Crystal`, and
    /// the row exists but is spelled `Black Separation Crystal (UNSAFE)`, so the lookup missed it
    /// and reported "no item called". That message is FALSE and sends the reader hunting for a row
    /// that is right there. The suffix is now stripped for matching, which means the refusal has to
    /// carry the real reason instead.
    #[test]
    fn an_unsafe_item_is_refused_for_being_unsafe() {
        let error = id_for("Black_Separation_Crystal").expect_err("flagged unsafe");
        assert!(
            matches!(error, ItemError::UnsafeToSpawn { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("UNSAFE"), "{error}");
        // A gesture is not an inventory item at all, and eight of them are flagged.
        assert!(matches!(
            id_for("Wave_Gesture"),
            Err(ItemError::UnsafeToSpawn { .. })
        ));
        // And the one with no unflagged twin, which used to read as missing.
        assert!(matches!(
            id_for("Darksign"),
            Err(ItemError::UnsafeToSpawn { .. })
        ));
    }

    /// **THE REAL ESTUS FLASK IS THE ONE THE CATALOGUE FLAGGED.**
    ///
    /// `Estus Flask` had four rows: `60155000` marked `(UNSAFE)` and three unmarked. Dropping the
    /// flagged one and keeping the rest looked obviously right and was exactly backwards. The three
    /// unmarked ids lack `ItemParam + 0x52` bit 3 -- "may exist in an inventory" -- because they are
    /// upgrade STATES of the one flask this game has, not three flasks. Granting by name picked the
    /// lowest of them and silently achieved nothing, while `ItemGive` still answered true.
    #[test]
    fn the_estus_flask_is_the_row_that_can_exist() {
        assert_eq!(id_for("Estus_Flask"), Ok(60155000));
        // And the upgrade states are gone from the catalogue entirely, so nothing can offer them.
        for state in NOT_INVENTORY_ITEMS {
            assert!(
                !catalogue()
                    .values()
                    .flatten()
                    .any(|entry| entry.id == state),
                "{state} should not be offerable"
            );
        }
    }

    /// A flagged row still hides behind unflagged namesakes where the flag is right.
    #[test]
    fn a_flagged_row_does_not_hide_its_unflagged_namesakes() {
        // Dragon Torso Stone is one flagged, one not, so it resolves cleanly to the good one.
        assert_eq!(id_for("Dragon_Torso_Stone"), Ok(60406000));
    }

    /// `(Recolor)` is NOT stripped, because it is what tells seven weapons from their twins.
    ///
    /// All seven recoloured weapons have a real unannotated row -- `Longsword` `1220000` beside
    /// `Longsword (Recolor)` `5600000`. Stripping that suffix the way `(UNSAFE)` is stripped would
    /// turn seven clean lookups into seven coin flips.
    #[test]
    fn the_recolour_annotation_is_left_alone_because_it_disambiguates() {
        for (plain, recolour) in [
            ("Longsword", 1220000),
            ("Murakumo", 5000000),
            ("Rapier", 1500000),
            ("Caestus", 3500000),
        ] {
            assert_eq!(id_for(plain), Ok(recolour), "{plain}");
        }
        assert_eq!(id_for("Longsword_(Recolor)"), Ok(5600000));
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
