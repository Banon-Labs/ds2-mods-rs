//! Bows, greatbows and crossbows: the script's `ranged_options`, `ranged_value` and
//! `ranged_pick`, over the `RG`, `RS` and `AM` records its `--export-backend` writes.
//!
//! A shot carries the launcher's whole attack plus the ammunition's flat attack, times the hand
//! scale, and its damage is the ammunition's motion value after the defense: one trigger pull,
//! not a window, because the fire rate is not read.

use ds2_build_import_core::Infusion;

use super::{ADP, DEX, FTH};
use super::{
    ATK_BLEED, ATK_POISON, Ar, CorpusBackend, Defending, Hit, SCALE_BLEED, SCALE_POISON, Stats,
    Weapon, bad, float, int,
};
use crate::model::Objective;

/// The script's `STATUS_UNIT`: `100 / (hpMax[99] + 8 x additionalHp[99])`, what the attack builder
/// multiplies a hit's status entries by.
const STATUS_UNIT: f64 = 100.0 / 2505.0;

/// What a launcher is: the script's `SHOOT_AMMO` kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Bow,
    Greatbow,
    Crossbow,
}

/// A bow, greatbow or crossbow: the script's `data.ranged` row.
#[derive(Clone, Debug)]
pub(super) struct Launcher {
    /// The `ArrowParam.arrowType` it fires.
    ammo: i32,
    pub(super) kind: Kind,
    /// `WeaponTypeParam.rightDamageScale`.
    hand: f64,
    /// The script's `VOLLEY_CROSSBOWS` names it.
    volley: bool,
    /// Its own shots, in the script's order.
    special: Vec<Special>,
}

/// A launcher's special shot: the script's `data.ranged[...]["special"]` entry.
#[derive(Clone, Debug)]
struct Special {
    /// The row's damage types as indices into [`Ar`], in the row's order: the only types the shot
    /// keeps.
    types: Vec<usize>,
    flat: [f64; 5],
    mv: f64,
    lower: f64,
    shots: u32,
}

/// One ammunition: the script's `data.ammo` row.
#[derive(Clone, Debug)]
pub(super) struct Ammo {
    name: String,
    kind: i32,
    flat: [f64; 5],
    /// Flat poison and bleed build-up, in the script's `STATUS_PROC` order.
    status: [f64; 2],
    mv: f64,
    lower: f64,
}

/// One shot a launcher can fire, as `ranged_options` lists it.
struct Shot {
    label: String,
    ar: Ar,
    mv: f64,
    lower: f64,
    shots: u32,
    /// Flat poison and bleed build-up; a special's is not read.
    status: [f64; 2],
    special: bool,
    /// The order the script's dict holds `ar`'s types in: DMG order for an ammunition's shot, the
    /// row's own for a special. Only the AR sum reads it.
    order: Vec<usize>,
}

impl CorpusBackend {
    /// Read an `RG`, `RS` or `AM` record.
    pub(super) fn parse_ranged<'a>(
        &mut self,
        tag: &str,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        let five = |fields: &mut dyn Iterator<Item = &'a str>| -> Result<[f64; 5], String> {
            let mut out = [0.0; 5];
            for value in &mut out {
                *value = float(fields.next(), line)?;
            }
            Ok(out)
        };
        let fields: &mut dyn Iterator<Item = &'a str> = fields;
        match tag {
            "RG" => {
                let ammo = int(fields.next(), line)?;
                let kind = match fields.next() {
                    Some("bow") => Kind::Bow,
                    Some("greatbow") => Kind::Greatbow,
                    Some("crossbow") => Kind::Crossbow,
                    _ => return Err(bad(line, "a launcher is a bow, greatbow or crossbow")),
                };
                let hand = float(fields.next(), line)?;
                let volley = fields.next() == Some("1");
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a launcher before any weapon"))?
                    .ranged = Some(Launcher {
                    ammo,
                    kind,
                    hand,
                    volley,
                    special: Vec::new(),
                });
            }
            "RS" => {
                let types = match fields.next() {
                    Some("-") | None => Vec::new(),
                    Some(list) => list
                        .split(',')
                        .map(|kind| {
                            kind.parse::<usize>()
                                .ok()
                                .filter(|&kind| kind < 5)
                                .ok_or_else(|| bad(line, "a damage type index"))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                };
                let flat = five(fields)?;
                let mv = float(fields.next(), line)?;
                let lower = float(fields.next(), line)?;
                let shots = u32::try_from(int(fields.next(), line)?)
                    .map_err(|_| bad(line, "shots per pull"))?;
                let special = Special {
                    types,
                    flat,
                    mv,
                    lower,
                    shots,
                };
                self.weapons
                    .last_mut()
                    .and_then(|weapon| weapon.ranged.as_mut())
                    .ok_or_else(|| bad(line, "a special shot before any launcher"))?
                    .special
                    .push(special);
            }
            _ => {
                let name = fields
                    .next()
                    .ok_or_else(|| bad(line, "ammunition name"))?
                    .to_owned();
                let kind = int(fields.next(), line)?;
                let flat = five(fields)?;
                let status = [float(fields.next(), line)?, float(fields.next(), line)?];
                let mv = float(fields.next(), line)?;
                let lower = float(fields.next(), line)?;
                self.ammo.push(Ammo {
                    name,
                    kind,
                    flat,
                    status,
                    mv,
                    lower,
                });
            }
        }
        Ok(())
    }

    /// The script's `ranged_options`: every shot `weapon` (a launcher) can fire at stats `eff` --
    /// one per ammunition of its arrow type, in name order, then (with `special`) one per special
    /// shot. A shot's attack is the launcher's (`attack_rating`, rings included) plus the
    /// ammunition's flat, times the hand scale; a special keeps only its row's types.
    fn ranged_options(
        &self,
        weapon: &Weapon,
        launcher: &Launcher,
        infusion: Infusion,
        eff: &Stats,
        rings: &[usize],
        special: bool,
    ) -> Vec<Shot> {
        let base = self.attack_rating(weapon.infusion(infusion), eff, rings);
        let mut out = Vec::new();
        for ammo in self.ammo.iter().filter(|ammo| ammo.kind == launcher.ammo) {
            let mut ar: Ar = [None; 5];
            for (kind, slot) in ar.iter_mut().enumerate() {
                let sum = base[kind].unwrap_or(0.0) + ammo.flat[kind];
                if sum != 0.0 {
                    *slot = Some(sum * launcher.hand);
                }
            }
            out.push(Shot {
                label: ammo.name.clone(),
                ar,
                mv: ammo.mv,
                lower: ammo.lower,
                shots: 1,
                status: ammo.status,
                special: false,
                order: (0..5).collect(),
            });
        }
        if special {
            let many = launcher.special.len() > 1;
            for (at, shot) in launcher.special.iter().enumerate() {
                let mut ar: Ar = [None; 5];
                let mut order = Vec::new();
                for &kind in &shot.types {
                    let sum = base[kind].unwrap_or(0.0) + shot.flat[kind];
                    if sum != 0.0 {
                        ar[kind] = Some(sum * launcher.hand);
                        if !order.contains(&kind) {
                            order.push(kind);
                        }
                    }
                }
                out.push(Shot {
                    label: if many {
                        format!("special shot {}", at + 1)
                    } else {
                        "special shot".to_owned()
                    },
                    ar,
                    mv: shot.mv,
                    lower: shot.lower,
                    shots: shot.shots,
                    status: [0.0; 2],
                    special: true,
                    order,
                });
            }
        }
        out
    }

    /// The script's `ranged_value`: a launcher's best shot by `objective`, as (value, its label,
    /// its attack). Damage is one trigger pull against the defender's answer to that shot, at the
    /// ammunition's motion value and floor, against general physical defense, times the weapon's
    /// damage scale and the shot count; AR the shot's summed attack; bleed and poison the build-up
    /// per hit, the launcher's own plus the ammunition's, times the hand scale. `(0, "", none)`
    /// when nothing fires.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the per-shot options.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ranged_value(
        &self,
        weapon: &Weapon,
        launcher: &Launcher,
        infusion: Infusion,
        eff: &Stats,
        objective: Objective,
        defending: &Defending<'_>,
        rings: &[usize],
        special: bool,
    ) -> (f64, String, Ar) {
        let row = weapon.infusion(infusion);
        let mut best: (f64, String, Ar) = (0.0, String::new(), [None; 5]);
        for shot in self.ranged_options(weapon, launcher, infusion, eff, rings, special) {
            let value = match objective {
                Objective::Ar => super::py_sum(shot.order.iter().filter_map(|&kind| shot.ar[kind])),
                Objective::Bleed | Objective::Poison => {
                    if shot.shots > 1 || shot.special {
                        continue;
                    }
                    let (atk, scale, second, status) = if objective == Objective::Bleed {
                        (ATK_BLEED, SCALE_BLEED, eff[FTH], 1)
                    } else {
                        (ATK_POISON, SCALE_POISON, eff[ADP], 0)
                    };
                    let index = 3 * eff[DEX] + second;
                    let own = row.map_or(0.0, |row| {
                        row.atk[atk] + row.scale[scale] * self.tables.aux.at(index)
                    });
                    (own + shot.status[status] * STATUS_UNIT) * launcher.hand
                }
                Objective::Damage => {
                    let (defense, _) = defending.respond(&shot.ar);
                    let hit = Hit {
                        at: 0.0,
                        motion_value: shot.mv,
                        defense: 0,
                        lower: shot.lower,
                        flat: [0.0; 5],
                    };
                    Self::hit_damage(&shot.ar, &defense, &hit)
                        * weapon.damage_scale
                        * f64::from(shot.shots)
                }
            };
            if value > best.0 {
                best = (value, shot.label, shot.ar);
            }
        }
        best
    }

    /// The script's `ranged_pick`: `ranged_value`'s best shot and the note naming it -- the
    /// ammunition, or the special shot (two-handed, its count) with the best ammunition beside it,
    /// and what is unproven -- as (value, note, attack, best ammunition). Nothing for a weapon that
    /// is not a launcher.
    pub(super) fn ranged_pick(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        eff: &Stats,
        objective: Objective,
        defending: &Defending<'_>,
        rings: &[usize],
    ) -> (f64, String, Ar, String) {
        let Some(launcher) = &weapon.ranged else {
            return (0.0, String::new(), [None; 5], String::new());
        };
        let (value, shot, ar) = self.ranged_value(
            weapon, launcher, infusion, eff, objective, defending, rings, true,
        );
        let (best_ammo, ammo, _) = self.ranged_value(
            weapon, launcher, infusion, eff, objective, defending, rings, false,
        );
        let volley = if launcher.volley {
            " (1 bolt; volley and ArrowParam set unproven)"
        } else {
            ""
        };
        let note = if shot.starts_with("special") {
            let at = if shot.ends_with(|c: char| c.is_ascii_digit()) {
                shot.rsplit(' ')
                    .next()
                    .and_then(|n| n.parse::<usize>().ok())
                    .map_or(0, |n| n - 1)
            } else {
                0
            };
            let shots = launcher.special.get(at).map_or(1, |special| special.shots);
            let times = if shots > 1 {
                format!(" x{shots}")
            } else {
                String::new()
            };
            let named = if ammo.is_empty() { "none" } else { &ammo };
            format!("2H special shot{times}, no ammo; best ammo {named} {best_ammo:.0}{volley}")
        } else {
            format!("{shot}{volley}")
        };
        (value, note, ar, ammo)
    }
}
