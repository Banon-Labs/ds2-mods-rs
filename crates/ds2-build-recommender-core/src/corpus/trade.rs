//! The R1 trade, and what poise buys a generated build.
//!
//! The script's `first_strike`, `trade`, `exchange_pairs`, `exchange_value`, `armor_poise` (with
//! its stat and requirement terms), `build_defense`, `armor_poise_steps`, `granted_trades` and
//! `armor_trade`, over the `FS` records (each weapon's R1 first hit per grip, timed at the game
//! play speed) and the `XT` records (each corpus build's counter-hit, with its poise, defense and
//! attack rating).
//!
//! Both sides press R1 at the same instant. The earlier hit lands; the later one lands unless the
//! first staggered its swinger: armorBreak 2 always, 1 outside hyperarmor, else when its poise
//! damage -- times the swinger's `uninterruptibleRate` inside a hyperarmor window -- reaches the
//! swinger's poise. A tie counts as mine first.

use std::cmp::Ordering;
use std::collections::HashMap;

use ds2_build_import_core::Infusion;

use super::{
    ADP, Ar, CorpusBackend, DEX, Defense, END, EQUIP_CAP, FTH, Hit, INT, LOAD_PRICE, STR, Stats,
    VIT, Wearable, bad, float, int, py_max, py_min, py_round, py_sum, sl_bracket,
};
use crate::backend::ArmorTrade;

/// The Stone Ring's flat poise damage per hit: the script's `STONE_RING_POISE`.
const STONE_RING_POISE: f64 = 30.0;

/// One R1's earliest live hit, as the trade model reads it: the script's `first_strike`.
#[derive(Clone, Debug)]
pub(super) struct Strike {
    /// Seconds from the input to the hit at the game play speed (`at`), its motion value, defense
    /// type, `damageLower` and flat attack.
    hit: Hit,
    /// Poise damage to a player.
    pd: f64,
    /// `DamageCtrlParam.armorBreak`.
    armor_break: i32,
    /// `WeaponParam.damageScale`.
    scale: f64,
    /// `WeaponParam.uninterruptibleRate`: the factor on poise damage taken inside `armor`.
    rate: f64,
    /// The R1's hyperarmor windows, in seconds; none when `rate` is 0.
    armor: Vec<(f64, f64)>,
}

/// A corpus build's counter-hit: the script's `trade_counter`.
#[derive(Clone, Debug)]
pub(super) struct Counter {
    /// Its first strike, the Stone Ring's poise damage added when it wears one.
    strike: Strike,
    /// Its own max poise.
    poise: f64,
    /// Its own defense, rings as worn.
    defense: Defense,
    /// Its weapon's attack rating at its stats and rings.
    ar: Ar,
}

/// A strike's fields from a record: t pd ab mv type lower flat(5) scale rate armor.
fn strike<'a>(fields: &mut impl Iterator<Item = &'a str>, line: usize) -> Result<Strike, String> {
    let at = float(fields.next(), line)?;
    let pd = float(fields.next(), line)?;
    let armor_break = int(fields.next(), line)?;
    let motion_value = float(fields.next(), line)?;
    let defense = match fields.next() {
        Some("p") => 0,
        Some("s") => 5,
        Some("k") => 6,
        Some("t") => 7,
        _ => return Err(bad(line, "hit type")),
    };
    let lower = float(fields.next(), line)?;
    let flat = floats::<5>(fields.next(), line)?;
    let scale = float(fields.next(), line)?;
    let rate = float(fields.next(), line)?;
    let armor = match fields.next() {
        Some("-") => Vec::new(),
        Some(list) => list
            .split(',')
            .map(|window| {
                let (start, end) = window
                    .split_once(':')
                    .ok_or_else(|| bad(line, "hyperarmor window start:end"))?;
                Ok((float(Some(start), line)?, float(Some(end), line)?))
            })
            .collect::<Result<Vec<_>, String>>()?,
        None => return Err(bad(line, "hyperarmor windows")),
    };
    Ok(Strike {
        hit: Hit {
            at,
            motion_value,
            defense,
            lower,
            flat,
        },
        pd,
        armor_break,
        scale,
        rate,
        armor,
    })
}

/// `N` comma-separated floats.
fn floats<const N: usize>(field: Option<&str>, line: usize) -> Result<[f64; N], String> {
    let mut out = [0.0; N];
    let mut values = field.unwrap_or("").split(',');
    for slot in &mut out {
        *slot = float(values.next(), line)?;
    }
    if values.next().is_some() {
        return Err(bad(line, "too many values"));
    }
    Ok(out)
}

/// The script's `stagger_threshold`: the poise damage `hit` deals `defender` at the hit's time.
///
/// `defender` is the strike the defender is swinging: inside its hyperarmor window the damage is
/// times its `uninterruptibleRate`. `None` when the hit staggers whatever the poise: armorBreak 2,
/// or 1 outside hyperarmor.
fn stagger_threshold(hit: &Strike, defender: &Strike) -> Option<f64> {
    let t = hit.hit.at;
    let armored = defender
        .armor
        .iter()
        .any(|&(start, end)| start <= t && t < end);
    if hit.armor_break == 2 || (hit.armor_break == 1 && !armored) {
        return None;
    }
    Some(hit.pd * if armored { defender.rate } else { 1.0 })
}

/// The script's `trade`: (my hit lands, theirs lands), both pressing R1 at once.
fn trade(mine: &Strike, theirs: &Strike, my_poise: f64, their_poise: f64) -> (bool, bool) {
    if theirs.hit.at < mine.hit.at {
        let held = stagger_threshold(theirs, mine).is_some_and(|th| th < my_poise);
        return (held, true);
    }
    let lands = stagger_threshold(mine, theirs).is_some_and(|th| th < their_poise);
    (true, lands)
}

/// The script's `exchange_pairs` table: every (my weapon, counter) trade reduced to what the
/// armour changes.
pub(super) struct Exchange<'a> {
    /// My damage in pairs where I hit first.
    fixed: f64,
    /// Pairs where I hit first.
    first: usize,
    /// Where the counter hits first and poise could hold: the stagger thresholds, sorted.
    keys: Vec<f64>,
    /// The running sum of my damage over `keys`, from 0.
    cum: Vec<f64>,
    /// Per counter, how many of my weapons it lands on.
    taken: Vec<u32>,
    counters: Vec<&'a Counter>,
    pairs: usize,
}

/// The script's `exchange_pairs`: `mine` are my strikes with their attack ratings.
fn exchange_pairs<'a>(mine: &[(Strike, Ar)], counters: Vec<&'a Counter>) -> Option<Exchange<'a>> {
    if mine.is_empty() || counters.is_empty() {
        return None;
    }
    let (mut fixed, mut first, mut held) = (0.0, 0, Vec::new());
    let mut taken = vec![0_u32; counters.len()];
    for (w, ar) in mine {
        for (i, c) in counters.iter().enumerate() {
            let out = CorpusBackend::hit_damage(ar, &c.defense, &w.hit) * w.scale;
            if c.strike.hit.at < w.hit.at {
                taken[i] += 1;
                if let Some(th) = stagger_threshold(&c.strike, w) {
                    held.push((th, out));
                }
            } else {
                fixed += out;
                first += 1;
                taken[i] += u32::from(trade(w, &c.strike, 0.0, c.poise).1);
            }
        }
    }
    // Python's `sort(key=...)`: stable, comparing as floats compare.
    held.sort_by(|a: &(f64, f64), b| cmp(a.0, b.0));
    let mut cum = Vec::with_capacity(held.len() + 1);
    let mut running = 0.0;
    cum.push(running);
    for &(_, out) in &held {
        running += out;
        cum.push(running);
    }
    let pairs = mine.len() * counters.len();
    Some(Exchange {
        fixed,
        first,
        keys: held.into_iter().map(|(th, _)| th).collect(),
        cum,
        taken,
        counters,
        pairs,
    })
}

/// Python's `bisect_left`: how many of the sorted `keys` are below `x`.
fn bisect_left(keys: &[f64], x: f64) -> usize {
    keys.partition_point(|&key| key < x)
}

/// A count as a double; the counts here are far below 2^32.
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The script's `exchange_value`: (exchange value, trade rate) of wearing `poise` and `defense`.
fn exchange_value(table: &Exchange<'_>, poise: f64, defense: &Defense) -> (f64, f64) {
    let k = bisect_left(&table.keys, poise);
    let taken = py_sum(
        table
            .taken
            .iter()
            .zip(&table.counters)
            .filter(|&(&n, _)| n != 0)
            .map(|(&n, c)| {
                f64::from(n)
                    * CorpusBackend::hit_damage(&c.ar, defense, &c.strike.hit)
                    * c.strike.scale
            }),
    );
    (
        (table.fixed + table.cum[k] - taken) / count(table.pairs),
        count(table.first + k) / count(table.pairs),
    )
}

/// The script's `lack_of_stats`: the share of a piece's poise lost to unmet requirements.
fn lack_of_stats(require: &[(usize, i32)], stats: &Stats) -> f64 {
    let v = py_min(
        1.0,
        py_sum(
            require
                .iter()
                .filter(|&&(_, need)| need > 0)
                .map(|&(stat, need)| py_max(0.0, 1.0 - f64::from(stats[stat]) / f64::from(need))),
        ),
    );
    if v > 0.0 {
        py_min(1.0, py_max(0.0, (30.0 + 50.0 * v) * 0.01))
    } else {
        0.0
    }
}

/// [`ArmorTrade`] before it is narrowed to `f32`: the script's `armor_trade` dict, rounded as it
/// rounds.
#[derive(Clone, Copy, Debug)]
pub(super) struct Numbers {
    poise: f64,
    poise_target: f64,
    trade_rate: f64,
    exchange: f64,
}

impl Numbers {
    /// As the panel carries them.
    pub(super) const fn public(self) -> ArmorTrade {
        ArmorTrade {
            poise: self.poise as f32,
            poise_target: self.poise_target as f32,
            trade_rate: self.trade_rate as f32,
            exchange: self.exchange as f32,
        }
    }
}

/// Python's `format(x, "+.1f")`.
fn signed(x: f64) -> String {
    format!("{x:+.1}")
}

/// Python's `format(x, ".0%")`: `x` times 100 as a double, no decimals.
fn percent(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// The keys of a set, head to legs: what the script compares sets by.
fn keys<'a>(set: &[&'a Wearable; 4]) -> [&'a str; 4] {
    set.map(|piece| piece.key.as_str())
}

/// Floats compared as Python compares them.
fn cmp(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// Who the armour is chosen for in the script's `generate_armor`: the levelled `stats`, the
/// weights of what it holds, the rings it wears and its load scarcity.
pub(super) struct Wearer<'a> {
    pub(super) stats: &'a Stats,
    pub(super) held: &'a [f64],
    pub(super) rings: &'a [usize],
    pub(super) scarcity: f64,
}

/// What [`CorpusBackend::trade_armor`] decided: the set, the numbers of it and of `best_armor`'s
/// set, and the note saying poise drove the pick when it did.
pub(super) struct Traded<'s> {
    pub(super) pieces: [&'s Wearable; 4],
    pub(super) numbers: Option<(Numbers, Numbers)>,
    pub(super) note: Option<String>,
}

/// One partial set of `armor_poise_steps`: (weight, defense value, pieces so far).
type State<'s> = (f64, f64, Vec<&'s Wearable>);

impl CorpusBackend {
    /// Read an `FS` record onto the last weapon, or an `XT` record onto the last corpus build.
    pub(super) fn parse_trade<'a>(
        &mut self,
        tag: &str,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        if tag == "FS" {
            let grip = match fields.next() {
                Some("1") => 0,
                Some("2") => 1,
                _ => return Err(bad(line, "grip is 1 or 2")),
            };
            let strike = strike(fields, line)?;
            self.weapons
                .last_mut()
                .ok_or_else(|| bad(line, "a first strike before any weapon"))?
                .strike[grip] = Some(strike);
        } else {
            let strike = strike(fields, line)?;
            let poise = float(fields.next(), line)?;
            let defense = floats::<8>(fields.next(), line)?;
            let ar = floats::<5>(fields.next(), line)?.map(Some);
            self.corpus
                .last_mut()
                .ok_or_else(|| bad(line, "a counter-hit before any corpus build"))?
                .trade = Some(Counter {
                strike,
                poise,
                defense,
                ar,
            });
        }
        Ok(())
    }

    /// The script's `armor_poise`: the pieces' poise, each less the share its unmet requirements
    /// take at `stats`, plus the stat poise at min(END, ADP), plus the rings'.
    ///
    /// Without `stats` the pieces count in full and there is no stat term.
    fn armor_poise(&self, pieces: &[&Wearable; 4], rings: &[usize], stats: Option<&Stats>) -> f64 {
        let mut total = 0.0;
        for piece in pieces {
            let lost = stats.map_or(0.0, |stats| lack_of_stats(&piece.require, stats));
            total += piece.poise * (1.0 - lost);
        }
        if let Some(stats) = stats
            && !self.tables.stat_poise.0.is_empty()
        {
            total += self.tables.stat_poise.at(stats[END].min(stats[ADP]));
        }
        total + py_sum(rings.iter().map(|&ring| self.ring_poise[ring]))
    }

    /// The script's `build_defense` of a build at levelled `stats` wearing `pieces` and `rings`.
    ///
    /// The pieces' stat changes and the rings' (their `ring_effects` adds, uncapped) raise the
    /// stats the defense reads; then each ring's defense changes are added.
    fn build_defense(&self, stats: &Stats, pieces: &[&Wearable; 4], rings: &[usize]) -> Defense {
        let mut eff = *stats;
        for piece in pieces {
            for (stat, add) in eff.iter_mut().zip(piece.alter) {
                *stat += add;
            }
        }
        for &ring in rings {
            let key = &self.rings[ring].0;
            if let Some(effect) = self.ring_effects.iter().find(|effect| effect.key == *key) {
                for (stat, add) in eff.iter_mut().zip(effect.add) {
                    *stat += add;
                }
            }
        }
        let tables = &self.tables.defense;
        let pb = tables[0].at(eff[END] + eff[VIT] + eff[STR] + eff[DEX]);
        let mut out = [0.0; 8];
        out[0] = py_sum(
            pieces
                .iter()
                .map(|piece| piece.defense[0] + piece.bonus * pb),
        );
        for (typed, value) in out[5..].iter_mut().enumerate() {
            *value = py_sum(
                pieces
                    .iter()
                    .map(|piece| piece.typed[typed] + piece.bonus * pb),
            );
        }
        let base = [
            eff[INT],
            eff[INT] + eff[FTH],
            eff[FTH],
            eff[INT].min(eff[FTH]),
        ];
        for (element, index) in base.into_iter().enumerate() {
            out[1 + element] = tables[1 + element].at(index)
                + py_sum(pieces.iter().map(|piece| piece.defense[1 + element]));
        }
        for &ring in rings {
            for &(kind, value) in self.ring_change(ring).map_or(&[][..], Vec::as_slice) {
                out[kind] += value;
                if kind == 0 {
                    for typed in &mut out[5..] {
                        *typed += value;
                    }
                }
            }
        }
        out
    }

    /// A piece's defense weighted by the corpus threat mix, summed as the script's `sum` does.
    fn armor_value(&self, piece: &Wearable) -> f64 {
        py_sum(
            self.threat_mix
                .iter()
                .zip(piece.defense)
                .map(|(share, defense)| share * defense),
        )
    }

    /// The script's `armor_poise_steps`: `best_armor`'s pick at every armour poise the budget
    /// reaches, poise rising, each set once.
    ///
    /// Per slot only the pieces no lighter-or-equal piece matches in defense and poise both
    /// count; the slots are combined keeping, per poise sum, the sets no lighter one outscores.
    fn armor_poise_steps(&self, wearer: &Wearer<'_>) -> Vec<(f64, [&Wearable; 4])> {
        let (stats, rings) = (wearer.stats, wearer.rings);
        let (_, _, Some(top)) = self.best_armor(stats, wearer.held, rings, 0.0) else {
            return Vec::new();
        };
        let value = top
            .iter()
            .fold(0.0, |total, piece| total + self.armor_value(piece));
        let weight = top.iter().fold(0.0, |total, piece| total + piece.weight);
        let price = if weight > 0.0 { value / weight } else { 0.0 };
        let k = wearer.scarcity * LOAD_PRICE * price;
        let mut budget = self.max_load(stats, rings) * EQUIP_CAP;
        budget -= py_sum(wearer.held.iter().copied());
        budget -= py_sum(rings.iter().map(|&ring| self.rings[ring].2));
        let st = self.gear_stats(stats, rings);
        let score = |w: f64, v: f64| v - k * w;

        // pieces' poise -> partial sets, in the order the script's dict first saw each poise
        let mut states: Vec<(f64, Vec<State<'_>>)> = vec![(0.0, vec![(0.0, 0.0, Vec::new())])];
        for slot in &self.wearable {
            let mut cands: Vec<(f64, f64, f64, &Wearable)> = slot
                .iter()
                .filter(|piece| piece.require.iter().all(|&(stat, need)| need <= st[stat]))
                .map(|piece| (piece.weight, self.armor_value(piece), piece.poise, piece))
                .collect();
            cands.sort_by(|a, b| {
                cmp(a.0, b.0)
                    .then(cmp(b.1, a.1))
                    .then(cmp(b.2, a.2))
                    .then_with(|| a.3.key.cmp(&b.3.key))
            });
            let mut front: Vec<(f64, f64, f64, &Wearable)> = Vec::new();
            for c in cands {
                if !front.iter().any(|f| f.1 >= c.1 && f.2 >= c.2) {
                    front.push(c);
                }
            }
            let mut next: Vec<(f64, Vec<State<'_>>)> = Vec::new();
            let mut at: HashMap<u64, usize> = HashMap::new();
            for (p, lst) in &states {
                for &(w, v, pp, piece) in &front {
                    let key = py_round(p + pp, 3);
                    for (sw, sv, pcs) in lst {
                        if sw + w <= budget {
                            let index = *at.entry(key.to_bits()).or_insert_with(|| {
                                next.push((key, Vec::new()));
                                next.len() - 1
                            });
                            let mut pieces = pcs.clone();
                            pieces.push(piece);
                            next[index].1.push((sw + w, sv + v, pieces));
                        }
                    }
                }
            }
            states = next
                .into_iter()
                .map(|(p, mut lst)| {
                    lst.sort_by(|a, b| cmp(a.0, b.0).then(cmp(-score(a.0, a.1), -score(b.0, b.1))));
                    let mut lean = Vec::new();
                    let mut best: Option<f64> = None;
                    for s in lst {
                        let here = score(s.0, s.1);
                        if best.is_none_or(|best| here > best) {
                            best = Some(here);
                            lean.push(s);
                        }
                    }
                    (p, lean)
                })
                .collect();
        }
        let rank = |a: &State<'_>, b: &State<'_>| {
            cmp(score(a.0, a.1), score(b.0, b.1))
                .then(cmp(a.1, b.1))
                .then(cmp(a.0, b.0))
                .then_with(|| {
                    a.2.iter()
                        .map(|piece| piece.key.as_str())
                        .cmp(b.2.iter().map(|piece| piece.key.as_str()))
                })
        };
        states.sort_by(|a, b| cmp(b.0, a.0));
        let mut out: Vec<(f64, [&Wearable; 4])> = Vec::new();
        let mut run: Option<&State<'_>> = None;
        for (p, lst) in &states {
            // Python's `max`: the first of the greatest.
            let Some((first, rest)) = lst.split_first() else {
                continue;
            };
            let mut s = first;
            for other in rest {
                if rank(other, s) == Ordering::Greater {
                    s = other;
                }
            }
            if run.is_none_or(|run| rank(s, run) == Ordering::Greater) {
                run = Some(s);
                if let Ok(set) = <[&Wearable; 4]>::try_from(s.2.as_slice()) {
                    out.push((*p, set));
                }
            }
        }
        out.reverse();
        out
    }

    /// The script's `granted_trades`: `exchange_pairs` for a build at `stats` wearing `rings`,
    /// swinging each of `granted` (weapon index, infusion, two-handed) against `sl`'s bracket
    /// counter-hits.
    ///
    /// A launcher, or a weapon with no first strike, is left out.
    fn granted_trades(
        &self,
        sl: u32,
        stats: &Stats,
        rings: &[usize],
        granted: &[(usize, Infusion, bool)],
    ) -> Option<Exchange<'_>> {
        let eff = self.gear_stats(stats, rings);
        let stone = rings.iter().any(|&ring| self.rings[ring].1 == "Stone Ring");
        let mut mine = Vec::new();
        for &(index, infusion, two) in granted {
            let weapon = &self.weapons[index];
            if weapon.ranged.is_some() {
                continue;
            }
            let Some(strike) = &weapon.strike[usize::from(two)] else {
                continue;
            };
            let mut strike = strike.clone();
            if stone {
                strike.pd += STONE_RING_POISE;
            }
            let ar = self.attack_rating(weapon.infusion(infusion), &eff, rings);
            mine.push((strike, ar));
        }
        let counters = self
            .bracket_builds(sl_bracket(sl))
            .into_iter()
            .filter_map(|build| build.trade.as_ref())
            .collect();
        exchange_pairs(&mine, counters)
    }

    /// The script's `armor_trade`: `pieces` on a build at levelled `stats` wearing `rings`, over
    /// `table`.
    fn armor_trade(
        &self,
        table: &Exchange<'_>,
        pieces: &[&Wearable; 4],
        stats: &Stats,
        rings: &[usize],
    ) -> Numbers {
        let poise = self.armor_poise(pieces, rings, Some(&self.gear_stats(stats, rings)));
        let defense = self.build_defense(stats, pieces, rings);
        let (value, rate) = exchange_value(table, poise, &defense);
        let k = bisect_left(&table.keys, poise);
        Numbers {
            poise: py_round(poise, 1),
            poise_target: if k > 0 {
                py_round(table.keys[k - 1], 1)
            } else {
                0.0
            },
            trade_rate: py_round(rate, 3),
            exchange: py_round(value, 2),
        }
    }

    /// The poise half of the script's `generate_armor`: `best_armor`'s set `pieces`, or the set
    /// of a poise step that trades better.
    ///
    /// Each candidate is valued by its exchange over `granted`'s weapons against `sl`'s bracket;
    /// the highest wins, a tie keeping `pieces`. `pieces` unchanged and no numbers when there is
    /// nothing to trade.
    pub(super) fn trade_armor<'s>(
        &'s self,
        pieces: [&'s Wearable; 4],
        wearer: &Wearer<'_>,
        sl: u32,
        granted: &[(usize, Infusion, bool)],
    ) -> Traded<'s> {
        let (stats, rings) = (wearer.stats, wearer.rings);
        let Some(table) = self.granted_trades(sl, stats, rings, granted) else {
            return Traded {
                pieces,
                numbers: None,
                note: None,
            };
        };
        let previous = self.armor_trade(&table, &pieces, stats, rings);
        let (mut best, mut best_pieces) = (previous, pieces);
        for (_, set) in self.armor_poise_steps(wearer) {
            if keys(&set) == keys(&pieces) {
                continue;
            }
            let numbers = self.armor_trade(&table, &set, stats, rings);
            if numbers.exchange > best.exchange {
                best = numbers;
                best_pieces = set;
            }
        }
        let note = (keys(&best_pieces) != keys(&pieces)).then(|| {
            format!(
                "poise drove the pick: {:.0} poise over best_armor's {:.0} holds through counter \
                 hits up to {:.0} poise damage; exchange {} per trade against {}, lands {} of \
                 trades against {}",
                best.poise,
                previous.poise,
                best.poise_target,
                signed(best.exchange),
                signed(previous.exchange),
                percent(best.trade_rate),
                percent(previous.trade_rate),
            )
        });
        Traded {
            pieces: best_pieces,
            numbers: Some((best, previous)),
            note,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(t: f64, pd: f64, armor_break: i32, armor: &[(f64, f64)], rate: f64) -> Strike {
        Strike {
            hit: Hit {
                at: t,
                motion_value: 1.0,
                defense: 0,
                lower: 0.0,
                flat: [0.0; 5],
            },
            pd,
            armor_break,
            scale: 1.0,
            rate,
            armor: armor.to_vec(),
        }
    }

    /// The script's `trade_selftest_cases`, the trade half.
    #[test]
    fn trades_are_the_scripts() {
        let hammer = hit(0.8, 60.0, 0, &[(0.6, 1.0)], 0.3);
        let dagger = hit(0.3, 35.0, 0, &[], 0.0);
        let plain = hit(0.8, 0.0, 0, &[], 0.0);
        let one = |t, pd| hit(t, pd, 1, &[], 0.0);
        assert_eq!(trade(&plain, &dagger, 36.0, 0.0), (true, true));
        assert_eq!(trade(&plain, &dagger, 35.0, 0.0), (false, true));
        assert_eq!(
            trade(&plain, &hit(0.3, 1.0, 2, &[], 0.0), 1000.0, 0.0),
            (false, true)
        );
        assert_eq!(trade(&hammer, &one(0.7, 100.0), 31.0, 0.0), (true, true));
        assert_eq!(trade(&hammer, &one(0.7, 100.0), 30.0, 0.0), (false, true));
        assert_eq!(trade(&hammer, &one(0.5, 1.0), 1000.0, 0.0), (false, true));
        assert_eq!(trade(&dagger, &plain, 0.0, 30.0), (true, false));
        assert_eq!(trade(&dagger, &plain, 0.0, 40.0), (true, true));
        let early = hit(0.8, 60.0, 0, &[(0.2, 1.0)], 0.3);
        assert_eq!(trade(&dagger, &early, 0.0, 11.0), (true, true));
        assert_eq!(trade(&dagger, &hammer, 0.0, 11.0), (true, false));
        let tie = hit(0.5, 50.0, 0, &[], 0.0);
        assert_eq!(trade(&tie, &tie, 0.0, 10.0), (true, false));
    }

    /// The script's lack-of-stats cases: none when met, 30% + 50% per unit short, capped.
    #[test]
    fn lack_of_stats_is_the_scripts() {
        let mut stats = [0; 9];
        stats[STR] = 20;
        assert!(lack_of_stats(&[(STR, 20)], &stats).abs() < f64::EPSILON);
        stats[STR] = 10;
        assert!((lack_of_stats(&[(STR, 20)], &stats) - 0.55).abs() < 1e-12);
        stats[STR] = 0;
        assert!((lack_of_stats(&[(STR, 20), (DEX, 20)], &stats) - 0.8).abs() < 1e-12);
    }

    /// One weapon, one counter, by hand: mine 100 AR into 200 DEF, theirs 120 AR into 300 DEF
    /// with 40 poise damage landing first.
    #[test]
    fn the_exchange_is_the_scripts() {
        let counter = Counter {
            strike: hit(0.5, 40.0, 0, &[], 0.0),
            poise: 0.0,
            defense: [200.0, 0.0, 0.0, 0.0, 0.0, 200.0, 200.0, 200.0],
            ar: [Some(120.0), None, None, None, None],
        };
        let mine = [(
            hit(0.8, 0.0, 0, &[], 0.0),
            [Some(100.0), None, None, None, None],
        )];
        let table = exchange_pairs(&mine, vec![&counter]).expect("a table");
        let me = [300.0, 0.0, 0.0, 0.0, 0.0, 300.0, 300.0, 300.0];
        let (value, rate) = exchange_value(&table, 30.0, &me);
        assert!((value + 75.0).abs() < 1e-9 && rate.abs() < f64::EPSILON);
        let (value, rate) = exchange_value(&table, 41.0, &me);
        assert!((value - (800.0 / 12.0 - 75.0)).abs() < 1e-9);
        assert!((rate - 1.0).abs() < f64::EPSILON);
    }
}
