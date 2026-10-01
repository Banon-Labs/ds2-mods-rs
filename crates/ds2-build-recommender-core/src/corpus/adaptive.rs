//! The defender who answers the attack: the script's `AdaptiveDefense`, `counter_rings`,
//! `respond`, `_least_useful` and `with_defense`, over the `RD` and `DB` records and the rings the
//! corpus builds wear.
//!
//! Per defender build, a counter ring c replaces a worn ring of its own upgrade group; else it
//! goes into an empty slot or in place of a ring that changes no defense; else (all four worn
//! rings change a defense) in place of the one that cuts this attack least. Only that last case
//! depends on the attack, so the rest is summed once per bracket here ([`Core`]). Every defense
//! change is a whole number, so the sums are exact in any order.

use std::borrow::Cow;

use super::{Ar, CorpusBackend, Defense, bad, float, py_sum};
use crate::model::Reply;

/// A defense change by damage type (an index into [`Ar`]) in displayed units, in the script's
/// key order.
pub(super) type Change = Vec<(usize, f64)>;

/// A defense buff the defender may take: the script's `data.defense_buffs` row.
#[derive(Clone, Debug)]
pub(super) struct Buff {
    name: String,
    /// A consumable anyone can use, rather than a spell.
    item: bool,
    add: [f64; 5],
}

/// What an adaptive defender is over one set of builds, before any attack: the script's
/// `AdaptiveDefense` `fixed`, `varying` and `n`.
#[derive(Clone, Debug, Default)]
pub(super) struct Core {
    /// Per counter ring, the summed defense change of swapping it in over the builds whose swap
    /// does not depend on the attack.
    fixed: Vec<[f64; 5]>,
    /// The worn rings of the builds whose four rings all change a defense.
    varying: Vec<Vec<usize>>,
    /// How many builds, at least one.
    n: usize,
}

/// `change`'s value for damage type `kind`, `0` when it has none.
fn get(change: &[(usize, f64)], kind: usize) -> f64 {
    change
        .iter()
        .find(|&&(at, _)| at == kind)
        .map_or(0.0, |&(_, value)| value)
}

/// The script's `with_defense`: `defense` plus a change by damage type, a physical one on every
/// physical type as well.
fn with_defense(defense: &Defense, add: &[f64; 5]) -> Defense {
    let mut out = *defense;
    for (kind, &value) in add.iter().enumerate() {
        if value == 0.0 {
            continue;
        }
        out[kind] += value;
        if kind == 0 {
            for typed in &mut out[5..] {
                *typed += value;
            }
        }
    }
    out
}

/// `k:v,k:v` as a [`Change`].
fn change(field: Option<&str>, line: usize) -> Result<Change, String> {
    match field {
        Some("-" | "") | None => Ok(Vec::new()),
        Some(list) => list
            .split(',')
            .map(|pair| {
                let (kind, value) = pair
                    .split_once(':')
                    .ok_or_else(|| bad(line, "type:value"))?;
                let kind = kind
                    .parse::<usize>()
                    .ok()
                    .filter(|&kind| kind < 5)
                    .ok_or_else(|| bad(line, "a damage type index"))?;
                Ok((kind, float(Some(value), line)?))
            })
            .collect(),
    }
}

impl CorpusBackend {
    /// Read an `RD` or `DB` record.
    pub(super) fn parse_adaptive<'a>(
        &mut self,
        tag: &str,
        line: usize,
        ring_index: &std::collections::HashMap<String, usize>,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        if tag == "RD" {
            let key = fields.next().ok_or_else(|| bad(line, "ring key"))?;
            let ring = *ring_index
                .get(key)
                .ok_or_else(|| bad(line, "a ring defense for a ring that is not a ring"))?;
            let change = change(fields.next(), line)?;
            self.ring_defense.push((ring, change));
        } else {
            let name = fields
                .next()
                .ok_or_else(|| bad(line, "buff name"))?
                .to_owned();
            let item = match fields.next() {
                Some("item") => true,
                Some("spell") => false,
                _ => return Err(bad(line, "a buff is a spell or an item")),
            };
            float(fields.next(), line)?;
            let mut add = [0.0; 5];
            for (kind, value) in change(fields.next(), line)? {
                add[kind] += value;
            }
            self.buffs.push(Buff { name, item, add });
        }
        Ok(())
    }

    /// The defense changes `ring` makes, when it makes any.
    fn ring_change(&self, ring: usize) -> Option<&Change> {
        self.ring_defense
            .iter()
            .find(|(at, _)| *at == ring)
            .map(|(_, change)| change)
    }

    /// The script's `counter_rings`: per ring upgrade group the last in `RD` order whose changes
    /// are all raises, never a no-use ring. Ring of Steel Protection+2, the four Quartz Rings+3
    /// and Dispelling Ring+1.
    pub(super) fn counter_rings(&self) -> Vec<usize> {
        let mut by_group: Vec<(&str, usize)> = Vec::new();
        for (ring, change) in &self.ring_defense {
            let raises = change
                .iter()
                .map(|&(_, value)| value)
                .fold(f64::INFINITY, f64::min)
                > 0.0;
            if self.no_use.contains(ring) || change.is_empty() || !raises {
                continue;
            }
            let group = self.ring_groups[*ring].as_str();
            match by_group.iter_mut().find(|(seen, _)| *seen == group) {
                Some(entry) => entry.1 = *ring,
                None => by_group.push((group, *ring)),
            }
        }
        by_group.into_iter().map(|(_, ring)| ring).collect()
    }

    /// An adaptive defender's [`Core`] over `builds`' worn rings (indices into the rings).
    pub(super) fn adaptive_core<'b>(&self, builds: impl IntoIterator<Item = &'b [usize]>) -> Core {
        let mut fixed = vec![[0.0; 5]; self.counters.len()];
        let (mut varying, mut n) = (Vec::new(), 0);
        for rings in builds {
            n += 1;
            let worn: Vec<(&str, Option<&Change>)> = rings
                .iter()
                .map(|&ring| (self.ring_groups[ring].as_str(), self.ring_change(ring)))
                .collect();
            if worn.len() >= 4 && worn.iter().all(|(_, change)| change.is_some()) {
                varying.push(rings.to_vec());
                continue;
            }
            for (sum, &counter) in fixed.iter_mut().zip(&self.counters) {
                let group = self.ring_groups[counter].as_str();
                let add = self.ring_change(counter).map_or(&[][..], Vec::as_slice);
                let same = worn
                    .iter()
                    .find(|(worn_group, _)| *worn_group == group)
                    .and_then(|(_, change)| *change);
                for (kind, total) in sum.iter_mut().enumerate() {
                    let value = get(add, kind) - same.map_or(0.0, |same| get(same, kind));
                    *total += value;
                }
            }
        }
        Core {
            fixed,
            varying,
            n: n.max(1),
        }
    }

    /// The script's `_least_useful`: of `worn`'s defense changes, the one that cuts an attack
    /// weighted `weight` least -- damage is linear in each type's defense -- the first on a tie.
    fn least_useful<'c>(worn: &[&'c Change], weight: &[f64; 5]) -> &'c Change {
        let mut best: Option<(f64, &Change)> = None;
        for &change in worn {
            let cut = py_sum(change.iter().map(|&(kind, value)| weight[kind] * value));
            if best.is_none_or(|(least, _)| cut < least) {
                best = Some((cut, change));
            }
        }
        best.map_or(&EMPTY_CHANGE, |(_, change)| change)
    }

    /// The script's `AdaptiveDefense._delta`: the mean defense change of swapping in counter ring
    /// number `at` over every build of `core`.
    fn delta(&self, core: &Core, at: usize, weight: &[f64; 5]) -> [f64; 5] {
        let counter = self.counters[at];
        let group = self.ring_groups[counter].as_str();
        let add = self.ring_change(counter).map_or(&[][..], Vec::as_slice);
        let mut total = core.fixed.get(at).copied().unwrap_or([0.0; 5]);
        for rings in &core.varying {
            let worn: Vec<&Change> = rings
                .iter()
                .filter_map(|&ring| self.ring_change(ring))
                .collect();
            let same = rings
                .iter()
                .position(|&ring| self.ring_groups[ring] == group)
                .and_then(|place| self.ring_change(rings[place]));
            let out = same.unwrap_or_else(|| Self::least_useful(&worn, weight));
            for (kind, sum) in total.iter_mut().enumerate() {
                *sum = *sum + get(add, kind) - get(out, kind);
            }
        }
        let n = core.n as f64;
        total.map(|value| if value == 0.0 { 0.0 } else { value / n })
    }
}

/// No defense change.
static EMPTY_CHANGE: Change = Vec::new();

/// A defender who answers each attack with its counter ring (and buff).
#[derive(Clone, Debug)]
pub(super) struct Adapt<'a> {
    pub(super) backend: &'a CorpusBackend,
    pub(super) core: Cow<'a, Core>,
    pub(super) reply: Reply,
}

impl Adapt<'_> {
    /// The script's `AdaptiveDefense.respond`: the defense against `ar` from `base`, and what the
    /// defender put on for it -- the counter ring that leaves the least damage when one leaves
    /// less than no swap, then the buff that does the same. Ties keep the first.
    pub(super) fn respond(&self, base: &Defense, ar: &Ar) -> (Defense, Vec<String>) {
        let backend = self.backend;
        let damage = |defense: &Defense| CorpusBackend::damage(ar, defense);
        let mut weight = [0.0; 5];
        weight[0] = if ar[0].unwrap_or(0.0) * 10.0 > base[0] {
            1.0 / 12.0
        } else {
            0.0
        };
        for (kind, value) in weight.iter_mut().enumerate().skip(1) {
            *value = ar[kind].unwrap_or(0.0) / 1000.0;
        }
        let (mut best, mut used) = (*base, Vec::new());
        for (at, &counter) in backend.counters.iter().enumerate() {
            let candidate = with_defense(base, &backend.delta(&self.core, at, &weight));
            if damage(&candidate) < damage(&best) - 1e-9 {
                best = candidate;
                used = vec![backend.rings[counter].1.clone()];
            }
        }
        let mut pick: Option<(Defense, &str)> = None;
        for buff in &backend.buffs {
            let allowed = match self.reply {
                Reply::RingAndAnyBuff => true,
                Reply::RingAndItem => buff.item,
                Reply::Static | Reply::Ring => false,
            };
            if !allowed {
                continue;
            }
            let candidate = with_defense(&best, &buff.add);
            let against = pick.map_or(best, |(defense, _)| defense);
            if damage(&candidate) < damage(&against) - 1e-9 {
                pick = Some((candidate, &buff.name));
            }
        }
        if let Some((defense, name)) = pick {
            best = defense;
            used.push(name.to_owned());
        }
        (best, used)
    }
}
