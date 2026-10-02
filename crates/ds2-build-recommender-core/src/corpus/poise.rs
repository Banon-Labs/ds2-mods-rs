//! Hyperarmor and poise: the script's `poise_metrics`, `stagger_hits` and `bracket_poise`, over
//! the `PO` records (each weapon's R1 hits with their poise damage) and the poise the `B`, `P` and
//! `X` records carry.
//!
//! A hit staggers when the defender's poise is at or below 0 after it, or its armorBreak is set;
//! poise regenerates [`POISE_REGEN`] a second between hits. Inside the R1's hyperarmor window the
//! attacker takes poise damage times the weapon's `uninterruptibleRate`.

use super::{CorpusBackend, Worn, bad, float, int, py_min, py_round, py_sum, sl_bracket};
use crate::backend::WeaponMetrics;

/// `ChrParam` row 100's `poiseRecoveryValue`, per second: the script's `POISE_REGEN`.
const POISE_REGEN: f64 = 0.56;

/// What the script's `poise_metrics` reads for one weapon and grip that no build changes.
#[derive(Clone, Debug)]
pub(super) struct Poise {
    /// `WeaponParam.uninterruptibleRate`: the factor on poise damage the attacker takes inside the
    /// hyperarmor window.
    rate: f64,
    /// The share of the R1's windup and active frames inside that window.
    cover: f64,
    /// The R1 chain's hits out to 10 s with poise data: (seconds, poise damage, armorBreak).
    hits: Vec<(f64, f64, i32)>,
}

/// The script's `stagger_hits`: how many of `hits` it takes to stagger a defender with `poise`
/// who is not in hyperarmor; `None` when they never do.
fn stagger_hits(hits: &[(f64, f64, i32)], poise: f64) -> Option<u32> {
    let (mut current, mut last) = (poise, 0.0);
    for (n, &(at, damage, armor_break)) in (1..).zip(hits) {
        current = py_min(poise, current + POISE_REGEN * (at - last)) - damage;
        last = at;
        if armor_break != 0 || current <= 0.0 {
            return Some(n);
        }
    }
    None
}

impl CorpusBackend {
    /// Read a `PO` record onto the last weapon.
    pub(super) fn parse_poise<'a>(
        &mut self,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        let grip = match fields.next() {
            Some("1") => 0,
            Some("2") => 1,
            _ => return Err(bad(line, "grip is 1 or 2")),
        };
        let rate = float(fields.next(), line)?;
        let cover = float(fields.next(), line)?;
        let hits = fields
            .next()
            .unwrap_or("")
            .split(' ')
            .filter(|hit| !hit.is_empty())
            .map(|hit| {
                let mut parts = hit.split(':');
                Ok((
                    float(parts.next(), line)?,
                    float(parts.next(), line)?,
                    int(parts.next(), line)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.weapons
            .last_mut()
            .ok_or_else(|| bad(line, "poise before any weapon"))?
            .poise[grip] = Some(Poise { rate, cover, hits });
        Ok(())
    }

    /// The script's `bracket_poise` poise: the bracket's mean max poise, or the chosen pieces'
    /// summed poise (no rings).
    fn defender_poise(&self, sl: u32, worn: Worn) -> f64 {
        match worn {
            None => self.bracket(sl).poise,
            Some(pieces) => py_sum(
                pieces
                    .iter()
                    .enumerate()
                    .map(|(slot, &piece)| self.wearable[slot][piece].poise),
            ),
        }
    }

    /// The script's `poise_metrics` for `weapon` (an index into the file's weapons) held `two`
    /// handed, against the defender at `sl`: its R1's hyperarmor and how much of the bracket's
    /// counter-hits it holds through, its poise damage and armorBreak, and the hits it takes to
    /// stagger the defender. Only the defender's poise for a launcher or a weapon with no poise
    /// data.
    pub(super) fn poise_metrics(
        &self,
        weapon: usize,
        two: bool,
        sl: u32,
        worn: Worn,
        out: &mut WeaponMetrics,
    ) {
        let poise = self.defender_poise(sl, worn);
        out.defender_poise = py_round(poise, 1);
        let weapon = &self.weapons[weapon];
        if weapon.ranged.is_some() {
            return;
        }
        let Some(data) = &weapon.poise[usize::from(two)] else {
            return;
        };
        let Some(&(_, damage, armor_break)) = data.hits.first() else {
            return;
        };
        out.hyperarmor = Some(py_round(data.cover, 3));
        out.hyperarmor_rate = Some(data.rate);
        out.poise_damage_per_hit = Some(py_round(damage, 2));
        out.armor_break = Some(armor_break);
        out.hits_to_stagger = stagger_hits(&data.hits, poise);
        let counters: Vec<(f64, i32)> = self
            .bracket_builds(sl_bracket(sl))
            .into_iter()
            .filter_map(|build| build.counter)
            .collect();
        if data.cover != 0.0 && !counters.is_empty() {
            let holds = counters
                .iter()
                .filter(|&&(damage, armor_break)| armor_break != 2 && damage * data.rate < poise)
                .count();
            let share = holds as f64 / counters.len() as f64;
            out.hyperarmor_holds = Some(py_round(share, 3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stagger_hits;

    /// The script's selftest cases for `stagger_hits`.
    #[test]
    fn stagger_is_the_scripts() {
        assert_eq!(stagger_hits(&[(0.0, 35.0, 0)], 0.0), Some(1), "no poise");
        let two = [(0.0, 35.0, 0), (0.5, 35.0, 0)];
        assert_eq!(stagger_hits(&two, 30.0), Some(1));
        assert_eq!(stagger_hits(&two, 50.0), Some(2));
        assert_eq!(stagger_hits(&[(0.0, 1.0, 1)], 132.0), Some(1), "armorBreak");
        let six: Vec<(f64, f64, i32)> = (0..6).map(|i| (f64::from(i) * 0.5, 35.0, 0)).collect();
        assert_eq!(stagger_hits(&six, 132.0), Some(4), "regen between hits");
        assert_eq!(
            stagger_hits(&[(0.0, 0.5, 0), (10.0, 0.5, 0)], 1.0),
            None,
            "regen can outlast a weak chain"
        );
    }
}
