//! Stance swapping: the script's `power_stance`, `stance_metrics` and `choose_offhand`, over the
//! `ST` records (each weapon's power stance category and left-hand damage scale) and the `SM`
//! records (its attacks in the slots beyond one- and two-handed: the left hand's L1/L2, the chain
//! attacks, power stance, the two-handed L2 and attacks out of a raised guard).
//!
//! A build changes stance between engagements: the right weapon one-handed, two-handed, the left
//! weapon two-handed, or power stance. Each stance is scored as `moveset_metrics` scores a grip,
//! with the other hand's engagements taken in where they raise the rate; the build stands in its
//! best stance that swings the right weapon, and a family may take another stance's engagement at
//! [`STANCE_SWAP`] more seconds. docs/DS2-STANCES.md has the sources.

use ds2_build_import_core::Infusion;

use super::moveset::{FAMILIES, Hand, Move, Scorer, entry, matching_law};
use super::{CorpusBackend, DEX, Defense, EQUIP_CAP, Ranked, STR, Stats, Weapon, Worn};
use crate::backend::{FamilyRate, Offhand, PowerStanceNeed, StanceFamily};

/// The script's `POWER_STANCE`: per category (`WeaponTypeParam.dualWieldingPermission`), the
/// categories it pairs with at the same priority (the right hand's weapon sets the moveset) and
/// those of higher priority (which set it). darksouls2.wiki.gg's table; a pair is listed on
/// either side.
const POWER_STANCE: [(u32, &[u32], &[u32]); 21] = [
    (20, &[20, 30, 170, 240], &[40, 50, 55, 150, 160]),
    (30, &[30], &[40, 50, 55, 150, 160]),
    (40, &[40], &[50, 55, 150, 160]),
    (50, &[50], &[55, 70, 80, 85, 90, 150, 160]),
    (55, &[55], &[70, 85, 90, 150, 160]),
    (60, &[60], &[140]),
    (70, &[70], &[90, 110, 120]),
    (80, &[80], &[85, 100, 110, 120]),
    (85, &[85], &[100, 110, 120]),
    (90, &[90], &[110, 120]),
    (100, &[100], &[110, 120]),
    (110, &[110], &[120]),
    (120, &[120], &[]),
    (130, &[130], &[140]),
    (140, &[140], &[]),
    (150, &[150], &[160]),
    (160, &[160], &[]),
    (170, &[170], &[]),
    (180, &[180], &[]),
    (190, &[190], &[]),
    (220, &[], &[230]),
];

/// The stats power stance raises (the script's `POWER_STANCE_STATS`) to 1.5x the
/// higher one-handed requirement of the two, rounded down.
const POWER_STANCE_STATS: [usize; 2] = [STR, DEX];

/// Seconds a stance change costs an engagement thrown from a stance the build does not stand in:
/// the script's `STANCE_SWAP`, an INFERRED stand-in (er-mods-rs's 20-frame roll entry).
const STANCE_SWAP: f64 = 20.0 / 30.0;

/// The script's `OFFHAND_TRIES` and `OFFHAND_PS_TRIES`.
const OFFHAND_TRIES: usize = 10;
const OFFHAND_PS_TRIES: usize = 5;

/// Movement openers a cross-hand engagement starts from: the script's `CROSS_MOVES`.
const CROSS_MOVES: [&str; 3] = ["Rolling", "Dash", "Step"];

/// The left hand's damage scale when the regulation was not read: the script's default.
const LEFT_SCALE: f64 = 0.9;

/// A pair's power stance: the weapon whose L1/L2 play, the STR and DEX it needs, whether met.
struct PowerStance {
    owner_key: String,
    owner_name: String,
    need: [i32; 2],
    met: bool,
}

/// One hand as an engagement is drafted: its weapon, the hands its attacks strike with, and the
/// mark its slots carry in a label (`L:` for the left).
type Side<'h, 'w> = (&'w Weapon, &'h [&'h Hand<'w>], &'static str);

/// One attack of an engagement: its hand, its full slot, and the hands it strikes with when they
/// are not its own (a power stance L1/L2's two blades).
type Part<'h, 'w> = (Side<'h, 'w>, String, Option<&'h [&'h Hand<'w>]>);

/// One engagement before it is scored: its family, its steps (each attack with the hands it
/// strikes with), the seconds before it and its label.
type Draft<'h, 'w> = (usize, Vec<(&'w Move, &'h [&'h Hand<'w>])>, f64, String);

/// One engagement of `parts`, or `None` when an attack has no live hit: the script's `eng`.
fn eng<'h, 'w>(family: usize, parts: &[Part<'h, 'w>], at: f64) -> Option<Draft<'h, 'w>> {
    let mut steps = Vec::new();
    for &((weapon, hands, _), ref slot, with) in parts {
        steps.push((attack(weapon, slot)?, with.unwrap_or(hands)));
    }
    let label = parts
        .iter()
        .map(|((_, _, tag), slot, _)| format!("{tag}{slot}"))
        .collect::<Vec<_>>()
        .join("+");
    Some((family, steps, at, label))
}

/// A hand's whole moveset under a slot `prefix`: the script's `moveset`.
fn moveset<'h, 'w>(side: Side<'h, 'w>, prefix: &str) -> Vec<Draft<'h, 'w>> {
    let mut out = Vec::new();
    for (family, (_, engagements)) in FAMILIES.iter().enumerate() {
        for engagement in *engagements {
            let parts: Vec<Part<'h, 'w>> = engagement
                .iter()
                .map(|suffix| (side, format!("{prefix}{suffix}"), None))
                .collect();
            out.extend(eng(family, &parts, entry(engagement[0])));
        }
    }
    out
}

/// Every draft scored, `extra` seconds before each, tagged with its `stance`: the script's `rows`.
fn rows(
    scorer: &Scorer<'_>,
    drafts: &[Draft<'_, '_>],
    extra: f64,
    stance: &'static str,
) -> Vec<Row> {
    drafts
        .iter()
        .map(|(family, steps, at, label)| {
            let (row, rate) = scorer.engagement(*family, steps, at + extra, label.clone());
            Row {
                family: *family,
                stance,
                row,
                rate,
            }
        })
        .collect()
}

/// A scored row: the script's row dict with `stance`, `_fam` and `_rate`.
#[derive(Clone)]
struct Row {
    family: usize,
    stance: &'static str,
    row: FamilyRate,
    rate: f64,
}

/// What `stance_metrics` answers.
struct Stances {
    stance: f64,
    home: &'static str,
    stances: Vec<(&'static str, f64)>,
    families: Vec<Row>,
    power_stance: Option<PowerStance>,
    chain: bool,
}

/// A weapon's attack in a full slot: `Single1Hand*` and `Single2Hand*` from its moveset, every
/// other one from its stance attacks. Only attacks with a live hit are recorded.
fn attack<'w>(weapon: &'w Weapon, slot: &str) -> Option<&'w Move> {
    for (grip, prefix) in ["Single1Hand", "Single2Hand"].into_iter().enumerate() {
        if let Some(suffix) = slot.strip_prefix(prefix) {
            return weapon.moves[grip].iter().find(|m| m.slot == suffix);
        }
    }
    weapon.stance_moves.iter().find(|m| m.slot == slot)
}

/// The script's `law`: the matching law over `families`, 0 when no rate is positive.
fn law(families: &[Row]) -> f64 {
    matching_law(families.iter().map(|r| &r.row)).unwrap_or(0.0)
}

/// The script's `offer`: `families` with each offered row taken in its family where it raises the
/// matching law, best rate first.
fn offer(families: &[Row], offered: Vec<Row>) -> Vec<Row> {
    let mut families = families.to_vec();
    let mut offered = offered;
    // Python's `sorted` is stable, as `sort_by` is.
    offered.sort_by(|a, b| b.rate.total_cmp(&a.rate));
    for r in offered {
        let at = families.iter().position(|f| f.family == r.family);
        if let Some(at) = at
            && r.rate <= families[at].row.rate
        {
            continue;
        }
        let mut tried = families.clone();
        match at {
            Some(at) => tried[at] = r,
            None => tried.push(r),
        }
        if law(&tried) > law(&families) {
            families = tried;
        }
    }
    families
}

impl CorpusBackend {
    /// The script's `power_stance`: whether `right` and `left` power stance at `st`, `None` when
    /// their categories cannot pair.
    fn power_stance(right: &Weapon, left: &Weapon, st: &Stats) -> Option<PowerStance> {
        let (a, b) = (right.stance?.0, left.stance?.0);
        if a == 0 || b == 0 {
            return None;
        }
        let pairs = |c: u32| {
            POWER_STANCE
                .iter()
                .find(|&&(category, ..)| category == c)
                .map_or((&[][..], &[][..]), |&(_, same, up)| (same, up))
        };
        let ((same_a, up_a), (same_b, up_b)) = (pairs(a), pairs(b));
        let owner = if same_a.contains(&b) || same_b.contains(&a) {
            right
        } else if up_a.contains(&b) {
            left
        } else if up_b.contains(&a) {
            right
        } else {
            return None;
        };
        let require = |weapon: &Weapon, stat: usize| {
            weapon
                .require
                .iter()
                .find(|&&(at, _)| at == stat)
                .map_or(0, |&(_, value)| value)
        };
        let need = POWER_STANCE_STATS.map(|stat| {
            let higher = require(right, stat).max(require(left, stat));
            // The script's int(1.5 x higher): exact in a double, so 3 x higher / 2 rounded down.
            higher * 3 / 2
        });
        let met = POWER_STANCE_STATS
            .iter()
            .zip(need)
            .all(|(&stat, need)| st[stat] >= need)
            && right.wieldable(st, false)
            && left.wieldable(st, false);
        Some(PowerStance {
            owner_key: owner.key.clone(),
            owner_name: owner.name.clone(),
            need,
            met,
        })
    }

    /// The script's `stance_metrics`: `right` with `rinf` in the right hand and `left` (a weapon
    /// or a shield, with its infusion) in the left, swapping stance. `None` when no stance has an
    /// attack.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the row's question.
    #[allow(clippy::too_many_arguments)]
    fn stance_metrics(
        &self,
        right: &Weapon,
        rinf: Infusion,
        left: Option<(&Weapon, Infusion)>,
        eff: &Stats,
        rings: &[usize],
        sl: u32,
        defense: &Defense,
        worn: Worn,
    ) -> Option<Stances> {
        let scorer = Scorer::new(self, eff, rings, sl, defense, worn);
        let r_hand = scorer.hand(right, rinf, None);
        let shield = left
            .filter(|(weapon, _)| weapon.shield)
            .map(|(weapon, _)| weapon);
        let l_hand = left
            .filter(|(weapon, _)| !weapon.shield)
            .map(|(weapon, inf)| {
                scorer.hand(weapon, inf, Some(weapon.stance.map_or(LEFT_SCALE, |s| s.1)))
            });
        let power_stance = l_hand
            .as_ref()
            .and_then(|l| Self::power_stance(right, l.weapon, eff));
        let chain = power_stance.as_ref().is_some_and(|ps| ps.met);
        let r_one = [&r_hand];
        let l_one: Vec<&Hand<'_>> = l_hand.iter().collect();
        let both: Vec<&Hand<'_>> = std::iter::once(&r_hand).chain(&l_hand).collect();
        let r_side: Side<'_, '_> = (right, &r_one, "");
        let l_side: Option<Side<'_, '_>> = l_hand.as_ref().map(|l| (l.weapon, &l_one[..], "L:"));
        let fam = |name: &str| FAMILIES.iter().position(|(f, _)| *f == name).unwrap_or(0);
        let plain = |side: Side<'_, '_>| {
            if side.2.is_empty() {
                "Single1Hand"
            } else {
                "Left1Hand"
            }
        };

        // Per stance: its own weapon's moveset, and what the other hand adds.
        let mut stances: Vec<(&'static str, Vec<Draft<'_, '_>>, Vec<Draft<'_, '_>>)> = Vec::new();
        if right.wieldable(eff, false) {
            let mut added = Vec::new();
            if let Some(l_side) = l_side
                && l_side.0.wieldable(eff, false)
            {
                added.extend(moveset(l_side, "Left1Hand"));
                for (x, y) in [(r_side, l_side), (l_side, r_side)] {
                    let follow = if chain { "Dual1Hand" } else { plain(y) };
                    for (family, a, b) in [
                        ("r1", "Normal1st", "Normal1st"),
                        ("r2", "Strong1st", "Strong1st"),
                    ] {
                        added.extend(eng(
                            fam(family),
                            &[
                                (x, format!("{}{a}", plain(x)), None),
                                (y, format!("{follow}{b}"), None),
                            ],
                            0.0,
                        ));
                    }
                    for m in CROSS_MOVES {
                        added.extend(eng(
                            fam("move"),
                            &[
                                (x, format!("{}{m}", plain(x)), None),
                                (y, format!("{follow}Normal1st"), None),
                            ],
                            entry(m),
                        ));
                    }
                }
            }
            if shield.is_some_and(|s| s.wieldable(eff, false)) {
                let guard = |slot: &str| (r_side, slot.to_owned(), None);
                added.extend(eng(fam("r1"), &[guard("GuardNormal1st")], 0.0));
                added.extend(eng(
                    fam("r1"),
                    &[guard("GuardNormal1st"), guard("GuardNormal2nd")],
                    0.0,
                ));
            }
            stances.push(("1H", moveset(r_side, "Single1Hand"), added));
        }
        if right.wieldable(eff, true) {
            stances.push((
                "2H",
                moveset(r_side, "Single2Hand"),
                two_handed_l2(r_side, fam("r2")),
            ));
        }
        if let Some(l_side) = l_side
            && l_side.0.wieldable(eff, true)
        {
            stances.push((
                "L2H",
                moveset(l_side, "Single2Hand"),
                two_handed_l2(l_side, fam("r2")),
            ));
        }
        if let (true, Some(ps), Some(l_side)) = (chain, power_stance.as_ref(), l_side) {
            let owner = if ps.owner_key == right.key {
                r_side
            } else {
                l_side
            };
            let pair = |slot: &str| (owner, slot.to_owned(), Some(&both[..]));
            let mut added = Vec::new();
            added.extend(eng(fam("r1"), &[pair("OppositeDual2HandNormal1st")], 0.0));
            added.extend(eng(
                fam("r1"),
                &[
                    pair("OppositeDual2HandNormal1st"),
                    pair("OppositeDual2HandNormal2nd"),
                ],
                0.0,
            ));
            added.extend(eng(fam("r2"), &[pair("OppositeDual2HandStrong")], 0.0));
            stances.push(("PS", moveset(r_side, "Dual2Hand"), added));
        }

        let mut own: Vec<(&'static str, Vec<Row>)> = Vec::new();
        for (name, base, added) in &stances {
            let mut families: Vec<Row> = Vec::new();
            // The stance's own weapon: its moveset_metrics, family by family.
            for r in rows(&scorer, base, 0.0, name) {
                match families.iter().position(|f| f.family == r.family) {
                    None => families.push(r),
                    Some(at) if r.rate > families[at].row.rate => families[at] = r,
                    Some(_) => {}
                }
            }
            let offered = rows(&scorer, added, 0.0, name);
            own.push((name, offer(&families, offered)));
        }
        let scores: Vec<(&'static str, f64)> = own
            .iter()
            .map(|(name, families)| (*name, law(families)))
            .collect();
        if scores.iter().all(|&(_, score)| score == 0.0) {
            return None;
        }
        // The right hand's weapon is the build's: it stands in a stance that swings it. Python's
        // `max` keeps the first of a tie.
        let home = scores
            .iter()
            .filter(|(name, _)| *name != "L2H")
            .fold(
                None,
                |best: Option<(&'static str, f64)>, &(name, score)| match best {
                    Some((_, top)) if score <= top => best,
                    _ => Some((name, score)),
                },
            )
            .map_or("L2H", |(name, _)| name);
        let home_own = own
            .iter()
            .find(|(name, _)| *name == home)
            .map_or(&[][..], |(_, families)| &families[..]);
        let mut swapped = Vec::new();
        for (name, base, added) in stances.iter().filter(|(name, ..)| *name != home) {
            swapped.extend(rows(&scorer, base, STANCE_SWAP, name));
            swapped.extend(rows(&scorer, added, STANCE_SWAP, name));
        }
        let families = offer(home_own, swapped);
        Some(Stances {
            stance: law(&families),
            home,
            stances: scores,
            families,
            power_stance,
            chain,
        })
    }
    /// The script's `choose_offhand`: of nothing, the primary's twin, the lightest shield when it
    /// attacks out of a raised guard, and `ranked`'s first [`OFFHAND_TRIES`] melee weapons and the
    /// next [`OFFHAND_PS_TRIES`] that power stance with it, the left hand that rates highest.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the row's question.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn choose_offhand(
        &self,
        primary: &Weapon,
        infusion: Infusion,
        eff: &Stats,
        rings: &[usize],
        sl: u32,
        defense: &Defense,
        worn: Worn,
        ranked: &[Ranked],
        room: Option<f64>,
    ) -> Option<Offhand> {
        if primary.ranged.is_some() {
            return None;
        }
        let mut melee: Vec<(&Weapon, Infusion)> = Vec::new();
        for row in ranked {
            // generate_build's: a name's first weapon.
            let name = &self.weapons[row.weapon].name;
            let Some(weapon) = self.weapons.iter().find(|w| w.name == *name) else {
                continue;
            };
            if melee.iter().any(|(w, _)| w.key == weapon.key)
                || weapon.ranged.is_some()
                || weapon.catalyst
                || weapon.shield
            {
                continue;
            }
            melee.push((weapon, row.infusion));
        }
        let mut picks: Vec<(&Weapon, Infusion)> =
            melee.iter().take(OFFHAND_TRIES).copied().collect();
        picks.extend(
            melee
                .iter()
                .skip(OFFHAND_TRIES)
                .filter(|(w, _)| Self::power_stance(primary, w, eff).is_some())
                .take(OFFHAND_PS_TRIES)
                .copied(),
        );
        if !melee.iter().any(|(w, _)| w.key == primary.key) {
            picks.push((primary, infusion));
        }
        if attack(primary, "GuardNormal1st").is_some() {
            let lightest = self
                .weapons
                .iter()
                .filter(|w| w.shield && w.wieldable(eff, false))
                .min_by(|a, b| {
                    a.weight
                        .total_cmp(&b.weight)
                        .then_with(|| a.key.cmp(&b.key))
                });
            if let Some(shield) = lightest {
                picks.push((shield, Infusion::None));
            }
        }
        let base = self.stance_metrics(primary, infusion, None, eff, rings, sl, defense, worn);
        let solo = base.as_ref().map_or(0.0, |b| b.stance);
        let mut best: (Option<(&Weapon, Infusion)>, Option<Stances>) = (None, base);
        let mut short = None;
        for (weapon, linf) in picks {
            if room.is_some_and(|room| weapon.weight > room) {
                continue; // it would leave no armour under the load cap (offhand_room)
            }
            let m = self.stance_metrics(
                primary,
                infusion,
                Some((weapon, linf)),
                eff,
                rings,
                sl,
                defense,
                worn,
            );
            let Some(m) = m else { continue };
            if short.is_none()
                && let Some(ps) = m.power_stance.as_ref()
                && !ps.met
            {
                short = Some((weapon.name.clone(), ps.need.map(to_u16)));
            }
            if m.stance > best.1.as_ref().map_or(0.0, |b| b.stance) {
                best = (Some((weapon, linf)), Some(m));
            }
        }
        let (left, stances) = best;
        let stances = stances?;
        Some(Offhand {
            left: left.map(|(w, inf)| (w.key.clone(), w.name.clone(), inf)),
            stance: stances.stance,
            solo,
            home: stances.home.to_owned(),
            stances: stances
                .stances
                .iter()
                .map(|&(name, score)| (name.to_owned(), score))
                .collect(),
            families: stances
                .families
                .into_iter()
                .map(|r| StanceFamily {
                    family: r.row.family,
                    stance: r.stance.to_owned(),
                    engagement: r.row.engagement,
                    rate: r.row.rate,
                })
                .collect(),
            power_stance: stances.power_stance.map(|ps| PowerStanceNeed {
                owner: ps.owner_name,
                need: ps.need.map(to_u16),
                met: ps.met,
            }),
            chain: stances.chain,
            ps_short: short,
        })
    }
}

/// A stat need as the panel's `u16`.
fn to_u16(value: i32) -> u16 {
    u16::try_from(value).unwrap_or(0)
}

/// A two-handed hand's L2 where it hits, as an engagement of `family`.
fn two_handed_l2<'h, 'w>(side: Side<'h, 'w>, family: usize) -> Vec<Draft<'h, 'w>> {
    eng(
        family,
        &[(side, "OppositeSingle2HandStrong".to_owned(), None)],
        0.0,
    )
    .into_iter()
    .collect()
}

impl CorpusBackend {
    /// The script's `offhand_room`: the weight a left hand may add and still leave a full set of
    /// armour under the load cap, beside the heaviest of `weapons`, the heaviest of `catalysts`
    /// and `rings`. A left hand is not worth going bare for.
    pub(super) fn offhand_room(
        &self,
        stats: &Stats,
        weapons: &[f64],
        catalysts: &[f64],
        rings: &[usize],
    ) -> f64 {
        // Python's `max` keeps the first of a tie, as generate_armor's does.
        let heaviest = |list: &[f64]| {
            list.split_first().map(|(&first, rest)| {
                rest.iter()
                    .fold(first, |best, &w| if w > best { w } else { best })
            })
        };
        let cap = self.max_load(stats, rings) * EQUIP_CAP;
        let eff = self.gear_stats(stats, rings);
        let carried = heaviest(weapons)
            .into_iter()
            .chain(heaviest(catalysts))
            .fold(0.0, |total, w| total + w);
        let ring_weight = rings
            .iter()
            .map(|&ring| self.rings[ring].2)
            .fold(0.0, |total, w| total + w);
        let lightest = self
            .wearable
            .iter()
            .map(|slot| {
                slot.iter()
                    .filter(|piece| piece.key != "Naked")
                    .filter(|piece| piece.require.iter().all(|&(stat, need)| need <= eff[stat]))
                    .map(|piece| piece.weight)
                    .min_by(f64::total_cmp)
                    .unwrap_or(0.0)
            })
            .fold(0.0, |total, w| total + w);
        cap - (carried + ring_weight) - lightest
    }
}
