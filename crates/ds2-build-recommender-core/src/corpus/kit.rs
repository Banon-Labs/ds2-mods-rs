//! What a generated build carries beyond its weapons, rings and armour: the script's `choose_kit`,
//! over the `EL`, `HL`, `IB` and `TH` records its `--export-backend` writes and the launchers and
//! ammunition [`super::ranged`] reads.
//!
//! The quick items are the Estus Flask, the fastest heal (its HP capped at the build's max HP), the
//! resin that adds the most to one hit of the primary, the best knife and the best urn by flat
//! damage on the defender, and the eaten buff -- a burr, the Dark Troches or a balm -- that moves
//! the fight's exchange most ([`CorpusBackend::buff_edges`]); short
//! of [`QUICK_MIN`], the runners-up fill the rest; then a Green Blossom and Repair Powder
//! ([`ALWAYS_ITEMS`]). None is an item the game refuses an invader
//! ([`CorpusBackend::invader_refuses`]), which today leaves every heal but the Estus Flask out.
//! The launcher is the bow, greatbow or crossbow
//! with the most damage per stamina point the build can fire and carry, and every ammunition slot
//! is filled.

use ds2_build_import_core::Infusion;

use super::ranged::Kind;
use super::{
    Against, Ar, CorpusBackend, Defending, FTH, Factor, Hit, INT, Stats, Weapon, bad, float,
    py_max, py_sum,
};
use crate::backend::{AMMO_SLOTS, Kit, KitPick, LauncherPick, QUICK_MIN, QUICK_SLOTS};
use crate::model::{Objective, STAT_LABELS};

/// The script's `EQUIP_CAP`: the share of max load a build is kept under.
const EQUIP_CAP: f64 = 0.70;

/// The script's `THROWN_GROUPS`: a thrown item's group by the words in its name, in its order.
const THROWN_GROUPS: [&[&str]; 2] = [&["Knife"], &["Urn", "Firebomb"]];

/// The script's `DMG`, as a defensive item's text names each type.
const DAMAGE_NAMES: [&str; 5] = ["physical", "magic", "fire", "lightning", "dark"];

/// What every quick item filling [`QUICK_MIN`] adds to the reason it was picked on.
pub(super) const FILLS: &str = "; fills the quick slots";

/// The script's `ALWAYS_ITEMS`: quick items every kit carries after its picks and the
/// [`QUICK_MIN`] fill, as (name, why). Green Blossom (60310000) is the only consumable whose text
/// names stamina, and its `SpEffectActiveItem.emevd` event carries `100090[1] [60 s, 1.15]`, the
/// Chloranthy Ring's instruction; Repair Powder (60410000) restores equipment durability. The
/// script's comment holds the evidence.
const ALWAYS_ITEMS: [(&str, &str); 2] = [
    ("Green Blossom", "always: stamina recovery x1.15 for 60 s"),
    ("Repair Powder", "always: restores equipment durability"),
];

/// A heal: the script's `data.heals` row.
#[derive(Clone, Debug)]
pub(super) struct Heal {
    name: String,
    hp: i64,
    seconds: f64,
}

/// A weapon buff a consumable puts on: the script's `data.item_buffs` row.
#[derive(Clone, Debug)]
pub(super) struct ItemBuff {
    name: String,
    /// The element it adds, an index into [`Ar`]; `None` for poison and bleed, which are not
    /// valued.
    element: Option<usize>,
    scale: f64,
    flat: f64,
    seconds: f64,
}

/// A stat a consumable raises for a while (the balms): the script's `data.stat_buffs` row.
#[derive(Clone, Debug)]
pub(super) struct StatBuff {
    pub(super) name: String,
    pub(super) seconds: f64,
    /// (stat, an index into [`Stats`]; how much), in the script's key order.
    pub(super) add: Vec<(usize, i32)>,
}

/// A thrown item: the script's `data.throwables` row.
#[derive(Clone, Debug)]
pub(super) struct Throwable {
    name: String,
    hits: Vec<ThrowHit>,
}

/// One damage row of a throw.
#[derive(Clone, Debug)]
struct ThrowHit {
    mv: f64,
    lower: f64,
    flat: [f64; 5],
}

/// A pool's entry, as the script's `(value, name, why)` tuple.
type Ranked = (f64, String, String);

/// A stand-in launcher for the ammunition slots: (wieldable, minus the points short, damage per
/// stamina point), the weapon, the infusion it was scored in (`None` when it cannot be fired).
type StandIn<'a> = ((i32, i32, f64), &'a Weapon, Option<Infusion>);

/// Python's `sorted(pool, reverse=True)` over `(value, name, why)`.
fn sort_desc(pool: &mut [Ranked]) {
    pool.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
    });
}

/// Python's `format(x, "g")` for the durations the regulation gives: its shortest form, which for
/// a value of six significant digits or fewer is what `g` prints.
fn py_g(x: f64) -> String {
    format!("{x}")
}

impl CorpusBackend {
    /// Whether the game refuses an invader the use of `item`: the script's `data.invader_refused`,
    /// every item whose `ItemUsageParam` check is the one that turns an invader away, but the ones
    /// the script exempts. A generated kit carries none of them.
    #[must_use]
    pub fn invader_refuses(&self, item: &str) -> bool {
        self.invader_refused.iter().any(|name| name == item)
    }

    /// Read an `EL`, `HL`, `IB`, `TH`, `IR` or `SB` record.
    pub(super) fn parse_kit<'a>(
        &mut self,
        tag: &str,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        let mut name = || {
            fields
                .next()
                .map(str::to_owned)
                .ok_or_else(|| bad(line, "a name"))
        };
        match tag {
            "EL" => {
                let mut elements = [(0.0, 0.0); 4];
                for element in &mut elements {
                    let (base, coef) = fields
                        .next()
                        .and_then(|pair| pair.split_once(':'))
                        .ok_or_else(|| bad(line, "base:coefficient"))?;
                    *element = (float(Some(base), line)?, float(Some(coef), line)?);
                }
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "elements before any weapon"))?
                    .elements = Some(elements);
            }
            "HL" => {
                let name = name()?;
                let hp = fields
                    .next()
                    .and_then(|hp| hp.parse::<i64>().ok())
                    .ok_or_else(|| bad(line, "a heal's hit points"))?;
                let seconds = float(fields.next(), line)?;
                self.heals.push(Heal { name, hp, seconds });
            }
            "IR" => {
                let name = name()?;
                self.invader_refused.push(name);
            }
            "SB" => {
                let name = name()?;
                let seconds = float(fields.next(), line)?;
                let mut add = Vec::new();
                for pair in fields.next().unwrap_or("").split(',') {
                    let (stat, value) = pair
                        .split_once(':')
                        .and_then(|(stat, value)| {
                            Some((
                                stat.parse::<usize>().ok().filter(|&stat| stat < 9)?,
                                value.parse::<i32>().ok()?,
                            ))
                        })
                        .ok_or_else(|| bad(line, "stat:value"))?;
                    add.push((stat, value));
                }
                self.stat_buffs.push(StatBuff { name, seconds, add });
            }
            "IB" => {
                let name = name()?;
                let element = match fields.next() {
                    Some("poison" | "bleed") => None,
                    Some(kind) => Some(
                        kind.parse::<usize>()
                            .ok()
                            .filter(|&kind| kind < 5)
                            .ok_or_else(|| bad(line, "a buff type"))?,
                    ),
                    None => return Err(bad(line, "a buff type")),
                };
                let scale = float(fields.next(), line)?;
                let flat = float(fields.next(), line)?;
                let seconds = float(fields.next(), line)?;
                self.item_buffs.push(ItemBuff {
                    name,
                    element,
                    scale,
                    flat,
                    seconds,
                });
            }
            _ => {
                let name = name()?;
                let mut hits = Vec::new();
                for hit in fields {
                    let mut parts = hit.splitn(3, ':');
                    let mv = float(parts.next(), line)?;
                    let lower = float(parts.next(), line)?;
                    let mut flat = [0.0; 5];
                    let mut values = parts.next().unwrap_or("").split(',');
                    for value in &mut flat {
                        *value = float(values.next(), line)?;
                    }
                    hits.push(ThrowHit { mv, lower, flat });
                }
                self.throwables.push(Throwable { name, hits });
            }
        }
        Ok(())
    }

    /// The script's `buff_attack`: the attack a weapon buff in `element` (an index into [`Ar`])
    /// adds to `weapon` at stats `eff` -- its uninfused row's base plus coefficient times the
    /// element's cast bonus, plus the buff's flat, times its scale over 100.
    fn buff_attack(&self, buff: &ItemBuff, element: usize, weapon: &Weapon, eff: &Stats) -> f64 {
        let (base, coef) = weapon
            .elements
            .map_or((0.0, 0.0), |elements| elements[element - 1]);
        let (int, fth) = (eff[INT], eff[FTH]);
        // magic reads INT, fire (INT + FTH) / 2, lightning FTH, dark the lower of the two.
        let at = [int, (int + fth).div_euclid(2), fth, int.min(fth)][element - 1];
        let bonus = self.tables.cast[element - 1].at(at);
        (base + coef * bonus + buff.flat) * buff.scale / 100.0
    }

    /// The script's `thrown_value`: HP one throw takes from the defender, its best hit's flat
    /// damage scored as the attack.
    fn thrown_value(throw: &Throwable, defending: &Defending<'_>) -> f64 {
        let mut best = 0.0;
        for hit in &throw.hits {
            let mut ar: Ar = [None; 5];
            for (kind, &value) in hit.flat.iter().enumerate() {
                if value != 0.0 {
                    ar[kind] = Some(value);
                }
            }
            if ar.iter().all(Option::is_none) {
                continue;
            }
            let (defense, _) = defending.respond(&ar);
            let at = Hit {
                at: 0.0,
                motion_value: hit.mv,
                defense: 0,
                lower: hit.lower,
                flat: [0.0; 5],
            };
            let value = Self::hit_damage(&ar, &defense, &at);
            if value > best {
                best = value;
            }
        }
        best
    }

    /// The script's `buff_edges`: the eaten buffs `choose_kit` ranks for its buff slot -- the
    /// burrs and Dark Troches (`data.defense_buffs` from an item) and the balms
    /// (`data.stat_buffs`) -- each valued as `ln(R' / R)` while it lasts, `R` the hits the
    /// bracket needs to kill the build over the hits it needs to kill them. A defense buff is
    /// `ln(T / T')`, `T` the bracket's mean hit taken at the build's own defense (armour, stats
    /// and rings) and `T'` with the buff added; a balm adds `ln` of the max HP, hit-taken and
    /// damage ratios at the stats it raises (each capped at 99). Above 0 only, best first. The
    /// script's docstring holds why these and only these are ranked against each other.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the build's question.
    #[allow(clippy::too_many_arguments)]
    fn buff_edges(
        &self,
        primary: &Weapon,
        infusion: Infusion,
        stats: &Stats,
        rings: &[usize],
        armor: &[String],
        sl: u32,
        defending: &Defending<'_>,
    ) -> Vec<Ranked> {
        // The script's `armor_keys`: the first piece of each slot with the name, else `Naked`. A
        // build generated with no armour names none, and then wears no piece at all, as the
        // script's `_armor_defense` over an empty list.
        let pieces: Vec<_> = armor
            .iter()
            .zip(&self.wearable)
            .filter_map(|(name, slot)| {
                slot.iter()
                    .find(|piece| piece.name == *name)
                    .or_else(|| slot.iter().find(|piece| piece.key == "Naked"))
            })
            .collect();
        // The script's `at`: max HP, mean hit taken and damage dealt at levelled stats `st`, and
        // the build's own defense there.
        let at = |st: &Stats| {
            let eff = self.gear_stats(st, rings);
            let mine = self.build_defense(st, &pieces, rings);
            let dealt =
                self.objective_value(primary, infusion, &eff, Objective::Damage, defending, rings);
            (
                self.hit_points(&eff),
                self.hit_taken_at(sl, &eff, &mine),
                py_max(dealt, 1e-9),
                mine,
            )
        };
        let eff = self.gear_stats(stats, rings);
        let (hp, taken, dealt, mine) = at(stats);
        let mut out: Vec<Ranked> = Vec::new();
        for buff in self
            .buffs
            .iter()
            .filter(|buff| buff.item && !self.invader_refuses(&buff.name))
        {
            let with = super::adaptive::with_defense(&mine, &buff.add);
            let value = (taken / self.hit_taken_at(sl, &eff, &with)).ln();
            if value > 0.0 {
                let adds: Vec<String> = buff
                    .change
                    .iter()
                    .map(|&(kind, x)| format!("{} +{x}", DAMAGE_NAMES[kind]))
                    .collect();
                out.push((
                    value,
                    buff.name.clone(),
                    format!(
                        "{} for {} s: exchange +{:.2}%",
                        adds.join(" "),
                        py_g(buff.seconds),
                        100.0 * value
                    ),
                ));
            }
        }
        for buff in self
            .stat_buffs
            .iter()
            .filter(|buff| !self.invader_refuses(&buff.name))
        {
            let mut raised = *stats;
            for &(stat, add) in &buff.add {
                raised[stat] = (raised[stat] + add).min(99);
            }
            let (hp2, taken2, dealt2, _) = at(&raised);
            let value = (hp2 / hp).ln() + (taken / taken2).ln() + (dealt2 / dealt).ln();
            if value > 0.0 {
                let adds: Vec<String> = buff
                    .add
                    .iter()
                    .map(|&(stat, x)| format!("{} +{x}", STAT_LABELS[stat]))
                    .collect();
                out.push((
                    value,
                    buff.name.clone(),
                    format!(
                        "{} for {} s: exchange +{:.2}%",
                        adds.join(" "),
                        py_g(buff.seconds),
                        100.0 * value
                    ),
                ));
            }
        }
        sort_desc(&mut out);
        out
    }

    /// The script's `choose_kit`: the quick items, launcher and ammunition of a generated build
    /// whose primary is `primary` in `infusion`, at levelled `stats` wearing `rings` (the first
    /// four of its suggested then common rings) and `armor` (names, head to legs), holding the
    /// weapons named `held` (the primary, then the offhand's left hand: what it equips), for an
    /// invader at `sl` scored against `against`.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the build's question.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn choose_kit(
        &self,
        primary: &Weapon,
        infusion: Infusion,
        stats: &Stats,
        rings: &[usize],
        armor: &[String],
        held: &[&str],
        sl: u32,
        against: Against,
    ) -> Kit {
        let eff = self.gear_stats(stats, rings);
        let defending = self.defending(sl, against);
        let ar = self.attack_rating(primary.infusion(infusion), &eff, rings);
        // `int(hit_points(eff) * prod(...))`: the product is over the rings' HP factors in order.
        // DEBT: ds2-mods-rs-59p7 -- Python's `int()` of a max HP, always in u32 range.
        #[allow(clippy::cast_possible_truncation)]
        let hp = (self.hit_points(&eff) * self.ring_factor(rings, Factor::Hp)) as i64;

        let mut items = Vec::new();
        if !self.invader_refuses("Estus Flask") {
            items.push(KitPick {
                name: "Estus Flask".to_owned(),
                why: "always".to_owned(),
            });
        }
        let mut heals: Vec<Ranked> = self
            .heals
            .iter()
            .filter(|heal| {
                heal.name != "Estus Flask"
                    && heal.seconds > 0.0
                    && !self.invader_refuses(&heal.name)
            })
            .map(|heal| {
                let got = heal.hp.min(hp);
                // DEBT: ds2-mods-rs-59p7 -- hit points, far inside f64's exact integers.
                #[allow(clippy::cast_precision_loss)]
                let per = got as f64 / heal.seconds.max(0.1);
                (
                    per,
                    heal.name.clone(),
                    format!(
                        "heals {got} of {hp} max HP in {} s ({per:.0} HP/s)",
                        py_g(heal.seconds)
                    ),
                )
            })
            .collect();
        sort_desc(&mut heals);

        // A resin coats only a weapon with no elemental attack.
        let mut resins: Vec<Ranked> = Vec::new();
        if ar[1..].iter().all(|value| value.unwrap_or(0.0) == 0.0) {
            let (defense, _) = defending.respond(&ar);
            let one = Hit {
                at: 0.0,
                motion_value: 1.0,
                defense: 0,
                lower: 0.0,
                flat: [0.0; 5],
            };
            let base = Self::hit_damage(&ar, &defense, &one);
            for buff in &self.item_buffs {
                let Some(element) = buff.element.filter(|&element| element > 0) else {
                    continue;
                };
                if self.invader_refuses(&buff.name) {
                    continue;
                }
                let mut add = ar;
                add[element] = Some(
                    add[element].unwrap_or(0.0) + self.buff_attack(buff, element, primary, &eff),
                );
                let (defense, _) = defending.respond(&add);
                let gain = Self::hit_damage(&add, &defense, &one) - base;
                if gain > 0.0 {
                    resins.push((
                        gain,
                        buff.name.clone(),
                        format!(
                            "+{gain:.0} per MV-1 hit ({:.0}%) for {} s",
                            gain / base * 100.0,
                            py_g(buff.seconds)
                        ),
                    ));
                }
            }
            sort_desc(&mut resins);
        }

        let mut throws: Vec<Vec<Ranked>> = THROWN_GROUPS
            .iter()
            .map(|words| {
                let mut pool: Vec<Ranked> = self
                    .throwables
                    .iter()
                    .filter(|throw| {
                        words.iter().any(|word| throw.name.contains(word))
                            && !self.invader_refuses(&throw.name)
                    })
                    .map(|throw| (Self::thrown_value(throw, &defending), throw))
                    .filter(|(value, _)| *value > 0.0)
                    .map(|(value, throw)| {
                        (
                            value,
                            throw.name.clone(),
                            format!("{value:.0} HP per throw (flat, unscaled)"),
                        )
                    })
                    .collect();
                sort_desc(&mut pool);
                pool
            })
            .collect();

        let buffs = self.buff_edges(primary, infusion, stats, rings, armor, sl, &defending);

        for pool in [&heals, &resins]
            .into_iter()
            .chain(throws.iter())
            .chain([&buffs])
        {
            if let Some((_, name, why)) = pool.first() {
                items.push(KitPick {
                    name: name.clone(),
                    why: why.clone(),
                });
            }
        }
        // QUICK_MIN: the next-best heal, throw (either group), eaten buff and resin in turn.
        let mut thrown_rest: Vec<Ranked> = throws
            .iter_mut()
            .flat_map(|pool| pool.drain(..).skip(1))
            .collect();
        sort_desc(&mut thrown_rest);
        let mut spare: Vec<std::collections::VecDeque<Ranked>> = [
            heals.into_iter().skip(1).collect(),
            thrown_rest.into_iter().collect(),
            buffs.into_iter().skip(1).collect(),
            resins.into_iter().skip(1).collect(),
        ]
        .into();
        while items.len() < QUICK_MIN && spare.iter().any(|pool| !pool.is_empty()) {
            for pool in &mut spare {
                if items.len() >= QUICK_MIN {
                    break;
                }
                if let Some((_, name, why)) = pool.pop_front() {
                    items.push(KitPick {
                        name,
                        why: why + FILLS,
                    });
                }
            }
        }
        items.extend(ALWAYS_ITEMS.iter().map(|&(name, why)| KitPick {
            name: name.to_owned(),
            why: why.to_owned(),
        }));
        items.truncate(QUICK_SLOTS);

        // The levelled stats set the cap, not `eff`: `max_load` adds the rings' VIT itself.
        let cap = self.max_load(stats, rings) * EQUIP_CAP;
        let (launcher, ammo) = self.choose_launcher(&eff, rings, armor, held, cap, &defending);
        Kit {
            items,
            launcher,
            ammo,
            max_hp: u32::try_from(hp).unwrap_or(0),
        }
    }

    /// `choose_kit`'s launcher and ammunition: the bow, greatbow or crossbow with the most damage
    /// per stamina point these stats fire and the load `cap` carries beside everything worn and
    /// `held`,
    /// with its best two ammunitions, and every other ammunition slot filled from a stand-in.
    fn choose_launcher(
        &self,
        eff: &Stats,
        rings: &[usize],
        armor: &[String],
        held: &[&str],
        cap: f64,
        defending: &Defending<'_>,
    ) -> (Option<LauncherPick>, [Option<KitPick>; AMMO_SLOTS]) {
        let mut worn = py_sum(armor.iter().zip(&self.wearable).map(|(name, slot)| {
            slot.iter()
                .find(|piece| piece.name == *name)
                .or_else(|| slot.iter().find(|piece| piece.key == "Naked"))
                .map_or(0.0, |piece| piece.weight)
        }));
        worn += py_sum(rings.iter().map(|&ring| self.rings[ring].2));
        worn += py_sum(held.iter().map(|name| self.weight_by_name(name)));

        let mut best: Option<(f64, &Weapon, Infusion, String, f64)> = None;
        for weapon in self.weapons.iter() {
            let Some(launcher) = &weapon.ranged else {
                continue;
            };
            if !weapon.wieldable(eff, launcher.kind != Kind::Crossbow) {
                continue;
            }
            if worn + weapon.weight > cap {
                continue;
            }
            let Some(cost) = weapon.shot_stamina().filter(|&cost| cost != 0.0) else {
                continue;
            };
            for row in &weapon.infusions {
                let (value, note, _, ammo) = self.ranged_pick(
                    weapon,
                    row.infusion,
                    eff,
                    Objective::Damage,
                    defending,
                    rings,
                );
                if value > 0.0
                    && !ammo.is_empty()
                    && best.as_ref().is_none_or(|b| value / cost > b.0)
                {
                    best = Some((value / cost, weapon, row.infusion, note, value));
                }
            }
        }

        let mut ammo: [Option<KitPick>; AMMO_SLOTS] = Default::default();
        let mut launcher = None;
        if let Some((per, weapon, infusion, note, value)) = best {
            let carried = worn + weapon.weight;
            launcher = Some(LauncherPick {
                key: weapon.key.clone(),
                name: weapon.name.clone(),
                infusion,
                why: format!(
                    "{value:.0} per shot ({note}), {per:.1} per stamina point; \
                     load {carried:.1} of {cap:.1}"
                ),
            });
            let slots = if weapon
                .ranged
                .as_ref()
                .is_some_and(|l| l.kind == Kind::Crossbow)
            {
                2
            } else {
                0
            };
            self.fill_ammo(
                &mut ammo, slots, weapon, infusion, eff, rings, defending, "",
            );
        }
        // User rule 2026-10-24: every arrow and bolt slot is full. The slots the launcher does not
        // fire from take the ammunition of a stand-in: the bow (arrow slots) or crossbow (bolt
        // slots) with the most damage per stamina point the build can fire, load aside, else the
        // one it is nearest to firing.
        for (first, kind) in [(0, Kind::Bow), (2, Kind::Crossbow)] {
            if ammo[first..first + 2].iter().any(Option::is_some) {
                continue;
            }
            // The first of equal keys wins, as Python's `max` keeps it.
            let mut stand_in: Option<StandIn<'_>> = None;
            let mut offer = |key: (i32, i32, f64), weapon, infusion| {
                let better = stand_in.as_ref().is_none_or(|(best, ..)| {
                    key.0
                        .cmp(&best.0)
                        .then(key.1.cmp(&best.1))
                        .then(key.2.total_cmp(&best.2))
                        .is_gt()
                });
                if better {
                    stand_in = Some((key, weapon, infusion));
                }
            };
            for weapon in self.weapons.iter() {
                if weapon.ranged.as_ref().is_none_or(|l| l.kind != kind) {
                    continue;
                }
                let short: i32 = weapon
                    .require
                    .iter()
                    .map(|&(stat, value)| (value - eff[stat]).max(0))
                    .sum();
                if !weapon.wieldable(eff, kind != Kind::Crossbow) {
                    offer((0, -short, 0.0), weapon, None);
                    continue;
                }
                let cost = weapon
                    .shot_stamina()
                    .filter(|&cost| cost != 0.0)
                    .unwrap_or(1.0);
                for row in &weapon.infusions {
                    let value = self
                        .ranged_pick(
                            weapon,
                            row.infusion,
                            eff,
                            Objective::Damage,
                            defending,
                            rings,
                        )
                        .0;
                    offer((1, 0, value / cost), weapon, Some(row.infusion));
                }
            }
            if let Some((_, weapon, infusion)) = stand_in {
                let infusion = infusion
                    .or_else(|| weapon.infusions.first().map(|row| row.infusion))
                    .unwrap_or(Infusion::None);
                let from = format!(" from a {}", weapon.name);
                self.fill_ammo(
                    &mut ammo, first, weapon, infusion, eff, rings, defending, &from,
                );
            }
        }
        (launcher, ammo)
    }

    /// The script's `fill`: the two slots from `first` take `weapon`'s ammunition by its own
    /// shot's damage -- the best, then the best of another element, else the next best.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the shot's question.
    #[allow(clippy::too_many_arguments)]
    fn fill_ammo(
        &self,
        ammo: &mut [Option<KitPick>; AMMO_SLOTS],
        first: usize,
        weapon: &Weapon,
        infusion: Infusion,
        eff: &Stats,
        rings: &[usize],
        defending: &Defending<'_>,
        why: &str,
    ) {
        let Some(launcher) = &weapon.ranged else {
            return;
        };
        let ranked = self.ammo_damage(weapon, launcher, infusion, eff, rings, defending);
        let mut picked: Vec<&(f64, String)> = ranked.iter().take(1).collect();
        if let Some((_, top)) = ranked.first() {
            let types = self.ammo_types(top);
            picked.extend(
                ranked[1..]
                    .iter()
                    .filter(|(_, name)| self.ammo_types(name) != types)
                    .take(1),
            );
        }
        let rest: Vec<&(f64, String)> = ranked
            .iter()
            .filter(|shot| !picked.contains(shot))
            .take(2 - picked.len())
            .collect();
        picked.extend(rest);
        for (slot, (value, name)) in ammo[first..first + 2].iter_mut().zip(picked) {
            *slot = Some(KitPick {
                name: name.clone(),
                why: format!("{value:.0} per shot{why}"),
            });
        }
    }
}
