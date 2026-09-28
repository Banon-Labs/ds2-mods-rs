//! What the panel asks, who answers, and the rules that hold whoever answers.
//!
//! [`RecommenderBackend`] is the contract. [`crate::corpus::CorpusBackend`] answers it with the
//! ranking in `scripts/ds2-builds-recommend.py`, ported, over the data file the script exports.
//! [`StubBackend`] answers it with fixed data, for a game folder with no data file; its numbers are
//! placeholders and the panel says so, from [`RecommenderBackend::is_stub`].
//!
//! The rules live OUTSIDE the trait, in [`ask`], [`generate`], [`split_weapons`], [`ring_grants`]
//! and [`to_import`], so a real backend cannot forget them:
//!
//! * **No result is shown for a build under its floors.** [`ask`] checks the stats against
//!   [`RecommenderBackend::floors`] before the backend is asked at all, and [`generate`] checks the
//!   build that comes back. A floor is the bracket's median VIG, VIT, ADP and ATT; a build under it
//!   is one the script would not recommend, so a ranking for it would be advice nobody should take.
//! * **A generated build carries the 15 best one-handable and the 5 best two-hand-only weapons**,
//!   one row per weapon, never the primary again.
//! * **Rings are granted three to a suggestion**, plus one of every ring at least a tenth of all
//!   builds wear that is not already suggested.

use ds2_build_import_core::{Build, Infusion, Stats};

use crate::model::{
    Grip, Mode, Objective, PanelState, STAT_COUNT, STAT_LABELS, StatusFilter, WeaponsForOpts,
};
use crate::weapons;

/// How many one-handable weapons a generated build lists.
pub const WEAPONS_1H_TOP: usize = 15;

/// How many two-hand-only weapons a generated build lists.
pub const WEAPONS_2H_ONLY_TOP: usize = 5;

/// How many rings a build suggests: one per ring slot.
pub const SUGGESTED_RINGS: usize = 4;

/// Copies granted of each suggested ring: one to wear and two for the +1 and +2 a player would
/// otherwise go and find.
pub const SUGGESTED_RING_COPIES: usize = 3;

/// A ring worn by at least this percentage of every build is common to all builds.
pub const COMMON_RING_PERCENT: u32 = 10;

/// The stats a floor applies to, by index into [`STAT_LABELS`]: VIG, VIT, ATT, ADP.
pub const FLOOR_STATS: [usize; 4] = [0, 2, 3, 6];

/// The damage types an attack rating is split into, in [`ResultRow::ar_by_type`] order.
pub const DAMAGE_TYPES: [&str; 5] = ["phys", "magic", "fire", "light", "dark"];

/// One row of a ranking.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    /// The weapon's display name.
    pub weapon: String,
    /// The infusion this row is for.
    pub infusion: Infusion,
    /// The score the mode ranks by: expected damage, attack rating, or for
    /// [`Mode::SimilarBuilds`] the number of neighbouring builds that carry the weapon.
    pub damage: f32,
    /// Attack rating per type, in [`DAMAGE_TYPES`] order.
    pub ar_by_type: [f32; 5],
    /// How the row is held, as the script labels it: `1H`, `2H only`, `2H 3 hits`.
    pub grip: String,
    /// Whether these stats can only wield it two-handed.
    pub two_hand_only: bool,
    /// The R1's hyperarmor multiplier, where it has one.
    pub hyperarmor: Option<f32>,
    /// The counter-hit multiplier, where the weapon's differs from nothing.
    pub counter: Option<f32>,
    /// The weapon's class.
    pub class: String,
}

/// What a ranking came back as.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The ranking, best first.
    Rows(Vec<ResultRow>),
    /// The stats are under the bracket's floors, and no ranking is shown. Each entry reads
    /// `VIG 5 < 12`.
    FloorViolations(Vec<String>),
}

/// A stat spread the backend chose.
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizedBuild {
    /// The starting class, `Deprived`.
    pub class: String,
    /// The soul level.
    pub sl: u16,
    /// The nine stats, in [`STAT_LABELS`] order.
    pub stats: [u16; STAT_COUNT],
    /// Whether the weapon is two-handed to meet its strength.
    pub two_handed: bool,
    /// The objective's value at these stats; `0` for a minimum build, which has no objective.
    pub value: f32,
    /// Armour and rings the stats count on, by display name: a minimum build meets its
    /// requirements with their stat bonuses and cannot wield the weapon without them.
    pub gear: Vec<String>,
}

/// How often the damage model's best infusion is the one real builds chose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    /// Builds that recorded an infusion and were counted.
    pub n: u32,
    /// Share where the model's first choice matched, `0.0..=1.0`.
    pub top1: f32,
    /// Share where the build's infusion was the model's first or second.
    pub top2: f32,
}

/// A whole build for one weapon, ready to show and to apply.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedBuild {
    /// The starting class, `Deprived`.
    pub class: String,
    /// The soul level.
    pub sl: u16,
    /// The nine stats, in [`STAT_LABELS`] order.
    pub stats: [u16; STAT_COUNT],
    /// The weapon the build is for, by soulsplanner key, and its infusion.
    pub primary: (String, Infusion),
    /// Whether the primary is two-handed.
    pub two_handed: bool,
    /// The best one-handable other weapons, at most [`WEAPONS_1H_TOP`].
    pub weapons_1h: Vec<ResultRow>,
    /// The best two-hand-only other weapons, at most [`WEAPONS_2H_ONLY_TOP`].
    pub weapons_2h_only: Vec<ResultRow>,
    /// The rings the nearest builds wear most, at most [`SUGGESTED_RINGS`].
    pub suggested_rings: Vec<String>,
    /// Rings common to all builds that are not already suggested.
    pub common_rings: Vec<String>,
    /// Head, chest, hands, legs, by item name, `Naked` for a slot left bare; empty for a build
    /// that wears none, which only [`PanelState::allow_naked`] asks for.
    pub armor: Vec<String>,
    /// Why the armour is not four pieces, when it is not: the load cap left a slot, or every slot,
    /// bare. Never silent.
    pub armor_note: Option<String>,
    /// Whether this came from [`StubBackend`], so the panel can say its numbers mean nothing.
    pub stub: bool,
}

/// The questions the panel asks. Every answer is for the soul level the caller passes.
pub trait RecommenderBackend: Sync {
    /// Whether the answers are placeholders.
    fn is_stub(&self) -> bool;
    /// The minimum each stat may be at `sl`, in [`STAT_LABELS`] order; `0` is no floor.
    fn floors(&self, sl: u16) -> [u16; STAT_COUNT];
    /// The weapons `stats` wield best.
    fn weapons_for(&self, stats: &[u16; STAT_COUNT], sl: u16, opts: &WeaponsForOpts) -> Outcome;
    /// The stats that make `weapon` hit hardest at `sl`, held in `grip`. `None` when no class can
    /// wield it there that way.
    fn optimize(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        grip: Grip,
    ) -> Option<OptimizedBuild>;
    /// The least a character needs to wield `weapon`. `None` when the weapon is unknown.
    fn minimum(&self, weapon: &str, infusion: Infusion, two_hand: bool) -> Option<OptimizedBuild>;
    /// The weapons the `k` builds nearest `stats` carry.
    fn similar(&self, stats: &[u16; STAT_COUNT], sl: u16, k: u16, status: StatusFilter) -> Outcome;
    /// How far the damage model agrees with real builds.
    fn calibration(&self) -> Calibration;
    /// A whole build for `weapon` held in `grip`, in armour unless `allow_naked`. `None` when no
    /// class can wield it at `sl` that way.
    fn generate_build(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        allow_naked: bool,
        grip: Grip,
    ) -> Option<GeneratedBuild>;
    /// The live character's nine stats, when they can be read.
    fn current_character_stats(&self) -> Option<[u16; STAT_COUNT]>;
    /// How many weapons `stats` wield, where that sits among the builds nearest them at `sl`, and
    /// the load `armor` (head to legs) and `rings`, by name, leave for weapons. `None` when this
    /// backend cannot say, which is the default: the stub has no neighbours to rank against.
    fn flexibility(
        &self,
        _stats: &[u16; STAT_COUNT],
        _sl: u16,
        _armor: &[String],
        _rings: &[String],
    ) -> Option<crate::flex::Flexibility> {
        None
    }
}

/// Each floor `stats` is under, as `VIG 5 < 12`.
pub fn floor_violations(stats: &[u16; STAT_COUNT], floors: &[u16; STAT_COUNT]) -> Vec<String> {
    FLOOR_STATS
        .iter()
        .filter(|&&index| stats[index] < floors[index])
        .map(|&index| {
            format!(
                "{} {} < {}",
                STAT_LABELS[index], stats[index], floors[index]
            )
        })
        .collect()
}

/// What the panel shows for one press of Run.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// A ranking, best first.
    Rows(Vec<ResultRow>),
    /// A stat spread.
    Build(OptimizedBuild),
    /// The stats are under the floors; nothing else is shown.
    FloorViolations(Vec<String>),
    /// The mode could not run, and why.
    Nothing(&'static str),
}

/// Run the panel's current mode against `backend`.
///
/// **The floors are checked here, before the backend is asked**, for the two modes that rank for
/// the player's own stats. A backend that answers anyway is overruled: its [`Outcome`] is only ever
/// shown as rows when the stats clear every floor.
pub fn ask(backend: &dyn RecommenderBackend, state: &PanelState) -> Answer {
    let sl = state.sl();
    let gate = |outcome: Outcome| match outcome {
        Outcome::Rows(rows) => Answer::Rows(rows),
        Outcome::FloorViolations(lines) => Answer::FloorViolations(lines),
    };
    match state.mode {
        Mode::WeaponsForStats | Mode::SimilarBuilds => {
            let violations = floor_violations(&state.stats, &backend.floors(sl));
            if !violations.is_empty() {
                return Answer::FloorViolations(violations);
            }
            gate(if state.mode == Mode::WeaponsForStats {
                let opts = WeaponsForOpts {
                    objective: state.objective,
                    ..state.weapons_for.clone()
                };
                backend.weapons_for(&state.stats, sl, &opts)
            } else {
                backend.similar(&state.stats, sl, state.similar_k, state.status)
            })
        }
        Mode::OptimizeForWeapon | Mode::MinimumForWeapon => {
            let Some(weapon) = state.weapon else {
                return Answer::Nothing("choose a weapon first");
            };
            let built = if state.mode == Mode::OptimizeForWeapon {
                backend.optimize(weapon, state.infusion, sl, state.objective, state.grip)
            } else {
                backend.minimum(weapon, state.infusion, state.two_hand)
            };
            built.map_or(Answer::Nothing("no class can wield it here"), Answer::Build)
        }
    }
}

/// Generate a build for the panel's weapon, refusing one under its own floors.
///
/// # Errors
///
/// The reason as lines for the panel: no weapon chosen, no build possible, or each floor the
/// backend's build is under.
pub fn generate(
    backend: &dyn RecommenderBackend,
    state: &PanelState,
) -> Result<GeneratedBuild, Vec<String>> {
    let Some(weapon) = state.weapon else {
        return Err(vec!["choose a weapon first".to_owned()]);
    };
    let Some(build) = backend.generate_build(
        weapon,
        state.infusion,
        state.sl(),
        state.objective,
        state.allow_naked,
        state.grip,
    ) else {
        return Err(vec!["no class can wield it at this soul level".to_owned()]);
    };
    let violations = floor_violations(&build.stats, &backend.floors(build.sl));
    if violations.is_empty() {
        Ok(build)
    } else {
        Err(violations)
    }
}

/// Split a ranking into the one-handable and two-hand-only lists a generated build carries.
///
/// One row per weapon -- the first, which is its best infusion -- and never `primary_name`.
pub fn split_weapons(ranked: &[ResultRow], primary_name: &str) -> (Vec<ResultRow>, Vec<ResultRow>) {
    let mut seen = vec![primary_name];
    let (mut one, mut two) = (Vec::new(), Vec::new());
    for row in ranked {
        if seen.contains(&row.weapon.as_str()) {
            continue;
        }
        seen.push(&row.weapon);
        if row.two_hand_only {
            two.push(row.clone());
        } else {
            one.push(row.clone());
        }
    }
    one.truncate(WEAPONS_1H_TOP);
    two.truncate(WEAPONS_2H_ONLY_TOP);
    (one, two)
}

/// The rings common to all builds: worn by at least [`COMMON_RING_PERCENT`] of `total_builds`,
/// most worn first, leaving out any already `suggested`.
pub fn common_rings(worn: &[(&str, u32)], total_builds: u32, suggested: &[String]) -> Vec<String> {
    let mut common: Vec<(&str, u32)> = worn
        .iter()
        .copied()
        .filter(|&(ring, count)| {
            u64::from(count) * 100 >= u64::from(COMMON_RING_PERCENT) * u64::from(total_builds)
                && !suggested.iter().any(|name| name == ring)
        })
        .collect();
    common.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    common
        .into_iter()
        .map(|(ring, _)| ring.to_owned())
        .collect()
}

/// Every ring a generated build grants: [`SUGGESTED_RING_COPIES`] of each suggestion, then one of
/// each common ring that is not a suggestion.
pub fn ring_grants(suggested: &[String], common: &[String]) -> Vec<String> {
    let mut grants = Vec::new();
    for ring in suggested.iter().take(SUGGESTED_RINGS) {
        grants.extend(std::iter::repeat_n(ring.clone(), SUGGESTED_RING_COPIES));
    }
    for ring in common {
        if !suggested.contains(ring) && !grants.contains(ring) {
            grants.push(ring.clone());
        }
    }
    grants
}

/// What a planner build calls an empty weapon hand.
const EMPTY_HAND: &str = "Bare_Fists";
/// What a planner build calls an empty ring slot.
const EMPTY_RING: &str = "No_Ring";
/// What a planner build calls an empty armour slot.
const EMPTY_ARMOR: &str = "Naked";
/// What a build with no covenant names.
const NO_COVENANT: &str = "No_Covenant";

/// Turn a generated build into what `ds2-build-import` applies: a planner [`Build`], plus the extra
/// grants its fixed slots cannot hold.
///
/// The build wears the suggested rings, holds the primary in the right hand, and wears the armour.
/// The extras are everything else the build lists: the second and third copy of each suggested
/// ring, one of each common ring, and every recommended weapon at its ranked infusion. They are
/// granted through the same per-name count as the build's own gear, so a player who already holds
/// a copy is given one fewer.
pub fn to_import(generated: &GeneratedBuild) -> (Build, Vec<(String, Infusion)>) {
    let [
        vigor,
        endurance,
        vitality,
        attunement,
        strength,
        dexterity,
        adaptability,
        intelligence,
        faith,
    ] = generated.stats;
    let stats = Stats {
        vigor,
        endurance,
        vitality,
        attunement,
        strength,
        dexterity,
        adaptability,
        intelligence,
        faith,
    };
    let (primary, infusion) = &generated.primary;
    // `LH1, RH1, LH2, RH2, LH3, RH3`, each followed by its infusion: the primary is RH1.
    let mut hands = vec![
        EMPTY_HAND.to_owned(),
        weapons::planner_name(Infusion::None).to_owned(),
    ];
    hands.push(primary.clone());
    hands.push(weapons::planner_name(*infusion).to_owned());
    while hands.len() < 12 {
        hands.push(EMPTY_HAND.to_owned());
        hands.push(weapons::planner_name(Infusion::None).to_owned());
    }
    let worn: Vec<String> = generated
        .suggested_rings
        .iter()
        .take(SUGGESTED_RINGS)
        .cloned()
        .collect();
    let mut rings = worn.clone();
    rings.resize(SUGGESTED_RINGS, EMPTY_RING.to_owned());
    let mut armor: Vec<String> = generated.armor.iter().take(4).cloned().collect();
    armor.resize(4, EMPTY_ARMOR.to_owned());
    let build = Build {
        id: 0,
        class: generated.class.to_lowercase().replace(' ', "_"),
        gender: 0,
        covenant: NO_COVENANT.to_owned(),
        grip: i64::from(generated.two_handed),
        armor,
        weapons: hands,
        rings,
        spells: Vec::new(),
        items: Vec::new(),
        stats,
    };

    // The rings the build does not already wear: every grant less one copy of each worn ring.
    let mut extras: Vec<(String, Infusion)> = Vec::new();
    let mut unworn = worn;
    for ring in ring_grants(&generated.suggested_rings, &generated.common_rings) {
        if let Some(at) = unworn.iter().position(|name| *name == ring) {
            unworn.remove(at);
            continue;
        }
        extras.push((ring, Infusion::None));
    }
    for row in generated
        .weapons_1h
        .iter()
        .chain(&generated.weapons_2h_only)
    {
        extras.push((row.weapon.clone(), row.infusion));
    }
    (build, extras)
}

/// Fixed answers, so the panel can be drawn and applied before the real ranking exists.
///
/// Every number it returns is a placeholder. The generated build is a real, valid one -- the
/// Moonlight Greatsword on a two-handed Deprived at soul level 33 -- because Apply puts it on a
/// character, and a stub that applied nonsense would test nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubBackend;

/// The stub build's stats: VIG 10 END 16 VIT 7 ATT 10 STR 6 DEX 13 ADP 8 INT 10 FTH 6.
pub const STUB_STATS: [u16; STAT_COUNT] = [10, 16, 7, 10, 6, 13, 8, 10, 6];

/// The stub's floors: its own build clears them, a fresh Deprived does not.
const STUB_FLOORS: [u16; STAT_COUNT] = [10, 0, 7, 10, 0, 0, 8, 0, 0];

/// The stub's ranking, best first, by soulsplanner key: infusion, placeholder damage, 2H only.
///
/// The primary and a second Falchion row are in it on purpose, so the split has something to drop.
const STUB_RANKING: [(&str, Infusion, f32, bool); 26] = [
    ("Moonlight_Greatsword", Infusion::None, 410.0, false),
    ("Giant_Stone_Axe", Infusion::Raw, 402.0, true),
    ("Falchion", Infusion::Raw, 395.0, false),
    ("Falchion", Infusion::Bleed, 390.0, false),
    ("Scimitar", Infusion::Raw, 388.0, false),
    ("Greataxe", Infusion::Raw, 385.0, true),
    ("Shotel", Infusion::Bleed, 380.0, false),
    ("Royal_Dirk", Infusion::Raw, 372.0, false),
    ("Demons_Great_Hammer", Infusion::Raw, 368.0, true),
    ("Bandits_Knife", Infusion::Bleed, 366.0, false),
    ("Blacksteel_Katana", Infusion::Raw, 361.0, false),
    ("Eleum_Loyce", Infusion::None, 355.0, false),
    ("Large_Club", Infusion::Raw, 350.0, true),
    ("Melu_Scimitar", Infusion::Raw, 347.0, false),
    ("Hand_Axe", Infusion::Raw, 342.0, false),
    ("Drakekeepers_Warpick", Infusion::Raw, 339.0, true),
    ("Infantry_Axe", Infusion::Raw, 336.0, false),
    ("Work_Hook", Infusion::Bleed, 330.0, false),
    ("Pickaxe", Infusion::Raw, 328.0, true),
    ("Retainers_Short_Sword", Infusion::Raw, 322.0, false),
    ("Whip", Infusion::Raw, 318.0, false),
    ("Drangleic_Sword", Infusion::Raw, 312.0, false),
    ("Royal_Greatsword", Infusion::Raw, 305.0, false),
    ("Black_Knight_Greatsword", Infusion::Raw, 300.0, false),
    ("Old_Mirrah_Greatsword", Infusion::Raw, 296.0, false),
    ("Helix_Halberd", Infusion::Raw, 290.0, false),
];

/// The stub's ring counts over its pretend corpus of [`STUB_CORPUS`] builds.
const STUB_RINGS_WORN: [(&str, u32); 5] = [
    ("Chloranthy Ring", 240),
    ("Dexterity Ring", 180),
    ("Ring of Blades", 130),
    ("Third Dragon Ring", 60),
    ("Flynn's Ring", 20),
];

/// How many builds the stub pretends to have read.
const STUB_CORPUS: u32 = 1000;

impl StubBackend {
    fn row(key: &str, infusion: Infusion, damage: f32, two_hand_only: bool) -> Option<ResultRow> {
        let table = weapons::by_key(key)?;
        Some(ResultRow {
            weapon: table.name.to_owned(),
            infusion,
            damage,
            ar_by_type: [damage, 0.0, 0.0, 0.0, 0.0],
            grip: if two_hand_only { "2H only" } else { "1H" }.to_owned(),
            two_hand_only,
            hyperarmor: None,
            counter: None,
            class: table.class.to_owned(),
        })
    }

    fn ranking() -> Vec<ResultRow> {
        STUB_RANKING
            .iter()
            .filter_map(|&(key, infusion, damage, two)| Self::row(key, infusion, damage, two))
            .collect()
    }

    fn stub_build(sl: u16, two_handed: bool) -> OptimizedBuild {
        OptimizedBuild {
            class: "Deprived".to_owned(),
            sl,
            stats: STUB_STATS,
            two_handed,
            value: 410.0,
            gear: Vec::new(),
        }
    }
}

impl RecommenderBackend for StubBackend {
    fn is_stub(&self) -> bool {
        true
    }

    fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
        STUB_FLOORS
    }

    fn weapons_for(&self, _stats: &[u16; STAT_COUNT], _sl: u16, opts: &WeaponsForOpts) -> Outcome {
        let mut rows: Vec<ResultRow> = Self::ranking()
            .into_iter()
            .filter(|row| !opts.one_hand || !row.two_hand_only)
            .filter(|row| opts.class.as_deref().is_none_or(|class| row.class == class))
            .collect();
        if opts.per_class {
            let mut seen: Vec<String> = Vec::new();
            rows.retain(|row| {
                let fresh = !seen.contains(&row.class);
                seen.push(row.class.clone());
                fresh
            });
        }
        Outcome::Rows(rows)
    }

    fn optimize(
        &self,
        weapon: &str,
        _infusion: Infusion,
        sl: u16,
        _objective: Objective,
        grip: Grip,
    ) -> Option<OptimizedBuild> {
        weapons::by_key(weapon).map(|_| Self::stub_build(sl, grip.two_handed()))
    }

    fn minimum(&self, weapon: &str, _infusion: Infusion, two_hand: bool) -> Option<OptimizedBuild> {
        weapons::by_key(weapon)
            .map(|_| Self::stub_build(crate::model::soul_level(&STUB_STATS), two_hand))
    }

    fn similar(
        &self,
        _stats: &[u16; STAT_COUNT],
        _sl: u16,
        k: u16,
        status: StatusFilter,
    ) -> Outcome {
        let rows = Self::ranking()
            .into_iter()
            .filter(|row| {
                !status.any()
                    || (status.bleed && row.infusion == Infusion::Bleed)
                    || (status.poison && row.infusion == Infusion::Poison)
            })
            .enumerate()
            .map(|(rank, mut row)| {
                // A count of neighbours, not damage, in this mode.
                row.damage = f32::from(k.saturating_sub(u16::try_from(rank * 3).unwrap_or(k)));
                row
            })
            .filter(|row| row.damage > 0.0)
            .collect();
        Outcome::Rows(rows)
    }

    fn calibration(&self) -> Calibration {
        Calibration {
            n: 0,
            top1: 0.0,
            top2: 0.0,
        }
    }

    fn generate_build(
        &self,
        _weapon: &str,
        _infusion: Infusion,
        _sl: u16,
        _objective: Objective,
        allow_naked: bool,
        grip: Grip,
    ) -> Option<GeneratedBuild> {
        let primary = weapons::by_key("Moonlight_Greatsword")?;
        let (weapons_1h, weapons_2h_only) = split_weapons(&Self::ranking(), primary.name);
        let suggested_rings: Vec<String> = ["Dexterity Ring", "Ring of Knowledge", "Strength Ring"]
            .map(str::to_owned)
            .to_vec();
        let common_rings = common_rings(&STUB_RINGS_WORN, STUB_CORPUS, &suggested_rings);
        let built = Self::stub_build(crate::model::soul_level(&STUB_STATS), grip.two_handed());
        Some(GeneratedBuild {
            class: built.class,
            sl: built.sl,
            stats: built.stats,
            primary: (primary.key.to_owned(), Infusion::None),
            two_handed: built.two_handed,
            weapons_1h,
            weapons_2h_only,
            suggested_rings,
            common_rings,
            armor: if allow_naked {
                Vec::new()
            } else {
                vec!["Desert Sorceress Hood".to_owned()]
            },
            armor_note: None,
            stub: true,
        })
    }

    fn current_character_stats(&self) -> Option<[u16; STAT_COUNT]> {
        Some(STUB_STATS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generated() -> GeneratedBuild {
        let state = PanelState {
            weapon: Some("Moonlight_Greatsword"),
            ..PanelState::default()
        };
        generate(&StubBackend, &state).expect("the stub build is valid")
    }

    #[test]
    fn the_stub_build_is_the_one_asked_for() {
        let build = generated();
        assert_eq!(build.class, "Deprived");
        assert_eq!(build.sl, 33);
        assert_eq!(build.stats, [10, 16, 7, 10, 6, 13, 8, 10, 6]);
        assert!(build.two_handed);
        assert_eq!(
            build.primary,
            ("Moonlight_Greatsword".to_owned(), Infusion::None)
        );
        assert_eq!(
            build.suggested_rings,
            ["Dexterity Ring", "Ring of Knowledge", "Strength Ring"]
        );
        assert_eq!(build.armor, ["Desert Sorceress Hood"]);
        assert!(build.stub);
    }

    /// 15 one-handable and 5 two-hand-only, one row per weapon, never the primary.
    #[test]
    fn the_weapon_lists_are_fifteen_and_five() {
        let build = generated();
        assert_eq!(build.weapons_1h.len(), WEAPONS_1H_TOP);
        assert_eq!(build.weapons_2h_only.len(), WEAPONS_2H_ONLY_TOP);
        assert!(build.weapons_1h.iter().all(|row| !row.two_hand_only));
        assert!(build.weapons_2h_only.iter().all(|row| row.two_hand_only));
        let names: Vec<&str> = build
            .weapons_1h
            .iter()
            .chain(&build.weapons_2h_only)
            .map(|row| row.weapon.as_str())
            .collect();
        assert!(!names.contains(&"Moonlight Greatsword"));
        let falchions = names.iter().filter(|name| **name == "Falchion").count();
        assert_eq!(
            falchions, 1,
            "a weapon is listed once, at its best infusion"
        );
        let falchion = build.weapons_1h.iter().find(|row| row.weapon == "Falchion");
        assert_eq!(falchion.map(|row| row.infusion), Some(Infusion::Raw));
    }

    #[test]
    fn the_split_keeps_fewer_when_there_are_fewer() {
        let (one, two) = split_weapons(&[], "x");
        assert!(one.is_empty() && two.is_empty());
    }

    /// Common: at least a tenth of every build, most worn first, suggestions left out.
    #[test]
    fn common_rings_are_a_tenth_of_all_builds() {
        let suggested = vec!["Dexterity Ring".to_owned()];
        let common = common_rings(
            &[
                ("Chloranthy Ring", 240),
                ("Dexterity Ring", 180),
                ("Third Dragon Ring", 99),
                ("Ring of Blades", 100),
            ],
            1000,
            &suggested,
        );
        assert_eq!(common, ["Chloranthy Ring", "Ring of Blades"]);
    }

    /// Three of each suggestion, then one of each common ring that is not one.
    #[test]
    fn rings_are_three_copies_plus_one_common() {
        let suggested = ["A", "B"].map(str::to_owned).to_vec();
        let common = ["B", "C"].map(str::to_owned).to_vec();
        assert_eq!(
            ring_grants(&suggested, &common),
            ["A", "A", "A", "B", "B", "B", "C"]
        );
        let five = ["A", "B", "C", "D", "E"].map(str::to_owned).to_vec();
        assert_eq!(
            ring_grants(&five, &[]).len(),
            SUGGESTED_RINGS * SUGGESTED_RING_COPIES
        );
    }

    /// No rows are shown for stats under a floor -- and the backend is never asked.
    #[test]
    fn a_build_under_its_floors_gets_no_results() {
        let mut state = PanelState::default();
        for mode in [Mode::WeaponsForStats, Mode::SimilarBuilds] {
            state.mode = mode;
            match ask(&StubBackend, &state) {
                Answer::FloorViolations(lines) => {
                    assert_eq!(
                        lines,
                        ["VIG 6 < 10", "VIT 6 < 7", "ATT 6 < 10", "ADP 6 < 8"]
                    );
                }
                other => panic!("{mode:?} answered {other:?} for a fresh Deprived"),
            }
            state.stats = STUB_STATS;
            assert!(matches!(ask(&StubBackend, &state), Answer::Rows(rows) if !rows.is_empty()));
            state.stats = [6; STAT_COUNT];
        }
    }

    /// Weapons for stats ranks by the panel's objective: Bleed there reaches the backend as Bleed.
    #[test]
    fn weapons_for_stats_is_asked_for_the_panels_objective() {
        struct Echo;
        impl RecommenderBackend for Echo {
            fn is_stub(&self) -> bool {
                true
            }
            fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
                [0; STAT_COUNT]
            }
            fn weapons_for(&self, s: &[u16; STAT_COUNT], sl: u16, o: &WeaponsForOpts) -> Outcome {
                let mut rows = rows_of(StubBackend.weapons_for(s, sl, o));
                rows.truncate(1);
                rows[0].grip = o.objective.label().to_owned();
                Outcome::Rows(rows)
            }
            fn optimize(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                g: Grip,
            ) -> Option<OptimizedBuild> {
                StubBackend.optimize(w, i, sl, o, g)
            }
            fn minimum(&self, w: &str, i: Infusion, t: bool) -> Option<OptimizedBuild> {
                StubBackend.minimum(w, i, t)
            }
            fn similar(&self, s: &[u16; STAT_COUNT], sl: u16, k: u16, f: StatusFilter) -> Outcome {
                StubBackend.similar(s, sl, k, f)
            }
            fn calibration(&self) -> Calibration {
                StubBackend.calibration()
            }
            fn generate_build(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                n: bool,
                g: Grip,
            ) -> Option<GeneratedBuild> {
                StubBackend.generate_build(w, i, sl, o, n, g)
            }
            fn current_character_stats(&self) -> Option<[u16; STAT_COUNT]> {
                None
            }
        }
        fn rows_of(outcome: Outcome) -> Vec<ResultRow> {
            match outcome {
                Outcome::Rows(rows) => rows,
                Outcome::FloorViolations(lines) => panic!("under floors: {lines:?}"),
            }
        }
        let mut state = PanelState {
            mode: Mode::WeaponsForStats,
            ..PanelState::default()
        };
        for objective in Objective::ALL {
            state.objective = objective;
            match ask(&Echo, &state) {
                Answer::Rows(rows) => assert_eq!(rows[0].grip, objective.label()),
                other => panic!("{objective:?} answered {other:?}"),
            }
        }
    }

    /// A generated build under its own floors is refused, whatever the backend says.
    #[test]
    fn a_generated_build_under_its_floors_is_refused() {
        struct Strict;
        impl RecommenderBackend for Strict {
            fn is_stub(&self) -> bool {
                true
            }
            fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
                [20; STAT_COUNT]
            }
            fn weapons_for(&self, s: &[u16; STAT_COUNT], sl: u16, o: &WeaponsForOpts) -> Outcome {
                StubBackend.weapons_for(s, sl, o)
            }
            fn optimize(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                g: Grip,
            ) -> Option<OptimizedBuild> {
                StubBackend.optimize(w, i, sl, o, g)
            }
            fn minimum(&self, w: &str, i: Infusion, t: bool) -> Option<OptimizedBuild> {
                StubBackend.minimum(w, i, t)
            }
            fn similar(&self, s: &[u16; STAT_COUNT], sl: u16, k: u16, f: StatusFilter) -> Outcome {
                StubBackend.similar(s, sl, k, f)
            }
            fn calibration(&self) -> Calibration {
                StubBackend.calibration()
            }
            fn generate_build(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                naked: bool,
                g: Grip,
            ) -> Option<GeneratedBuild> {
                StubBackend.generate_build(w, i, sl, o, naked, g)
            }
            fn current_character_stats(&self) -> Option<[u16; STAT_COUNT]> {
                None
            }
        }
        let state = PanelState {
            weapon: Some("Moonlight_Greatsword"),
            ..PanelState::default()
        };
        let refused = generate(&Strict, &state).expect_err("under every floor");
        assert_eq!(refused.len(), FLOOR_STATS.len());
        assert!(
            generate(&StubBackend, &PanelState::default()).is_err(),
            "no weapon"
        );
    }

    #[test]
    fn the_weapon_modes_need_a_weapon() {
        let mut state = PanelState {
            mode: Mode::OptimizeForWeapon,
            ..PanelState::default()
        };
        assert_eq!(
            ask(&StubBackend, &state),
            Answer::Nothing("choose a weapon first")
        );
        state.choose_weapon("Moonlight_Greatsword");
        assert!(matches!(ask(&StubBackend, &state), Answer::Build(_)));
    }

    /// The import: stats in the game's order, the primary in the right hand, the rings worn, and
    /// the extras bringing each suggested ring to three.
    #[test]
    fn to_import_wears_the_build_and_grants_the_rest() {
        let build = generated();
        let (import, extras) = to_import(&build);
        assert_eq!(import.class, "deprived");
        // The planner's order, which is the panel's: ADP seventh, not last as the game keeps it.
        assert_eq!(import.stats.each().map(|(_, value)| value), build.stats);
        assert_eq!(import.stats.adaptability, 8);
        assert_eq!(import.stats.in_game_order()[8], 8);
        assert_eq!(
            ds2_build_import_core::level::soul_level(&import.stats.in_game_order()),
            33
        );
        assert_eq!(import.weapons.len(), 12);
        assert_eq!(import.weapons[2], "Moonlight_Greatsword");
        assert_eq!(
            Infusion::from_name(&import.weapons[3]),
            Some(Infusion::None)
        );
        assert!(ds2_build_import_core::is_empty_slot(&import.weapons[0]));
        assert_eq!(import.rings[..3], build.suggested_rings[..]);
        assert!(ds2_build_import_core::is_empty_slot(&import.rings[3]));
        assert_eq!(import.armor[0], "Desert Sorceress Hood");
        assert!(
            import.armor[1..]
                .iter()
                .all(|piece| ds2_build_import_core::is_empty_slot(piece))
        );

        for ring in &build.suggested_rings {
            let worn = import.rings.iter().filter(|name| *name == ring).count();
            let extra = extras.iter().filter(|(name, _)| name == ring).count();
            assert_eq!(worn + extra, SUGGESTED_RING_COPIES, "{ring}");
        }
        for ring in &build.common_rings {
            assert_eq!(
                extras.iter().filter(|(name, _)| name == ring).count(),
                1,
                "{ring}"
            );
        }
        assert_eq!(build.common_rings, ["Chloranthy Ring", "Ring of Blades"]);
        let weapons = extras
            .iter()
            .filter(|(name, _)| !name.contains("Ring"))
            .count();
        assert_eq!(weapons, WEAPONS_1H_TOP + WEAPONS_2H_ONLY_TOP);
    }

    /// Everything Apply would grant resolves in the item catalogue, so the stub tests the real path.
    #[test]
    fn every_stub_grant_names_a_real_item() {
        use ds2_build_import_core::{ItemError, id_for, is_empty_slot};
        let (import, extras) = to_import(&generated());
        let names = import
            .weapons
            .chunks(2)
            .map(|pair| pair[0].clone())
            .chain(import.armor.iter().cloned())
            .chain(import.rings.iter().cloned())
            .chain(extras.into_iter().map(|(name, _)| name));
        for name in names {
            if is_empty_slot(&name) {
                continue;
            }
            match id_for(&name) {
                Ok(_) | Err(ItemError::Ambiguous { .. }) => {}
                Err(error) => panic!("{name:?}: {error}"),
            }
        }
    }
}
