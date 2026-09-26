//! Say, when a character loads, whether its soul memory could have paid for its soul level.
//!
//! Soul memory counts every soul the character has ever gained, and every level bought past the
//! class's starting level was paid for in souls. So a character at level `N` that started at level
//! `S` has a soul memory of at least `cost(S) + cost(S + 1) + ... + cost(N - 1)`, where `cost(L)` is
//! the game's own price of going from `L` to `L + 1`. A character below that floor was not levelled
//! in the game.
//!
//! # What it does, and what it does not
//!
//! It logs one verdict line per load and changes nothing. No verified mechanism exists for turning
//! a load away once the world-entry spawn has started restoring the character, and `[offline]`
//! already keeps such a character off the network, so a line in the log is the whole feature.
//!
//! # The start level is a bound, not a lookup
//!
//! The save does not carry the level the class started at, and the per-class values were not read
//! from the binary. So the floor is summed from [`HIGHEST_CLASS_START_LEVEL`] rather than from the
//! character's own class. That can only lower the floor, which means a legitimate character never
//! fails the check and a forged one is caught once its level is far enough past any start.
//!
//! `docs/DS2-SOUL-LEVEL.md` has the level formula, the cost function and the load path this reads.

/// What every line this crate writes begins with, so its lines can be grepped out of the shared log.
pub const LOG_PREFIX: &str = "ds2-soul-memory-guard:";

/// The highest level any starting class begins at. **Inferred**, not read from the binary.
///
/// Community class tables give Cleric at 14 as the highest (Deprived 1, Explorer 10, Bandit and
/// Sorcerer 11, Warrior and Swordsman 12, Knight 13). A live Knight-shaped character read at level
/// 13 with a soul memory of 1650, which a start of 1 would have called short by thousands, so the
/// start really does matter. Summing from a value at or above every class's start is what keeps
/// the check from failing a real character; if this is ever read from the character-creation data
/// and turns out higher, raising it is the whole fix.
pub const HIGHEST_CLASS_START_LEVEL: u32 = 14;

/// The highest soul level nine stats can produce: `9 * 99 - 53`.
pub const MAX_SOUL_LEVEL: u32 = 838;

/// What the check concluded about one character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Soul memory is at or above the floor.
    Supported,
    /// Soul memory is below the floor by `missing` souls.
    Short {
        /// How far below the floor it is.
        missing: u64,
    },
    /// The level is outside `1..=MAX_SOUL_LEVEL`, which no stat spread the game accepts produces.
    LevelOutOfRange,
    /// The cost function could not be asked, so there is no floor to compare with.
    NoCost,
}

impl Verdict {
    /// The word the log line leads with.
    pub const fn word(self) -> &'static str {
        match self {
            Verdict::Supported => "supported",
            Verdict::Short { .. } => "short",
            Verdict::LevelOutOfRange => "level-out-of-range",
            Verdict::NoCost => "unknown",
        }
    }
}

/// One character, judged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Check {
    /// The soul level, as the game stored it after the load.
    pub level: u32,
    /// Soul memory, the first counter. This is the one the verdict is taken against.
    pub soul_memory: u32,
    /// The second soul-memory counter, reported beside the first.
    pub soul_memory_2: u32,
    /// The level the floor is summed from.
    pub from_level: u32,
    /// The least soul memory that level could have been reached with, when it could be computed.
    pub floor: Option<u64>,
    /// The conclusion.
    pub verdict: Verdict,
}

/// Souls spent getting from `from_level` to `level`, asking `cost` for each step.
///
/// `cost(L)` is the price of `L -> L + 1`. A level at or below `from_level` costs nothing, because
/// the character could have started there. A negative price, which only a broken or edited table
/// produces, counts as zero: it can only lower the floor, never fail a character. `None` when
/// `cost` has no answer for a step.
pub fn soul_memory_floor(
    level: u32,
    from_level: u32,
    mut cost: impl FnMut(u32) -> Option<i32>,
) -> Option<u64> {
    let mut total: u64 = 0;
    for step in from_level..level {
        let price = cost(step)?;
        total = total.saturating_add(u64::try_from(price).unwrap_or(0));
    }
    Some(total)
}

/// Judge one character. `cost` is only asked when the level is in range.
pub fn judge(level: u32, soul_memory: [u32; 2], cost: impl FnMut(u32) -> Option<i32>) -> Check {
    let from_level = HIGHEST_CLASS_START_LEVEL;
    let (floor, verdict) = if level == 0 || level > MAX_SOUL_LEVEL {
        (None, Verdict::LevelOutOfRange)
    } else {
        match soul_memory_floor(level, from_level, cost) {
            None => (None, Verdict::NoCost),
            Some(floor) => {
                let held = u64::from(soul_memory[0]);
                let verdict = if held >= floor {
                    Verdict::Supported
                } else {
                    Verdict::Short {
                        missing: floor - held,
                    }
                };
                (Some(floor), verdict)
            }
        }
    };
    Check {
        level,
        soul_memory: soul_memory[0],
        soul_memory_2: soul_memory[1],
        from_level,
        floor,
        verdict,
    }
}

impl core::fmt::Display for Check {
    /// The verdict line, without the prefix and without the address.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "verdict={} level={} soul_memory={} soul_memory_2={} from_level={}",
            self.verdict.word(),
            self.level,
            self.soul_memory,
            self.soul_memory_2,
            self.from_level,
        )?;
        match self.floor {
            Some(floor) => write!(f, " floor={floor}")?,
            None => write!(f, " floor=none")?,
        }
        match self.verdict {
            Verdict::Supported => Ok(()),
            Verdict::Short { missing } => write!(
                f,
                " missing={missing} -- soul memory cannot pay for this level; logged only, the \
                 load goes ahead"
            ),
            Verdict::LevelOutOfRange => write!(
                f,
                " -- no spread of nine stats in 1..=99 gives this level; logged only, the load \
                 goes ahead"
            ),
            Verdict::NoCost => write!(
                f,
                " -- the level-up cost table could not be read, so nothing was judged"
            ),
        }
    }
}

#[cfg(windows)]
mod install;

#[cfg(windows)]
pub use install::{LogFn, Outcome, install, set_logger};

#[cfg(test)]
mod tests {
    use super::*;

    /// The first rows of the shipped table as a live process held them: `cost(1) = 500`,
    /// `cost(2) = 528`, `cost(3) = 557`. Past that a flat stand-in, so the sums stay checkable.
    fn shipped(level: u32) -> Option<i32> {
        Some(match level {
            1 => 500,
            2 => 528,
            3 => 557,
            _ => 1000,
        })
    }

    #[test]
    fn nothing_is_owed_at_or_below_the_start() {
        assert_eq!(soul_memory_floor(1, 1, shipped), Some(0));
        assert_eq!(soul_memory_floor(14, 14, shipped), Some(0));
        assert_eq!(soul_memory_floor(13, 14, shipped), Some(0));
    }

    #[test]
    fn the_floor_sums_the_steps_left_not_the_one_reached() {
        // 1 -> 2 -> 3 -> 4 pays cost(1) + cost(2) + cost(3).
        assert_eq!(soul_memory_floor(4, 1, shipped), Some(500 + 528 + 557));
        assert_eq!(soul_memory_floor(2, 1, shipped), Some(500));
    }

    #[test]
    fn a_missing_price_is_no_floor() {
        assert_eq!(soul_memory_floor(20, 14, |_| None), None);
        assert_eq!(
            judge(20, [0, 0], |_| None).verdict,
            Verdict::NoCost,
            "an unreadable table judges nothing"
        );
    }

    #[test]
    fn a_negative_price_lowers_the_floor_and_never_fails_anyone() {
        assert_eq!(soul_memory_floor(17, 14, |_| Some(-5)), Some(0));
    }

    /// The character the live read found: level 13, soul memory 1650. From level 1 it would owe
    /// thousands; from the highest class start it owes nothing.
    #[test]
    fn a_low_level_character_is_never_short() {
        let check = judge(13, [1650, 1650], shipped);
        assert_eq!(check.floor, Some(0));
        assert_eq!(check.verdict, Verdict::Supported);
    }

    #[test]
    fn exactly_the_floor_is_enough_and_one_less_is_not() {
        // 14 -> 16 is two steps of the flat stand-in.
        assert_eq!(judge(16, [2000, 0], shipped).verdict, Verdict::Supported);
        assert_eq!(
            judge(16, [1999, 0], shipped).verdict,
            Verdict::Short { missing: 1 }
        );
    }

    #[test]
    fn the_verdict_is_taken_against_the_first_counter() {
        assert_eq!(
            judge(16, [0, 5000], shipped).verdict,
            Verdict::Short { missing: 2000 }
        );
        assert_eq!(judge(16, [5000, 0], shipped).verdict, Verdict::Supported);
    }

    #[test]
    fn a_level_no_stats_produce_is_reported_without_asking_the_table() {
        let mut asked = false;
        let check = judge(MAX_SOUL_LEVEL + 1, [0, 0], |_| {
            asked = true;
            Some(1)
        });
        assert_eq!(check.verdict, Verdict::LevelOutOfRange);
        assert!(!asked);
        assert_eq!(judge(0, [0, 0], shipped).verdict, Verdict::LevelOutOfRange);
    }

    #[test]
    fn the_line_names_every_number() {
        let line = judge(16, [1999, 1999], shipped).to_string();
        assert_eq!(
            line,
            "verdict=short level=16 soul_memory=1999 soul_memory_2=1999 from_level=14 floor=2000 \
             missing=1 -- soul memory cannot pay for this level; logged only, the load goes ahead"
        );
        assert_eq!(
            judge(13, [1650, 1650], shipped).to_string(),
            "verdict=supported level=13 soul_memory=1650 soul_memory_2=1650 from_level=14 floor=0"
        );
    }
}
