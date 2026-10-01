//! The R1 chain as the script's `chain_timeline` repeats it, over the `CH` records its
//! `--export-backend` writes: each attack's chain step, mean play speed and live hits.
//!
//! The chain alternates its 1st and 2nd attack; each next attack starts at the attack's chain
//! step (TAE 111500), never before its own hitbox has closed, over its play speed. A hitbox that
//! re-hits adds a tick per interval.

use std::cmp::Ordering;

use super::{CorpusBackend, Hit, bad, float, py_max};

/// One live hitbox of a chain attack.
#[derive(Clone, Debug)]
struct ChainHit {
    /// TAE 2200 window, in frames at 30 fps.
    start: f64,
    end: f64,
    /// Its motion value.
    rate: f64,
    /// Ticks, at least one.
    ticks: u32,
    /// Seconds between ticks.
    interval: f64,
    /// The physical type's place in the script's string order: physical, slash, standard, strike,
    /// thrust.
    kind: u8,
    lower: f64,
    flat: [f64; 5],
}

/// One attack of a chain.
#[derive(Clone, Debug)]
struct Attack {
    /// The script's `chain_open`: seconds of animation time until the next attack can start;
    /// `None` without the TAE.
    step: Option<f64>,
    /// The mean of its start and end play speeds.
    speed: f64,
    hits: Vec<ChainHit>,
}

/// An R1 chain: its 1st attack and, when it has one, its 2nd.
#[derive(Clone, Debug)]
pub(super) struct Chain {
    first: Attack,
    second: Option<Attack>,
}

/// One hit a repeated chain lands: the script's `chain_timeline` tuple.
#[derive(Clone, Copy, Debug)]
pub(super) struct Landed {
    /// Seconds from the input.
    pub(super) at: f64,
    mv: f64,
    kind: u8,
    lower: f64,
    /// The second its attack began.
    pub(super) start: f64,
    flat: [f64; 5],
}

impl Landed {
    /// The hit as `hit_damage` reads it.
    pub(super) fn hit(&self) -> Hit {
        Hit {
            at: self.at,
            motion_value: self.mv,
            // Physical and standard read general physical defense; slash, strike and thrust their
            // own.
            defense: match self.kind {
                1 => 5,
                3 => 6,
                4 => 7,
                _ => 0,
            },
            lower: self.lower,
            flat: self.flat,
        }
    }
}

/// The type names in the script's string order, which its sort compares.
const KINDS: [&str; 5] = ["physical", "slash", "standard", "strike", "thrust"];

/// One `start:end:rate:n:interval:type:lower:flat` hit.
fn chain_hit(hit: &str, line: usize) -> Result<ChainHit, String> {
    let parts: Vec<&str> = hit.split(':').collect();
    let [start, end, rate, ticks, interval, kind, lower, flat] = parts[..] else {
        return Err(bad(line, "a chain hit is eight fields"));
    };
    let kind = KINDS
        .iter()
        .position(|&name| name == kind)
        .and_then(|at| u8::try_from(at).ok())
        .ok_or_else(|| bad(line, "a hit's physical type"))?;
    let ticks = int_ticks(float(Some(ticks), line)?);
    let mut out = [0.0; 5];
    if flat != "-" {
        let mut values = flat.split(',');
        for slot in &mut out {
            *slot = float(values.next(), line)?;
        }
    }
    Ok(ChainHit {
        start: float(Some(start), line)?,
        end: float(Some(end), line)?,
        rate: float(Some(rate), line)?,
        ticks,
        interval: float(Some(interval), line)?,
        kind,
        lower: float(Some(lower), line)?,
        flat: out,
    })
}

/// A tick count as the file writes it, at least one.
fn int_ticks(ticks: f64) -> u32 {
    let mut n = 1;
    while f64::from(n + 1) <= ticks {
        n += 1;
    }
    n
}

/// One attack's `step speed hits` fields.
fn attack(step: &str, speed: &str, hits: &str, line: usize) -> Result<Attack, String> {
    let step = match step {
        "-" => None,
        text => Some(float(Some(text), line)?),
    };
    let speed = float(Some(speed), line)?;
    let hits = match hits {
        "-" | "" => Vec::new(),
        list => list
            .split(' ')
            .filter(|hit| !hit.is_empty())
            .map(|hit| chain_hit(hit, line))
            .collect::<Result<_, _>>()?,
    };
    Ok(Attack { step, speed, hits })
}

impl CorpusBackend {
    /// Read a `CH` record onto the last weapon.
    pub(super) fn parse_chain<'a>(
        &mut self,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        let grip = match fields.next() {
            Some("1") => 0,
            Some("2") => 1,
            _ => return Err(bad(line, "grip is 1 or 2")),
        };
        let mut next = || {
            fields
                .next()
                .ok_or_else(|| bad(line, "a chain attack's fields"))
        };
        let first = attack(next()?, next()?, next()?, line)?;
        let (step, speed, hits) = (next()?, next()?, next()?);
        let second = if speed == "-" {
            None
        } else {
            Some(attack(step, speed, hits, line)?)
        };
        self.weapons
            .last_mut()
            .ok_or_else(|| bad(line, "a chain before any weapon"))?
            .chains[grip]
            .push(Chain { first, second });
        Ok(())
    }
}

/// Python's order for two numbers in a tuple: equal unless one is less.
fn by(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

impl Chain {
    /// The script's `chain_timeline` for this chain out to `horizon` seconds: every hit of the
    /// repeated chain whose attack began before it, sorted as the script sorts its tuples -- by
    /// time, motion value, type name, floor, then (`with_start`) the attack's start, then the flat
    /// attack.
    pub(super) fn timeline(&self, horizon: f64, with_start: bool) -> Vec<Landed> {
        let seq = [&self.first, self.second.as_ref().unwrap_or(&self.first)];
        let (mut out, mut t0, mut k) = (Vec::new(), 0.0_f64, 0_usize);
        while t0 < horizon {
            let attack = seq[k % 2];
            for hit in &attack.hits {
                for tick in 0..hit.ticks {
                    out.push(Landed {
                        at: t0 + (hit.start / 30.0 + f64::from(tick) * hit.interval) / attack.speed,
                        mv: hit.rate,
                        kind: hit.kind,
                        lower: hit.lower,
                        start: t0,
                        flat: hit.flat,
                    });
                }
            }
            let Some(step) = attack.step.filter(|&step| step > 0.0) else {
                break;
            };
            // Never chain before this attack's own hitbox has closed.
            let live = attack
                .hits
                .iter()
                .map(|hit| hit.end / 30.0)
                .reduce(py_max)
                .unwrap_or(0.0);
            t0 += py_max(step, live) / attack.speed;
            k += 1;
        }
        out.sort_by(|a, b| {
            by(a.at, b.at)
                .then(by(a.mv, b.mv))
                .then(a.kind.cmp(&b.kind))
                .then(by(a.lower, b.lower))
                .then(if with_start {
                    by(a.start, b.start)
                } else {
                    Ordering::Equal
                })
                .then_with(|| {
                    a.flat
                        .iter()
                        .zip(&b.flat)
                        .map(|(x, y)| by(*x, *y))
                        .find(|order| order.is_ne())
                        .unwrap_or(Ordering::Equal)
                })
        });
        out
    }
}
