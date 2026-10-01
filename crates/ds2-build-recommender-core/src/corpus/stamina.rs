//! Stamina: the script's `attack_stamina`, `shot_stamina`, `max_stamina`, `bar_attacks`,
//! `stamina_metrics` and `shot_metrics`, over the `SC` records (each weapon's base costs and its
//! R1 attacks' cost multipliers).
//!
//! The regulation's stamina is ten times the menu's; every number here is in menu points. An
//! attack begins while stamina is above 0 and may take it below, and stamina does not regenerate
//! between attacks of a chain.

use super::{Ar, CorpusBackend, Defense, Factor, Stats, Weapon, bad, float, py_round, py_sum};
use crate::backend::StaminaMetrics;

/// The script's `STAMINA_UNIT`: the regulation's stamina per menu point.
const STAMINA_UNIT: f64 = 10.0;

/// Seconds of R1 chain read for a full bar: the script's `BAR_HORIZON`.
const BAR_HORIZON: f64 = 30.0;

/// A weapon's stamina costs: the script's `data.stamina_cost` row.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cost {
    /// `WeaponParam.meleeAttackBaseCost`.
    melee: f64,
    /// `WeaponParam.rangedAttackBaseCost`.
    ranged: f64,
    /// The R1 chain's cost multipliers, 1H 1st, 1H 2nd, 2H 1st, 2H 2nd: the script's
    /// `R1_COST_FIELD`.
    rates: [f64; 4],
}

/// The script's `bar_attacks`: how many attacks of `costs` begin from a full bar of `bar`.
fn bar_attacks(costs: impl IntoIterator<Item = f64>, bar: f64) -> u32 {
    let (mut n, mut left) = (0, bar);
    for cost in costs {
        if left <= 0.0 || cost <= 0.0 {
            break;
        }
        left -= cost;
        n += 1;
    }
    n
}

impl Weapon {
    /// The script's `attack_stamina`: menu stamina one R1 chain attack costs, its 1st or (`second`)
    /// 2nd; `None` without the regulation's row.
    fn attack_stamina(&self, two: bool, second: bool) -> Option<f64> {
        let cost = self.stamina?;
        let field = usize::from(two) * 2 + usize::from(second);
        Some(cost.melee * cost.rates[field] / STAMINA_UNIT)
    }

    /// The script's `shot_stamina`: menu stamina one shot costs.
    fn shot_stamina(&self) -> Option<f64> {
        self.stamina.map(|cost| cost.ranged / STAMINA_UNIT)
    }
}

impl CorpusBackend {
    /// Read an `SC` record onto the last weapon.
    pub(super) fn parse_stamina<'a>(
        &mut self,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        let melee = float(fields.next(), line)?;
        let ranged = float(fields.next(), line)?;
        let mut rates = [0.0; 4];
        for rate in &mut rates {
            *rate = float(fields.next(), line)?;
        }
        self.weapons
            .last_mut()
            .ok_or_else(|| bad(line, "stamina costs before any weapon"))?
            .stamina = Some(Cost {
            melee,
            ranged,
            rates,
        });
        Ok(())
    }

    /// The script's `max_stamina`: menu max stamina of levelled stats `stats` wearing `rings`,
    /// `None` without the table.
    fn max_stamina(&self, stats: &Stats, rings: &[usize]) -> Option<f64> {
        let table = &self.tables.stamina_max;
        if table.0.is_empty() {
            return None;
        }
        let end = self.gear_stats(stats, rings)[super::END];
        Some(table.at(end) * self.ring_factor(rings, Factor::Stamina) / STAMINA_UNIT)
    }

    /// The script's `stamina_metrics`: the stamina side of a melee weapon's R1 chain at attack
    /// `ar` against `defense` (each hit times the damage scale) -- the 1st and 2nd attack's cost,
    /// what the attacks whose hits land within `window` cost and the damage per stamina of those
    /// hits, and from a full bar of the build's max stamina the attacks it begins, their damage
    /// and when the last begins. `None` without attack timing or stamina costs.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the row's question.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn stamina_metrics(
        &self,
        weapon: &Weapon,
        two: bool,
        ar: &Ar,
        defense: &Defense,
        window: f64,
        stats: &Stats,
        rings: &[usize],
    ) -> Option<StaminaMetrics> {
        weapon.stamina?;
        let timeline = weapon.chains[usize::from(two)]
            .iter()
            .map(|chain| chain.timeline(BAR_HORIZON, true))
            .find(|timeline| !timeline.is_empty())?;
        // Each attack's hits, by the second it began, in the timeline's order.
        let mut by: Vec<(f64, Vec<(f64, f64)>)> = Vec::new();
        for landed in &timeline {
            let dealt = Self::hit_damage(ar, defense, &landed.hit()) * weapon.damage_scale;
            match by.iter_mut().find(|(start, _)| *start == landed.start) {
                Some((_, hits)) => hits.push((landed.at, dealt)),
                None => by.push((landed.start, vec![(landed.at, dealt)])),
            }
        }
        by.sort_by(|a, b| a.0.total_cmp(&b.0));
        let costs: Vec<f64> = (0..by.len())
            .map(|at| weapon.attack_stamina(two, at % 2 == 1).unwrap_or(0.0))
            .collect();
        let landed = |hits: &[(f64, f64)]| hits.iter().any(|&(at, _)| at <= window);
        let spent = py_sum(
            by.iter()
                .zip(&costs)
                .filter(|((_, hits), _)| landed(hits))
                .map(|(_, &cost)| cost),
        );
        let dealt = py_sum(
            by.iter()
                .flat_map(|(_, hits)| hits.iter())
                .filter(|&&(at, _)| at <= window)
                .map(|&(_, damage)| damage),
        );
        let mut out = StaminaMetrics {
            per_attack: costs
                .iter()
                .take(2)
                .map(|&cost| py_round(cost, 2))
                .collect(),
            per_window: py_round(spent, 2),
            damage_per_stamina: if spent != 0.0 {
                py_round(dealt / spent, 3)
            } else {
                0.0
            },
            ..StaminaMetrics::default()
        };
        if let Some(bar) = self.max_stamina(stats, rings).filter(|&bar| bar != 0.0) {
            let n = bar_attacks(costs.iter().copied(), bar);
            let begun = usize::try_from(n).unwrap_or(usize::MAX);
            out.max_stamina = Some(py_round(bar, 1));
            out.bar_attacks = Some(n);
            out.bar_damage = Some(py_round(
                py_sum(
                    by.iter()
                        .take(begun)
                        .flat_map(|(_, hits)| hits.iter().map(|&(_, damage)| damage)),
                ),
                1,
            ));
            out.bar_seconds = Some(
                begun
                    .checked_sub(1)
                    .and_then(|last| by.get(last))
                    .map_or(0.0, |(start, _)| py_round(*start, 2)),
            );
        }
        Some(out)
    }

    /// The script's `shot_metrics`: `stamina_metrics` for one launcher shot of damage `shot`, a
    /// window being one shot. `None` without a shot cost.
    pub(super) fn shot_metrics(
        &self,
        weapon: &Weapon,
        shot: f64,
        stats: &Stats,
        rings: &[usize],
    ) -> Option<StaminaMetrics> {
        let cost = weapon.shot_stamina().filter(|&cost| cost != 0.0)?;
        let mut out = StaminaMetrics {
            per_attack: vec![py_round(cost, 2)],
            per_window: py_round(cost, 2),
            damage_per_stamina: py_round(shot / cost, 3),
            ..StaminaMetrics::default()
        };
        if let Some(bar) = self.max_stamina(stats, rings).filter(|&bar| bar != 0.0) {
            let n = bar_attacks(std::iter::repeat(cost), bar);
            out.max_stamina = Some(py_round(bar, 1));
            out.bar_attacks = Some(n);
            out.bar_damage = Some(py_round(f64::from(n) * shot, 1));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::bar_attacks;

    /// The script's selftest cases for `bar_attacks`.
    #[test]
    fn a_bar_is_the_scripts() {
        assert_eq!(
            bar_attacks([50.0, 50.0, 50.0], 120.0),
            3,
            "begins above 0, may end below"
        );
        assert_eq!(bar_attacks([50.0; 5], 100.0), 2, "at exactly 0 none begins");
        assert_eq!(
            bar_attacks([50.0, 0.0, 50.0], 120.0),
            1,
            "a zero cost ends the count"
        );
    }
}
