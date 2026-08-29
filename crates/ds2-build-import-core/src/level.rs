//! Soul memory, soul level, and the order the two must be written in.
//!
//! # The rule this module exists to make unbreakable
//!
//! **Soul memory must be calculated and assigned BEFORE the level is set.** A character's soul
//! memory is the total souls they have ever held, and DARK SOULS II derives matchmaking from it; a
//! level that soul memory cannot account for is a character the game's own rules say cannot exist.
//! The repo already knows this from the other direction -- `ds2-mods-rs-lmd` is "refuse a save whose
//! soul memory cannot support its level".
//!
//! A comment saying "write soul memory first" is a rule that gets broken by the next person in a
//! hurry, and the failure is silent. So the ordering is not a comment here. **There is no way to
//! obtain a [`LevelChange`] without having computed the soul memory for it**, because computing the
//! soul memory is what constructs one, and the writer takes a [`LevelChange`] rather than a number.
//! Getting the order wrong is not a mistake you can make; it is a program that does not compile.
//!
//! # Where the numbers come from, and where they do not
//!
//! The per-level costs are DATA, injected by the caller, never a formula written from memory. They
//! live in **`PlayerLevelUpSoulsParam`** -- 852 rows, stride 12, `{u16 level; u16 pad; i32 gradient;
//! i32 souls}` -- inside `enc_regulation.bnd.dcx`. `scripts/ds2-regulation.py souls` prints it, and
//! `ds2-build-import` reads it out of the params the game has already loaded, which cannot drift
//! from the running build and reflects any param mod.
//!
//! **This pointed at `LevelUpStatusCalcParam` for two commits and that is the wrong param.** It has
//! a promising name, it is genuinely in the regulation, and it is a NINE-ROW menu table -- one
//! packed `u32` per stat -- listed in the executable among `FeTimeSetting`, `FeColorPalette` and
//! other frontend params. Nine rows is nine stats, not 850 levels. Nothing about the name was
//! wrong; everything about the inference from it was.
//!
//! # The direction, which is where one level goes missing
//!
//! **Row `L` holds the cost of going from level `L` to `L+1`.** Verified from the game's own
//! level-up code both ways: the increment at `FUN_1401fb970` pays the value stored for the level it
//! is LEAVING, and the decrement at `FUN_1401fb800` steps down from `L` and refunds `lookup(L-1)` --
//! which is only correct under this reading.
//!
//! So the soul memory a level implies is the sum of rows `1..level-1`, and [`SoulCosts`] indexes
//! `costs[0]` as row 1. Level 1 costs nothing to have reached. Anchors from the shipped table:
//! `1 -> 2` costs `500`, soul memory at level 3 is `1,028`, and at level 838 -- every stat at 99 --
//! it is `407,405,588`.
//!
//! **This module contains no cost numbers and will not invent any.** A caller with no table gets a
//! refusal, not a guess. That is deliberate: a soul-memory figure that is confidently wrong is worse
//! than none, because it produces a character that looks legitimate and is not.

/// A character level. The game's own ceiling is 838 -- every stat at 99 from the lowest start.
pub const MAX_LEVEL: u32 = 838;

/// The per-level soul costs, as read out of the game's own data.
///
/// `costs[n]` is the souls needed to go from level `n` to level `n + 1`. Index `0` is therefore the
/// cost of the first level-up, which is why the table is addressed by the level being LEFT rather
/// than the one being reached -- the same convention the param uses.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SoulCosts {
    costs: Vec<u64>,
}

/// Why a level change could not be computed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LevelError {
    /// No cost table was supplied. **Not recoverable by guessing.**
    NoCostTable,
    /// The requested level is outside `1..=MAX_LEVEL`.
    OutOfRange { level: u32 },
    /// The cost table does not reach the requested level.
    TableTooShort { level: u32, covered: u32 },
    /// The souls implied by the level do not fit a `u64`. Only reachable with a corrupt table.
    Overflow,
}

impl core::fmt::Display for LevelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LevelError::NoCostTable => write!(
                f,
                "no soul cost table -- PlayerLevelUpSoulsParam has not been read"
            ),
            LevelError::OutOfRange { level } => {
                write!(f, "level {level} is outside 1..={MAX_LEVEL}")
            }
            LevelError::TableTooShort { level, covered } => write!(
                f,
                "the cost table covers levels up to {covered}, not {level}"
            ),
            LevelError::Overflow => write!(f, "the soul total does not fit a u64"),
        }
    }
}

impl SoulCosts {
    /// Build a table from the game's own per-level costs.
    ///
    /// `costs[n]` is the cost of going from level `n` to `n + 1`. Refuses an empty table rather than
    /// becoming a table that silently answers zero for every level.
    pub fn new(costs: Vec<u64>) -> Result<Self, LevelError> {
        if costs.is_empty() {
            return Err(LevelError::NoCostTable);
        }
        Ok(Self { costs })
    }

    /// The highest level this table can answer for.
    pub fn covers(&self) -> u32 {
        // A table of N costs takes a character from level 1 to level N + 1.
        self.costs.len() as u32 + 1
    }

    /// Total souls spent reaching `level` from level 1.
    ///
    /// **This is the soul memory that level implies**, and the reason is worth stating rather than
    /// assuming: soul memory counts every soul ever HELD, and reaching a level requires having held
    /// every soul spent on the way. So the cumulative cost is the FLOOR of a legitimate soul memory
    /// at that level, never the exact value -- a real character has also held the souls they spent
    /// on equipment and lost to deaths. A floor is the right thing for a consistency rule: below it
    /// the character is impossible, at or above it they are merely thrifty.
    pub fn souls_to_reach(&self, level: u32) -> Result<u64, LevelError> {
        if level == 0 || level > MAX_LEVEL {
            return Err(LevelError::OutOfRange { level });
        }
        if level > self.covers() {
            return Err(LevelError::TableTooShort {
                level,
                covered: self.covers(),
            });
        }
        // Levels 1..level means costs[0..level-1].
        let mut total: u64 = 0;
        for cost in self.costs.iter().take((level - 1) as usize) {
            total = total.checked_add(*cost).ok_or(LevelError::Overflow)?;
        }
        Ok(total)
    }
}

/// A level change with its soul memory already computed.
///
/// **The only way to make one is [`LevelChange::to_level`], which computes the soul memory as part
/// of construction.** That is the enforcement: a writer that takes this cannot be handed a level
/// whose soul memory nobody worked out, because there is no such value to hand it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LevelChange {
    level: u32,
    soul_memory: u64,
}

impl LevelChange {
    /// Work out the soul memory for `level`, and pair the two.
    pub fn to_level(level: u32, costs: &SoulCosts) -> Result<Self, LevelError> {
        Ok(Self {
            level,
            soul_memory: costs.souls_to_reach(level)?,
        })
    }

    /// The level to write. **Second.**
    pub const fn level(self) -> u32 {
        self.level
    }

    /// The soul memory to write. **First.**
    pub const fn soul_memory(self) -> u64 {
        self.soul_memory
    }

    /// Whether an existing soul memory already accounts for this level.
    ///
    /// A character who has played past this level legitimately has MORE soul memory than the floor,
    /// and overwriting it downward would erase progress and move them backwards in matchmaking. So a
    /// writer should raise soul memory to the floor and otherwise leave it alone -- this is the test
    /// for "leave it alone".
    pub const fn already_satisfied_by(self, current_soul_memory: u64) -> bool {
        current_soul_memory >= self.soul_memory
    }
}

/// The nine stats a build names, in the planner's order.
///
/// Kept as its own type rather than reusing [`crate::Stats`] so the level arithmetic cannot be
/// handed a partially-filled build by accident.
pub type StatSpread = [u16; 9];

/// The level a stat spread implies, given the class's starting spread and starting level.
///
/// A DS2 level is one point in one stat, so the level is the starting level plus every point spent
/// above the class's base. Both inputs are DATA -- the starting spreads live in the game's own
/// params, and this module will not invent them either.
///
/// `None` when any stat is BELOW the class base, which is not a build the game can produce and is
/// almost always a sign the wrong class was paired with the spread.
pub fn level_from_stats(
    stats: &StatSpread,
    class_base: &StatSpread,
    class_level: u32,
) -> Option<u32> {
    let mut spent: u32 = 0;
    for (stat, base) in stats.iter().zip(class_base.iter()) {
        if stat < base {
            return None;
        }
        spent += u32::from(stat - base);
    }
    Some(class_level + spent)
}

/// What the soul-level sum subtracts. `level = max(1, sum(nine stats) - 53)`.
///
/// `9 * 6 - 1`: a Deprived starts with 6 in every stat, `9 * 6 = 54`, and `54 - 53 = 1`.
pub const SOUL_LEVEL_BIAS: u32 = 53;

/// The soul level a stat spread IS. **Read off the game's own arithmetic, not inferred.**
///
/// # A soul level is a cached sum, not a number you choose
///
/// `0x14038e310` computes exactly this and writes it to `PlayerParam + 0xD0` every time the stats
/// change. So there is no such thing as setting a level: set the nine stats and the level is
/// already decided. That retires the whole problem [`level_from_stats`] exists for -- it needs the
/// class's starting spread, and this needs nothing.
///
/// It also means a build and a level cannot disagree. If a planner shows a level that this does not
/// produce from the same stats, the planner is wrong or the stats were misread; there is no third
/// possibility and nothing to reconcile.
///
/// The `max(1, ...)` matters for spreads below a Deprived's, which no real character has but a
/// malformed build can name.
pub fn soul_level(stats: &StatSpread) -> u32 {
    let total: u32 = stats.iter().copied().map(u32::from).sum();
    total.saturating_sub(SOUL_LEVEL_BIAS).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **THE ONE THAT PROVES THE FORMULA.** soulsplanner build 253 shows level 150.
    ///
    /// Its nine stats total 203, and the game's own `0x14038e310` subtracts 53. Two independent
    /// sources -- a website's displayed number and a disassembled sum -- agreeing on 150 is what
    /// makes this arithmetic a fact rather than a reading of it.
    #[test]
    fn a_real_builds_stats_produce_the_level_the_planner_shows() {
        // vigor, endurance, vitality, attunement, strength, dexterity, adaptability, int, faith --
        // as the log printed them for build 253.
        let stats: StatSpread = [50, 20, 4, 16, 25, 16, 16, 28, 28];
        assert_eq!(stats.iter().map(|s| u32::from(*s)).sum::<u32>(), 203);
        assert_eq!(soul_level(&stats), 150);
    }

    /// A Deprived is level 1, which is what fixes the bias at 53 rather than 54 or 52.
    #[test]
    fn a_deprived_is_level_one() {
        assert_eq!(soul_level(&[6; 9]), 1);
        // And one point in one stat is one level, which is the whole of DS2 levelling.
        assert_eq!(soul_level(&[7, 6, 6, 6, 6, 6, 6, 6, 6]), 2);
    }

    /// A spread below a Deprived's cannot go below level 1.
    #[test]
    fn nothing_is_below_level_one() {
        assert_eq!(soul_level(&[1; 9]), 1);
        assert_eq!(soul_level(&[0; 9]), 1);
    }

    /// The cap the game enforces on nine stats at 99 agrees with [`MAX_LEVEL`].
    #[test]
    fn nine_stats_at_ninety_nine_is_the_maximum_level() {
        assert_eq!(soul_level(&[99; 9]), MAX_LEVEL);
    }

    /// A small table standing in for the game's, so the arithmetic can be pinned without pretending
    /// to know the real costs. Cost of level 1->2 is 10, 2->3 is 20, 3->4 is 30.
    fn costs() -> SoulCosts {
        SoulCosts::new(vec![10, 20, 30]).expect("a non-empty table")
    }

    /// Soul memory is the cumulative cost, and level 1 costs nothing.
    #[test]
    fn soul_memory_is_everything_spent_getting_there() {
        let costs = costs();
        assert_eq!(costs.souls_to_reach(1), Ok(0));
        assert_eq!(costs.souls_to_reach(2), Ok(10));
        assert_eq!(costs.souls_to_reach(3), Ok(30));
        assert_eq!(costs.souls_to_reach(4), Ok(60));
    }

    /// THE ORDER IS NOT A CONVENTION, IT IS THE ONLY THING THE TYPE PERMITS.
    ///
    /// There is no constructor for `LevelChange` that does not compute soul memory, so a writer
    /// taking one cannot be given a level without it. This test pins the property by pinning the
    /// only route in -- if someone adds a second constructor that skips the computation, the
    /// enforcement is gone and this comment is the warning.
    #[test]
    fn a_level_cannot_be_produced_without_its_soul_memory() {
        let change = LevelChange::to_level(3, &costs()).expect("in range");
        assert_eq!(change.level(), 3);
        assert_eq!(change.soul_memory(), 30);
        // And the pair travels together -- there is no way to hold one without the other.
        let copied = change;
        assert_eq!(copied.soul_memory(), change.soul_memory());
    }

    /// A missing table refuses rather than guessing a formula.
    #[test]
    fn no_table_is_a_refusal_not_a_guess() {
        assert_eq!(SoulCosts::new(Vec::new()), Err(LevelError::NoCostTable));
        assert!(
            LevelError::NoCostTable
                .to_string()
                .contains("PlayerLevelUpSoulsParam"),
            "the refusal must name the param a caller has to go and read"
        );
    }

    /// A level the table cannot reach says so, with both numbers.
    #[test]
    fn a_short_table_names_what_it_covers() {
        let costs = costs();
        assert_eq!(costs.covers(), 4);
        assert_eq!(
            costs.souls_to_reach(5),
            Err(LevelError::TableTooShort {
                level: 5,
                covered: 4
            })
        );
    }

    /// Level zero and past the ceiling are both out of range.
    #[test]
    fn the_level_range_is_the_games_own() {
        let costs = costs();
        assert_eq!(
            costs.souls_to_reach(0),
            Err(LevelError::OutOfRange { level: 0 })
        );
        assert_eq!(
            costs.souls_to_reach(MAX_LEVEL + 1),
            Err(LevelError::OutOfRange {
                level: MAX_LEVEL + 1
            })
        );
        // 838 is nine stats at 99 from the lowest start, which is the game's own ceiling.
        assert_eq!(MAX_LEVEL, 838);
    }

    /// A character who has played further keeps their soul memory.
    ///
    /// Lowering it would erase progress and move them backwards in matchmaking, so the floor is a
    /// minimum to raise TO, never a value to overwrite WITH.
    #[test]
    fn soul_memory_is_a_floor_and_not_a_target() {
        let change = LevelChange::to_level(3, &costs()).expect("in range");
        assert!(
            change.already_satisfied_by(30),
            "exactly the floor is enough"
        );
        assert!(change.already_satisfied_by(1_000_000), "more is fine");
        assert!(!change.already_satisfied_by(29), "one short is not");
    }

    /// The level a spread implies is the points spent above the class base.
    #[test]
    fn a_spread_implies_a_level() {
        let base: StatSpread = [7, 6, 6, 5, 15, 11, 5, 5, 7];
        // The class as shipped is its own starting level.
        assert_eq!(level_from_stats(&base, &base, 1), Some(1));
        // One point in one stat is one level.
        let mut one = base;
        one[0] += 1;
        assert_eq!(level_from_stats(&one, &base, 1), Some(2));
        // Ten points across three stats is ten levels.
        let mut spread = base;
        spread[0] += 5;
        spread[4] += 3;
        spread[8] += 2;
        assert_eq!(level_from_stats(&spread, &base, 1), Some(11));
    }

    /// A stat below the class base is not a build the game can produce.
    #[test]
    fn a_spread_under_the_class_base_is_refused() {
        let base: StatSpread = [7, 6, 6, 5, 15, 11, 5, 5, 7];
        let mut under = base;
        under[4] -= 1;
        assert_eq!(level_from_stats(&under, &base, 1), None);
    }
}
