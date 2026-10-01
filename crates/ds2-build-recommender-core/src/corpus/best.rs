//! Best weapons: the script's `best_weapons`, `best_weapon_row` and the metrics its `--json`
//! prints for a row (`row_metrics`), ported.
//!
//! The reverse of Best infusion: every weapon an infusion goes on, each at the build the optimizer
//! makes for it at the soul level (its own class, stats and rings), scored so weapons compare --
//! with a window, the R1 hits that build lands in it rather than one swing, which ranked a slow
//! weapon's single hit against a fast one's.

use ds2_build_import_core::Infusion;

use super::{Against, CorpusBackend, Defense, EMPTY, Hit, Stats, Weapon, norm, py_round, py_sum};
use crate::backend::{BestWeaponRow, WeaponMetrics};
use crate::model::{Grip, Objective, Rank};

/// How long the sustained-damage metric repeats the R1 chain: the script's `SUSTAIN_HORIZON`.
const SUSTAIN_HORIZON: f64 = 5.0;

/// One Best weapons question, besides the weapon.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ask<'a> {
    pub(super) infusion: Infusion,
    pub(super) sl: u32,
    pub(super) objective: Objective,
    pub(super) grip: Grip,
    /// Indices into the file's spells.
    pub(super) spells: &'a [usize],
    pub(super) class: Option<&'a str>,
    pub(super) floors: bool,
    pub(super) against: Against,
    /// Seconds; `0` scores the optimizer's own value.
    pub(super) window: f64,
    pub(super) rank: Rank,
}

impl CorpusBackend {
    /// The script's `best_weapons_jobs` and `best_weapons`: every weapon that takes `ask`'s
    /// infusion -- shields, catalysts and the empty keys aside, of `weapon_class` alone when given
    /// -- in file order, each `best_weapon_row`, best first.
    pub(super) fn best_weapons_of(
        &self,
        ask: &Ask<'_>,
        weapon_class: Option<&str>,
    ) -> Vec<BestWeaponRow> {
        let class = weapon_class.map(norm);
        let mut rows: Vec<BestWeaponRow> = self
            .weapons
            .iter()
            .enumerate()
            .filter(|(_, weapon)| {
                weapon.infusion(ask.infusion).is_some()
                    && !EMPTY.contains(&weapon.key.as_str())
                    && !weapon.catalyst
                    && !weapon.shield
                    && class
                        .as_ref()
                        .is_none_or(|class| norm(&weapon.class) == *class)
            })
            .filter_map(|(index, _)| self.best_weapon_row(index, ask))
            .collect();
        rows.sort_by(|a, b| b.score.total_cmp(&a.score));
        rows
    }

    /// The script's `best_weapon_row`: the weapon at the build `optimize_build` makes for it,
    /// scored. `None` when no class wields it, when with a window a melee weapon has no attack
    /// timing, or when the rank asks for a metric the row has none of.
    fn best_weapon_row(&self, index: usize, ask: &Ask<'_>) -> Option<BestWeaponRow> {
        let weapon = &self.weapons[index];
        let (value, class, two, stats, worn) = self.optimize_build(
            weapon,
            ask.infusion,
            ask.sl,
            ask.objective,
            ask.grip,
            ask.spells,
            ask.class,
            ask.floors,
            ask.against,
        )?;
        let eff = self.gear_stats(&stats, &worn);
        let defending = self.defending(ask.sl, ask.against);
        let (mut score, mut label) = (value, if two { "2H" } else { "1H" }.to_owned());
        let mut metrics = WeaponMetrics::default();
        // The stamina metric a rank other than the window's reads; none is ported yet, so such a
        // rank finds none, as the script's does for a row without it.
        let by =
            (ask.objective == Objective::Damage && ask.window != 0.0 && ask.rank != Rank::Window)
                .then_some(ask.rank);
        let ammo = if weapon.ranged.is_some() {
            // One shot with its best ammunition at the optimized stats, window or not: the fire
            // rate is not read.
            let (_, note, _, ammo) =
                self.ranged_pick(weapon, ask.infusion, &eff, ask.objective, &defending, &worn);
            if by.is_some() {
                return None;
            }
            label.push_str(" 1 shot");
            Some((ammo, note))
        } else {
            if ask.window != 0.0 && matches!(ask.objective, Objective::Bleed | Objective::Poison) {
                let (hits, hits_label) = Self::status_hits(weapon, &[two], ask.window);
                if hits == 0 {
                    return None;
                }
                score = value * f64::from(hits);
                label = hits_label;
            } else if ask.window != 0.0 && ask.objective == Objective::Damage {
                let timeline = &weapon.timeline[usize::from(two)];
                if timeline.is_empty() {
                    return None;
                }
                let ar = self.attack_rating(weapon.infusion(ask.infusion), &eff, &worn);
                // Scored against the defender's numbers before any answer, as the script's row
                // reads the defense it holds.
                let hits: Vec<&Hit> = timeline.iter().filter(|hit| hit.at <= ask.window).collect();
                score = py_sum(
                    hits.iter()
                        .map(|hit| Self::hit_damage(&ar, &defending.base, hit)),
                ) * weapon.damage_scale;
                label = format!("{label} {} hits", hits.len());
                if by.is_some() {
                    return None;
                }
            }
            self.r1_metrics(
                weapon,
                ask.infusion,
                two,
                &eff,
                &worn,
                &defending.base,
                &mut metrics,
            );
            None
        };
        Some(BestWeaponRow {
            score,
            weapon: weapon.key.clone(),
            name: weapon.name.clone(),
            value,
            class: self.classes[class].name.clone(),
            two_handed: two,
            stats: stats.map(|value| u16::try_from(value).unwrap_or(0)),
            rings: worn
                .iter()
                .map(|&ring| self.rings[ring].1.clone())
                .collect(),
            label,
            ammo,
            metrics,
        })
    }

    /// The script's `r1_metrics` for a melee row: the build-independent numbers the file carries,
    /// and the R1 chain repeated for [`SUSTAIN_HORIZON`] seconds at the row's attack against
    /// `defense`, times the weapon's damage scale.
    // DEBT: ds2-mods-rs-59p7 -- the row's build, which wants bundling as optimize_build's does.
    #[allow(clippy::too_many_arguments)]
    fn r1_metrics(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        two: bool,
        eff: &Stats,
        worn: &[usize],
        defense: &Defense,
        out: &mut WeaponMetrics,
    ) {
        let [reach, startup, recovery, first] = weapon.r1[usize::from(two)];
        out.reach_m = reach;
        out.startup_s = startup;
        out.recovery_s = recovery;
        out.time_to_first_hit_s = first;
        let ar = self.attack_rating(weapon.infusion(infusion), eff, worn);
        out.damage_per_5s = weapon.chains[usize::from(two)]
            .iter()
            .map(|chain| chain.timeline(SUSTAIN_HORIZON, false))
            .find(|timeline| !timeline.is_empty())
            .map(|timeline| {
                let dealt = py_sum(
                    timeline
                        .iter()
                        .filter(|landed| landed.at <= SUSTAIN_HORIZON)
                        .map(|landed| Self::hit_damage(&ar, defense, &landed.hit())),
                );
                py_round(dealt * weapon.damage_scale, 1)
            });
    }
}
