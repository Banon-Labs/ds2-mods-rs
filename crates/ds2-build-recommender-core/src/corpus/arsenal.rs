//! The arsenal: what else a build's stats swing well, the script's `scaling_axis`, `arsenal` and
//! `with_arsenal`.
//!
//! A build's stats are optimized for one weapon, but a build that also swings other weapon
//! classes well can swap movesets to what the fight needs. At a row's own stats and rings, each
//! other melee class's best moveset rate is read, and Best weapons ranks its moveset and stance
//! rows by a blend of the row's own rate and the mean of the build's best [`ARSENAL_K`] slots.
//! Only weapons whose uninfused scaling stays on the primary's own stats count: no STR weapon on
//! a DEX build unless the primary is innately both.

use std::cmp::Ordering;

use ds2_build_import_core::Infusion;

use super::{CorpusBackend, Defense, EMPTY, REQ_STATS, STAT_COUNT, Stats, Worn, py_min, py_sum};
use crate::backend::{Arsenal, BestWeaponRow};

/// The stat line an uninfused weapon's scaling is read at, and how far each stat is raised: the
/// script's `AXIS_BASE` and `AXIS_STEP`.
const AXIS_BASE: i32 = 20;
const AXIS_STEP: i32 = 20;
/// A weapon scales on every requirement stat whose gain is at least this share of its largest:
/// the script's `AXIS_BOTH`, whose comment carries the distribution it was chosen from.
const AXIS_BOTH: f64 = 0.75;
/// Weapon classes that are not a melee moveset to swap to: the script's `NOT_ARSENAL`.
const NOT_ARSENAL: [&str; 9] = [
    "Normal Shield",
    "Small Shield",
    "Greatshield",
    "Staff",
    "Chime",
    "Flame",
    "Bow",
    "Greatbow",
    "Crossbow",
];
/// Per class, how many of its weapons -- best one-hit damage first -- have their moveset scored:
/// the script's `ARSENAL_SHORTLIST`.
const ARSENAL_SHORTLIST: usize = 2;
/// The arsenal's slots, the primary and its best `ARSENAL_K - 1` other classes: the script's
/// `ARSENAL_K`, whose comment carries the run it was read from.
const ARSENAL_K: u8 = 9;
/// The arsenal mean's share of a row's ranked value: the script's `ARSENAL_WEIGHT`.
const ARSENAL_WEIGHT: f64 = 0.25;
/// Rows read best first until this many are in their final order: the script's
/// `ARSENAL_EXACT`.
const ARSENAL_EXACT: usize = 15;

/// A class's candidate, kept per class in first-seen order: its one-hit damage at its best
/// infusion, the weapon and that infusion.
type Candidate = (f64, usize, Infusion);

/// One Best weapons row with what its arsenal is read at.
pub(super) struct Scored {
    pub(super) row: BestWeaponRow,
    pub(super) weapon: usize,
    pub(super) eff: Stats,
    pub(super) worn: Vec<usize>,
}

impl CorpusBackend {
    /// The script's `scaling_axis`: per requirement stat ([`REQ_STATS`] order) whether the
    /// uninfused weapon scales on it -- its raise from [`AXIS_BASE`] by [`AXIS_STEP`] (the others
    /// at [`AXIS_BASE`] or the weapon's requirement) adds at least [`AXIS_BOTH`] of the largest
    /// such gain to its attack rating. None when no stat adds any.
    fn scaling_axis(&self, weapon: usize) -> [bool; 4] {
        let weapon = &self.weapons[weapon];
        let row = weapon.infusion(Infusion::None);
        let mut base: Stats = [AXIS_BASE; STAT_COUNT];
        for &(stat, value) in &weapon.require {
            base[stat] = base[stat].max(value);
        }
        let total =
            |stats: &Stats| py_sum(self.attack_rating(row, stats, &[]).into_iter().flatten());
        let at_base = total(&base);
        let gains = REQ_STATS.map(|stat| {
            let mut up = base;
            up[stat] += AXIS_STEP;
            total(&up) - at_base
        });
        // Python's max: the first of the largest.
        let top = gains
            .into_iter()
            .reduce(|best, gain| if gain > best { gain } else { best })
            .unwrap_or(0.0);
        gains.map(|gain| top > 0.0 && gain >= AXIS_BOTH * top)
    }

    /// The script's `arsenal`: per melee class other than `weapon`'s, its best moveset rate (in
    /// the better grip `eff` wields) among the weapons whose scaling axis is within `weapon`'s,
    /// each at the infusion with the most one-hit damage against `defense` -- the class's
    /// [`ARSENAL_SHORTLIST`] best by that damage scored. Unclassed weapons are left out. `own` is
    /// the row's own rate.
    // DEBT: ds2-mods-rs-59p7 -- the row's build, which wants bundling as optimize_build's does.
    #[allow(clippy::too_many_arguments)]
    fn arsenal(
        &self,
        weapon: usize,
        eff: &Stats,
        rings: &[usize],
        sl: u32,
        defense: &Defense,
        worn: Worn,
        own: f64,
    ) -> Arsenal {
        let axis = self.scaling_axis(weapon);
        let mine = self.weapons[weapon].class.as_str();
        let mut shortlist: Vec<(&str, Vec<Candidate>)> = Vec::new();
        for (index, candidate) in self.weapons.iter().enumerate() {
            let class = candidate.class.as_str();
            if class.is_empty()
                || class == mine
                || NOT_ARSENAL.contains(&class)
                || EMPTY.contains(&candidate.key.as_str())
                || candidate.catalyst
                || candidate.shield
                || candidate.ranged.is_some()
                || candidate.infusion(Infusion::None).is_none()
                || !candidate.wieldable(eff, true)
                || self
                    .scaling_axis(index)
                    .iter()
                    .zip(axis)
                    .any(|(&theirs, ours)| theirs && !ours)
            {
                continue;
            }
            let mut best: Option<(f64, Infusion)> = None;
            for row in &candidate.infusions {
                let ar = self.attack_rating(Some(row), eff, rings);
                let hit = Self::damage(&ar, defense);
                if ar.iter().any(Option::is_some) && best.is_none_or(|(top, _)| hit > top) {
                    best = Some((hit, row.infusion));
                }
            }
            if let Some((hit, infusion)) = best {
                match shortlist.iter_mut().find(|(seen, _)| *seen == class) {
                    Some((_, list)) => list.push((hit, index, infusion)),
                    None => shortlist.push((class, vec![(hit, index, infusion)])),
                }
            }
        }
        let mut classes: Vec<(String, String, Infusion, bool, f64)> = Vec::new();
        for (class, mut candidates) in shortlist {
            candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
            let mut top: Option<(usize, Infusion, bool, f64)> = None;
            for &(_, index, infusion) in candidates.iter().take(ARSENAL_SHORTLIST) {
                let candidate = &self.weapons[index];
                for two in [false, true] {
                    if !candidate.wieldable(eff, two) {
                        continue;
                    }
                    let Some(rate) = self
                        .moveset_metrics(candidate, infusion, two, eff, rings, sl, defense, worn)
                        .map(|m| m.moveset)
                    else {
                        continue;
                    };
                    if top.is_none_or(|(.., best)| rate > best) {
                        top = Some((index, infusion, two, rate));
                    }
                }
            }
            if let Some((index, infusion, two, rate)) = top {
                classes.push((
                    class.to_owned(),
                    self.weapons[index].key.clone(),
                    infusion,
                    two,
                    rate,
                ));
            }
        }
        classes.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(Ordering::Equal));
        classes.truncate(usize::from(ARSENAL_K - 1));
        let mean =
            (own + py_sum(classes.iter().map(|class| py_min(own, class.4)))) / f64::from(ARSENAL_K);
        Arsenal {
            mean,
            value: (1.0 - ARSENAL_WEIGHT) * own + ARSENAL_WEIGHT * mean,
            classes,
        }
    }

    /// The script's `with_arsenal`: Best weapons' rows (best first) ranked by their arsenal's
    /// value, read best first until [`ARSENAL_EXACT`] rows have one and the next row's own score
    /// is no higher than the [`ARSENAL_EXACT`]-th value. The rows after keep their own score and
    /// no arsenal; as an arsenal never raises a score, none of them belongs above those.
    pub(super) fn with_arsenal(
        &self,
        rows: Vec<Scored>,
        sl: u32,
        defense: &Defense,
        worn: Worn,
    ) -> Vec<BestWeaponRow> {
        let mut values: Vec<f64> = Vec::new();
        let mut out = Vec::with_capacity(rows.len());
        let mut rows = rows.into_iter();
        for scored in rows.by_ref() {
            if values.len() >= ARSENAL_EXACT {
                let mut sorted = values.clone();
                sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
                if scored.row.score <= sorted[ARSENAL_EXACT - 1] {
                    out.push(scored.row);
                    break;
                }
            }
            let Scored {
                mut row,
                weapon,
                eff,
                worn: rings,
            } = scored;
            let arsenal = self.arsenal(weapon, &eff, &rings, sl, defense, worn, row.score);
            values.push(arsenal.value);
            row.score = arsenal.value;
            row.metrics.arsenal = Some(arsenal);
            out.push(row);
        }
        out.extend(rows.map(|scored| scored.row));
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out
    }
}
