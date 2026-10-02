//! Poison and bleed procs: the script's `status_cut`, `defender_status_cuts`, `status_metrics` and
//! `row_status`, over the `SP` records (what a proc deals and how long the status is locked out
//! after), the stat resistance tables, each armour piece's resistance and each corpus build's.
//!
//! A hit's build-up, less the defender's resistance, fills a gauge; at 100 it procs, the gauge
//! goes back to its floor (the procing hit's surplus is lost) and the status takes no build-up for
//! its lockout.

use ds2_build_import_core::Infusion;

use super::{
    ADP, ATK_BLEED, ATK_POISON, CorpusBackend, DEX, FTH, SCALE_BLEED, SCALE_POISON, Stats, VIT,
    Weapon, Worn, bad, float, py_max, py_min, py_sum, sl_bracket,
};
use crate::backend::StatusMetrics;

/// The statuses in the script's `STATUS_PROC` order.
pub(super) const STATUSES: [&str; 2] = ["poison", "bleed"];

/// What a proc does: the script's `data.status_procs` row.
#[derive(Clone, Copy, Debug)]
pub(super) struct Proc {
    /// Damage one proc deals.
    damage: f64,
    /// Seconds the status takes no build-up after it.
    lockout: f64,
}

/// The script's `status_metrics` for one status: (hits to proc for the median defender, the proc
/// damage per window of a chain repeated without pause, the proc damage those hits deal a
/// defender whose gauge starts empty), averaged over the defenders who resist `cuts` of it.
fn status_metrics(
    proc: Option<Proc>,
    per_hit: f64,
    cuts: &[f64],
    hits: u32,
    window: f64,
) -> (u32, f64, f64) {
    let Some(proc) = proc.filter(|_| per_hit > 0.0 && hits > 0 && window > 0.0) else {
        return (0, 0.0, 0.0);
    };
    let (mut ks, mut rate, mut first) = (Vec::new(), 0.0, 0.0);
    for &cut in cuts {
        let buildup = per_hit * (1.0 - cut);
        let k = if buildup > 0.0 {
            Some((100.0 / buildup - 1e-9).ceil())
        } else {
            None
        };
        ks.push(k);
        if let Some(k) = k {
            rate += proc.damage * window / (proc.lockout + k * window / f64::from(hits));
            first += if f64::from(hits) >= k {
                proc.damage
            } else {
                0.0
            };
        }
    }
    // `None` (never procs) sorts after every count, as the script's infinity does.
    ks.sort_by(|a, b| match (a, b) {
        (Some(a), Some(b)) => a.total_cmp(b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    let n = cuts.len();
    let mid = n
        .checked_sub(1)
        .and_then(|last| ks.get(last / 2).copied().flatten());
    let to_proc = mid.map_or(0, |k| k as u32);
    let count = n as f64;
    (to_proc, rate / count, first / count)
}

impl CorpusBackend {
    /// Read an `SP` record.
    pub(super) fn parse_proc<'a>(
        &mut self,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        let status = fields.next().ok_or_else(|| bad(line, "status"))?;
        let at = STATUSES
            .iter()
            .position(|&name| name == status)
            .ok_or_else(|| bad(line, "status is poison or bleed"))?;
        let damage = float(fields.next(), line)?;
        let lockout = float(fields.next(), line)?;
        self.procs[at] = Some(Proc { damage, lockout });
        Ok(())
    }

    /// The script's `status_cut` of a chosen defender: the share of status `at` (an index into
    /// [`STATUSES`]) the pieces `worn` resist at the bracket's median stats, no rings -- the stat
    /// column at row trunc((3 x ADP + VIT or FTH) / 4) plus each piece's own, each x 0.01, clamped
    /// to 0 to 1.
    fn chosen_cut(&self, sl: u32, pieces: [usize; 4], at: usize) -> f64 {
        let mut eff = self.bracket(sl).stats;
        for (slot, &piece) in pieces.iter().enumerate() {
            for (stat, add) in eff.iter_mut().zip(self.wearable[slot][piece].alter) {
                *stat += add;
            }
        }
        let column = &self.tables.status_resist[at].0;
        if column.is_empty() {
            return 0.0;
        }
        let second = if at == 0 { eff[VIT] } else { eff[FTH] };
        let row = (3 * eff[ADP] + second).div_euclid(4);
        let row = usize::try_from(row)
            .ok()
            .filter(|&row| row >= 1 && row < column.len())
            .unwrap_or(1);
        let mut cut = column.get(row).copied().unwrap_or(0.0) * 0.01;
        cut += py_sum(
            pieces
                .iter()
                .enumerate()
                .map(|(slot, &piece)| self.wearable[slot][piece].status[at]),
        ) * 0.01;
        py_min(1.0, py_max(0.0, cut))
    }

    /// The script's `defender_status_cuts`: the share of status `at` every defender resists, the
    /// bracket's builds or the chosen set; `[0]` when there is none.
    fn defender_cuts(&self, sl: u32, worn: Worn, at: usize) -> Vec<f64> {
        let cuts: Vec<f64> = match worn {
            Some(pieces) => vec![self.chosen_cut(sl, pieces, at)],
            None => self
                .bracket_builds(sl_bracket(sl))
                .into_iter()
                .map(|build| build.cuts.map_or(0.0, |cuts| cuts[at]))
                .collect(),
        };
        if cuts.is_empty() { vec![0.0] } else { cuts }
    }

    /// The script's `row_status`: per status the weapon builds up at stats `eff`, its build-up
    /// per hit before resistance and the hits the median defender takes to proc, and summed over
    /// the statuses the proc damage per window and in the first window, for `hits` landed in
    /// `window` seconds.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the row's question.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row_status(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        eff: &Stats,
        hits: u32,
        window: f64,
        sl: u32,
        worn: Worn,
    ) -> StatusMetrics {
        let mut out = StatusMetrics {
            hits,
            ..StatusMetrics::default()
        };
        for at in 0..STATUSES.len() {
            let per = weapon.infusion(infusion).map_or(0.0, |row| {
                let (atk, scale, second) = if at == 0 {
                    (ATK_POISON, SCALE_POISON, eff[ADP])
                } else {
                    (ATK_BLEED, SCALE_BLEED, eff[FTH])
                };
                row.atk[atk] + row.scale[scale] * self.tables.aux.at(3 * eff[DEX] + second)
            });
            if per <= 0.0 {
                continue;
            }
            let (to_proc, per_window, first) = status_metrics(
                self.procs[at],
                per,
                &self.defender_cuts(sl, worn, at),
                hits,
                window,
            );
            out.buildup_per_hit[at] = Some(per);
            out.hits_to_proc[at] = Some(to_proc);
            out.damage_per_window += per_window;
            out.damage_first_window += first;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Proc, status_metrics};

    /// The script's selftest cases for `status_metrics`.
    #[test]
    fn procs_are_the_scripts() {
        let bleed = Some(Proc {
            damage: 200.0,
            lockout: 10.0,
        });
        // 32 a hit: 4 to proc, all 4 land in the window.
        let (to_proc, per_window, first) = status_metrics(bleed, 40.0, &[0.2], 4, 1.5);
        assert_eq!(to_proc, 4);
        assert_eq!(first, 200.0);
        assert_eq!(
            (per_window * 1000.0).round(),
            (200.0 * 1.5 / 11.5 * 1000.0_f64).round()
        );
        assert_eq!(
            status_metrics(bleed, 50.0, &[0.0], 2, 1.0).0,
            2,
            "the surplus is lost"
        );
        assert_eq!(status_metrics(bleed, 40.0, &[0.2], 3, 1.5).2, 0.0);
        assert_eq!(
            status_metrics(bleed, 50.0, &[0.0, 0.5, 1.0], 2, 1.0).0,
            4,
            "the median defender's"
        );
        assert_eq!(
            status_metrics(bleed, 50.0, &[1.0], 2, 1.0),
            (0, 0.0, 0.0),
            "never procs"
        );
        assert_eq!(status_metrics(bleed, 0.0, &[0.0], 4, 1.5).1, 0.0);
    }
}
