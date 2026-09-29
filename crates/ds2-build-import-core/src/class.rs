//! A character's starting class, and the stats no build may take it below.
//!
//! # Why the stats and the class have to agree
//!
//! A character's starting class fixes the stat spread its levels are counted from, and nothing
//! the game offers -- not a level-up, not a Soul Vessel respec -- puts a stat below that spread:
//! the level-up menu floors each stat at the class's `PlayerStatusParam` row. So stats counted from
//! one class's base, written onto a character of another class, make a character the game could
//! never produce.
//!
//! Measured 2026-09-28 from `ds2-loader.log`: a Sorcerer (ATT 12, INT 14) was given a generated
//! Warrior build, and the import wrote ATT 6 and INT 5 -- below the Sorcerer's own starting stats,
//! and with them every attunement slot -- while the character stayed a Sorcerer.
//!
//! The game has no class change after creation, so the import makes one: [`plan_class`] says the
//! class moves to the build's, and `ds2-build-import` writes it where the game keeps it before it
//! writes the stats. [`check_build`] is the stricter rule for a caller that must keep the class.
//!
//! # Where the numbers come from
//!
//! The id is the `u32` at `player_data + 0x64` (`ds2_rva::PLAYER_DATA_CLASS_OFFSET`), which the
//! profile loader widens out of the save slot record's `u16` at `+0x1D6` (`0x1400fc311`,
//! `0x1400ee402`). On disk the same field is at record `+0x1EA` -- the file's record is `0x14`
//! further on, as the name is (`+0x19E` on disk, `+0x18A` at runtime) -- and the user's Sorcerer
//! reads `7` there, its blank slots `10`.
//!
//! The base spreads and levels are `PlayerStatusParam` rows `20, 30, 50, 70, 80, 90, 100, 110`,
//! read out of `enc_regulation.bnd.dcx` with `scripts/ds2-regulation.py param PlayerStatusParam`
//! on 2026-09-28: `u16` level at `+0x04`, then VIG `+0x06`, END `+0x0C`, ATT `+0x0E`, VIT `+0x10`,
//! STR `+0x12`, DEX `+0x14`, INT `+0x16`, FTH `+0x18`, ADP `+0x1A`. Row `(id + 1) * 10` holds
//! class `id`: the rows skip `40` and `60` exactly where the ids skip `3` and `5`, and row `80`
//! is the Sorcerer spread the save above carries under id `7`. The same spreads are soulsplanner's
//! `C` lines in `ds2-build-recommender.dat`, and a test in `ds2-build-recommender-core` holds the
//! two together.

use crate::level::StatSpread;
#[cfg(test)]
use crate::level::soul_level;

/// The nine stat names, in the game's order -- the order [`StatSpread`]s here are in, and the
/// order `crate::Stats::in_game_order` returns.
pub const GAME_ORDER_NAMES: [&str; 9] = [
    "vigor",
    "endurance",
    "vitality",
    "attunement",
    "strength",
    "dexterity",
    "intelligence",
    "faith",
    "adaptability",
];

/// A DARK SOULS II starting class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartingClass {
    /// Id 1.
    Warrior,
    /// Id 2.
    Knight,
    /// Id 4.
    Bandit,
    /// Id 6.
    Cleric,
    /// Id 7.
    Sorcerer,
    /// Id 8.
    Explorer,
    /// Id 9.
    Swordsman,
    /// Id 10.
    Deprived,
}

impl StartingClass {
    /// Every class, in id order.
    pub const ALL: [Self; 8] = [
        Self::Warrior,
        Self::Knight,
        Self::Bandit,
        Self::Cleric,
        Self::Sorcerer,
        Self::Explorer,
        Self::Swordsman,
        Self::Deprived,
    ];

    /// The class the game stores as `id`. `None` for an id no class has -- the gaps at 3 and 5
    /// are the game's.
    pub const fn from_game_id(id: u32) -> Option<Self> {
        Some(match id {
            1 => Self::Warrior,
            2 => Self::Knight,
            4 => Self::Bandit,
            6 => Self::Cleric,
            7 => Self::Sorcerer,
            8 => Self::Explorer,
            9 => Self::Swordsman,
            10 => Self::Deprived,
            _ => return None,
        })
    }

    /// The id the game stores for this class.
    pub const fn game_id(self) -> u32 {
        match self {
            Self::Warrior => 1,
            Self::Knight => 2,
            Self::Bandit => 4,
            Self::Cleric => 6,
            Self::Sorcerer => 7,
            Self::Explorer => 8,
            Self::Swordsman => 9,
            Self::Deprived => 10,
        }
    }

    /// The class as soulsplanner and the recommender key it: `sorcerer`. What `crate::Build::class`
    /// holds.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Warrior => "warrior",
            Self::Knight => "knight",
            Self::Bandit => "bandit",
            Self::Cleric => "cleric",
            Self::Sorcerer => "sorcerer",
            Self::Explorer => "explorer",
            Self::Swordsman => "swordsman",
            Self::Deprived => "deprived",
        }
    }

    /// The class `key` names, ignoring case and surrounding space: `Sorcerer` and `sorcerer` both.
    pub fn from_key(key: &str) -> Option<Self> {
        let key = key.trim();
        Self::ALL
            .into_iter()
            .find(|class| class.key().eq_ignore_ascii_case(key))
    }

    /// The nine starting stats, in the game's order ([`GAME_ORDER_NAMES`]).
    pub const fn base(self) -> StatSpread {
        match self {
            Self::Warrior => [7, 6, 6, 5, 15, 11, 5, 5, 5],
            Self::Knight => [12, 6, 7, 4, 11, 8, 3, 6, 9],
            Self::Bandit => [9, 7, 11, 2, 9, 14, 1, 8, 3],
            Self::Cleric => [10, 3, 8, 10, 11, 5, 4, 12, 4],
            Self::Sorcerer => [5, 6, 5, 12, 3, 7, 14, 4, 8],
            Self::Explorer => [7, 6, 9, 7, 6, 6, 5, 5, 12],
            Self::Swordsman => [4, 8, 4, 6, 9, 16, 7, 5, 6],
            Self::Deprived => [6; 9],
        }
    }

    /// The starting soul level, as `PlayerStatusParam` records it. Always `soul_level(base())`.
    pub const fn level(self) -> u32 {
        match self {
            Self::Warrior | Self::Swordsman => 12,
            Self::Knight => 13,
            Self::Bandit | Self::Sorcerer => 11,
            Self::Cleric => 14,
            Self::Explorer => 10,
            Self::Deprived => 1,
        }
    }
}

impl core::fmt::Display for StartingClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} (id {})", self.key(), self.game_id())
    }
}

/// Why a build's stats may not go on a character.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClassRefusal {
    /// The build names no class this knows.
    UnknownBuildClass(String),
    /// The build is for another class. Its stats were counted from that class's base.
    ClassMismatch {
        /// The class the build names.
        build: StartingClass,
        /// The class the character has.
        character: StartingClass,
    },
    /// Stats below the character's class base: `(index into GAME_ORDER_NAMES, wanted, base)`.
    BelowBase(Vec<(usize, u16, u16)>),
}

impl core::fmt::Display for ClassRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownBuildClass(class) => {
                write!(
                    f,
                    "the build names class {class:?}, which is no starting class"
                )
            }
            Self::ClassMismatch { build, character } => write!(
                f,
                "the build is for a {build} and this character is a {character} -- the game has \
                 no class change, and the build's stats were counted from the wrong base"
            ),
            Self::BelowBase(below) => {
                f.write_str("stats below the character's class base:")?;
                for &(index, wanted, base) in below {
                    write!(f, " {} {wanted} < {base}", GAME_ORDER_NAMES[index])?;
                }
                Ok(())
            }
        }
    }
}

/// Whether a build for `build_class` with stats `wanted` (game order) may go on a `character`.
///
/// Two rules, checked in this order: the build names the character's own class, and no stat is
/// below that class's base. The second holds even when the first does, because a build can name the
/// right class and still carry a spread from somewhere else.
///
/// # Errors
///
/// The first rule the build breaks.
pub fn check_build(
    build_class: &str,
    character: StartingClass,
    wanted: &StatSpread,
) -> Result<(), ClassRefusal> {
    let Some(build) = StartingClass::from_key(build_class) else {
        return Err(ClassRefusal::UnknownBuildClass(build_class.to_owned()));
    };
    if build != character {
        return Err(ClassRefusal::ClassMismatch { build, character });
    }
    let below: Vec<(usize, u16, u16)> = wanted
        .iter()
        .zip(character.base())
        .enumerate()
        .filter(|&(_, (&stat, base))| stat < base)
        .map(|(index, (&stat, base))| (index, stat, base))
        .collect();
    if below.is_empty() {
        Ok(())
    } else {
        Err(ClassRefusal::BelowBase(below))
    }
}

/// What applying a build does to the character's class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClassPlan {
    /// The build is for the class the character already has.
    Keep(StartingClass),
    /// The build is for another class, and the character becomes it.
    Change {
        /// The class the character has now.
        from: StartingClass,
        /// The class the build names, and the character's after the apply.
        to: StartingClass,
    },
}

impl ClassPlan {
    /// The class the character has once the build is on.
    pub const fn target(self) -> StartingClass {
        match self {
            Self::Keep(class) | Self::Change { to: class, .. } => class,
        }
    }
}

/// What a build for `build_class` with stats `wanted` (game order) does to a `character`'s class.
///
/// A build for another class changes the class to the build's, so that the stats and the base they
/// were counted from agree. What is still refused is a build no class can explain: one naming no
/// class, or one with a stat under its own class's base.
///
/// # Errors
///
/// [`ClassRefusal::UnknownBuildClass`] or [`ClassRefusal::BelowBase`], the latter measured against
/// the build's class, which is the one the character is about to have.
pub fn plan_class(
    build_class: &str,
    character: StartingClass,
    wanted: &StatSpread,
) -> Result<ClassPlan, ClassRefusal> {
    let Some(build) = StartingClass::from_key(build_class) else {
        return Err(ClassRefusal::UnknownBuildClass(build_class.to_owned()));
    };
    check_build(build_class, build, wanted)?;
    Ok(if build == character {
        ClassPlan::Keep(character)
    } else {
        ClassPlan::Change {
            from: character,
            to: build,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured case: a Sorcerer given a Warrior build at SL 90, in the log's own numbers.
    #[test]
    fn a_warrior_build_on_a_sorcerer_is_refused() {
        let sorcerer = StartingClass::from_game_id(7).expect("id 7");
        assert_eq!(sorcerer, StartingClass::Sorcerer);
        // `ds2-build-import: character now: ... vigor=5 endurance=6 vitality=5 attunement=12
        // strength=3 dexterity=7 intelligence=14 faith=4 adaptability=8` -- the Sorcerer's base.
        assert_eq!(sorcerer.base(), [5, 6, 5, 12, 3, 7, 14, 4, 8]);
        // `stats [5, 6, 5, 12, 3, 7, 14, 4, 8] -> [22, 6, 11, 6, 28, 42, 5, 5, 18], level 11 -> 90`
        let written: StatSpread = [22, 6, 11, 6, 28, 42, 5, 5, 18];
        assert_eq!(soul_level(&written), 90);
        assert_eq!(
            check_build("warrior", sorcerer, &written),
            Err(ClassRefusal::ClassMismatch {
                build: StartingClass::Warrior,
                character: StartingClass::Sorcerer,
            })
        );
        // Named as a Sorcerer build, the same spread is still refused -- and the refusal names
        // exactly the two stats the character lost.
        let refusal = check_build("sorcerer", sorcerer, &written).expect_err("below base");
        assert_eq!(
            refusal,
            ClassRefusal::BelowBase(vec![(3, 6, 12), (6, 5, 14)])
        );
        let text = refusal.to_string();
        assert!(text.contains("attunement 6 < 12"), "{text}");
        assert!(text.contains("intelligence 5 < 14"), "{text}");
    }

    /// The measured build, applied: the Sorcerer becomes a Warrior, and the Warrior spread is at
    /// or above the Warrior base, so nothing is refused.
    #[test]
    fn a_warrior_build_makes_a_sorcerer_a_warrior() {
        let written: StatSpread = [22, 6, 11, 6, 28, 42, 5, 5, 18];
        let plan = plan_class("warrior", StartingClass::Sorcerer, &written).expect("a Warrior");
        assert_eq!(
            plan,
            ClassPlan::Change {
                from: StartingClass::Sorcerer,
                to: StartingClass::Warrior,
            }
        );
        assert_eq!(plan.target(), StartingClass::Warrior);
        assert_eq!(
            plan_class(
                "sorcerer",
                StartingClass::Sorcerer,
                &StartingClass::Sorcerer.base()
            ),
            Ok(ClassPlan::Keep(StartingClass::Sorcerer))
        );
    }

    /// A class change does not excuse a spread under the new class's base: a "Warrior" with
    /// Sorcerer strength is no Warrior.
    #[test]
    fn a_change_is_checked_against_the_new_base() {
        let refusal = plan_class(
            "warrior",
            StartingClass::Sorcerer,
            &StartingClass::Sorcerer.base(),
        )
        .expect_err("STR 3 < 15");
        assert!(refusal.to_string().contains("strength 3 < 15"), "{refusal}");
        assert!(plan_class("", StartingClass::Sorcerer, &[99; 9]).is_err());
    }

    /// A Sorcerer build that keeps every stat at or above the Sorcerer base goes through.
    #[test]
    fn a_sorcerer_build_at_or_above_base_is_accepted() {
        let sorcerer = StartingClass::Sorcerer;
        assert_eq!(check_build("Sorcerer", sorcerer, &sorcerer.base()), Ok(()));
        let levelled: StatSpread = [21, 20, 14, 12, 32, 8, 14, 4, 18];
        assert_eq!(check_build("sorcerer", sorcerer, &levelled), Ok(()));
    }

    /// A build naming no class is refused, not waved through as "any class".
    #[test]
    fn an_unknown_class_is_refused() {
        assert_eq!(
            check_build("", StartingClass::Deprived, &[6; 9]),
            Err(ClassRefusal::UnknownBuildClass(String::new()))
        );
        assert!(check_build("pyromancer", StartingClass::Deprived, &[6; 9]).is_err());
    }

    /// Every class's spread is the level `PlayerStatusParam` records for it, by the game's own
    /// `sum - 53` -- a typo in the table breaks this.
    #[test]
    fn every_base_is_its_own_starting_level() {
        for class in StartingClass::ALL {
            assert_eq!(soul_level(&class.base()), class.level(), "{class}");
        }
    }

    /// Ids and keys round-trip, and the ids the game skips answer `None`.
    #[test]
    fn ids_and_keys_round_trip() {
        for class in StartingClass::ALL {
            assert_eq!(StartingClass::from_game_id(class.game_id()), Some(class));
            assert_eq!(StartingClass::from_key(class.key()), Some(class));
        }
        for id in [0, 3, 5, 11, u32::MAX] {
            assert_eq!(StartingClass::from_game_id(id), None, "{id}");
        }
    }

    /// The names here are the order `Stats::in_game_order` fills.
    #[test]
    fn the_names_are_in_game_order() {
        let stats = crate::Stats {
            vigor: 1,
            endurance: 2,
            vitality: 3,
            attunement: 4,
            strength: 5,
            dexterity: 6,
            adaptability: 9,
            intelligence: 7,
            faith: 8,
        };
        assert_eq!(stats.in_game_order(), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(GAME_ORDER_NAMES[6], "intelligence");
        assert_eq!(GAME_ORDER_NAMES[8], "adaptability");
    }
}
