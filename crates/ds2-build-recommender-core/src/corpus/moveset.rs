//! The whole moveset: the script's `moveset_metrics`, `moveset_bracket` and `moveset_exponents`,
//! over the `MV` records (each weapon's attacks per grip with a live hit: their startup and commit
//! seconds, reach, stamina cost, hits and first strike), the `SH` records (what a landed hit costs
//! its wielder), the `MW` record (the fitted exponents and family weights), the `SR` record
//! (stamina regained a second) and the `MB` records (per SL bracket, what an attack is weighed
//! against).
//!
//! er-mods-rs's `slot_score` and family law ported to what DS2's files hold. Per engagement -- an
//! opener, or an opener and its chained follow-up --
//!
//! ```text
//! rate = (damage + status - self) / (entry + commit)
//!        x f_reach^e x f_stagger^e x meet x f_stamina^e
//! meet = (1 - REACT_SHARE) x f_trade^e x f_punish^e + REACT_SHARE x caught
//! ```
//!
//! `caught` is the `MC` record's: the share of the bracket's reacting rolls the opener still hits.
//!
//! times its family's weight; a family takes its best engagement, and the weapon's rate is
//! sum(s^2) / sum(s) over its families' positive rates.

use ds2_build_import_core::Infusion;

use super::status::{Proc, STATUSES};
use super::trade::{Counter, Strike, count, strike};
use super::{
    Ar, CorpusBackend, Defense, Hit, Stats, Weapon, Worn, bad, float, py_max, py_min, py_round,
    py_sum, sl_bracket,
};
use crate::backend::{FamilyRate, MovesetMetrics};

/// The script's `MOVE_FAMILIES`: per family its engagements, each an opener or an opener and its
/// chained follow-up, as the attack slot suffixes after the grip.
pub(super) const FAMILIES: [(&str, &[&[&str]]); 4] = [
    ("r1", &[&["Normal1st"], &["Normal1st", "Normal2nd"]]),
    ("r2", &[&["Strong1st"], &["Strong1st", "Strong2nd"]]),
    ("move", &[&["Rolling"], &["Dash"], &["Step"]]),
    ("jump", &[&["Jump"]]),
];

/// The script's `MOVE_ENTRY`: seconds of movement before a movement attack can start
/// (er-mods-rs's entry frames at 30 fps; DS2's own were not read).
pub(super) fn entry(slot: &str) -> f64 {
    match slot {
        "Rolling" | "Dash" => 20.0 / 30.0,
        "Step" => 14.0 / 30.0,
        "Jump" => 6.0 / 30.0,
        _ => 0.0,
    }
}

/// Seconds a stamina model looks ahead: the script's `MOVESET_STAMINA_WINDOW`.
const STAMINA_WINDOW: f64 = 10.0;
/// The chance a follow-up lands when the opener did not stagger: the script's
/// `FOLLOW_UNSTAGGERED`.
const FOLLOW_UNSTAGGERED: f64 = 0.5;
/// The script's `REACH_EXP` and `REACH_CLAMP`.
const REACH_EXP: f64 = 0.5;
const REACH_CLAMP: (f64, f64) = (0.6, 1.5);
/// The script's `STAGGER_BONUS`, `TRADE_BONUS` and `MOVESET_PUNISH`.
const STAGGER_BONUS: f64 = 0.5;
const TRADE_BONUS: f64 = 0.25;
const MOVESET_PUNISH: f64 = 0.25;
/// The share of openers thrown at a defender who waits and rolls on reaction: the script's
/// `REACT_SHARE` (er-mods-rs's, INFERRED).
const REACT_SHARE: f64 = 0.5;

/// One attack of a weapon's moveset in one grip.
#[derive(Clone, Debug)]
pub(super) struct Move {
    /// Its slot suffix: `Normal1st`, `Rolling`, ...
    pub(super) slot: String,
    /// Seconds from input to its first live hit, and to when the next action can start.
    startup: f64,
    commit: f64,
    /// The script's `attack_reach`.
    reach: Option<f64>,
    /// Menu stamina it costs: the script's `move_stamina`.
    stamina: Option<f64>,
    /// Its distinct live hits, each with its tick count. A hit's `at` is unused.
    hits: Vec<(u32, Hit)>,
    /// Its earliest live hit as the trade model reads it: the script's `strike_of`.
    strike: Option<Strike>,
    /// Per SL bracket, the share of the bracket's reacting rolls its live hits catch: the
    /// script's `rolls_caught` (`MC`); `None` where nobody rolls.
    caught: Vec<Option<f64>>,
}

/// The script's `moveset_exponents`: factor exponents, then family weights in [`FAMILIES`] order.
#[derive(Clone, Copy, Debug)]
pub(super) struct Weights {
    reach: f64,
    stagger: f64,
    trade: f64,
    punish: f64,
    stamina: f64,
    families: [f64; 4],
}

impl Default for Weights {
    /// Every exponent and weight 1: er-mods-rs's hand-set product, the script's without the file.
    fn default() -> Self {
        Self {
            reach: 1.0,
            stagger: 1.0,
            trade: 1.0,
            punish: 1.0,
            stamina: 1.0,
            families: [1.0; 4],
        }
    }
}

/// What the script's `moveset_bracket` weighs an attack against at one SL bracket.
#[derive(Clone, Debug, Default)]
pub(super) struct MoveBracket {
    /// The median R1 reach of the bracket's melee weapons.
    reach: Option<f64>,
    /// The mean of `poises`: the attacker's own poise in a trade.
    poise: f64,
    /// Every bracket build's poise.
    poises: Vec<f64>,
    /// Every bracket build's R1 recovery, sorted.
    recovery: Vec<f64>,
}

/// The physical type a `p`, `s`, `k` or `t` code reads, as an index into [`Defense`].
fn defense_of(code: &str, line: usize) -> Result<usize, String> {
    match code {
        "p" => Ok(0),
        "s" => Ok(5),
        "k" => Ok(6),
        "t" => Ok(7),
        _ => Err(bad(line, "hit type")),
    }
}

/// An optional float: `-` for none.
fn opt(field: Option<&str>, line: usize) -> Result<Option<f64>, String> {
    match field {
        Some("-") => Ok(None),
        text => Ok(Some(float(text, line)?)),
    }
}

/// Comma-separated floats, `-` for none.
fn list(field: Option<&str>, line: usize) -> Result<Vec<f64>, String> {
    match field {
        Some("-") | Some("") | None => Ok(Vec::new()),
        Some(text) => text.split(',').map(|v| float(Some(v), line)).collect(),
    }
}

impl Move {
    /// An `MV` record after its grip, or an `SM` record: `slot startup commit reach stamina hits
    /// strike`.
    fn parse<'a>(line: usize, fields: &mut impl Iterator<Item = &'a str>) -> Result<Self, String> {
        let slot = fields.next().ok_or_else(|| bad(line, "slot"))?.to_owned();
        let startup = float(fields.next(), line)?;
        let commit = float(fields.next(), line)?;
        let reach = opt(fields.next(), line)?;
        let stamina = opt(fields.next(), line)?;
        let hits = fields
            .next()
            .unwrap_or("")
            .split(' ')
            .filter(|hit| !hit.is_empty())
            .map(|hit| move_hit(hit, line))
            .collect::<Result<Vec<_>, _>>()?;
        let mut rest = fields.peekable();
        let strike = if rest.peek() == Some(&"-") {
            None
        } else {
            Some(strike(&mut rest, line)?)
        };
        Ok(Self {
            slot,
            startup,
            commit,
            reach,
            stamina,
            hits,
            strike,
            caught: Vec::new(),
        })
    }
}

/// One `n:mv:type:lower:flat` hit.
fn move_hit(text: &str, line: usize) -> Result<(u32, Hit), String> {
    let parts: Vec<&str> = text.split(':').collect();
    let [n, rate, kind, lower, flat] = parts[..] else {
        return Err(bad(line, "a moveset hit is five fields"));
    };
    let n = n.parse::<u32>().map_err(|_| bad(line, "a hit's ticks"))?;
    let mut out = [0.0; 5];
    if flat != "-" {
        let mut values = flat.split(',');
        for slot in &mut out {
            *slot = float(values.next(), line)?;
        }
    }
    Ok((
        n,
        Hit {
            at: 0.0,
            motion_value: float(Some(rate), line)?,
            defense: defense_of(kind, line)?,
            lower: float(Some(lower), line)?,
            flat: out,
        },
    ))
}

impl CorpusBackend {
    /// Read an `MV`, `SH`, `MW`, `SR` or `MB` record.
    pub(super) fn parse_moveset<'a>(
        &mut self,
        tag: &str,
        line: usize,
        fields: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        match tag {
            "MV" => {
                let grip = match fields.next() {
                    Some("1") => 0,
                    Some("2") => 1,
                    _ => return Err(bad(line, "grip is 1 or 2")),
                };
                let attack = Move::parse(line, fields)?;
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a moveset attack before any weapon"))?
                    .moves[grip]
                    .push(attack);
            }
            "SM" => {
                let attack = Move::parse(line, fields)?;
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a stance attack before any weapon"))?
                    .stance_moves
                    .push(attack);
            }
            "ST" => {
                let category = fields
                    .next()
                    .and_then(|text| text.parse::<u32>().ok())
                    .ok_or_else(|| bad(line, "power stance category"))?;
                let left = float(fields.next(), line)?;
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a stance before any weapon"))?
                    .stance = Some((category, left));
            }
            "MC" => {
                let caught = fields
                    .map(|field| opt(Some(field), line))
                    .collect::<Result<Vec<_>, _>>()?;
                let weapon = self
                    .weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a roll catch before any weapon"))?;
                // It follows its MV or SM record: every 1H record comes before the 2H ones, and
                // every SM record after both.
                let [one, two] = &mut weapon.moves;
                let attack = if weapon
                    .stance_moves
                    .last()
                    .is_some_and(|m| m.caught.is_empty())
                {
                    weapon.stance_moves.last_mut()
                } else if two.last().is_some_and(|m| m.caught.is_empty()) {
                    two.last_mut()
                } else {
                    one.last_mut().filter(|m| m.caught.is_empty())
                };
                attack
                    .ok_or_else(|| bad(line, "a roll catch with no attack before it"))?
                    .caught = caught;
            }
            "SH" => {
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "self-damage before any weapon"))?
                    .self_hit = float(fields.next(), line)?;
            }
            "MW" => {
                let mut values = [0.0; 9];
                for value in &mut values {
                    *value = float(fields.next(), line)?;
                }
                let [reach, stagger, trade, punish, stamina, r1, r2, moves, jump] = values;
                self.moveset_weights = Weights {
                    reach,
                    stagger,
                    trade,
                    punish,
                    stamina,
                    families: [r1, r2, moves, jump],
                };
            }
            "SR" => self.stamina_regen = Some(float(fields.next(), line)?),
            _ => {
                let at = fields
                    .next()
                    .and_then(|at| at.parse::<usize>().ok())
                    .ok_or_else(|| bad(line, "bracket"))?;
                let bracket = MoveBracket {
                    reach: opt(fields.next(), line)?,
                    poise: float(fields.next(), line)?,
                    poises: list(fields.next(), line)?,
                    recovery: list(fields.next(), line)?,
                };
                if self.move_brackets.len() <= at {
                    self.move_brackets.resize(at + 1, MoveBracket::default());
                }
                self.move_brackets[at] = bracket;
            }
        }
        Ok(())
    }

    /// The script's `moveset_metrics`: the `PvP` rate of `weapon`'s whole moveset in the grip at
    /// effective stats `eff` with `rings`, against `defense` and `sl`'s bracket. `None` when the
    /// grip has no attack with a live hit, or no family rates above 0.
    // DEBT: ds2-mods-rs-59p7 -- the script's own argument list; bundle the row's question.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn moveset_metrics(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        two: bool,
        eff: &Stats,
        rings: &[usize],
        sl: u32,
        defense: &Defense,
        worn: Worn,
    ) -> Option<MovesetMetrics> {
        let scorer = Scorer::new(self, eff, rings, sl, defense, worn);
        let hand = scorer.hand(weapon, infusion, None);
        let hands = [&hand];
        let moves = &weapon.moves[usize::from(two)];
        let find = |slot: &str| moves.iter().find(|m| m.slot == slot);
        let mut families = Vec::new();
        for (fam, (_, engagements)) in FAMILIES.iter().enumerate() {
            let mut best: Option<FamilyRate> = None;
            for engagement in *engagements {
                let Some(steps) = engagement
                    .iter()
                    .map(|&slot| find(slot).map(|attack| (attack, &hands[..])))
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let (row, rate) =
                    scorer.engagement(fam, &steps, entry(engagement[0]), engagement.join("+"));
                // The script compares the rate against the best's rounded one.
                if best.as_ref().is_none_or(|best| rate > best.rate) {
                    best = Some(row);
                }
            }
            families.extend(best);
        }
        let moveset = matching_law(families.iter())?;
        Some(MovesetMetrics { moveset, families })
    }
}

/// One hand's weapon as an engagement step reads it: the script's `MoveScorer.hand`.
pub(super) struct Hand<'a> {
    pub(super) weapon: &'a Weapon,
    infusion: Infusion,
    ar: Ar,
    /// `WeaponParam.damageScale`, times `leftDamageScale` for a weapon in the left hand.
    scale: f64,
    /// Per status the weapon builds up: its proc, build-up per hit and the defenders' cuts.
    status: Vec<(Proc, f64, Vec<f64>)>,
}

/// The bracket of an SL with no `MB` record.
static NO_BRACKET: MoveBracket = MoveBracket {
    reach: None,
    poise: 0.0,
    poises: Vec::new(),
    recovery: Vec::new(),
};

/// What every engagement at one build's stats is weighed against: the script's `MoveScorer`.
pub(super) struct Scorer<'a> {
    backend: &'a CorpusBackend,
    eff: &'a Stats,
    rings: &'a [usize],
    sl: u32,
    defense: &'a Defense,
    worn: Worn,
    bracket: &'a MoveBracket,
    poises: Vec<f64>,
    counters: Vec<&'a Counter>,
    bar: Option<f64>,
}

impl<'a> Scorer<'a> {
    pub(super) fn new(
        backend: &'a CorpusBackend,
        eff: &'a Stats,
        rings: &'a [usize],
        sl: u32,
        defense: &'a Defense,
        worn: Worn,
    ) -> Self {
        let bracket = backend
            .move_brackets
            .get(sl_bracket(sl))
            .unwrap_or(&NO_BRACKET);
        let poises = if bracket.poises.is_empty() {
            vec![0.0]
        } else {
            bracket.poises.clone()
        };
        Self {
            backend,
            eff,
            rings,
            sl,
            defense,
            worn,
            bracket,
            poises,
            counters: backend
                .bracket_builds(sl_bracket(sl))
                .into_iter()
                .filter_map(|build| build.trade.as_ref())
                .collect(),
            bar: backend.max_stamina(eff, rings),
        }
    }

    /// `weapon` with `infusion` in a hand: `left` its `leftDamageScale` in the left hand, `None`
    /// in the right.
    pub(super) fn hand(
        &self,
        weapon: &'a Weapon,
        infusion: Infusion,
        left: Option<f64>,
    ) -> Hand<'a> {
        let backend = self.backend;
        let mut status = Vec::new();
        for at in 0..STATUSES.len() {
            let per = backend.buildup(weapon, infusion, self.eff, at);
            if per > 0.0
                && let Some(proc) = backend.procs[at]
            {
                status.push((proc, per, backend.defender_cuts(self.sl, self.worn, at)));
            }
        }
        Hand {
            weapon,
            infusion,
            ar: backend.attack_rating(weapon.infusion(infusion), self.eff, self.rings),
            scale: left.map_or(weapon.damage_scale, |side| weapon.damage_scale * side),
            status,
        }
    }

    /// One engagement's row and its unrounded rate: the script's `MoveScorer.engagement`.
    /// `steps` are its attacks in order, each with the hands its live hits strike with (two for
    /// a power stance L1/L2); `family` indexes [`FAMILIES`]; `entry` is the seconds before the
    /// opener can start. The opener's strike, reach and roll catch are its own row's.
    pub(super) fn engagement(
        &self,
        family: usize,
        steps: &[(&Move, &[&Hand<'_>])],
        entry: f64,
        label: String,
    ) -> (FamilyRate, f64) {
        let (bracket, poises) = (self.bracket, &self.poises);
        let opener_move = steps[0].0;
        let opener = opener_move.strike.as_ref();
        let startup = opener_move.startup;
        let stagger = opener.map_or(0.0, |strike| {
            let n = poises.iter().filter(|&&p| strike.staggers(p)).count();
            count(n) / count(poises.len())
        });
        let follow = stagger + (1.0 - stagger) * FOLLOW_UNSTAGGERED;
        let mut dmg = 0.0;
        // Per weapon and infusion struck with, in the order it first struck: its hand, hits and
        // landed hits (the script's Counters, keyed by weapon and infusion).
        let mut struck: Vec<(&Hand<'_>, u32, f64)> = Vec::new();
        let mut commit = entry;
        let mut cost = Some(0.0);
        for (i, &(attack, hands)) in steps.iter().enumerate() {
            let p = if i == 0 { 1.0 } else { follow };
            for &(n, hit) in &attack.hits {
                for &hand in hands {
                    match struck.iter_mut().find(|(seen, ..)| {
                        seen.weapon.key == hand.weapon.key && seen.infusion == hand.infusion
                    }) {
                        Some(seen) => {
                            seen.1 += n;
                            seen.2 += p * f64::from(n);
                        }
                        None => struck.push((hand, n, p * f64::from(n))),
                    }
                    dmg += p
                        * f64::from(n)
                        * CorpusBackend::hit_damage(&hand.ar, self.defense, &hit)
                        * hand.scale;
                }
            }
            commit += attack.commit;
            cost = cost.zip(attack.stamina).map(|(total, c)| total + c);
        }
        let procs = py_sum(struck.iter().flat_map(|&(hand, hits, _)| {
            hand.status.iter().map(move |(proc, per, cuts)| {
                super::status::status_metrics(Some(*proc), *per, cuts, hits, commit).1
            })
        }));
        let own = py_sum(
            struck
                .iter()
                .map(|&(hand, _, landed)| landed * hand.weapon.self_hit),
        );
        let reach = opener_move.reach;
        let f_reach = match (reach, bracket.reach) {
            (Some(reach), Some(median)) if reach != 0.0 && median != 0.0 => py_min(
                REACH_CLAMP.1,
                py_max(REACH_CLAMP.0, (reach / median).powf(REACH_EXP)),
            ),
            _ => 1.0,
        };
        let f_stagger = 1.0 + STAGGER_BONUS * stagger;
        let f_trade = match opener {
            Some(strike) if !self.counters.is_empty() => {
                let (won, lost) = strike.trades(&self.counters, bracket.poise);
                1.0 + TRADE_BONUS * f64::from(won - lost) / count(self.counters.len())
            }
            _ => 1.0,
        };
        let cover = if bracket.recovery.is_empty() {
            0.5
        } else {
            let below = bracket.recovery.partition_point(|&r| r < startup);
            count(bracket.recovery.len() - below) / count(bracket.recovery.len())
        };
        let f_punish = 1.0 + MOVESET_PUNISH * (2.0 * cover - 1.0);
        let mut f_stamina = 1.0;
        if let (Some(cost), Some(bar), Some(regen)) = (cost, self.bar, self.backend.stamina_regen)
            && cost != 0.0
            && bar != 0.0
        {
            f_stamina = py_min(
                1.0,
                (bar + regen * STAMINA_WINDOW) / (cost + regen * commit) * commit / STAMINA_WINDOW,
            );
        }
        let w = &self.backend.moveset_weights;
        let net = dmg + procs - own;
        // A committed defender meets the contest; a reacting one rolls and is caught at
        // `caught`. The two split the defenders rather than multiply.
        let contest = f_trade.powf(w.trade) * f_punish.powf(w.punish);
        let caught = opener_move
            .caught
            .get(sl_bracket(self.sl))
            .copied()
            .flatten();
        let meet = caught.map_or(contest, |caught| {
            (1.0 - REACT_SHARE) * contest + REACT_SHARE * caught
        });
        let rate = w.families[family] * net / commit
            * (f_reach.powf(w.reach)
                * f_stagger.powf(w.stagger)
                * meet
                * f_stamina.powf(w.stamina));
        let row = FamilyRate {
            family: FAMILIES[family].0.to_owned(),
            engagement: label,
            rate: py_round(rate, 2),
            raw: py_round(net / commit, 3),
            damage: py_round(dmg, 1),
            status: py_round(procs, 1),
            self_damage: py_round(own, 1),
            startup_s: py_round(startup, 3),
            commit_s: py_round(commit, 3),
            stamina: cost.map(|cost| py_round(cost, 1)),
            reach_m: reach,
            f_reach: py_round(f_reach, 3),
            f_stagger: py_round(f_stagger, 3),
            f_trade: py_round(f_trade, 3),
            f_punish: py_round(f_punish, 3),
            f_stamina: py_round(f_stamina, 3),
            caught: caught.map(|caught| py_round(caught, 3)),
        };
        (row, rate)
    }
}

/// The script's `matching_law`: sum(s^2) / sum(s) over the positive rates, rounded as it rounds;
/// `None` when no rate is positive.
pub(super) fn matching_law<'f>(rates: impl Iterator<Item = &'f FamilyRate>) -> Option<f64> {
    let s: Vec<f64> = rates.map(|f| f.rate).filter(|&rate| rate > 0.0).collect();
    (!s.is_empty()).then(|| {
        py_round(
            py_sum(s.iter().map(|x| x * x)) / py_sum(s.iter().copied()),
            2,
        )
    })
}
