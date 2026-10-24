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
//!        x f_reach^e x f_stagger^e x f_trade^e x f_punish^e x f_stamina^e
//! ```
//!
//! times its family's weight; a family takes its best engagement, and the weapon's rate is
//! sum(s^2) / sum(s) over its families' positive rates.

use ds2_build_import_core::Infusion;

use super::status::STATUSES;
use super::trade::{Strike, count, strike};
use super::{
    CorpusBackend, Defense, Hit, Stats, Weapon, Worn, bad, float, py_max, py_min, py_round, py_sum,
    sl_bracket,
};
use crate::backend::{FamilyRate, MovesetMetrics};

/// The script's `MOVE_FAMILIES`: per family its engagements, each an opener or an opener and its
/// chained follow-up, as the attack slot suffixes after the grip.
const FAMILIES: [(&str, &[&[&str]]); 4] = [
    ("r1", &[&["Normal1st"], &["Normal1st", "Normal2nd"]]),
    ("r2", &[&["Strong1st"], &["Strong1st", "Strong2nd"]]),
    ("move", &[&["Rolling"], &["Dash"], &["Step"]]),
    ("jump", &[&["Jump"]]),
];

/// The script's `MOVE_ENTRY`: seconds of movement before a movement attack can start
/// (er-mods-rs's entry frames at 30 fps; DS2's own were not read).
fn entry(slot: &str) -> f64 {
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

/// One attack of a weapon's moveset in one grip.
#[derive(Clone, Debug)]
pub(super) struct Move {
    /// Its slot suffix: `Normal1st`, `Rolling`, ...
    slot: String,
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
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a moveset attack before any weapon"))?
                    .moves[grip]
                    .push(Move {
                        slot,
                        startup,
                        commit,
                        reach,
                        stamina,
                        hits,
                        strike,
                    });
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
        let moves = &weapon.moves[usize::from(two)];
        let find = |slot: &str| moves.iter().find(|m| m.slot == slot);
        let empty = MoveBracket::default();
        let bracket = self.move_brackets.get(sl_bracket(sl)).unwrap_or(&empty);
        let zero = [0.0];
        let poises: &[f64] = if bracket.poises.is_empty() {
            &zero
        } else {
            &bracket.poises
        };
        let counters: Vec<_> = self
            .bracket_builds(sl_bracket(sl))
            .into_iter()
            .filter_map(|build| build.trade.as_ref())
            .collect();
        let ar = self.attack_rating(weapon.infusion(infusion), eff, rings);
        let bar = self.max_stamina(eff, rings);
        let regen = self.stamina_regen;
        let mut status = Vec::new();
        for at in 0..STATUSES.len() {
            let per = self.buildup(weapon, infusion, eff, at);
            if per > 0.0
                && let Some(proc) = self.procs[at]
            {
                status.push((proc, per, self.defender_cuts(sl, worn, at)));
            }
        }
        let w = &self.moveset_weights;
        let mut families = Vec::new();
        for (fam, (family, engagements)) in FAMILIES.iter().enumerate() {
            let mut best: Option<FamilyRate> = None;
            for engagement in *engagements {
                let Some(atts) = engagement
                    .iter()
                    .map(|&slot| find(slot))
                    .collect::<Option<Vec<&Move>>>()
                else {
                    continue;
                };
                let opener = atts[0].strike.as_ref();
                let startup = atts[0].startup;
                let stagger = opener.map_or(0.0, |strike| {
                    let n = poises.iter().filter(|&&p| strike.staggers(p)).count();
                    count(n) / count(poises.len())
                });
                let follow = stagger + (1.0 - stagger) * FOLLOW_UNSTAGGERED;
                let (mut dmg, mut hits, mut landed) = (0.0, 0_u32, 0.0);
                let mut commit = entry(engagement[0]);
                let mut cost = Some(0.0);
                for (i, attack) in atts.iter().enumerate() {
                    let p = if i == 0 { 1.0 } else { follow };
                    for &(n, hit) in &attack.hits {
                        hits += n;
                        landed += p * f64::from(n);
                        dmg += p
                            * f64::from(n)
                            * Self::hit_damage(&ar, defense, &hit)
                            * weapon.damage_scale;
                    }
                    commit += attack.commit;
                    cost = cost.zip(attack.stamina).map(|(total, c)| total + c);
                }
                let procs = py_sum(status.iter().map(|(proc, per, cuts)| {
                    super::status::status_metrics(Some(*proc), *per, cuts, hits, commit).1
                }));
                let own = landed * weapon.self_hit;
                let reach = atts[0].reach;
                let f_reach = match (reach, bracket.reach) {
                    (Some(reach), Some(median)) if reach != 0.0 && median != 0.0 => py_min(
                        REACH_CLAMP.1,
                        py_max(REACH_CLAMP.0, (reach / median).powf(REACH_EXP)),
                    ),
                    _ => 1.0,
                };
                let f_stagger = 1.0 + STAGGER_BONUS * stagger;
                let f_trade = match opener {
                    Some(strike) if !counters.is_empty() => {
                        let (won, lost) = strike.trades(&counters, bracket.poise);
                        1.0 + TRADE_BONUS * f64::from(won - lost) / count(counters.len())
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
                if let (Some(cost), Some(bar), Some(regen)) = (cost, bar, regen)
                    && cost != 0.0
                    && bar != 0.0
                {
                    f_stamina = py_min(
                        1.0,
                        (bar + regen * STAMINA_WINDOW) / (cost + regen * commit) * commit
                            / STAMINA_WINDOW,
                    );
                }
                let net = dmg + procs - own;
                let rate = w.families[fam] * net / commit
                    * (f_reach.powf(w.reach)
                        * f_stagger.powf(w.stagger)
                        * f_trade.powf(w.trade)
                        * f_punish.powf(w.punish)
                        * f_stamina.powf(w.stamina));
                // The script compares the rate against the best's rounded one.
                if best.as_ref().is_none_or(|best| rate > best.rate) {
                    best = Some(FamilyRate {
                        family: (*family).to_owned(),
                        engagement: engagement.join("+"),
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
                    });
                }
            }
            families.extend(best);
        }
        let s: Vec<f64> = families
            .iter()
            .map(|f| f.rate)
            .filter(|&rate| rate > 0.0)
            .collect();
        if s.is_empty() {
            return None;
        }
        Some(MovesetMetrics {
            moveset: py_round(
                py_sum(s.iter().map(|x| x * x)) / py_sum(s.iter().copied()),
                2,
            ),
            families,
        })
    }
}
