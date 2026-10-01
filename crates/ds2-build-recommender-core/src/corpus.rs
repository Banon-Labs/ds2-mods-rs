//! The real backend: `scripts/ds2-builds-recommend.py`'s ranking, ported, over the data file its
//! `--export-backend` writes.
//!
//! [`CorpusBackend`] answers every [`RecommenderBackend`] question with the script's own answer for
//! the same input. It is a line-for-line port, not a re-derivation: the same iteration orders, the
//! same stable sorts, the same float operations in the same order (the data file carries every
//! float as Python's `repr`, which parses back to the same bits), so a tie breaks the same way on
//! both sides. `tests/python_parity.rs` holds it to answers the script wrote.
//!
//! What the file holds and what is computed here:
//!
//! * **Precomputed by the script**, because they depend only on the corpus: each SL bracket's floors
//!   and average defender, the infusion calibration, the rings common to all builds, and each
//!   weapon's R1 hit timeline, hyperarmor and counter multipliers, and its R1/R2 hits per attack
//!   and chain hit times for the bleed/poison ranking.
//! * **Computed here**, because they depend on the question: attack rating and damage for any
//!   stats, the ranking, the stat optimizer, the minimum-build search, and the nearest builds. A
//!   bow, greatbow or crossbow is scored by its best shot, its ammunition's attack added
//!   (`ranged`), from the launcher and ammunition rows the script reads from the regulation.
//!
//! The corpus is carried as what nearest-build search reads and nothing else: each build's SL
//! bracket, its nine stat brackets, its rings and its weapons with their infusions.

use std::path::Path;

use ds2_build_import_core::Infusion;

use crate::backend::{
    BestWeaponRow, Calibration, CatalystPick, Change, DefenderDefense, Fix, GeneratedBuild, Limits,
    OptimizedBuild, Outcome, RecommenderBackend, Refusal, RefusalKind, Requirement, ResultRow,
    SpellRow, WEAPONS_1H_TOP, WEAPONS_2H_ONLY_TOP, WeaponCard, Wield,
};
use crate::flex::{FLEX_K, Flexibility};
use crate::model::{
    BestWeaponsOpts, Defender, Grip, Objective, Reply, SL_MAX, STAT_COUNT, STAT_LABELS,
    StatusFilter, WeaponsForOpts,
};
use crate::weapons;

mod adaptive;
mod best;
mod chain;
mod poise;
mod ranged;

/// What the file is called beside `DarkSoulsII.exe`.
pub const DATA_FILE_NAME: &str = "ds2-build-recommender.dat";

/// The file's first line. A different one is a file this port does not read.
pub const FORMAT: &str = "ds2-build-recommender-data 16";

/// Nine stats as the script computes with them, in [`crate::model::STAT_LABELS`] order.
type Stats = [i32; STAT_COUNT];

const VIG: usize = 0;
const END: usize = 1;
const VIT: usize = 2;
const ATT: usize = 3;
const STR: usize = 4;
const DEX: usize = 5;
const ADP: usize = 6;
const INT: usize = 7;
const FTH: usize = 8;

/// The script's `FLOOR_STATS`, in its order: VIG, VIT, ADP, ATT.
const FLOOR_STATS: [usize; 4] = [VIG, VIT, ADP, ATT];

/// The script's `floor_stats`: the [`FLOOR_STATS`] a build casting `spells` (indices into
/// `table`) is held to, ATT only when it casts something other than
/// [`crate::backend::NO_FLOOR_SPELLS`]. Without spells ATT buys nothing but agility, which ADP
/// buys three times as fast; Warmth alone counts as nothing.
fn floored(spells: &[usize], table: &[Spell]) -> &'static [usize] {
    if !crate::backend::casts_for_floors(spells.iter().map(|&at| table[at].key.as_str())) {
        &FLOOR_STATS[..3]
    } else {
        &FLOOR_STATS
    }
}

/// The script's `FREE_STATS`, with the names its tie-breaks compare.
const FREE_STATS: [(usize, &str); 5] = [
    (VIG, "vigor"),
    (END, "endurance"),
    (VIT, "vitality"),
    (ATT, "attunement"),
    (ADP, "adaptability"),
];

/// The highest soul level of each SL bracket but the last, which is open.
const SL_BRACKET_TOPS: [u32; 8] = [20, 40, 70, 100, 125, 155, 200, 250];

/// How many SL brackets there are.
const SL_BRACKETS: usize = SL_BRACKET_TOPS.len() + 1;

/// The share of equip load a build may carry: 70%.
const EQUIP_CAP: f64 = 0.70;

/// Keys the script treats as an empty slot.
const EMPTY: [&str; 7] = [
    "",
    "Bare_Fists",
    "No_Spell",
    "No_Ring",
    "Naked",
    "No_Item",
    "None",
];

/// A weapon's other infusions stay in the ranking while within this share of its best.
const WITHIN: f64 = 0.10;

/// How many rows Weapons for Stats lists.
const WEAPONS_FOR_TOP: usize = 25;

/// How many rows Similar Builds lists.
const SIMILAR_TOP: usize = 15;

/// How many nearest builds a generated build's rings are counted over.
const GENERATE_K: usize = 50;

/// The R1 window a generated build's weapons are ranked over, in seconds.
const GENERATE_WINDOW: f64 = 1.5;

/// The optimizer's soft flexibility term: the script's `FLEX_WEIGHT`, whose comment carries the
/// sweep it was chosen from. A step of `n` points that lets the build wield `d` more weapons (one-
/// and two-handed each counted) gains `FLEX_WEIGHT * d / n` on top of its objective weight.
const FLEX_WEIGHT: f64 = 0.02;

/// The script's `LOAD_SCARCE_FROM` and `LOAD_SCARCE_FULL`: armour weight starts to count against
/// defense below the first number of weapons wielded one-handed, fully at the second. Measured by
/// the script's `--load-evidence` over the corpus: builds wielding fewer weapons carry less load.
const LOAD_SCARCE_FROM: i32 = 80;
const LOAD_SCARCE_FULL: i32 = 40;

/// The script's `LOAD_PRICE`: the fraction of the best set's defense per weight that a unit of
/// armour weight costs at full scarcity.
const LOAD_PRICE: f64 = 0.6;

/// The requirement stats, the only ones that change what a build wields: the script's
/// `REQ_STATS`.
const REQ_STATS: [usize; 4] = [STR, DEX, INT, FTH];

/// The rings `builds` wear, a build counted once per ring, most worn first and ties first seen
/// first: the script's `Counter(...).most_common()`.
fn ring_counts<'a>(builds: impl IntoIterator<Item = &'a CorpusBuild>) -> Vec<(usize, u32)> {
    let mut worn: Vec<(usize, u32)> = Vec::new();
    for build in builds {
        for (at, &ring) in build.rings.iter().enumerate() {
            if build.rings[..at].contains(&ring) {
                continue;
            }
            match worn.iter_mut().find(|(seen, _)| *seen == ring) {
                Some((_, count)) => *count += 1,
                None => worn.push((ring, 1)),
            }
        }
    }
    worn.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    worn
}

/// The index of `sl`'s bracket.
fn sl_bracket(sl: u32) -> usize {
    SL_BRACKET_TOPS
        .iter()
        .position(|&top| sl <= top)
        .unwrap_or(SL_BRACKETS - 1)
}

/// A stat's bracket: 0-10 is 0, then one per ten, 91-99 is 9.
fn stat_bracket(value: i32) -> i32 {
    if value <= 10 {
        0
    } else {
        (value - 1).div_euclid(10).min(9)
    }
}

/// Displayed AGL, as both planner sites compute it.
pub fn agility(adp: i32, att: i32) -> i32 {
    if adp >= 99 && att >= 99 {
        return 120;
    }
    let x = 3 * adp + att;
    let agl = if x <= 120 {
        80 + x.div_euclid(4)
    } else {
        110 + (x - 120).div_euclid(28)
    };
    agl.max(85)
}

/// The stats whose `additionalHp` max HP adds to VGR's `hpMax`: the script's `HP_STATS`.
const HP_STATS: [usize; 8] = [END, VIT, ATT, STR, DEX, INT, FTH, ADP];

/// The script's `norm`: lowercase, letters and digits only.
fn norm(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// Python's `max(a, b)`: `a` unless `b` is greater.
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// Python's `min(a, b)`: `a` unless `b` is smaller.
fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// A table, read as the script's `_tab` reads it: the index clamped into the table.
#[derive(Clone, Debug, Default)]
struct Table(Vec<f64>);

impl Table {
    fn at(&self, index: i32) -> f64 {
        let last = self.0.len().saturating_sub(1);
        let index = usize::try_from(index.max(0)).unwrap_or(0).min(last);
        self.0.get(index).copied().unwrap_or(0.0)
    }
}

/// The tables the script reads by name.
#[derive(Clone, Debug, Default)]
struct Tables {
    physical: Table,
    magic: Table,
    fire: Table,
    lightning: Table,
    dark: Table,
    aux: Table,
    mundane: Table,
    equip_load: Table,
    /// Attunement slots per ATT: the script's `att_slots`.
    attunement_slots: Table,
    /// Max stamina per END: the script's `stamina_max`, empty when it did not read the regulation.
    stamina_max: Table,
    /// Max HP's two columns, `hpMax` per VGR and `additionalHp` per other stat: the script's
    /// `hp_max` and `additional_hp`, empty when it did not read the regulation.
    hp_max: Table,
    additional_hp: Table,
    /// Per-stat cast-power bonus, magic, fire, lightning, dark: the script's `cast_bonus`.
    cast: [Table; 4],
    /// `physicalDEFBonus` then the magic, fire, lightning and dark `DEFBonus` tables: what
    /// `build_defense` reads for a chosen defender's stats.
    defense: [Table; 5],
}

/// A spell the file lists: the script's `data.spells` row with its `spell_req`.
#[derive(Clone, Debug)]
struct Spell {
    key: String,
    name: String,
    slots: i32,
    /// `SpellParam.spellCategory`, `-1` when the script did not read it.
    category: i32,
    require: Vec<(usize, i32)>,
}

/// A catalyst the file lists: the script's `data.catalysts` row.
#[derive(Clone, Debug)]
struct Catalyst {
    name: String,
    /// The spell categories it casts.
    categories: Vec<i32>,
    require: Vec<(usize, i32)>,
    /// Per element, magic, fire, lightning, dark: base and scale.
    power: [(f64, f64); 4],
}

/// The script's `SPELL_SCHOOLS`: per spell category, its label and the element (an index into
/// [`Catalyst::power`]) its cast power is read in.
const SPELL_SCHOOLS: [(&str, usize); 5] = [
    ("sorcery", 0),
    ("miracle", 2),
    ("pyromancy", 1),
    ("hex", 3),
    ("hex", 3),
];

/// A starting class.
#[derive(Clone, Debug)]
struct Class {
    /// soulsplanner's key, `sorcerer`: what a class restriction names.
    key: String,
    name: String,
    level: i32,
    base: Stats,
}

/// `atk` keys, in the file's order: physical, magic, fire, lightning, dark, bleed, poison.
const ATK_PHYSICAL: usize = 0;
const ATK_BLEED: usize = 5;
const ATK_POISON: usize = 6;

/// `atkScale` keys, in the file's order: strength, dexterity, magic, fire, lightning, dark, bleed,
/// poison, modifier.
const SCALE_STR: usize = 0;
const SCALE_DEX: usize = 1;
const SCALE_MAGIC: usize = 2;
const SCALE_BLEED: usize = 6;
const SCALE_POISON: usize = 7;
const SCALE_MODIFIER: usize = 8;

/// One infusion of a weapon: base attack and scaling.
#[derive(Clone, Debug)]
struct InfusionRow {
    infusion: Infusion,
    atk: [f64; 7],
    scale: [f64; 9],
    /// The moved rates, physical, magic, fire, lightning, dark: what a ring's flat attack add is
    /// multiplied by (the script's `ring_attack_add`); `0` where the regulation was not read.
    rate: [f64; 5],
}

/// One hit of an R1 chain.
#[derive(Clone, Copy, Debug)]
struct Hit {
    /// Seconds from the input.
    at: f64,
    motion_value: f64,
    /// The defense it is read against, as an index into [`Defense`]: general physical, or slash,
    /// strike or thrust.
    defense: usize,
    /// `PlayerDamageParam.damageLower`.
    lower: f64,
    /// The damage row's flat attack per type, in [`Ar`] order, which the attack builder adds to
    /// the weapon's before the defense: the script's `regulation_hit_flat`.
    flat: [f64; 5],
}

/// A weapon, with everything the ranking reads.
#[derive(Clone, Debug)]
struct Weapon {
    key: String,
    name: String,
    class: String,
    shield: bool,
    catalyst: bool,
    /// Its R1 costs at least the 90th percentile of stamina.
    high_stamina: bool,
    weight: f64,
    /// `(stat, value)`, in the planner's order.
    require: Vec<(usize, i32)>,
    /// The R1's hyperarmor multiplier one- and two-handed, `0` for none.
    hyperarmor: [f64; 2],
    /// The counter-hit multiplier, `0` for none.
    counter: f64,
    /// `WeaponParam.damageScale`, which every hit's damage is multiplied by after the defense:
    /// the script's `regulation_damage_scale`.
    damage_scale: f64,
    infusions: Vec<InfusionRow>,
    /// The R1 chain one- and two-handed, empty where there is no timing.
    timeline: [Vec<Hit>; 2],
    /// What the script's `status_hits` reads, by grip (1H, 2H) and chain (R1, R2): one entry per
    /// name it tries, in its order.
    status: [[Vec<StatusChain>; 2]; 2],
    /// A bow, greatbow or crossbow's shot: the script's `data.ranged` row. `None` for every other
    /// weapon.
    ranged: Option<ranged::Launcher>,
    /// The R1 chain per grip (1H, 2H): one per name the script's `chain_timeline` tries, in its
    /// order.
    chains: [Vec<chain::Chain>; 2],
    /// Per grip, the script's `r1_metrics` reach, startup, recovery and first-hit seconds, which
    /// no build changes.
    r1: [[Option<f64>; 4]; 2],
    /// Per grip, what the script's `poise_metrics` reads that no build changes.
    poise: [Option<poise::Poise>; 2],
}

/// One attack chain as the bleed/poison ranking counts it.
#[derive(Clone, Debug, Default)]
struct StatusChain {
    /// How many times one attack of it hits one target.
    per_attack: u32,
    /// Every hit of the repeated chain out to the script's `STATUS_HORIZON`, as `(second its attack
    /// began, second it lands)`.
    hits: Vec<(f64, f64)>,
}

/// The labels the script's `status_hits` gives a chain.
const STATUS_CHAINS: [&str; 2] = ["R1", "R2"];

impl Weapon {
    fn infusion(&self, infusion: Infusion) -> Option<&InfusionRow> {
        self.infusions.iter().find(|row| row.infusion == infusion)
    }

    /// The script's `weapon_ok` over these stats, with no armour.
    fn wieldable(&self, stats: &Stats, two_hand: bool) -> bool {
        self.require
            .iter()
            .all(|&(stat, value)| Self::meets(stats, stat, value, two_hand))
    }

    /// Whether `stats` meet one requirement of `value` in `stat`: strength's is halved
    /// two-handed.
    fn meets(stats: &Stats, stat: usize, value: i32, two_hand: bool) -> bool {
        let need = if two_hand && stat == STR {
            value.div_euclid(2)
        } else {
            value
        };
        need <= stats[stat]
    }
}

/// physical, magic, fire, lightning, dark, slash, strike, thrust.
type Defense = [f64; 8];

/// Who a hit lands on: the script's `defender_defense` answer. `base` is the defense as a plain
/// dict reads it -- what every score without an attack in hand sees -- and [`Self::respond`] is
/// the defense against one attack.
#[derive(Clone, Debug)]
struct Defending<'a> {
    base: Defense,
    /// How it answers an attack; `None` for a static defender, whose defense is `base` whatever
    /// hits it.
    adapt: Option<adaptive::Adapt<'a>>,
}

impl Defending<'_> {
    /// The script's `respond`: the defense to score attack `ar` against, and what the defender put
    /// on for it (ring and buff names, empty for a static defender).
    fn respond(&self, ar: &Ar) -> (Defense, Vec<String>) {
        match &self.adapt {
            Some(adapt) => adapt.respond(&self.base, ar),
            None => (self.base, Vec::new()),
        }
    }
}

/// Who the damage objective is scored against: the defender's armour ([`Worn`]) and how it
/// answers each weapon.
#[derive(Clone, Copy, Debug)]
struct Against {
    worn: Worn,
    reply: Reply,
}

impl Against {
    /// The bracket's average defender, answering as `reply` says.
    const fn average(reply: Reply) -> Self {
        Self { worn: None, reply }
    }
}

/// The script's ` vs Ring + Buff` label tail: what the defender put on, nothing when it put on
/// nothing.
fn versus(worn: &[String]) -> String {
    if worn.is_empty() {
        String::new()
    } else {
        format!(" vs {}", worn.join(" + "))
    }
}

/// Python's built-in `sum` over floats (3.12 and later): Neumaier's compensated sum from 0, the
/// compensation added at the end when it is finite and not zero.
fn py_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut total, mut compensation) = (0.0_f64, 0.0_f64);
    for x in values {
        let t = total + x;
        if total.abs() >= x.abs() {
            compensation += (total - t) + x;
        } else {
            compensation += (x - t) + total;
        }
        total = t;
    }
    if compensation != 0.0 && compensation.is_finite() {
        total + compensation
    } else {
        total
    }
}

/// Python's `round(x, digits)`: the decimal nearest the double, ties to even, read back.
fn py_round(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().unwrap_or(x)
}

/// One SL bracket's floors, average defender and median stats.
#[derive(Clone, Debug)]
struct Bracket {
    /// VIG, VIT, ADP, ATT, then END for a high-stamina weapon.
    floors: [i32; 5],
    defense: Defense,
    /// The median levelled stats of the builds `defense` averages over: the script's
    /// `bracket_stats`, what a chosen defender's armour is worn at.
    stats: Stats,
    /// The mean max poise of the same builds: the script's `bracket_poise`.
    poise: f64,
}

/// A chosen defender: an index into each slot of [`CorpusBackend::wearable`], head to legs, or
/// `None` for the bracket's average.
type Worn = Option<[usize; 4]>;

/// A ring the minimum search may wear.
#[derive(Clone, Debug)]
struct RingEffect {
    key: String,
    weight: f64,
    load_mul: f64,
    add: Stats,
}

/// What wearing a ring does in place of stat points: the script's `ring_gear` row.
#[derive(Clone, Debug)]
struct RingGear {
    /// An index into the file's rings.
    ring: usize,
    /// Attunement slots it adds.
    slots: i32,
    /// Max HP, max equip load and max stamina factors.
    hp: f64,
    load: f64,
    stamina: f64,
    /// Flat stat bonuses.
    add: Stats,
    /// `(stat, low, high, bonus at or below low, bonus at or above high)`: Ring of the Embedded's.
    scaled: Vec<(usize, i32, i32, i32, i32)>,
}

impl RingGear {
    /// Whether it raises `stat` itself, not only what the stat gives.
    fn touches(&self, stat: usize) -> bool {
        self.add[stat] != 0 || self.scaled.iter().any(|x| x.0 == stat)
    }
}

/// The script's `scaled_bonus`: `at_low` at or below `low`, `at_high` at or above `high`, floored
/// linear between.
fn scaled_bonus(s: i32, low: i32, high: i32, at_low: i32, at_high: i32) -> i32 {
    if s <= low {
        at_low
    } else if s >= high {
        at_high
    } else {
        at_low + ((at_high - at_low) * (s - low)).div_euclid(high - low)
    }
}

/// Which of a ring's factors: the script's `"hp"`, `"load"`, `"stamina"`.
#[derive(Clone, Copy)]
enum Factor {
    Hp,
    Load,
    Stamina,
}

/// The script's `RING_SLOT_POINTS`: a ring worn in place of stat points must free at least this
/// many, what one ring slot buys as a stat ring (+5). An offensive ring in the slot is weighed by
/// the objective instead (`choose_rings`); a stat ring is not run, and needs no run: five free
/// points gain at least what +5 in one stat does.
const RING_SLOT_POINTS: i32 = 5;

/// The script's `UNREACHABLE`: the lift of a class whose spells no ATT holds.
const UNREACHABLE: i32 = 1_000_000;

/// The optimizer's pick that raises INT and FTH together, for a dark attack.
const BOTH: usize = usize::MAX;

/// `st` with INT and FTH each set to `to`.
fn both_at(st: &Stats, to: i32) -> Stats {
    let mut out = *st;
    out[INT] = to;
    out[FTH] = to;
    out
}

/// `st` with INT and FTH each raised by `n`: the script's `both`.
fn both_up(st: &Stats, n: i32) -> Stats {
    let mut out = *st;
    out[INT] += n;
    out[FTH] += n;
    out
}

/// A best build: `(value, class, two-handed, levelled stats, worn rings)`.
type Best = (f64, usize, bool, Stats, Vec<usize>);

/// A piece of armour the minimum search may wear.
#[derive(Clone, Debug)]
struct ArmorPiece {
    name: String,
    weight: f64,
    alter: Stats,
    require: Vec<(usize, i32)>,
}

/// An armour piece, as `best_armor` reads it.
#[derive(Clone, Debug)]
struct Wearable {
    key: String,
    name: String,
    weight: f64,
    /// physical, magic, fire, lightning, dark.
    defense: [f64; 5],
    require: Vec<(usize, i32)>,
    /// slash, strike, thrust defense: the general physical one where the piece has none.
    typed: [f64; 3],
    /// `physicalDEFBonus`: how much of the stat bonus each physical defense gains.
    bonus: f64,
    /// What wearing it adds to each stat.
    alter: Stats,
    /// Its poise: `ArmorParam.strong` where the script read the regulation.
    poise: f64,
}

/// One corpus build, as nearest-build search reads it.
#[derive(Clone, Debug)]
struct CorpusBuild {
    bracket: usize,
    stat_brackets: Stats,
    /// Indices into the file's rings, slot order.
    rings: Vec<usize>,
    /// Indices into the file's weapons with the infusion; `None` is `MugenMonkey`'s unrecorded
    /// one.
    weapons: Vec<(usize, Option<Infusion>)>,
    /// How many weapons its levelled stats wield one- and two-handed, what it scores when it is a
    /// neighbour in [`CorpusBackend::flexibility`]. `None` in a file written before the script
    /// exported it.
    flex: Option<(u32, u32)>,
    /// Its melee weapon's one-handed R1 first hit as (poise damage, armorBreak): the counter-hit
    /// the script's `bracket_poise` weighs hyperarmor against. `None` without one.
    counter: Option<(f64, i32)>,
}

/// Attack rating per type, physical, magic, fire, lightning, dark; `None` where the weapon has none.
type Ar = [Option<f64>; 5];

/// One ranked weapon and infusion, before it is a [`ResultRow`].
#[derive(Clone, Debug)]
struct Ranked {
    damage: f64,
    weapon: usize,
    infusion: Infusion,
    ar: Ar,
    label: String,
}

/// What `weapons_for` is asked besides the stats and soul level.
#[derive(Clone, Copy, Debug)]
struct Query<'a> {
    /// Only weapons the stats wield one-handed.
    one_hand: bool,
    /// Only this weapon class.
    class: Option<&'a str>,
    /// Only each class's best row.
    per_class: bool,
    /// Rank by the R1 hits landing within this many seconds; `0` ranks one hit.
    window: f64,
    /// Rank by total attack rating rather than damage.
    raw_ar: bool,
    /// Damage, or bleed/poison build-up times hits per attack.
    objective: Objective,
    /// How many rows to keep when not `per_class`.
    top: usize,
    /// This weapon alone (an index into the file's weapons), every infusion of it, and no
    /// high-stamina END gate: the script's `weapons_for(weapon=...)`, which `best_infusion` asks.
    weapon: Option<usize>,
    /// Who damage is scored against, and how it answers each weapon.
    defender: Against,
}

/// A weapon the similar builds carry: how many carry it, and how many carry each infusion of it.
type Carried = (usize, u32, Vec<(Option<Infusion>, u32)>);

/// One class's cheapest way to wield a weapon, as `minimum_build` returns it.
#[derive(Clone, Debug)]
struct MinimumPick {
    sl: i32,
    class: usize,
    stats: Stats,
    /// The worn pieces' names; naked slots are left out.
    armor: Vec<String>,
    /// Indices into the file's ring effects.
    rings: Vec<usize>,
}

/// The script's recommendations over the data file `--export-backend` writes.
#[derive(Clone, Debug, Default)]
pub struct CorpusBackend {
    classes: Vec<Class>,
    tables: Tables,
    weapons: Vec<Weapon>,
    brackets: Vec<Bracket>,
    /// `(key, name, weight)`.
    rings: Vec<(String, String, f64)>,
    /// Each ring's upgrade line, beside `rings`: Ring of Blades + 2's is `Ring_of_Blades`.
    ring_groups: Vec<String>,
    /// Rings a generated build never suggests or grants (the script's `NO_USE_RINGS`), as indices
    /// into `rings`.
    no_use: Vec<usize>,
    /// The script's `data.ring_attack`, in its order: a ring (an index into `rings`) and its flat
    /// attack adds, physical, magic, fire, lightning, dark.
    ring_attack: Vec<(usize, [f64; 5])>,
    ring_effects: Vec<RingEffect>,
    /// The script's `ring_gear`, in its order: every ring the optimizer counts.
    gear: Vec<RingGear>,
    /// head, chest, hands, legs.
    armor: [Vec<ArmorPiece>; 4],
    /// Every armour piece, `Naked` among them, head, chest, hands, legs: what `best_armor` picks
    /// from.
    wearable: [Vec<Wearable>; 4],
    /// The share of damage the corpus's weapons deal per type, physical, magic, fire, lightning,
    /// dark: what `best_armor` weights each defense by.
    threat_mix: [f64; 5],
    calibration: Option<Calibration>,
    /// Rings at least a tenth of all builds wear, most worn first, as indices into `rings`.
    common: Vec<usize>,
    spells: Vec<Spell>,
    catalysts: Vec<Catalyst>,
    corpus: Vec<CorpusBuild>,
    /// Every ammunition, in name order: the script's `sorted(data.ammo.items())`.
    ammo: Vec<ranged::Ammo>,
    /// The script's `data.ring_defense`, in its order: a ring (an index into `rings`) and the
    /// defense it changes.
    ring_defense: Vec<(usize, adaptive::Change)>,
    /// The script's `data.defense_buffs`, in its order.
    buffs: Vec<adaptive::Buff>,
    /// The script's `counter_rings`, as indices into `rings`: what an adaptive defender swaps in.
    counters: Vec<usize>,
    /// Per SL bracket, the adaptive average defender over its `bracket_builds`.
    cores: Vec<adaptive::Core>,
}

/// Why a data file could not be read.
fn bad(line: usize, what: &str) -> String {
    format!("line {line}: {what}")
}

fn int(field: Option<&str>, line: usize) -> Result<i32, String> {
    field
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| bad(line, "expected an integer"))
}

fn float(field: Option<&str>, line: usize) -> Result<f64, String> {
    field
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| bad(line, "expected a number"))
}

fn stats<'a>(fields: &mut impl Iterator<Item = &'a str>, line: usize) -> Result<Stats, String> {
    let mut out = [0; STAT_COUNT];
    for value in &mut out {
        *value = int(fields.next(), line)?;
    }
    Ok(out)
}

/// `stat:value,...`, or `-` for none.
fn stat_pairs(field: Option<&str>, line: usize) -> Result<Vec<(usize, i32)>, String> {
    let field = field.ok_or_else(|| bad(line, "missing requirements"))?;
    if field == "-" {
        return Ok(Vec::new());
    }
    field
        .split(',')
        .map(|pair| {
            let (stat, value) = pair
                .split_once(':')
                .ok_or_else(|| bad(line, "bad stat:value"))?;
            let stat: usize = stat.parse().map_err(|_| bad(line, "bad stat index"))?;
            if stat >= STAT_COUNT {
                return Err(bad(line, "stat index out of range"));
            }
            Ok((stat, int(Some(value), line)?))
        })
        .collect()
}

fn infusion_code(code: &str, line: usize) -> Result<Option<Infusion>, String> {
    let mut chars = code.chars();
    match (chars.next(), chars.next()) {
        (Some('?'), None) => Ok(None),
        (Some(letter), None) => weapons::infusion_for_code(letter)
            .map(Some)
            .ok_or_else(|| bad(line, "unknown infusion code")),
        _ => Err(bad(line, "bad infusion code")),
    }
}

/// head, chest, hands, legs: the script's `ARMOR_SLOTS`, by index.
const ARMOR_SLOTS: [&str; 4] = ["head", "chest", "hands", "legs"];

fn armor_slot(name: &str, line: usize) -> Result<usize, String> {
    ARMOR_SLOTS
        .iter()
        .position(|&slot| slot == name)
        .ok_or_else(|| bad(line, "armour slot"))
}

fn indices(field: Option<&str>, line: usize) -> Result<Vec<usize>, String> {
    match field {
        Some("-") | None => Ok(Vec::new()),
        Some(list) => list
            .split(',')
            .map(|index| index.parse().map_err(|_| bad(line, "bad index")))
            .collect(),
    }
}

impl CorpusBackend {
    /// Read the data file at `path`.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or [`CorpusBackend::parse`] refuses it.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        Self::parse(&text)
    }

    /// Parse a data file's text.
    ///
    /// # Errors
    ///
    /// The first line is not [`FORMAT`], a line is malformed, or a table the ranking needs is
    /// missing. The message names the line.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, first)) if first == FORMAT => {}
            _ => return Err(format!("not a `{FORMAT}` file")),
        }
        let mut backend = Self::default();
        let mut ring_index = std::collections::HashMap::new();
        for (at, text) in lines {
            backend.parse_line(at + 1, text, &mut ring_index)?;
        }
        backend.check()?;
        backend.counters = backend.counter_rings();
        backend.cores = (0..SL_BRACKETS)
            .map(|bracket| {
                let builds = backend.bracket_builds(bracket);
                backend.adaptive_core(builds.iter().map(|build| build.rings.as_slice()))
            })
            .collect();
        Ok(backend)
    }

    fn parse_line(
        &mut self,
        line: usize,
        text: &str,
        ring_index: &mut std::collections::HashMap<String, usize>,
    ) -> Result<(), String> {
        let mut fields = text.split('\t');
        let tag = fields.next().unwrap_or("");
        let mut next = |what: &str| fields.next().ok_or_else(|| bad(line, what));
        match tag {
            "" => {}
            tag if tag.starts_with('#') => {}
            "C" => {
                let key = next("class key")?.to_owned();
                let name = next("class name")?.to_owned();
                let level = int(Some(next("level")?), line)?;
                let base = stats(&mut fields, line)?;
                self.classes.push(Class {
                    key,
                    name,
                    level,
                    base,
                });
            }
            "T" => {
                let name = next("table name")?;
                let values = fields
                    .map(|value| float(Some(value), line))
                    .collect::<Result<Vec<_>, _>>()?;
                let tables = &mut self.tables;
                let slot = match name {
                    "physicalATKBonus" => &mut tables.physical,
                    "magicATKBonus" => &mut tables.magic,
                    "fireATKBonus" => &mut tables.fire,
                    "lightningATKBonus" => &mut tables.lightning,
                    "darkATKBonus" => &mut tables.dark,
                    "auxATKBonus" => &mut tables.aux,
                    "mundaneATKBonus" => &mut tables.mundane,
                    "equipmentLoad" => &mut tables.equip_load,
                    "attunementSlots" => &mut tables.attunement_slots,
                    "staminaMax" => &mut tables.stamina_max,
                    "hpMax" => &mut tables.hp_max,
                    "additionalHp" => &mut tables.additional_hp,
                    "castMagic" => &mut tables.cast[0],
                    "castFire" => &mut tables.cast[1],
                    "castLightning" => &mut tables.cast[2],
                    "castDark" => &mut tables.cast[3],
                    "physicalDEFBonus" => &mut tables.defense[0],
                    "magicDEFBonus" => &mut tables.defense[1],
                    "fireDEFBonus" => &mut tables.defense[2],
                    "lightningDEFBonus" => &mut tables.defense[3],
                    "darkDEFBonus" => &mut tables.defense[4],
                    _ => return Ok(()),
                };
                *slot = Table(values);
            }
            "W" => {
                let key = next("weapon key")?.to_owned();
                let name = next("weapon name")?.to_owned();
                let class = next("weapon class")?.to_owned();
                let flags = next("weapon flags")?;
                let (shield, catalyst, high_stamina) = (
                    flags.contains('S'),
                    flags.contains('C'),
                    flags.contains('H'),
                );
                let weight = float(Some(next("weight")?), line)?;
                let require = stat_pairs(Some(next("requirements")?), line)?;
                let hyperarmor = [
                    float(Some(next("hyperarmor")?), line)?,
                    float(Some(next("hyperarmor")?), line)?,
                ];
                let counter = float(Some(next("counter")?), line)?;
                let damage_scale = float(Some(next("damage scale")?), line)?;
                self.weapons.push(Weapon {
                    key,
                    name,
                    class,
                    shield,
                    catalyst,
                    high_stamina,
                    weight,
                    require,
                    hyperarmor,
                    counter,
                    damage_scale,
                    infusions: Vec::new(),
                    timeline: [Vec::new(), Vec::new()],
                    status: Default::default(),
                    ranged: None,
                    chains: Default::default(),
                    r1: [[None; 4]; 2],
                    poise: Default::default(),
                });
            }
            "CH" => self.parse_chain(line, &mut fields)?,
            "PO" => self.parse_poise(line, &mut fields)?,
            "RM" => {
                let grip = match next("grip")? {
                    "1" => 0,
                    "2" => 1,
                    _ => return Err(bad(line, "grip is 1 or 2")),
                };
                let mut values = [None; 4];
                for value in &mut values {
                    *value = match fields.next() {
                        Some("-") => None,
                        text => Some(float(text, line)?),
                    };
                }
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "R1 metrics before any weapon"))?
                    .r1[grip] = values;
            }
            "RG" | "RS" | "AM" => self.parse_ranged(tag, line, &mut fields)?,
            "RD" | "DB" => self.parse_adaptive(tag, line, ring_index, &mut fields)?,
            "I" => {
                let infusion = infusion_code(next("infusion")?, line)?
                    .ok_or_else(|| bad(line, "a weapon's infusion cannot be unknown"))?;
                let mut atk = [0.0; 7];
                for value in &mut atk {
                    *value = float(fields.next(), line)?;
                }
                let mut scale = [0.0; 9];
                for value in &mut scale {
                    *value = float(fields.next(), line)?;
                }
                let mut rate = [0.0; 5];
                for value in &mut rate {
                    *value = float(fields.next(), line)?;
                }
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "an infusion before any weapon"))?
                    .infusions
                    .push(InfusionRow {
                        infusion,
                        atk,
                        scale,
                        rate,
                    });
            }
            "L" => {
                let grip = match next("grip")? {
                    "1" => 0,
                    "2" => 1,
                    _ => return Err(bad(line, "grip is 1 or 2")),
                };
                let hits = next("hits")?
                    .split(' ')
                    .filter(|hit| !hit.is_empty())
                    .map(|hit| {
                        let mut parts = hit.split(':');
                        let at = float(parts.next(), line)?;
                        let motion_value = float(parts.next(), line)?;
                        let defense = match parts.next() {
                            Some("p") => 0,
                            Some("s") => 5,
                            Some("k") => 6,
                            Some("t") => 7,
                            _ => return Err(bad(line, "hit type")),
                        };
                        let lower = float(parts.next(), line)?;
                        let mut flat = [0.0; 5];
                        if let Some(text) = parts.next() {
                            let mut values = text.split(',');
                            for slot in &mut flat {
                                *slot = float(values.next(), line)?;
                            }
                            if values.next().is_some() {
                                return Err(bad(line, "five flat attacks"));
                            }
                        }
                        Ok(Hit {
                            at,
                            motion_value,
                            defense,
                            lower,
                            flat,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a timeline before any weapon"))?
                    .timeline[grip] = hits;
            }
            "S" => {
                let grip = match next("grip")? {
                    "1" => 0,
                    "2" => 1,
                    _ => return Err(bad(line, "grip is 1 or 2")),
                };
                let tag = next("chain")?;
                let chain = STATUS_CHAINS
                    .iter()
                    .position(|&chain| chain == tag)
                    .ok_or_else(|| bad(line, "chain is R1 or R2"))?;
                let per_attack = u32::try_from(int(Some(next("hits per attack")?), line)?)
                    .map_err(|_| bad(line, "hits per attack"))?;
                let hits = match next("hits")? {
                    "-" => Vec::new(),
                    list => list
                        .split(' ')
                        .filter(|hit| !hit.is_empty())
                        .map(|hit| {
                            let (start, at) =
                                hit.split_once(':').ok_or_else(|| bad(line, "start:time"))?;
                            Ok((float(Some(start), line)?, float(Some(at), line)?))
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                };
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a status chain before any weapon"))?
                    .status[grip][chain]
                    .push(StatusChain { per_attack, hits });
            }
            "B" => {
                let index = int(Some(next("bracket")?), line)?;
                if usize::try_from(index).ok() != Some(self.brackets.len()) {
                    return Err(bad(line, "brackets out of order"));
                }
                let mut floors = [0; 5];
                for value in &mut floors {
                    *value = int(fields.next(), line)?;
                }
                let mut defense = [0.0; 8];
                for value in &mut defense {
                    *value = float(fields.next(), line)?;
                }
                let stats = stats(&mut fields, line)?;
                let poise = float(fields.next(), line)?;
                self.brackets.push(Bracket {
                    floors,
                    defense,
                    stats,
                    poise,
                });
            }
            "R" => {
                let key = next("ring key")?.to_owned();
                let name = next("ring name")?.to_owned();
                let weight = float(Some(next("weight")?), line)?;
                let group = next("ring group")?.to_owned();
                ring_index.insert(key.clone(), self.rings.len());
                self.rings.push((key, name, weight));
                self.ring_groups.push(group);
            }
            "N" => {
                let key = next("ring key")?;
                let index = *ring_index
                    .get(key)
                    .ok_or_else(|| bad(line, "a no-use ring that is not a ring"))?;
                self.no_use.push(index);
            }
            "O" => {
                let key = next("ring key")?;
                let ring = *ring_index
                    .get(key)
                    .ok_or_else(|| bad(line, "a ring attack add for a ring that is not a ring"))?;
                let mut add = [0.0; 5];
                for value in &mut add {
                    *value = float(fields.next(), line)?;
                }
                self.ring_attack.push((ring, add));
            }
            "P" => {
                let slot = armor_slot(next("armour slot")?, line)?;
                let key = next("armour key")?.to_owned();
                let name = next("armour name")?.to_owned();
                let weight = float(Some(next("weight")?), line)?;
                let mut defense = [0.0; 5];
                for value in &mut defense {
                    *value = float(fields.next(), line)?;
                }
                let require = stat_pairs(fields.next(), line)?;
                let mut typed = [0.0; 3];
                for value in &mut typed {
                    *value = float(fields.next(), line)?;
                }
                let bonus = float(fields.next(), line)?;
                let alter = stats(&mut fields, line)?;
                let poise = float(fields.next(), line)?;
                self.wearable[slot].push(Wearable {
                    key,
                    name,
                    weight,
                    defense,
                    require,
                    typed,
                    bonus,
                    alter,
                    poise,
                });
            }
            "H" => {
                for value in &mut self.threat_mix {
                    *value = float(fields.next(), line)?;
                }
            }
            "E" => {
                let key = next("ring key")?.to_owned();
                let weight = float(Some(next("weight")?), line)?;
                let load_mul = float(Some(next("load multiplier")?), line)?;
                let add = stats(&mut fields, line)?;
                self.ring_effects.push(RingEffect {
                    key,
                    weight,
                    load_mul,
                    add,
                });
            }
            "G" => {
                let key = next("ring key")?;
                let ring = *ring_index
                    .get(key)
                    .ok_or_else(|| bad(line, "ring gear for a ring that is not a ring"))?;
                let slots = int(Some(next("slots")?), line)?;
                let hp = float(Some(next("hp factor")?), line)?;
                let load = float(Some(next("load factor")?), line)?;
                let stamina = float(Some(next("stamina factor")?), line)?;
                let add = stats(&mut fields, line)?;
                let scaled = match fields.next() {
                    Some("-") | None => Vec::new(),
                    Some(list) => list
                        .split(',')
                        .map(|entry| {
                            let parts: Vec<i32> = entry
                                .split(':')
                                .map(|part| part.parse().map_err(|_| bad(line, "scaled bonus")))
                                .collect::<Result<_, _>>()?;
                            match parts[..] {
                                [stat, low, high, at_low, at_high]
                                    if usize::try_from(stat).is_ok_and(|s| s < STAT_COUNT)
                                        && high > low =>
                                {
                                    Ok((
                                        usize::try_from(stat).unwrap_or(0),
                                        low,
                                        high,
                                        at_low,
                                        at_high,
                                    ))
                                }
                                _ => Err(bad(line, "scaled bonus is stat:low:high:low:high")),
                            }
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                };
                self.gear.push(RingGear {
                    ring,
                    slots,
                    hp,
                    load,
                    stamina,
                    add,
                    scaled,
                });
            }
            "A" => {
                let slot = armor_slot(next("armour slot")?, line)?;
                next("armour key")?;
                let name = next("armour name")?.to_owned();
                let weight = float(Some(next("weight")?), line)?;
                let alter = stats(&mut fields, line)?;
                let require = stat_pairs(fields.next(), line)?;
                self.armor[slot].push(ArmorPiece {
                    name,
                    weight,
                    alter,
                    require,
                });
            }
            "K" => {
                let n = int(Some(next("count")?), line)?;
                let top1 = float(Some(next("top1")?), line)?;
                let top2 = float(Some(next("top2")?), line)?;
                self.calibration = Some(Calibration {
                    n: u32::try_from(n).map_err(|_| bad(line, "negative count"))?,
                    top1: top1 as f32,
                    top2: top2 as f32,
                });
            }
            "M" => {
                let key = next("ring key")?;
                let index = *ring_index
                    .get(key)
                    .ok_or_else(|| bad(line, "a common ring that is not a ring"))?;
                self.common.push(index);
            }
            "Z" => {
                let key = next("spell key")?.to_owned();
                let name = next("spell name")?.to_owned();
                let slots = int(Some(next("slots")?), line)?;
                let category = int(Some(next("category")?), line)?;
                let require = stat_pairs(fields.next(), line)?;
                self.spells.push(Spell {
                    key,
                    name,
                    slots,
                    category,
                    require,
                });
            }
            "Y" => {
                next("catalyst key")?;
                let name = next("catalyst name")?.to_owned();
                let categories = next("categories")?
                    .chars()
                    .map(|digit| {
                        digit
                            .to_digit(10)
                            .and_then(|digit| i32::try_from(digit).ok())
                            .filter(|&category| usize::try_from(category).unwrap_or(9) < 5)
                            .ok_or_else(|| bad(line, "spell category"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let require = stat_pairs(fields.next(), line)?;
                let mut power = [(0.0, 0.0); 4];
                for terms in &mut power {
                    let (base, scale) = fields
                        .next()
                        .and_then(|pair| pair.split_once(':'))
                        .ok_or_else(|| bad(line, "base:scale"))?;
                    *terms = (float(Some(base), line)?, float(Some(scale), line)?);
                }
                self.catalysts.push(Catalyst {
                    name,
                    categories,
                    require,
                    power,
                });
            }
            "X" => {
                let bracket = usize::try_from(int(Some(next("bracket")?), line)?)
                    .ok()
                    .filter(|&bracket| bracket < SL_BRACKETS)
                    .ok_or_else(|| bad(line, "SL bracket out of range"))?;
                let digits = next("stat brackets")?;
                let mut stat_brackets = [0; STAT_COUNT];
                let mut chars = digits.chars();
                for value in &mut stat_brackets {
                    let digit = chars
                        .next()
                        .and_then(|c| c.to_digit(10))
                        .ok_or_else(|| bad(line, "stat brackets are nine digits"))?;
                    *value = i32::try_from(digit).unwrap_or(0);
                }
                let rings = indices(fields.next(), line)?;
                let carried = match fields.next() {
                    Some("-") | None => Vec::new(),
                    Some(list) => list
                        .split(',')
                        .map(|pair| {
                            let (weapon, code) = pair
                                .split_once(':')
                                .ok_or_else(|| bad(line, "weapon:infusion"))?;
                            let weapon: usize =
                                weapon.parse().map_err(|_| bad(line, "weapon index"))?;
                            Ok((weapon, infusion_code(code, line)?))
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                };
                // `one:two`, trailing and optional: added without a format bump, so a file from
                // before it still reads, with no flexibility to rank against.
                let flex = match fields.next() {
                    None => None,
                    Some(pair) => {
                        let (one, two) = pair
                            .split_once(':')
                            .ok_or_else(|| bad(line, "flexibility one:two"))?;
                        let count = |text: &str| {
                            text.parse::<u32>()
                                .map_err(|_| bad(line, "flexibility count"))
                        };
                        Some((count(one)?, count(two)?))
                    }
                };
                // `poise:break`, the counter-hit the script's bracket_poise counts for the build.
                let counter = match fields.next() {
                    Some("-") | None => None,
                    Some(pair) => {
                        let (damage, armor_break) = pair
                            .split_once(':')
                            .ok_or_else(|| bad(line, "counter-hit poise:break"))?;
                        Some((float(Some(damage), line)?, int(Some(armor_break), line)?))
                    }
                };
                self.corpus.push(CorpusBuild {
                    bracket,
                    stat_brackets,
                    rings,
                    weapons: carried,
                    flex,
                    counter,
                });
            }
            other => return Err(bad(line, &format!("unknown record {other:?}"))),
        }
        Ok(())
    }

    /// Every index in range and every table present, so no answer can panic on the file.
    fn check(&self) -> Result<(), String> {
        if self.classes.is_empty() {
            return Err("no classes".to_owned());
        }
        if self.brackets.len() != SL_BRACKETS {
            return Err(format!(
                "{} SL brackets, not {SL_BRACKETS}",
                self.brackets.len()
            ));
        }
        let tables = &self.tables;
        for (name, table) in [
            ("physicalATKBonus", &tables.physical),
            ("magicATKBonus", &tables.magic),
            ("fireATKBonus", &tables.fire),
            ("lightningATKBonus", &tables.lightning),
            ("darkATKBonus", &tables.dark),
            ("auxATKBonus", &tables.aux),
            ("mundaneATKBonus", &tables.mundane),
            ("equipmentLoad", &tables.equip_load),
            ("attunementSlots", &tables.attunement_slots),
            ("physicalDEFBonus", &tables.defense[0]),
            ("magicDEFBonus", &tables.defense[1]),
            ("fireDEFBonus", &tables.defense[2]),
            ("lightningDEFBonus", &tables.defense[3]),
            ("darkDEFBonus", &tables.defense[4]),
        ] {
            if table.0.is_empty() {
                return Err(format!("no {name} table"));
            }
        }
        let names_missing = self.corpus.iter().any(|build| {
            build.rings.iter().any(|&ring| ring >= self.rings.len())
                || build
                    .weapons
                    .iter()
                    .any(|&(weapon, _)| weapon >= self.weapons.len())
        });
        if names_missing {
            return Err("a corpus build names an item the file does not".to_owned());
        }
        Ok(())
    }

    /// How many corpus builds the file carries.
    pub fn corpus_len(&self) -> usize {
        self.corpus.len()
    }

    fn weapon_by_key(&self, key: &str) -> Option<&Weapon> {
        self.weapons.iter().find(|weapon| weapon.key == key)
    }

    fn bracket(&self, sl: u32) -> &Bracket {
        &self.brackets[sl_bracket(sl).min(self.brackets.len() - 1)]
    }

    /// `defender`'s pieces as indices into [`Self::wearable`], `Some(None)` for the average
    /// defender, `None` when a piece is not in the file's armour table.
    fn worn(&self, defender: &Defender) -> Option<Worn> {
        let Some(keys) = defender.pieces() else {
            return Some(None);
        };
        let mut out = [0; 4];
        for (slot, key) in keys.iter().enumerate() {
            out[slot] = self.wearable[slot]
                .iter()
                .position(|piece| piece.key == *key)?;
        }
        Some(Some(out))
    }

    /// The script's `defender_defense`: the bracket's average defender when `worn` is `None`,
    /// otherwise its `build_defense` of those pieces at the bracket's median stats plus what the
    /// pieces add to them, with no rings. Physical defense is each piece's own plus its bonus
    /// coefficient times the stat bonus at END + VIT + STR + DEX; each element's is the stat table's
    /// base (INT, INT + FTH, FTH, the lesser of INT and FTH) plus the pieces' flat values. Summed in
    /// the script's order, head to legs, so the doubles are its.
    fn defense_at(&self, sl: u32, worn: Worn) -> Defense {
        let bracket = self.bracket(sl);
        let Some(worn) = worn else {
            return bracket.defense;
        };
        let pieces: [&Wearable; 4] = std::array::from_fn(|slot| &self.wearable[slot][worn[slot]]);
        let mut eff = bracket.stats;
        for piece in pieces {
            for (stat, add) in eff.iter_mut().zip(piece.alter) {
                *stat += add;
            }
        }
        let tables = &self.tables.defense;
        let pb = tables[0].at(eff[END] + eff[VIT] + eff[STR] + eff[DEX]);
        let mut out = [0.0; 8];
        out[0] = pieces.iter().fold(0.0, |sum, piece| {
            sum + (piece.defense[0] + piece.bonus * pb)
        });
        for (typed, value) in out[5..].iter_mut().enumerate() {
            *value = pieces.iter().fold(0.0, |sum, piece| {
                sum + (piece.typed[typed] + piece.bonus * pb)
            });
        }
        let base = [
            eff[INT],
            eff[INT] + eff[FTH],
            eff[FTH],
            eff[INT].min(eff[FTH]),
        ];
        for (element, index) in base.into_iter().enumerate() {
            let flat = pieces
                .iter()
                .fold(0.0, |sum, piece| sum + piece.defense[1 + element]);
            out[1 + element] = tables[1 + element].at(index) + flat;
        }
        out
    }

    /// [`Self::defense_at`] as the scoring reads it: the script's `defender_defense`. An adaptive
    /// defender answers over the bracket's builds' rings (the script's `bracket_builds`), a chosen
    /// set as one build with no rings.
    fn defending(&self, sl: u32, against: Against) -> Defending<'_> {
        let adapt = against.reply.adapts().then(|| adaptive::Adapt {
            backend: self,
            core: match (against.worn, self.cores.get(sl_bracket(sl))) {
                (None, Some(core)) => std::borrow::Cow::Borrowed(core),
                (None, None) => std::borrow::Cow::Owned(self.adaptive_core([])),
                (Some(_), _) => std::borrow::Cow::Owned(self.adaptive_core([&[][..]])),
            },
            reply: against.reply,
        });
        Defending {
            base: self.defense_at(sl, against.worn),
            adapt,
        }
    }

    /// The script's `bracket_builds`: the corpus builds of SL bracket `bracket`, or of the nearest
    /// bracket (the lower on a tie) with 20 or more; none when no bracket has.
    fn bracket_builds(&self, bracket: usize) -> Vec<&CorpusBuild> {
        let mut counts = [0_usize; SL_BRACKETS];
        for build in &self.corpus {
            counts[build.bracket.min(SL_BRACKETS - 1)] += 1;
        }
        let mut order: Vec<usize> = (0..SL_BRACKETS).collect();
        order.sort_by_key(|&at| (at.abs_diff(bracket), at));
        order
            .into_iter()
            .find(|&at| counts[at] >= 20)
            .map(|at| {
                self.corpus
                    .iter()
                    .filter(|build| build.bracket.min(SL_BRACKETS - 1) == at)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The script's `ring_attack_add`: what the worn `rings` add to `row`'s attack rating per type,
    /// physical, magic, fire, lightning, dark -- each ring's flat add times the row's rate in that
    /// type, only in a type the row has a base in.
    fn ring_attack_add(&self, row: &InfusionRow, rings: &[usize]) -> [f64; 5] {
        let mut out = [0.0; 5];
        for (kind, total) in out.iter_mut().enumerate() {
            if row.atk[kind] == 0.0 {
                continue;
            }
            for &ring in rings {
                let flat = self
                    .ring_attack
                    .iter()
                    .find(|(at, _)| *at == ring)
                    .map_or(0.0, |(_, add)| add[kind]);
                if flat != 0.0 {
                    *total += flat * row.rate[kind];
                }
            }
        }
        out
    }

    /// The script's `offense_rings`: per ring upgrade group the last ring with an attack add,
    /// never a no-use ring, when it adds some attack to `weapon`'s `infusion`; most attack added
    /// first, file order on a tie. None for bleed or poison; AR takes damage's, since an add
    /// raises both.
    fn offense_rings(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        objective: Objective,
    ) -> Vec<usize> {
        if matches!(objective, Objective::Bleed | Objective::Poison) {
            return Vec::new();
        }
        let mut by_group: Vec<(&str, usize)> = Vec::new();
        for &(ring, _) in &self.ring_attack {
            if self.no_use.contains(&ring) {
                continue;
            }
            let group = self.ring_groups[ring].as_str();
            match by_group.iter_mut().find(|(seen, _)| *seen == group) {
                Some(entry) => entry.1 = ring,
                None => by_group.push((group, ring)),
            }
        }
        let row = weapon.infusion(infusion);
        let mut ranked: Vec<(f64, usize)> = by_group
            .into_iter()
            .map(|(_, ring)| {
                let gain = row.map_or(0.0, |row| {
                    self.ring_attack_add(row, &[ring]).iter().sum::<f64>()
                });
                (gain, ring)
            })
            .filter(|&(gain, _)| gain > 0.0)
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranked.into_iter().map(|(_, ring)| ring).collect()
    }

    /// The script's `attack_rating`: per-type attack rating at full upgrade, with what the worn
    /// `rings` add.
    fn attack_rating(&self, row: Option<&InfusionRow>, eff: &Stats, rings: &[usize]) -> Ar {
        let mut out = [None; 5];
        let Some(row) = row else {
            return out;
        };
        let (atk, sc) = (&row.atk, &row.scale);
        let add = self.ring_attack_add(row, rings);
        let t = &self.tables;
        if atk[ATK_PHYSICAL] != 0.0 {
            let mut v = atk[ATK_PHYSICAL]
                + add[ATK_PHYSICAL]
                + sc[SCALE_STR] * t.physical.at(eff[STR])
                + sc[SCALE_DEX] * t.physical.at(eff[DEX]);
            if row.infusion == Infusion::Enchanted && sc[SCALE_MAGIC] != 0.0 {
                v += sc[SCALE_MAGIC] * t.magic.at(eff[INT]);
            }
            if row.infusion == Infusion::Mundane {
                let least = eff.iter().copied().min().unwrap_or(0);
                v += sc[SCALE_MODIFIER] * t.mundane.at(least);
            }
            out[0] = Some(v.trunc());
        }
        let feed = [
            (&t.magic, eff[INT]),
            (&t.fire, eff[INT] + eff[FTH]),
            (&t.lightning, eff[FTH]),
            (&t.dark, eff[INT].min(eff[FTH])),
        ];
        for (element, (table, index)) in feed.into_iter().enumerate() {
            let base = atk[1 + element];
            // A type with a rate and a coefficient but no base still attacks: `AR = (bonus +
            // base) x rate`. The Sanctum Crossbows' dark is this. Enchanted's magic is its INT
            // term on physical, above, unless the row also has a magic base.
            let rated = row.rate[1 + element] != 0.0
                && sc[2 + element] != 0.0
                && !(row.infusion == Infusion::Enchanted && element == 0);
            if base != 0.0 || rated {
                out[1 + element] =
                    Some((base + add[1 + element] + sc[2 + element] * table.at(index)).trunc());
            }
        }
        out
    }

    /// The script's `damage`, summed over the types as its `sum` does: one hit against the
    /// bracket's defender, motion value 1.
    fn damage(ar: &Ar, defense: &Defense) -> f64 {
        py_sum(ar.iter().enumerate().filter_map(|(kind, value)| {
            let value = (*value)?;
            Some(if value == 0.0 {
                0.0
            } else if kind == 0 {
                py_max(0.0, (value * 10.0 - defense[0]) / 12.0)
            } else {
                value * (1.0 - py_min(0.99, (defense[kind] + 100.0) / 1000.0))
            })
        }))
    }

    /// The script's `hit_damage`: one timed hit against the bracket's defender, the hit's flat
    /// attack added to the weapon's.
    fn hit_damage(ar: &Ar, defense: &Defense, hit: &Hit) -> f64 {
        let mut total = 0.0;
        for (kind, value) in ar.iter().enumerate() {
            let value = value.unwrap_or(0.0) + hit.flat[kind];
            if value == 0.0 {
                continue;
            }
            if kind == 0 {
                total += py_max(value * 10.0 - defense[hit.defense], hit.lower) / 12.0;
            } else {
                total += py_max(value * 6.0, hit.lower) / 6.0
                    * (1.0 - py_min(0.99, (defense[kind] + 100.0) / 1000.0));
            }
        }
        total * hit.motion_value
    }

    /// The script's `weapons_for`, before the rows become [`ResultRow`]s.
    fn rank(&self, stats: &Stats, sl: u32, query: &Query<'_>) -> Vec<Ranked> {
        let Query {
            one_hand,
            class,
            per_class,
            window,
            raw_ar,
            objective,
            top,
            weapon: only,
            defender,
        } = *query;
        // The AR goal is raw AR by another name, so the Goal the panel shares between its tabs
        // ranks the Weapons tab as it optimizes. It drops the window: attack rating is one hit's,
        // and the window counts hits of damage, so a window ranked by AR would be damage under the
        // wrong name.
        let (raw_ar, window) = if objective == Objective::Ar {
            (true, 0.0)
        } else {
            (raw_ar, window)
        };
        let bracket = self.bracket(sl);
        let defending = self.defending(sl, defender);
        let class = class.map(norm);
        // Per weapon: its best three while within WITHIN of its best, or for one weapon asked
        // about by itself, every infusion.
        let every = only.is_some();
        let per_weapon = if every { usize::MAX } else { 3 };
        let mut rows: Vec<Ranked> = Vec::new();
        for (index, weapon) in self.weapons.iter().enumerate() {
            if EMPTY.contains(&weapon.key.as_str())
                || weapon.catalyst
                || weapon.shield
                || only.is_some_and(|only| only != index)
            {
                continue;
            }
            let one = weapon.wieldable(stats, false);
            if !weapon.wieldable(stats, true) || (one_hand && !one) {
                continue;
            }
            if class
                .as_ref()
                .is_some_and(|class| norm(&weapon.class) != *class)
            {
                continue;
            }
            if !every && weapon.high_stamina && stats[END] < bracket.floors[4] {
                continue;
            }
            if let Some(launcher) = &weapon.ranged {
                // One trigger pull with the best ammunition per infusion, whatever the window
                // is: the fire rate is not read. A bow fires two-handed.
                let shot = if raw_ar { Objective::Ar } else { objective };
                let grip = if launcher.kind != ranged::Kind::Crossbow {
                    "2H"
                } else if one {
                    "1H"
                } else {
                    "2H only"
                };
                let mut scored: Vec<Ranked> = Vec::new();
                for row in &weapon.infusions {
                    let (value, note, ar, _) =
                        self.ranged_pick(weapon, row.infusion, stats, shot, &defending, &[]);
                    let worn = if shot == Objective::Damage {
                        defending.respond(&ar).1
                    } else {
                        Vec::new()
                    };
                    if value > 0.0 {
                        let lead = if note.starts_with("2H special") {
                            String::new()
                        } else {
                            format!("{grip} ")
                        };
                        scored.push(Ranked {
                            damage: value,
                            weapon: index,
                            infusion: row.infusion,
                            ar: ar.map(|value| value.map(f64::round_ties_even)),
                            label: format!("{lead}1 shot: {note}{}", versus(&worn)),
                        });
                    }
                }
                scored.sort_by(|a, b| b.damage.total_cmp(&a.damage));
                let Some(best) = scored.first().map(|row| row.damage) else {
                    continue;
                };
                rows.extend(
                    scored
                        .into_iter()
                        .take(per_weapon)
                        .filter(|row| every || row.damage >= best * (1.0 - WITHIN)),
                );
                continue;
            }
            if matches!(objective, Objective::Bleed | Objective::Poison) {
                // Build-up per hit times the hits of the best R1/R2 attack (or chain within the
                // window), the script's bleed/poison branch.
                let grips: &[bool] = if one { &[false, true] } else { &[true] };
                let (hits, label) = Self::status_hits(weapon, grips, window);
                if hits == 0 {
                    continue;
                }
                let label = if one || label.starts_with("1H") {
                    label
                } else {
                    format!("{label} (2H only)")
                };
                let mut scored: Vec<Ranked> = Vec::new();
                for row in &weapon.infusions {
                    let per = self.objective_value(
                        weapon,
                        row.infusion,
                        stats,
                        objective,
                        &defending,
                        &[],
                    );
                    if per > 0.0 {
                        scored.push(Ranked {
                            damage: per * f64::from(hits),
                            weapon: index,
                            infusion: row.infusion,
                            ar: self.attack_rating(Some(row), stats, &[]),
                            label: label.clone(),
                        });
                    }
                }
                scored.sort_by(|a, b| b.damage.total_cmp(&a.damage));
                let Some(best) = scored.first().map(|row| row.damage) else {
                    continue;
                };
                rows.extend(
                    scored
                        .into_iter()
                        .take(per_weapon)
                        .filter(|row| every || row.damage >= best * (1.0 - WITHIN)),
                );
                continue;
            }
            // The R1 hits landing within the window, per grip these stats can use.
            let mut lines: Vec<(&str, Vec<&Hit>)> = Vec::new();
            if window != 0.0 {
                for two_hand in [false, true] {
                    let timeline = &weapon.timeline[usize::from(two_hand)];
                    if (two_hand || one) && !timeline.is_empty() {
                        lines.push((
                            if two_hand { "2H" } else { "1H" },
                            timeline.iter().filter(|hit| hit.at <= window).collect(),
                        ));
                    }
                }
                if lines.is_empty() {
                    continue;
                }
            }
            let mut scored: Vec<Ranked> = Vec::new();
            for row in &weapon.infusions {
                let ar = self.attack_rating(Some(row), stats, &[]);
                if ar.iter().all(Option::is_none) {
                    continue;
                }
                // The defender's answer to this weapon's attack: chosen once from its attack
                // ratings and worn for every hit. Raw AR has no defender.
                let (defense, worn) = if raw_ar {
                    (defending.base, Vec::new())
                } else {
                    defending.respond(&ar)
                };
                let defense = &defense;
                let (damage, label) = if window == 0.0 {
                    let damage = if raw_ar {
                        ar.iter().flatten().fold(0.0, |total, value| total + value)
                    } else {
                        Self::damage(&ar, defense) * weapon.damage_scale
                    };
                    (damage, if one { "1H" } else { "2H only" }.to_owned())
                } else {
                    // The grip whose hits deal the most; the first on a tie, as `max` keeps it.
                    let mut best: Option<(&str, usize, f64)> = None;
                    for (grip, hits) in &lines {
                        let damage =
                            py_sum(hits.iter().map(|hit| Self::hit_damage(&ar, defense, hit)));
                        if best.is_none_or(|(_, _, top)| damage > top) {
                            best = Some((grip, hits.len(), damage));
                        }
                    }
                    let Some((grip, hits, damage)) = best else {
                        continue;
                    };
                    let damage = damage * weapon.damage_scale;
                    let only = if one || grip == "1H" {
                        ""
                    } else {
                        " (2H only)"
                    };
                    (damage, format!("{grip} {hits} hits{only}"))
                };
                scored.push(Ranked {
                    damage,
                    weapon: index,
                    infusion: row.infusion,
                    ar,
                    label: label + &versus(&worn),
                });
            }
            scored.sort_by(|a, b| b.damage.total_cmp(&a.damage));
            let Some(best) = scored.first().map(|row| row.damage) else {
                continue;
            };
            rows.extend(
                scored
                    .into_iter()
                    .take(per_weapon)
                    .filter(|row| every || row.damage >= best * (1.0 - WITHIN)),
            );
        }
        rows.sort_by(|a, b| b.damage.total_cmp(&a.damage));
        if per_class {
            let mut seen: Vec<&str> = Vec::new();
            rows.retain(|row| {
                let class = self.class_of(row.weapon);
                let fresh = !seen.contains(&class);
                if fresh {
                    seen.push(class);
                }
                fresh
            });
            return rows;
        }
        rows.truncate(top);
        rows
    }

    /// The script's `status_hits`: the most hits one target takes from the weapon's R1 or R2, and
    /// its label. Without a window, the hits of one attack; with one, the hits of the repeated
    /// chain landing within it, over the chain's attacks begun before `max(3, window)` (the
    /// script's horizon). `grips` are the grips tried, `true` two-handed, in order; ties keep the
    /// earlier grip and R1 over R2.
    fn status_hits(weapon: &Weapon, grips: &[bool], window: f64) -> (u32, String) {
        let (mut best, mut label) = (0, String::new());
        for &two_hand in grips {
            for (chain, tag) in STATUS_CHAINS.iter().enumerate() {
                let mut hits = 0;
                for candidate in &weapon.status[usize::from(two_hand)][chain] {
                    hits = if window == 0.0 {
                        candidate.per_attack
                    } else {
                        let horizon = py_max(3.0, window);
                        let landed = candidate
                            .hits
                            .iter()
                            .filter(|&&(start, at)| start < horizon && at <= window)
                            .count();
                        u32::try_from(landed).unwrap_or(u32::MAX)
                    };
                    if hits != 0 {
                        break;
                    }
                }
                if hits > best {
                    best = hits;
                    label = format!(
                        "{} {tag} {hits} hit{}",
                        if two_hand { "2H" } else { "1H" },
                        if hits > 1 { "s" } else { "" }
                    );
                }
            }
        }
        (best, label)
    }

    /// A weapon's class, `?` where `MugenMonkey` has none, as the script tags a per-class row.
    fn class_of(&self, weapon: usize) -> &str {
        let class = self.weapons[weapon].class.as_str();
        if class.is_empty() { "?" } else { class }
    }

    fn result_row(&self, ranked: &Ranked) -> ResultRow {
        let weapon = &self.weapons[ranked.weapon];
        let hyperarmor = weapon.hyperarmor[usize::from(ranked.label.starts_with("2H"))];
        ResultRow {
            weapon: weapon.name.clone(),
            infusion: ranked.infusion,
            damage: ranked.damage as f32,
            ar_by_type: ranked.ar.map(|value| value.unwrap_or(0.0) as f32),
            grip: ranked.label.clone(),
            two_hand_only: ranked.label.contains("2H only"),
            hyperarmor: (hyperarmor != 0.0).then_some(hyperarmor as f32),
            counter: (weapon.counter != 0.0).then_some(weapon.counter as f32),
            class: self.class_of(ranked.weapon).to_owned(),
        }
    }

    /// The script's `objective_value`, the worn `rings`' attack adds counted. AR keeps them because
    /// the game adds a ring's flat to the attack rating itself (`AR = (bonus + base) x rate`), so a
    /// Ring of Blades raises the menu's number; it leaves out the weapon's damage scale, which
    /// applies after the defence.
    fn objective_value(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        stats: &Stats,
        objective: Objective,
        defending: &Defending,
        rings: &[usize],
    ) -> f64 {
        if let Some(launcher) = &weapon.ranged {
            // A bow, greatbow or crossbow: its best shot, ammunition included.
            return self
                .ranged_value(
                    weapon, launcher, infusion, stats, objective, defending, rings, true,
                )
                .0;
        }
        let row = weapon.infusion(infusion);
        match objective {
            Objective::Bleed | Objective::Poison => {
                let Some(row) = row else {
                    return 0.0;
                };
                let (atk, scale, second) = if objective == Objective::Bleed {
                    (ATK_BLEED, SCALE_BLEED, stats[FTH])
                } else {
                    (ATK_POISON, SCALE_POISON, stats[ADP])
                };
                row.atk[atk] + row.scale[scale] * self.tables.aux.at(3 * stats[DEX] + second)
            }
            Objective::Damage => {
                // Against the defender's answer to this attack: the best attack given the
                // defender's best reply.
                let ar = self.attack_rating(row, stats, rings);
                Self::damage(&ar, &defending.respond(&ar).0) * weapon.damage_scale
            }
            Objective::Ar => self
                .attack_rating(row, stats, rings)
                .iter()
                .flatten()
                .fold(0.0, |total, value| total + value),
        }
    }

    /// The script's `spell_floors`: the least each stat may be for `spells` (indices into
    /// `self.spells`) to be attuned and cast -- each requirement at the highest any needs, ATT at the
    /// least whose slots plus `extra_slots` (from worn rings) hold their summed cost, every other
    /// stat 0. `None` when no ATT holds them.
    fn spell_floors(&self, spells: &[usize], extra_slots: i32) -> Option<Stats> {
        let mut need = [0; STAT_COUNT];
        for &spell in spells {
            for &(stat, value) in &self.spells[spell].require {
                need[stat] = need[stat].max(value);
            }
        }
        let cost: i32 = spells.iter().map(|&spell| self.spells[spell].slots).sum();
        let slots = &self.tables.attunement_slots.0;
        let att = slots
            .iter()
            .position(|&n| n + f64::from(extra_slots) >= f64::from(cost))?;
        need[ATT] = i32::try_from(att).ok()?;
        Some(need)
    }

    /// The ring gear of an index into the file's rings, when the ring has any.
    fn gear_of(&self, ring: usize) -> Option<&RingGear> {
        self.gear.iter().find(|gear| gear.ring == ring)
    }

    /// The script's `gear_stats`: `st` with the worn `rings`' stat bonuses, each stat a ring
    /// raised capped at 99.
    fn gear_stats(&self, st: &Stats, rings: &[usize]) -> Stats {
        let mut out = *st;
        for &ring in rings {
            let Some(gear) = self.gear_of(ring) else {
                continue;
            };
            for (stat, &add) in gear.add.iter().enumerate() {
                if add != 0 {
                    out[stat] = (out[stat] + add).min(99);
                }
            }
            for &(stat, low, high, at_low, at_high) in &gear.scaled {
                out[stat] =
                    (out[stat] + scaled_bonus(st[stat], low, high, at_low, at_high)).min(99);
            }
        }
        out
    }

    /// The script's `ring_slots`.
    fn ring_slots(&self, rings: &[usize]) -> i32 {
        rings
            .iter()
            .filter_map(|&ring| self.gear_of(ring))
            .map(|gear| gear.slots)
            .sum()
    }

    /// The script's `ring_factor`: the product of the worn rings' factors, in ring order.
    fn ring_factor(&self, rings: &[usize], what: Factor) -> f64 {
        let mut f = 1.0;
        for gear in rings.iter().filter_map(|&ring| self.gear_of(ring)) {
            f *= match what {
                Factor::Hp => gear.hp,
                Factor::Load => gear.load,
                Factor::Stamina => gear.stamina,
            };
        }
        f
    }

    /// The script's `max_load`: equipLoadMax at VIT plus the rings' bonus, times their load factor.
    fn max_load(&self, eff: &Stats, rings: &[usize]) -> f64 {
        let vit = self.gear_stats(eff, rings)[VIT];
        self.tables.equip_load.at(vit) * self.ring_factor(rings, Factor::Load)
    }

    /// The script's `hit_points`: max HP of the effective stats `st`, `hpMax` at VGR plus
    /// `additionalHp` at each [`HP_STATS`] stat, a stat outside 1-99 read at row 1. Without the
    /// regulation's columns it is the planner site's `getHP` from VGR alone.
    fn hit_points(&self, st: &Stats) -> f64 {
        let t = &self.tables;
        if t.hp_max.0.is_empty() {
            let vgr = st[VIG];
            return f64::from(
                500 + 30 * vgr.min(20) + 20 * (vgr.min(50) - 20).max(0) + 5 * (vgr - 50).max(0),
            );
        }
        let row = |v: i32| if (1..=99).contains(&v) { v } else { 1 };
        t.hp_max.at(row(st[VIG]))
            + HP_STATS
                .iter()
                .map(|&stat| t.additional_hp.at(row(st[stat])))
                .sum::<f64>()
    }

    /// The script's `sub_rings`: per ring upgrade group, the ring gear ring with the highest item id
    /// (the last in file order), when none of its factors is below 1, never a no-use ring.
    fn sub_rings(&self) -> Vec<usize> {
        let mut by_group: Vec<(&str, usize)> = Vec::new();
        for gear in &self.gear {
            if self.no_use.contains(&gear.ring)
                || py_min(py_min(gear.hp, gear.load), gear.stamina) < 1.0
            {
                continue;
            }
            let group = self.ring_groups[gear.ring].as_str();
            match by_group.iter_mut().find(|(seen, _)| *seen == group) {
                Some(entry) => entry.1 = gear.ring,
                None => by_group.push((group, gear.ring)),
            }
        }
        by_group.into_iter().map(|(_, ring)| ring).collect()
    }

    /// The script's `ring_effect_text`: what `ring` does, "attunement slots +3", "STR +5", ...
    fn ring_effect_text(&self, ring: usize) -> String {
        let Some(gear) = self.gear_of(ring) else {
            return String::new();
        };
        let mut out: Vec<String> = gear
            .add
            .iter()
            .enumerate()
            .filter(|&(_, &add)| add != 0)
            .map(|(stat, add)| format!("{} +{add}", STAT_LABELS[stat]))
            .collect();
        if let Some(&(_, _, _, low, high)) = gear.scaled.first() {
            let stats: Vec<&str> = gear.scaled.iter().map(|x| STAT_LABELS[x.0]).collect();
            out.push(format!("{} +{low} to +{high} by the stat", stats.join("/")));
        }
        if gear.slots != 0 {
            out.push(format!("attunement slots +{}", gear.slots));
        }
        for (factor, name) in [
            (gear.hp, "max HP"),
            (gear.load, "equip load"),
            (gear.stamina, "max stamina"),
        ] {
            if factor != 1.0 {
                out.push(format!("{name} x{factor}"));
            }
        }
        out.join(", ")
    }

    /// The script's `ring_trades` as `trade_line`s: per worn ring, its name, what it does, and the
    /// stats `class` levels less for it -- `ring_lift` with every worn ring against the same
    /// without that one. An offensive ring stands in for nothing and is not listed.
    fn ring_trades(
        &self,
        class: usize,
        floors: &[(usize, i32)],
        require: &[(usize, i32)],
        spells: &[usize],
        worn: &[usize],
    ) -> Vec<String> {
        let base = &self.classes[class].base;
        let lift = |rings: &[usize]| self.ring_lift(base, floors, require, spells, rings);
        let with_all = lift(worn);
        worn.iter()
            .filter(|&&ring| self.gear_of(ring).is_some())
            .map(|&ring| {
                let rest: Vec<usize> = worn.iter().copied().filter(|&r| r != ring).collect();
                let head = format!(
                    "{}: {} -> ",
                    self.rings[ring].1,
                    self.ring_effect_text(ring)
                );
                match (lift(&rest), with_all) {
                    (Some(without), Some(with)) => {
                        let moved: Vec<String> = (0..STAT_COUNT)
                            .filter(|&stat| without[stat] != with[stat])
                            .map(|stat| {
                                format!("{} {} -> {}", STAT_LABELS[stat], without[stat], with[stat])
                            })
                            .collect();
                        if moved.is_empty() {
                            format!("{head}no stat lowered")
                        } else {
                            format!("{head}{}", moved.join(", "))
                        }
                    }
                    _ => format!("{head}no ATT holds the spells without it"),
                }
            })
            .collect()
    }

    /// The script's `ring_lift`: `base` raised to what a build wearing `rings` must level to --
    /// each `floors` stat until what it gives matches what the floor gives with no ring (VIG by HP,
    /// VIT by max load, END by max stamina, each at the stat plus the rings' bonus times their
    /// factor; the rest by the stat plus bonus), each `require` stat and the spells' INT/FTH until
    /// the stat plus the rings' bonus meets it, ATT to the least whose slots plus the rings' hold
    /// the spells. `None` when no ATT holds them.
    fn ring_lift(
        &self,
        base: &Stats,
        floors: &[(usize, i32)],
        require: &[(usize, i32)],
        spells: &[usize],
        rings: &[usize],
    ) -> Option<Stats> {
        let need = self.spell_floors(spells, self.ring_slots(rings))?;
        let mut out = *base;
        let worn: Vec<&RingGear> = rings
            .iter()
            .filter_map(|&ring| self.gear_of(ring))
            .collect();
        let touched = |stat: usize| worn.iter().any(|gear| gear.touches(stat));
        let eff = |out: &Stats, stat: usize, v: i32| -> i32 {
            if !touched(stat) {
                return v;
            }
            let mut at = *out;
            at[stat] = v;
            self.gear_stats(&at, rings)[stat]
        };
        let least = |out: &mut Stats, stat: usize, ok: &dyn Fn(&Stats, i32) -> bool| {
            let mut v = out[stat];
            while v < 99 && !ok(out, v) {
                v += 1;
            }
            out[stat] = v;
        };
        let stamina = &self.tables.stamina_max;
        // What a floor stat gives, read off a whole stat block -- VIG max HP, VIT max load, END max
        // stamina: the curve a ring's factor is weighed against.
        let tab = |stat: usize, st: &Stats| -> f64 {
            match stat {
                VIG => self.hit_points(st),
                VIT => self.tables.equip_load.at(st[VIT]),
                _ => stamina.at(st[END]),
            }
        };
        // Whether a worn ring raises a stat the curve reads: max HP reads VGR and every
        // `HP_STATS` stat.
        let reads = |stat: usize| -> bool {
            touched(stat) || (stat == VIG && HP_STATS.iter().any(|&s| touched(s)))
        };
        for &(stat, floor) in floors {
            let curve = match stat {
                VIG => Some(self.ring_factor(rings, Factor::Hp)),
                VIT => Some(self.ring_factor(rings, Factor::Load)),
                END if !stamina.0.is_empty() => Some(self.ring_factor(rings, Factor::Stamina)),
                _ => None,
            };
            match curve {
                Some(factor) if factor != 1.0 || reads(stat) => {
                    let mut at = out;
                    at[stat] = floor;
                    let target = tab(stat, &at);
                    least(&mut out, stat, &|out, v| {
                        let mut at = *out;
                        at[stat] = v;
                        tab(stat, &self.gear_stats(&at, rings)) * factor >= target
                    });
                }
                _ => least(&mut out, stat, &|out, v| eff(out, stat, v) >= floor),
            }
        }
        let spell_need = need
            .iter()
            .enumerate()
            .filter(|&(stat, _)| stat != ATT)
            .map(|(stat, &value)| (stat, value));
        for (stat, value) in require.iter().copied().chain(spell_need) {
            least(&mut out, stat, &|out, v| eff(out, stat, v) >= value);
        }
        out[ATT] = out[ATT].max(need[ATT]);
        Some(out)
    }

    /// Spell keys as indices into `self.spells`; `None` when one is unknown.
    fn spell_indices(&self, keys: &[String]) -> Option<Vec<usize>> {
        keys.iter()
            .map(|key| self.spells.iter().position(|spell| spell.key == *key))
            .collect()
    }

    /// The script's `slots_of`: the attunement slots `stats`' ATT gives.
    fn slots_of(&self, stats: &Stats) -> i32 {
        self.tables.attunement_slots.at(stats[ATT]) as i32
    }

    /// The script's `cast_power`: `catalyst`'s full-upgrade cast power in `element` (an index into
    /// [`Catalyst::power`]) at `stats`.
    fn cast_power(&self, catalyst: &Catalyst, element: usize, stats: &Stats) -> f64 {
        let (base, scale) = catalyst.power[element];
        let (int, fth) = (stats[INT], stats[FTH]);
        let at = match element {
            0 => int,
            1 => (int + fth).div_euclid(2),
            2 => fth,
            _ => int.min(fth),
        };
        base + scale * self.tables.cast[element].at(at)
    }

    /// The script's `best_catalysts`: for each spell category `spells` need, in category order,
    /// the catalyst with the most cast power there at `stats` among those that cast it and whose
    /// requirements `stats` meet, first in file order on a tie.
    fn best_catalysts(&self, spells: &[usize], stats: &Stats) -> Vec<CatalystPick> {
        let mut categories: Vec<i32> = spells
            .iter()
            .map(|&spell| self.spells[spell].category)
            .filter(|&category| category >= 0)
            .collect();
        categories.sort_unstable();
        categories.dedup();
        let mut out = Vec::new();
        for category in categories {
            let Some(&(school, element)) = usize::try_from(category)
                .ok()
                .and_then(|at| SPELL_SCHOOLS.get(at))
            else {
                continue;
            };
            // The best wieldable, and the best whether or not it is: what was passed over.
            let mut best: Option<(&Catalyst, f64)> = None;
            let mut top: Option<(&Catalyst, f64)> = None;
            for catalyst in &self.catalysts {
                if !catalyst.categories.contains(&category) {
                    continue;
                }
                let power = self.cast_power(catalyst, element, stats);
                if top.is_none_or(|(_, most)| power > most) {
                    top = Some((catalyst, power));
                }
                if catalyst
                    .require
                    .iter()
                    .any(|&(stat, value)| stats[stat] < value)
                {
                    continue;
                }
                if best.is_none_or(|(_, most)| power > most) {
                    best = Some((catalyst, power));
                }
            }
            if let Some((catalyst, power)) = best {
                out.push(CatalystPick {
                    school: school.to_owned(),
                    name: catalyst.name.clone(),
                    power: power as f32,
                    passed_over: top
                        .filter(|&(_, most)| most > power)
                        .map(|(over, _)| over.name.clone()),
                });
            }
        }
        out
    }

    /// The script's `_floors_at` for every stat: the least `optimize_build` lifts each to at
    /// `bracket` -- VIG, VIT and ADP always, ATT with `spells` ([`floored`]), END for a
    /// high-stamina weapon -- or all `0` when the floors are off.
    fn floor_stats(
        &self,
        bracket: &Bracket,
        weapon: &Weapon,
        floors: bool,
        spells: &[usize],
    ) -> Stats {
        let mut out = [0; STAT_COUNT];
        if floors {
            for (at, &stat) in floored(spells, &self.spells).iter().enumerate() {
                out[stat] = bracket.floors[at];
            }
            if weapon.high_stamina {
                out[END] = bracket.floors[4];
            }
        }
        out
    }

    /// The script's `_grip_req`: the weapon's requirements with STR halved (rounded up) two-handed.
    fn grip_require(weapon: &Weapon, two: bool) -> Stats {
        let mut out = [0; STAT_COUNT];
        for &(stat, need) in &weapon.require {
            out[stat] = if two && stat == STR {
                (need + 1).div_euclid(2)
            } else {
                need
            };
        }
        out
    }

    /// The script's `refusal`: why `optimize_build` finds nothing for these arguments, and each
    /// fix that, run through `optimize_build`, finds something. `None` when it finds a build.
    // DEBT: ds2-mods-rs-59p7 -- optimize_build's arguments, which want bundling as it does.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn refusal_of(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        grip: Grip,
        spells: &[usize],
        only_class: Option<&str>,
        floors: bool,
        reply: Reply,
    ) -> Option<Refusal> {
        let run = |sl: u16, spells: &[usize], class: Option<&str>, floors: bool| {
            self.optimize_build(
                weapon,
                infusion,
                u32::from(sl),
                objective,
                grip,
                spells,
                class,
                floors,
                // Who the objective is scored against never decides whether a build exists; the
                // script asks for the average defender, answering as its DEFENDER says.
                Against::average(reply),
            )
        };
        if run(sl, spells, only_class, floors).is_some() {
            return None;
        }
        let mut names: Vec<&str> = Vec::new();
        for &spell in spells {
            let name = self.spells[spell].name.as_str();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let spell_list = names.join(", ");
        let two = grip.two_handed();
        let require = Self::grip_require(weapon, two);
        let floors_at =
            |sl: u16| self.floor_stats(self.bracket(u32::from(sl)), weapon, floors, spells);
        let classes: Vec<usize> = (0..self.classes.len())
            .filter(|&at| {
                only_class.is_none_or(|only| self.classes[at].key.eq_ignore_ascii_case(only))
            })
            .collect();
        let mut need = self.spell_floors(spells, 0);
        // No ATT alone holds the spells but the sub ring with the most slots makes one that does:
        // the arithmetic is then the one with it worn, as the optimizer wears it.
        let band =
            self.sub_rings()
                .into_iter()
                .fold(None, |most: Option<usize>, ring| match most {
                    Some(top) if self.ring_slots(&[ring]) <= self.ring_slots(&[top]) => Some(top),
                    _ => Some(ring),
                });
        let band_slots = band.map_or(0, |ring| self.ring_slots(&[ring]));
        let mut band_note = None;
        if need.is_none()
            && band_slots != 0
            && let Some(ring) = band
        {
            need = self.spell_floors(spells, band_slots);
            if need.is_some() {
                band_note = Some(format!(
                    "no attunement alone holds {spell_list}; wearing a {} ({}) does",
                    self.rings[ring].1,
                    self.ring_effect_text(ring)
                ));
            }
        }
        // Points above `class`'s base the layers up to `layer` need at `sl`: 1 the weapon,
        // 2 and the spells, 3 and the floors.
        let cost = |class: usize, sl: u16, layer: u8| -> i32 {
            let base = &self.classes[class].base;
            let (need, floor) = (need.unwrap_or([0; STAT_COUNT]), floors_at(sl));
            (0..STAT_COUNT)
                .map(|stat| {
                    let mut value = base[stat].max(require[stat]);
                    if layer >= 2 {
                        value = value.max(need[stat]);
                    }
                    if layer >= 3 {
                        value = value.max(floor[stat]);
                    }
                    value - base[stat]
                })
                .sum()
        };
        let have = |class: usize, sl: u16| -> i32 {
            i32::from(sl) + 53 - self.classes[class].base.iter().sum::<i32>()
        };
        // The least SL a build fits at, per floors bracket, wearing the rings the optimizer would.
        let lift_require = Self::grip_require_pairs(weapon, two);
        let mut least: Vec<Option<Option<i32>>> = vec![None; SL_BRACKETS];
        let mut fits = |sl: u16| {
            let at = sl_bracket(u32::from(sl));
            let bound = *least[at].get_or_insert_with(|| {
                let floor = self.lift_floors(self.bracket(u32::from(sl)), weapon, floors, spells);
                self.least_sl_with_rings(&classes, &floor, &lift_require, spells)
            });
            bound.is_some_and(|bound| i32::from(sl) >= bound)
        };
        // Per stat: (raise over the base, the value, what lifted it) -- the script's `_raises`.
        let raises = |class: usize, need: &Stats| -> Vec<(usize, i32, i32, &'static str)> {
            let base = &self.classes[class].base;
            let floor = floors_at(sl);
            (0..STAT_COUNT)
                .map(|stat| {
                    let (mut value, mut source) = (base[stat], "");
                    for (name, least) in [
                        ("spells", need[stat]),
                        ("weapon", require[stat]),
                        ("floors", floor[stat]),
                    ] {
                        if least > value {
                            (value, source) = (least, name);
                        }
                    }
                    (stat, value - base[stat], value, source)
                })
                .collect()
        };
        let label = |stat: usize| STAT_LABELS[stat];
        let (mut lines, mut fixes) = (Vec::new(), Vec::new());
        let (kind, closest, short) = match need {
            None => {
                let total: i32 = spells.iter().map(|&spell| self.spells[spell].slots).sum();
                let most = self
                    .tables
                    .attunement_slots
                    .0
                    .iter()
                    .copied()
                    .fold(0.0, f64::max);
                let with_band = band
                    .filter(|_| band_slots != 0)
                    .map_or(String::new(), |ring| {
                        format!(
                            ", {} with a {}",
                            most as i64 + i64::from(band_slots),
                            self.rings[ring].1
                        )
                    });
                lines.push(format!(
                    "no attunement holds {spell_list}: they cost {total} slots, and ATT 99 gives {}{with_band}",
                    most as i64
                ));
                (RefusalKind::Slots, None, 0)
            }
            Some(need) => {
                let kind = [
                    (RefusalKind::Weapon, 1),
                    (RefusalKind::Spells, 2),
                    (RefusalKind::Floors, 3),
                ]
                .into_iter()
                .find(|&(_, layer)| {
                    classes
                        .iter()
                        .all(|&class| cost(class, sl, layer) > have(class, sl))
                })
                .map_or(RefusalKind::Floors, |(kind, _)| kind);
                // The least short class; the first in data order on a tie.
                let mut closest = *classes.first()?;
                for &class in &classes {
                    if cost(class, sl, 3) - have(class, sl)
                        < cost(closest, sl, 3) - have(closest, sl)
                    {
                        closest = class;
                    }
                }
                let short = cost(closest, sl, 3) - have(closest, sl);
                let class_name = &self.classes[closest].name;
                let wname = &weapon.name;
                let grip_word = if two { "two-handed" } else { "one-handed" };
                let mut head = match kind {
                    RefusalKind::Weapon => {
                        format!("{wname} cannot be wielded {grip_word} at SL {sl}")
                    }
                    RefusalKind::Spells => format!(
                        "{wname} can be wielded at SL {sl}, but not while casting {spell_list}"
                    ),
                    _ => format!(
                        "{wname}{} SL {sl}, but not above its typical-build minimums (the median \
                         {} of real builds at this level; not a game rule)",
                        if spells.is_empty() {
                            " fits".to_owned()
                        } else {
                            format!(" and {spell_list} fit")
                        },
                        floored(spells, &self.spells)
                            .iter()
                            .map(|&stat| label(stat))
                            .collect::<Vec<_>>()
                            .join("/")
                    ),
                };
                if only_class.is_some() {
                    head = format!("as a {class_name}: {head}");
                }
                lines.push(head);
                if let Some(note) = band_note.take() {
                    lines.push(note);
                }
                let lifted = raises(closest, &need);
                let mut groups = Vec::new();
                for (source, name) in [
                    ("floors", "floors"),
                    ("spells", spell_list.as_str()),
                    ("weapon", "weapon"),
                ] {
                    let got: Vec<String> = lifted
                        .iter()
                        .filter(|&&(_, raise, _, from)| from == source && raise > 0)
                        .map(|&(stat, _, value, _)| format!("{} {value}", label(stat)))
                        .collect();
                    if !got.is_empty() {
                        groups.push(format!("{name} {}", got.join(" ")));
                    }
                }
                lines.push(format!(
                    "SL {sl} is {short} points short for a {class_name}: {} need {} points above \
                     its base, SL {sl} gives {}",
                    groups.join(" + "),
                    cost(closest, sl, 3),
                    have(closest, sl)
                ));
                let mut top: Vec<_> = lifted.iter().filter(|r| r.1 > 0).copied().collect();
                top.sort_by_key(|r| -r.1);
                let source_name = |from: &str| match from {
                    "floors" => "floor".to_owned(),
                    "spells" => spell_list.clone(),
                    _ => "weapon".to_owned(),
                };
                lines.push(format!(
                    "most from {}",
                    top.iter()
                        .take(3)
                        .map(|&(stat, raise, _, from)| format!(
                            "{} +{raise} ({})",
                            label(stat),
                            source_name(from)
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                (kind, Some(closest), short)
            }
        };
        // 1. The least soul level that fits, nothing else changed.
        let up = (sl.saturating_add(1)..=SL_MAX).find(|&at| fits(at));
        match up {
            Some(up) if run(up, spells, only_class, floors).is_some() => {
                fixes.push(Fix {
                    change: Change::RaiseSl(up),
                    label: format!("Raise SL to {up}"),
                });
                // The floors this build is held to (no ATT without spells), as they read.
                let shown = |floor: &Stats| {
                    floored(spells, &self.spells)
                        .iter()
                        .map(|&stat| format!("{} {}", label(stat), floor[stat]))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let (then, now) = (shown(&floors_at(sl)), shown(&floors_at(up)));
                if floors && then != now {
                    lines.push(format!(
                        "the least SL that fits is {up}: the floors change with soul level ({then} \
                         at SL {sl}, {now} at SL {up})"
                    ));
                } else {
                    lines.push(format!("the least SL that fits is {up}"));
                }
            }
            _ if need.is_some() => lines.push(format!("no soul level up to {SL_MAX} fits it")),
            _ => {}
        }
        // 2. One spell fewer.
        let mut tried: Vec<usize> = Vec::new();
        for &spell in spells {
            if tried.contains(&spell) {
                continue;
            }
            tried.push(spell);
            let mut rest = spells.to_vec();
            if let Some(at) = rest.iter().position(|&s| s == spell) {
                rest.remove(at);
            }
            if run(sl, &rest, only_class, floors).is_some() {
                fixes.push(Fix {
                    change: Change::RemoveSpell(self.spells[spell].key.clone()),
                    label: format!("Remove {}", self.spells[spell].name),
                });
            }
        }
        // 3. No floors.
        if floors
            && let (Some(need), Some(closest)) = (need, closest)
            && run(sl, spells, only_class, false).is_some()
        {
            let binding: Vec<String> = raises(closest, &need)
                .into_iter()
                .filter(|&(_, raise, _, from)| from == "floors" && raise > 0)
                .map(|(stat, _, value, _)| format!("{} {value}", label(stat)))
                .collect();
            fixes.push(Fix {
                change: Change::IgnoreFloors,
                label: format!("Ignore typical-build minimums ({})", binding.join(" ")),
            });
        }
        // 4. Another class, when a class was asked for.
        // Checked as that class alone too: the rings chosen for any class can differ from the
        // rings chosen for one.
        if only_class.is_some()
            && let Some((_, class, ..)) = run(sl, spells, None, floors)
            && run(sl, spells, Some(self.classes[class].key.as_str()), floors).is_some()
        {
            fixes.push(Fix {
                change: Change::Class(self.classes[class].key.clone()),
                label: format!("Use a {} instead", self.classes[class].name),
            });
        }
        Some(Refusal {
            kind,
            class: closest.map(|class| self.classes[class].name.clone()),
            short,
            lines,
            fixes,
        })
    }

    /// The script's `optimize_build`: `(value, class, two-handed, stats, worn rings)`, or `None`
    /// when no class fits the floors, the weapon's requirements and what `spells` (indices into
    /// `self.spells`) need into `sl`, wearing what rings [`Self::choose_rings`] picks in place of
    /// stat points. `only_class` is the script's: that class key alone, for a character that
    /// already has one.
    /// `floors` false is the script's `use_floors=False`: the bracket floors are not applied.
    /// `defender` is who the damage objective is scored against ([`Self::defense_at`]).
    // DEBT: ds2-mods-rs-59p7 -- the class made this eight arguments; bundle the per-build options.
    #[allow(clippy::too_many_arguments)]
    fn optimize_build(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        sl: u32,
        objective: Objective,
        grip: Grip,
        spells: &[usize],
        only_class: Option<&str>,
        floors: bool,
        defender: Against,
    ) -> Option<Best> {
        let classes = self.class_indices(only_class);
        let floor = self.lift_floors(self.bracket(sl), weapon, floors, spells);
        let require = Self::grip_require_pairs(weapon, grip.two_handed());
        let run = |rings: &[usize]| {
            self.optimize_with(
                weapon, infusion, sl, objective, grip, spells, &classes, &floor, &require, rings,
                defender,
            )
        };
        let offense = self.offense_rings(weapon, infusion, objective);
        self.choose_rings(&classes, &floor, &require, spells, &run, &offense)
    }

    /// The classes `only_class` allows, as indices, in file order.
    fn class_indices(&self, only_class: Option<&str>) -> Vec<usize> {
        (0..self.classes.len())
            .filter(|&at| {
                only_class.is_none_or(|only| self.classes[at].key.eq_ignore_ascii_case(only))
            })
            .collect()
    }

    /// The floors `ring_lift` lifts to, in the script's `need_floors` order: VIG, VIT, ADP, ATT
    /// with `spells`, then END for a high-stamina weapon; each `0` when the floors are off.
    fn lift_floors(
        &self,
        bracket: &Bracket,
        weapon: &Weapon,
        floors: bool,
        spells: &[usize],
    ) -> Vec<(usize, i32)> {
        let at = self.floor_stats(bracket, weapon, floors, spells);
        let mut out: Vec<(usize, i32)> = floored(spells, &self.spells)
            .iter()
            .map(|&stat| (stat, at[stat]))
            .collect();
        if weapon.high_stamina {
            out.push((END, at[END]));
        }
        out
    }

    /// The script's `_grip_req` as `(stat, value)` pairs in the weapon's order.
    fn grip_require_pairs(weapon: &Weapon, two: bool) -> Vec<(usize, i32)> {
        weapon
            .require
            .iter()
            .map(|&(stat, need)| {
                (
                    stat,
                    if two && stat == STR {
                        (need + 1).div_euclid(2)
                    } else {
                        need
                    },
                )
            })
            .collect()
    }

    /// The script's `lift_points`: per class, the points `ring_lift` raises its base by.
    fn lift_points(
        &self,
        classes: &[usize],
        floors: &[(usize, i32)],
        require: &[(usize, i32)],
        spells: &[usize],
        rings: &[usize],
    ) -> Vec<i32> {
        classes
            .iter()
            .map(|&class| {
                let base = &self.classes[class].base;
                self.ring_lift(base, floors, require, spells, rings)
                    .map_or(UNREACHABLE, |up| {
                        up.iter().sum::<i32>() - base.iter().sum::<i32>()
                    })
            })
            .collect()
    }

    /// The script's `next_ring`: the sub ring, outside the groups of those `worn` and with fewer
    /// than four worn, that frees the most points (first on a tie), when it frees at least
    /// [`RING_SLOT_POINTS`].
    fn next_ring(
        &self,
        classes: &[usize],
        floors: &[(usize, i32)],
        require: &[(usize, i32)],
        spells: &[usize],
        worn: &[usize],
    ) -> Option<usize> {
        if worn.len() >= 4 {
            return None;
        }
        let taken: Vec<&str> = worn.iter().map(|&r| self.ring_groups[r].as_str()).collect();
        let now = self.lift_points(classes, floors, require, spells, worn);
        let mut pick: Option<(i32, usize)> = None;
        for ring in self.sub_rings() {
            if taken.contains(&self.ring_groups[ring].as_str()) {
                continue;
            }
            let mut with = worn.to_vec();
            with.push(ring);
            let freed = now
                .iter()
                .zip(self.lift_points(classes, floors, require, spells, &with))
                .map(|(a, b)| a - b)
                .max()?;
            if freed >= RING_SLOT_POINTS && pick.is_none_or(|(most, _)| freed > most) {
                pick = Some((freed, ring));
            }
        }
        pick.map(|(_, ring)| ring)
    }

    /// The script's `choose_rings`: one `next_ring` at a time, each set run with the slots it
    /// leaves holding the first of `offense`; worn while no build fits, and once one does, only
    /// when `run` scores the objective higher with it -- so a stand-in that takes an offensive
    /// ring's slot has to beat that ring.
    fn choose_rings(
        &self,
        classes: &[usize],
        floors: &[(usize, i32)],
        require: &[(usize, i32)],
        spells: &[usize],
        run: &dyn Fn(&[usize]) -> Option<Best>,
        offense: &[usize],
    ) -> Option<Best> {
        let fill = |rings: &[usize]| -> Vec<usize> {
            let mut out = rings.to_vec();
            out.extend(offense.iter().take(4usize.saturating_sub(rings.len())));
            out
        };
        let mut worn: Vec<usize> = Vec::new();
        let mut best = run(&fill(&worn));
        while let Some(ring) = self.next_ring(classes, floors, require, spells, &worn) {
            let mut with = worn.clone();
            with.push(ring);
            let got = run(&fill(&with));
            if let Some((top, ..)) = best
                && got.as_ref().is_none_or(|(value, ..)| *value <= top)
            {
                break;
            }
            worn = with;
            best = got;
        }
        best
    }

    /// The script's `least_sl_with_rings`: the least soul level at which a build fits under
    /// `floors`, wearing each `next_ring` in turn while none does.
    fn least_sl_with_rings(
        &self,
        classes: &[usize],
        floors: &[(usize, i32)],
        require: &[(usize, i32)],
        spells: &[usize],
    ) -> Option<i32> {
        let mut worn: Vec<usize> = Vec::new();
        let mut best: Option<i32> = None;
        loop {
            for (&class, points) in classes
                .iter()
                .zip(self.lift_points(classes, floors, require, spells, &worn))
            {
                if points < UNREACHABLE {
                    let sl = points - 53 + self.classes[class].base.iter().sum::<i32>();
                    best = Some(best.map_or(sl, |least| least.min(sl)));
                }
            }
            match self.next_ring(classes, floors, require, spells, &worn) {
                Some(ring) => worn.push(ring),
                None => return best,
            }
        }
    }

    /// The script's `_optimize_with`: the search at one set of worn `rings`.
    // DEBT: ds2-mods-rs-59p7 -- optimize's arguments plus the rings; bundle the per-build options.
    #[allow(clippy::too_many_arguments)]
    fn optimize_with(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        sl: u32,
        objective: Objective,
        grip: Grip,
        spells: &[usize],
        classes: &[usize],
        floor: &[(usize, i32)],
        require: &[(usize, i32)],
        rings: &[usize],
        defender: Against,
    ) -> Option<Best> {
        // What the worn rings give: every curve reads it, the flexibility term the levelled stats.
        let worn = |st: &Stats| -> Stats {
            if rings.is_empty() {
                *st
            } else {
                self.gear_stats(st, rings)
            }
        };
        #[derive(Clone, Copy, PartialEq)]
        enum Curve {
            Objective,
            Agility,
            HitPoints,
            Stamina,
            Load,
        }
        let defending = self.defending(sl, defender);
        let adaptability = if objective == Objective::Poison {
            Curve::Objective
        } else {
            Curve::Agility
        };
        // The script's `curves`, in its order. END by max stamina and VIT by max equip load, each
        // over its own early rate as VIG is, so leftover points past VIG's soft cap do not all
        // pile into VIG; END has no curve without the regulation's stamina table.
        let mut curves = vec![
            (STR, Curve::Objective),
            (DEX, Curve::Objective),
            (INT, Curve::Objective),
            (FTH, Curve::Objective),
            (ADP, adaptability),
            (VIG, Curve::HitPoints),
        ];
        if !self.tables.stamina_max.0.is_empty() {
            curves.push((END, Curve::Stamina));
        }
        curves.push((VIT, Curve::Load));
        curves.push((ATT, Curve::Agility));
        let value = |curve: Curve, st: &Stats| -> f64 {
            let st = worn(st);
            match curve {
                Curve::Objective => {
                    self.objective_value(weapon, infusion, &st, objective, &defending, rings)
                }
                Curve::Agility => f64::from(agility(st[ADP], st[ATT])),
                Curve::HitPoints => self.hit_points(&st),
                Curve::Stamina => self.tables.stamina_max.at(st[END]),
                Curve::Load => self.tables.equip_load.at(st[VIT]),
            }
        };
        let with = |st: &Stats, stat: usize, to: i32| {
            let mut next = *st;
            next[stat] = to;
            next
        };
        let sl = i32::try_from(sl).unwrap_or(i32::MAX - 53);
        let mut best: Option<Best> = None;
        for &class_index in classes {
            let class = &self.classes[class_index];
            // The script's GRIP_TRIES: one grip, never a fallback to the other.
            {
                let two = grip.two_handed();
                let mut st = self.ring_lift(&class.base, floor, require, spells, rings)?;
                let mut free = sl + 53 - st.iter().sum::<i32>();
                if free < 0 {
                    continue;
                }
                // Each stat weighted by its own curve's early rate, from 5 to 25.
                let mut peak: Vec<f64> = curves
                    .iter()
                    .map(|&(stat, curve)| {
                        py_max(
                            (value(curve, &with(&st, stat, 25))
                                - value(curve, &with(&st, stat, 5)))
                                / 20.0,
                            1e-9,
                        )
                    })
                    .collect();
                // The stats that feed the objective share one unit, the steepest of their early
                // rates: a point of damage is a point of damage whichever stat buys it.
                let mut shared = curves
                    .iter()
                    .zip(peak.iter().copied())
                    .filter(|&(&(_, curve), _)| curve == Curve::Objective)
                    .fold(f64::NEG_INFINITY, |most, (_, rate)| py_max(most, rate));
                // Dark reads min(INT, FTH), so a point of INT alone or FTH alone buys no dark and a
                // one-stat step never finds it. With a dark attack the two also move together, a
                // step of n each costing 2n.
                let dark =
                    self.attack_rating(weapon.infusion(infusion), &worn(&st), rings)[4].is_some();
                if dark {
                    let early = (value(Curve::Objective, &both_at(&st, 25))
                        - value(Curve::Objective, &both_at(&st, 5)))
                        / 40.0;
                    shared = py_max(shared, early);
                }
                for (at, &(_, curve)) in curves.iter().enumerate() {
                    if curve == Curve::Objective {
                        peak[at] = shared;
                    }
                }
                if adaptability == Curve::Agility {
                    // ATT is the last curve and ADP the fifth, in the script's order.
                    let last = peak.len() - 1;
                    peak[last] = peak[4];
                }
                while free > 0 {
                    let mut pick: Option<(usize, i32)> = None;
                    let mut best_w = 0.0;
                    for (at, &(stat, curve)) in curves.iter().enumerate() {
                        let current = value(curve, &st);
                        // Only a requirement stat changes what the build wields; the term is left
                        // off the rest rather than added as a zero, as the script leaves it.
                        let flex_now = (FLEX_WEIGHT != 0.0 && REQ_STATS.contains(&stat))
                            .then(|| self.flex_score(&st));
                        for n in 1..=free.min(99 - st[stat]).min(8) {
                            let next = with(&st, stat, st[stat] + n);
                            let mut w = (value(curve, &next) - current) / f64::from(n) / peak[at];
                            if let Some(now) = flex_now {
                                w += FLEX_WEIGHT * f64::from(self.flex_score(&next) - now)
                                    / f64::from(n);
                            }
                            if w > best_w + 1e-12 {
                                pick = Some((stat, n));
                                best_w = w;
                            }
                        }
                    }
                    if dark {
                        let current = value(Curve::Objective, &st);
                        let flex_now = (FLEX_WEIGHT != 0.0).then(|| self.flex_score(&st));
                        for n in 1..=(free / 2).min(99 - st[INT].max(st[FTH])).min(8) {
                            let next = both_up(&st, n);
                            let cost = f64::from(2 * n);
                            let mut w = (value(Curve::Objective, &next) - current) / cost / shared;
                            if let Some(now) = flex_now {
                                w += FLEX_WEIGHT * f64::from(self.flex_score(&next) - now) / cost;
                            }
                            if w > best_w + 1e-12 {
                                pick = Some((BOTH, n));
                                best_w = w;
                            }
                        }
                    }
                    // Every curve is flat: the points go to vigor.
                    let pick = pick.or_else(|| {
                        [VIG, VIT, END, ATT]
                            .into_iter()
                            .find(|&stat| st[stat] < 99)
                            .map(|stat| (stat, 1))
                    });
                    let Some((stat, n)) = pick else {
                        break;
                    };
                    if stat == BOTH {
                        st = both_up(&st, n);
                        free -= 2 * n;
                        continue;
                    }
                    st[stat] += n;
                    free -= n;
                }
                let val = self.objective_value(
                    weapon,
                    infusion,
                    &worn(&st),
                    objective,
                    &defending,
                    rings,
                );
                if best.as_ref().is_none_or(|(top, ..)| val > *top) {
                    best = Some((val, class_index, two, st, rings.to_vec()));
                }
            }
        }
        best
    }

    /// The script's `nearest_builds`: the `k` builds in `sl`'s bracket nearest `stats`.
    fn nearest(&self, stats: &Stats, sl: u32, k: usize) -> Vec<&CorpusBuild> {
        let query = stats.map(stat_bracket);
        let bracket = sl_bracket(sl);
        let mut same: Vec<(i32, &CorpusBuild)> = self
            .corpus
            .iter()
            .filter(|build| build.bracket == bracket)
            .map(|build| {
                let distance = build
                    .stat_brackets
                    .iter()
                    .zip(query)
                    .map(|(&have, want)| (have - want).abs())
                    .sum();
                (distance, build)
            })
            .collect();
        same.sort_by_key(|&(distance, _)| distance);
        same.into_iter().take(k).map(|(_, build)| build).collect()
    }

    /// Every weapon a hand can hold: the script's `flex_pool`, `Bare_Fists` and the other empty
    /// keys left out, shields and catalysts in.
    fn flex_pool(&self) -> impl Iterator<Item = &Weapon> {
        self.weapons
            .iter()
            .filter(|weapon| !EMPTY.contains(&weapon.key.as_str()))
    }

    /// The script's `flex_counts`: how many weapons `stats` wield one-handed and two-handed.
    /// The script's `load_scarcity`: 0 at [`LOAD_SCARCE_FROM`] or more weapons wielded one-handed,
    /// 1 at [`LOAD_SCARCE_FULL`] or fewer, linear between.
    fn load_scarcity(&self, stats: &Stats) -> f64 {
        let one = i32::try_from(self.flex_counts(stats).0).unwrap_or(i32::MAX);
        (f64::from(LOAD_SCARCE_FROM - one) / f64::from(LOAD_SCARCE_FROM - LOAD_SCARCE_FULL))
            .clamp(0.0, 1.0)
    }

    /// The armour keys, head to legs, a build holding `weapon` alone and no rings wears at `stats`
    /// and `scarcity` (the build's own load scarcity when `None`), for the parity tests. `None` for
    /// an unknown weapon or when nothing fits.
    #[doc(hidden)]
    pub fn armor_for(
        &self,
        weapon: &str,
        stats: &[u16; STAT_COUNT],
        scarcity: Option<f64>,
    ) -> Option<[String; 4]> {
        let stats = to_stats(stats);
        let weapon = self.weapon_by_key(weapon)?;
        let scarcity = scarcity.unwrap_or_else(|| self.load_scarcity(&stats));
        let (_, _, set) = self.best_armor(&stats, weapon, &[], scarcity);
        set.map(|set| set.map(|piece| piece.key.clone()))
    }

    fn flex_counts(&self, stats: &Stats) -> (u32, u32) {
        let mut counts = (0, 0);
        for weapon in self.flex_pool() {
            counts.0 += u32::from(weapon.wieldable(stats, false));
            counts.1 += u32::from(weapon.wieldable(stats, true));
        }
        counts
    }

    /// One-handed plus two-handed: what the optimizer's flexibility term and the neighbour ranking
    /// compare.
    fn flex_score(&self, stats: &Stats) -> i32 {
        let (one, two) = self.flex_counts(stats);
        i32::try_from(one + two).unwrap_or(i32::MAX)
    }

    /// The script's `flexibility`, with the armour and rings named rather than keyed: a piece is
    /// the first of its slot with that name, a ring the first with that name, and a name that is
    /// neither weighs nothing, as the script's `Naked` does. `None` when the data file carries no
    /// neighbour counts.
    fn flex(
        &self,
        stats: &Stats,
        sl: u32,
        armor: &[String],
        rings: &[String],
    ) -> Option<Flexibility> {
        let (one, two) = self.flex_counts(stats);
        let score = one + two;
        let mut scores = Vec::new();
        for build in self.nearest(stats, sl, FLEX_K) {
            let (a, b) = build.flex?;
            scores.push(a + b);
        }
        let below = scores.iter().filter(|&&s| s < score).count();
        let equal = scores.iter().filter(|&&s| s == score).count();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let n = scores.len();
        let percentile = if n == 0 {
            50.0
        } else {
            100.0 * (f64::from(count(below)) + f64::from(count(equal)) / 2.0) / f64::from(count(n))
        };
        // Taken off one at a time, in the script's order: the script does not use `sum()`, whose
        // compensated float total would round differently.
        let worn: Vec<usize> = rings
            .iter()
            .filter_map(|name| self.rings.iter().position(|ring| ring.1 == *name))
            .collect();
        let mut spare = self.max_load(stats, &worn) * EQUIP_CAP;
        for (name, slot) in armor.iter().zip(&self.wearable) {
            if let Some(piece) = slot.iter().find(|piece| piece.name == *name) {
                spare -= piece.weight;
            }
        }
        for name in rings {
            if let Some(ring) = self.rings.iter().find(|ring| ring.1 == *name) {
                spare -= ring.2;
            }
        }
        let fits = self
            .flex_pool()
            .filter(|weapon| weapon.wieldable(stats, true) && weapon.weight <= spare)
            .count();
        Some(Flexibility {
            one_handed: one,
            two_handed: two,
            total: count(self.flex_pool().count()),
            below: count(below),
            equal: count(equal),
            neighbours: count(n),
            percentile,
            spare_load: spare,
            fits: count(fits),
        })
    }

    /// The script's `minimum_build` with no AGL target: per class, the lowest SL that wields
    /// `weapon` with the armour and rings that help, cheapest first. `floors` raises each to its
    /// bracket's floors, as the corpus-typical search does: `Some(true)` END too, for a
    /// high-stamina weapon.
    fn minimum_build(
        &self,
        weapon: &Weapon,
        two_hand: bool,
        floors: Option<bool>,
    ) -> Vec<MinimumPick> {
        let mut require = weapon.require.clone();
        if two_hand {
            for (stat, value) in &mut require {
                if *stat == STR {
                    *value = value.div_euclid(2);
                }
            }
        }
        let mut ring_opts: Vec<Vec<usize>> = vec![Vec::new()];
        for size in 1..=self.ring_effects.len().min(4) {
            combinations(self.ring_effects.len(), size, &mut ring_opts);
        }
        let useful = |stat: usize| {
            require.iter().any(|&(needed, _)| needed == stat) || [VIT, ADP, ATT].contains(&stat)
        };
        let armor_opts: Vec<Vec<Option<&ArmorPiece>>> = self
            .armor
            .iter()
            .map(|slot| {
                std::iter::once(None)
                    .chain(
                        slot.iter()
                            .filter(|piece| {
                                piece
                                    .alter
                                    .iter()
                                    .enumerate()
                                    .any(|(stat, &x)| useful(stat) && x > 0)
                            })
                            .map(Some),
                    )
                    .collect()
            })
            .collect();
        let mut sets: Vec<[Option<&ArmorPiece>; 4]> = Vec::new();
        for &head in &armor_opts[0] {
            for &chest in &armor_opts[1] {
                for &hands in &armor_opts[2] {
                    for &legs in &armor_opts[3] {
                        sets.push([head, chest, hands, legs]);
                    }
                }
            }
        }
        let mut results: Vec<MinimumPick> = Vec::new();
        for (class_index, class) in self.classes.iter().enumerate() {
            let mut best: Option<MinimumPick> = None;
            for arm in &sets {
                for rings in &ring_opts {
                    let (sl, stats) = self.minimum_at(weapon, class, &require, arm, rings, floors);
                    if best.as_ref().is_none_or(|best| sl < best.sl) {
                        best = Some(MinimumPick {
                            sl,
                            class: class_index,
                            stats,
                            armor: arm
                                .iter()
                                .flatten()
                                .map(|piece| piece.name.clone())
                                .collect(),
                            rings: rings.clone(),
                        });
                    }
                }
            }
            results.extend(best);
        }
        results.sort_by_key(|pick| pick.sl);
        results
    }

    /// One class, one set of armour and rings, in `minimum_build`: the SL and stats it takes.
    fn minimum_at(
        &self,
        weapon: &Weapon,
        class: &Class,
        require: &[(usize, i32)],
        arm: &[Option<&ArmorPiece>; 4],
        rings: &[usize],
        floors: Option<bool>,
    ) -> (i32, Stats) {
        let base = class.base;
        let mut bonus = [0; STAT_COUNT];
        for piece in arm.iter().flatten() {
            for (stat, value) in piece.alter.iter().enumerate() {
                bonus[stat] += value;
            }
        }
        let mut mul = 1.0;
        for &ring in rings {
            let effect = &self.ring_effects[ring];
            for (stat, value) in effect.add.iter().enumerate() {
                bonus[stat] += value;
            }
            mul *= effect.load_mul;
        }
        // Raise each requirement stat until the stat with gear meets it; armour has its own.
        let mut need = require.to_vec();
        for piece in arm.iter().flatten() {
            for &(stat, value) in &piece.require {
                match need.iter_mut().find(|(needed, _)| *needed == stat) {
                    Some((_, have)) => *have = (*have).max(value),
                    None => need.push((stat, value)),
                }
            }
        }
        let mut required = base;
        for &(stat, value) in &need {
            required[stat] = required[stat].max(value - bonus[stat]);
        }
        let armor_weight = arm
            .iter()
            .map(|piece| piece.map_or(0.0, |piece| piece.weight))
            .fold(0.0, |total, weight| total + weight);
        let ring_weight = rings
            .iter()
            .map(|&ring| self.ring_effects[ring].weight)
            .fold(0.0, |total, weight| total + weight);
        let load = weapon.weight + armor_weight + ring_weight;
        // The floors depend on the SL bracket, which depends on the floors.
        let floors_end = floors == Some(true);
        let mut used: Option<usize> = None;
        let mut stats = required;
        let mut sl = 0;
        for _ in 0..8 {
            stats = required;
            if let Some(used) = used {
                let floors = &self.brackets[used].floors;
                for (at, stat) in FLOOR_STATS.into_iter().enumerate() {
                    stats[stat] = stats[stat].max(floors[at]);
                }
                if floors_end {
                    stats[END] = stats[END].max(floors[4]);
                }
            }
            while self.tables.equip_load.at((stats[VIT] + bonus[VIT]).min(99)) * mul * EQUIP_CAP
                < load
            {
                stats[VIT] += 1;
                if stats[VIT] > 99 {
                    break;
                }
            }
            sl = class.level
                + stats
                    .iter()
                    .zip(base)
                    .map(|(have, from)| have - from)
                    .sum::<i32>();
            let bracket = sl_bracket(u32::try_from(sl.max(0)).unwrap_or(0));
            if floors.is_none() || used == Some(bracket) {
                break;
            }
            used = Some(bracket);
        }
        (sl, stats)
    }

    /// The script's `recommended_minimum` with no SL given: the cheapest class's hard minimum,
    /// clamped at that class's corpus-typical SL, with the free stats spread the corpus-typical way.
    fn recommended_minimum(&self, weapon: &Weapon, two_hand: bool) -> Option<OptimizedBuild> {
        let high = weapon.high_stamina;
        let hard = self.minimum_build(weapon, two_hand, None);
        let typical = self.minimum_build(weapon, two_hand, Some(high));
        let pick = hard.first()?;
        let target_sl = typical.iter().find(|typ| typ.class == pick.class)?.sl;
        let class = &self.classes[pick.class];
        let hard_sl = class.level
            + pick
                .stats
                .iter()
                .zip(class.base)
                .map(|(have, from)| have - from)
                .sum::<i32>();
        let left = target_sl - hard_sl;
        if left < 0 {
            return None;
        }
        let floors = &self.bracket(u32::try_from(target_sl).unwrap_or(0)).floors;
        let mut targets = [0; STAT_COUNT];
        for (at, stat) in FLOOR_STATS.into_iter().enumerate() {
            targets[stat] = floors[at];
        }
        targets[END] = if high { floors[4] } else { pick.stats[END] };
        let mut stats = pick.stats;
        for _ in 0..left {
            let free: Vec<(usize, &str)> = FREE_STATS
                .into_iter()
                .filter(|&(stat, _)| stats[stat] < 99)
                .collect();
            // Biggest shortfall against the bracket's median first, a tie to the later name; once
            // every median is met, the stat furthest under its median's proportion, a tie to the
            // earlier name. Python's `max` and `min` over `(value, name)`.
            let Some(&(short, _)) = free.iter().max_by(|a, b| {
                (targets[a.0] - stats[a.0], a.1).cmp(&(targets[b.0] - stats[b.0], b.1))
            }) else {
                break;
            };
            let stat = if targets[short] - stats[short] > 0 {
                short
            } else {
                let ratio = |stat: usize| f64::from(stats[stat]) / f64::from(targets[stat].max(1));
                let Some(&(under, _)) = free
                    .iter()
                    .min_by(|a, b| ratio(a.0).total_cmp(&ratio(b.0)).then_with(|| a.1.cmp(b.1)))
                else {
                    break;
                };
                under
            };
            stats[stat] += 1;
        }
        let ring_names = pick.rings.iter().filter_map(|&ring| {
            let key = &self.ring_effects[ring].key;
            self.rings
                .iter()
                .find(|(ring_key, ..)| ring_key == key)
                .map(|(_, name, _)| name.clone())
        });
        Some(OptimizedBuild {
            class: class.name.clone(),
            sl: u16::try_from(target_sl).unwrap_or(0),
            stats: stats.map(|value| u16::try_from(value).unwrap_or(0)),
            two_handed: two_hand,
            value: 0.0,
            gear: pick.armor.iter().cloned().chain(ring_names).collect(),
        })
    }
}

/// One armour set: its threat-weighted defense, its weight, and its pieces head to legs.
type ArmorSet<'a> = (f64, f64, [&'a Wearable; 4]);

impl CorpusBackend {
    /// The script's `best_armor` with `top=1`: the equip-load cap, what the weapon and rings
    /// carry, and the set, head to legs, whose defense weighted by the corpus threat mix is
    /// highest among those these stats can wear under the cap. `None` when nothing fits, which
    /// only happens when the weapon and rings alone are over it: `Naked` is a piece in every slot.
    /// `scarcity` is the script's: above 0, a set scores its defense less `scarcity` x
    /// [`LOAD_PRICE`] x the best set's defense per weight x its weight.
    fn best_armor(
        &self,
        stats: &Stats,
        weapon: &Weapon,
        rings: &[usize],
        scarcity: f64,
    ) -> (f64, f64, Option<[&Wearable; 4]>) {
        // The rings count: their load factor and VIT bonus in the cap, their stat bonuses toward
        // the armour's requirements.
        let cap = self.max_load(stats, rings) * EQUIP_CAP;
        let stats = &self.gear_stats(stats, rings);
        let ring_weight = rings
            .iter()
            .map(|&ring| self.rings[ring].2)
            .fold(0.0, |total, weight| total + weight);
        let carried = weapon.weight + ring_weight;
        let budget = cap - carried;
        let value = |piece: &Wearable| {
            piece
                .defense
                .iter()
                .zip(self.threat_mix)
                .fold(0.0, |total, (defense, share)| total + share * defense)
        };
        // Per slot, only the pieces that beat every lighter one.
        let fronts: Vec<Vec<(f64, f64, &Wearable)>> = self
            .wearable
            .iter()
            .map(|slot| {
                let mut candidates: Vec<(f64, f64, &Wearable)> = slot
                    .iter()
                    .filter(|piece| {
                        piece
                            .require
                            .iter()
                            .all(|&(stat, need)| need <= stats[stat])
                    })
                    .map(|piece| (piece.weight, value(piece), piece))
                    .collect();
                candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.total_cmp(&a.1)));
                let mut front = Vec::new();
                let mut best = -1.0;
                for candidate in candidates {
                    if candidate.1 > best {
                        best = candidate.1;
                        front.push(candidate);
                    }
                }
                front
            })
            .collect();
        fn keys<'a>(set: &[&'a Wearable; 4]) -> [&'a str; 4] {
            set.map(|piece| piece.key.as_str())
        }
        let mut sets: Vec<ArmorSet<'_>> = Vec::new();
        for h in &fronts[0] {
            for c in &fronts[1] {
                if h.0 + c.0 > budget {
                    break;
                }
                for g in &fronts[2] {
                    if h.0 + c.0 + g.0 > budget {
                        break;
                    }
                    for l in &fronts[3] {
                        let weight = h.0 + c.0 + g.0 + l.0;
                        if weight > budget {
                            break;
                        }
                        sets.push((h.1 + c.1 + g.1 + l.1, weight, [h.2, c.2, g.2, l.2]));
                    }
                }
            }
        }
        // Python's descending sort of `(value, weight, keys)`, first place.
        let first = |score: &dyn Fn(&ArmorSet<'_>) -> f64| {
            sets.iter()
                .max_by(|a, b| {
                    score(a)
                        .total_cmp(&score(b))
                        .then(a.0.total_cmp(&b.0))
                        .then(a.1.total_cmp(&b.1))
                        .then_with(|| keys(&a.2).cmp(&keys(&b.2)))
                })
                .copied()
        };
        let mut top = first(&|set| set.0);
        if scarcity > 0.0
            && let Some((value, weight, _)) = top
        {
            // The script's re-sort by `(value - scarcity * LOAD_PRICE * price * weight, value, ...)`.
            let price = if weight > 0.0 { value / weight } else { 0.0 };
            top = first(&|set| set.0 - scarcity * LOAD_PRICE * price * set.1);
        }
        (cap, carried, top.map(|(_, _, set)| set))
    }

    /// The script's `suggest_rings`: the [`crate::backend::SUGGESTED_RINGS`] rings `near` counts
    /// most, each no-use ring's place going, in place, to the ring the nearest builds wear most
    /// (then all builds) that is neither a no-use ring nor in the upgrade group of one already in
    /// the list. Rings in the group of one already `worn` are left out, and only the slots `worn`
    /// leaves are filled.
    fn suggest_rings(&self, near: &[(usize, u32)], worn: &[usize]) -> Vec<usize> {
        let group = |ring: usize| self.ring_groups[ring].as_str();
        let held: Vec<&str> = worn.iter().map(|&ring| group(ring)).collect();
        let near: Vec<(usize, u32)> = near
            .iter()
            .copied()
            .filter(|&(ring, _)| !held.contains(&group(ring)))
            .collect();
        let top: Vec<usize> = near
            .iter()
            .take(crate::backend::SUGGESTED_RINGS.saturating_sub(worn.len()))
            .map(|&(ring, _)| ring)
            .collect();
        let mut taken: Vec<&str> = top
            .iter()
            .filter(|ring| !self.no_use.contains(ring))
            .map(|&ring| group(ring))
            .collect();
        taken.extend(&held);
        let every: Vec<(usize, u32)> = ring_counts(&self.corpus)
            .into_iter()
            .filter(|&(ring, _)| !held.contains(&group(ring)))
            .collect();
        let mut out = Vec::with_capacity(top.len());
        for ring in top {
            if !self.no_use.contains(&ring) {
                out.push(ring);
                continue;
            }
            let Some(instead) = near
                .iter()
                .chain(&every)
                .map(|&(candidate, _)| candidate)
                .find(|&c| !self.no_use.contains(&c) && !taken.contains(&group(c)))
            else {
                continue;
            };
            taken.push(group(instead));
            out.push(instead);
        }
        out
    }

    /// The script's `generate_armor`: the pieces' names head to legs, `Naked` for a slot left
    /// bare, and the note that says why a slot or the whole set is bare.
    fn generate_armor(
        &self,
        stats: &Stats,
        weapon: &Weapon,
        rings: &[usize],
    ) -> (Vec<String>, Option<String>) {
        let (cap, carried, set) = self.best_armor(stats, weapon, rings, self.load_scarcity(stats));
        let percent = EQUIP_CAP * 100.0;
        let Some(set) = set else {
            return (
                Vec::new(),
                Some(format!(
                    "no armor fits: the weapon and rings weigh {carried:.1}, over the {cap:.1} a \
                     {percent:.0}% load allows at VIT {}",
                    stats[VIT]
                )),
            );
        };
        let bare: Vec<&str> = ARMOR_SLOTS
            .iter()
            .zip(set)
            .filter(|(_, piece)| piece.key == "Naked")
            .map(|(slot, _)| *slot)
            .collect();
        let note = (!bare.is_empty()).then(|| {
            format!(
                "{} left bare: nothing wearable there fits the {:.1} of load left under \
                 {percent:.0}%",
                bare.join(", "),
                cap - carried
            )
        });
        (set.iter().map(|piece| piece.name.clone()).collect(), note)
    }
}

/// Every `size`-combination of `0..n`, in `itertools.combinations` order, appended to `out`.
fn combinations(n: usize, size: usize, out: &mut Vec<Vec<usize>>) {
    if size > n {
        return;
    }
    let mut pick: Vec<usize> = (0..size).collect();
    loop {
        out.push(pick.clone());
        let Some(at) = (0..size).rev().find(|&at| pick[at] != at + n - size) else {
            return;
        };
        pick[at] += 1;
        for next in at + 1..size {
            pick[next] = pick[next - 1] + 1;
        }
    }
}

/// The panel's stats at the script's integer type.
fn to_stats(stats: &[u16; STAT_COUNT]) -> Stats {
    stats.map(i32::from)
}

/// `window_s` as the script would read it typed: the shortest decimal that is this `f32`, so a
/// typed `1.1` compares as the double `1.1` and not as `1.100000023841858`.
fn window_seconds(window: f32) -> f64 {
    window.to_string().parse().unwrap_or(0.0)
}

impl RecommenderBackend for CorpusBackend {
    fn is_stub(&self) -> bool {
        false
    }

    fn floors(&self, sl: u16) -> [u16; STAT_COUNT] {
        let floors = &self.bracket(u32::from(sl)).floors;
        let mut out = [0; STAT_COUNT];
        for (at, stat) in FLOOR_STATS.into_iter().enumerate() {
            out[stat] = u16::try_from(floors[at]).unwrap_or(0);
        }
        out
    }

    fn weapons_for(&self, stats: &[u16; STAT_COUNT], sl: u16, opts: &WeaponsForOpts) -> Outcome {
        // A defender wearing a piece this file does not have ranks nothing rather than something
        // scored against a defender nobody chose.
        let Some(defender) = self.worn(&opts.defender) else {
            return Outcome::Rows(Vec::new());
        };
        let query = Query {
            one_hand: opts.one_hand,
            class: opts.class.as_deref(),
            per_class: opts.per_class,
            window: window_seconds(opts.window_s),
            raw_ar: opts.raw_ar,
            objective: opts.objective,
            top: WEAPONS_FOR_TOP,
            weapon: None,
            defender: Against {
                worn: defender,
                reply: opts.reply,
            },
        };
        let ranked = self.rank(&to_stats(stats), u32::from(sl), &query);
        Outcome::Rows(ranked.iter().map(|row| self.result_row(row)).collect())
    }

    fn best_infusion(
        &self,
        weapon: &str,
        stats: &[u16; STAT_COUNT],
        sl: u16,
        opts: &WeaponsForOpts,
    ) -> Outcome {
        let Some(index) = self.weapons.iter().position(|row| row.key == weapon) else {
            return Outcome::Rows(Vec::new());
        };
        let Some(defender) = self.worn(&opts.defender) else {
            return Outcome::Rows(Vec::new());
        };
        let query = Query {
            one_hand: false,
            class: None,
            per_class: false,
            window: window_seconds(opts.window_s),
            raw_ar: opts.raw_ar,
            objective: opts.objective,
            top: usize::MAX,
            weapon: Some(index),
            defender: Against {
                worn: defender,
                reply: opts.reply,
            },
        };
        let ranked = self.rank(&to_stats(stats), u32::from(sl), &query);
        Outcome::Rows(ranked.iter().map(|row| self.result_row(row)).collect())
    }

    fn optimize(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        grip: Grip,
        limits: &Limits<'_>,
    ) -> Option<OptimizedBuild> {
        let weapon = self.weapon_by_key(weapon)?;
        let spells = self.spell_indices(limits.spells)?;
        let (value, class, two_handed, stats, worn) = self.optimize_build(
            weapon,
            infusion,
            u32::from(sl),
            objective,
            grip,
            &spells,
            limits.class,
            limits.floors,
            Against {
                worn: self.worn(limits.defender)?,
                reply: limits.reply,
            },
        )?;
        Some(OptimizedBuild {
            class: self.classes[class].name.clone(),
            sl,
            stats: stats.map(|value| u16::try_from(value).unwrap_or(0)),
            two_handed,
            value: value as f32,
            gear: worn
                .iter()
                .map(|&ring| self.rings[ring].1.clone())
                .collect(),
        })
    }

    fn refusal(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        grip: Grip,
        limits: &Limits<'_>,
    ) -> Option<Refusal> {
        let weapon = self.weapon_by_key(weapon)?;
        let spells = self.spell_indices(limits.spells)?;
        self.refusal_of(
            weapon,
            infusion,
            sl,
            objective,
            grip,
            &spells,
            limits.class,
            limits.floors,
            limits.reply,
        )
    }

    fn minimum(&self, weapon: &str, _infusion: Infusion, two_hand: bool) -> Option<OptimizedBuild> {
        self.recommended_minimum(self.weapon_by_key(weapon)?, two_hand)
    }

    fn similar(&self, stats: &[u16; STAT_COUNT], sl: u16, k: u16, status: StatusFilter) -> Outcome {
        let stats = to_stats(stats);
        // Each weapon the nearest builds carry, first seen first; a build counts once per weapon.
        let mut used: Vec<Carried> = Vec::new();
        for build in self.nearest(&stats, u32::from(sl), usize::from(k)) {
            let mut mine: Vec<(usize, Vec<Option<Infusion>>)> = Vec::new();
            for &(index, infusion) in &build.weapons {
                let weapon = &self.weapons[index];
                if weapon.shield {
                    continue;
                }
                if status.any() {
                    // MugenMonkey's unknown infusion counts when the base weapon deals it.
                    let atk = weapon
                        .infusion(infusion.unwrap_or(Infusion::None))
                        .map_or([0.0; 7], |row| row.atk);
                    if !((status.bleed && atk[ATK_BLEED] != 0.0)
                        || (status.poison && atk[ATK_POISON] != 0.0))
                    {
                        continue;
                    }
                }
                match mine.iter_mut().find(|(carried, _)| *carried == index) {
                    Some((_, infusions)) if !infusions.contains(&infusion) => {
                        infusions.push(infusion);
                    }
                    Some(_) => {}
                    None => mine.push((index, vec![infusion])),
                }
            }
            for (index, infusions) in mine {
                let at = used
                    .iter()
                    .position(|(carried, ..)| *carried == index)
                    .unwrap_or_else(|| {
                        used.push((index, 0, Vec::new()));
                        used.len() - 1
                    });
                let entry = &mut used[at];
                entry.1 += 1;
                for infusion in infusions {
                    match entry.2.iter_mut().find(|(seen, _)| *seen == infusion) {
                        Some((_, count)) => *count += 1,
                        None => entry.2.push((infusion, 1)),
                    }
                }
            }
        }
        used.sort_by_key(|&(_, count, _)| std::cmp::Reverse(count));
        let rows = used
            .into_iter()
            .filter_map(|(index, count, mut infusions)| {
                let weapon = &self.weapons[index];
                let grip = if weapon.wieldable(&stats, false) {
                    "1H"
                } else if weapon.wieldable(&stats, true) {
                    "2H only"
                } else {
                    return None;
                };
                infusions.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
                // The script lists its top three; a row holds one: the most carried that was
                // recorded, uninfused when only MugenMonkey's unknown was.
                let infusion = infusions
                    .iter()
                    .take(3)
                    .find_map(|&(infusion, _)| infusion)
                    .unwrap_or(Infusion::None);
                Some(ResultRow {
                    weapon: weapon.name.clone(),
                    infusion,
                    damage: count as f32,
                    ar_by_type: [0.0; 5],
                    grip: grip.to_owned(),
                    two_hand_only: grip == "2H only",
                    hyperarmor: None,
                    counter: None,
                    class: self.class_of(index).to_owned(),
                })
            })
            .take(SIMILAR_TOP)
            .collect();
        Outcome::Rows(rows)
    }

    fn calibration(&self) -> Calibration {
        self.calibration.unwrap_or(Calibration {
            n: 0,
            top1: 0.0,
            top2: 0.0,
        })
    }

    fn defense(&self, sl: u16, defender: &Defender, reply: Reply) -> Option<DefenderDefense> {
        let sl = u32::from(sl);
        let worn = self.worn(defender)?;
        // The builds the script's bracket_defense and bracket_stats read: `sl`'s bracket, or the
        // nearest (the lower on a tie) with 20 or more.
        let builds = u32::try_from(self.bracket_builds(sl_bracket(sl)).len()).unwrap_or(u32::MAX);
        let stats = worn.map(|_| {
            self.bracket(sl)
                .stats
                .map(|value| u16::try_from(value).unwrap_or(0))
        });
        let counters = if reply.adapts() {
            self.counters
                .iter()
                .map(|&ring| self.rings[ring].1.clone())
                .collect()
        } else {
            Vec::new()
        };
        Some(DefenderDefense {
            defense: self.defense_at(sl, worn).map(|value| value as f32),
            builds,
            stats,
            reply,
            counters,
        })
    }

    fn best_weapons(
        &self,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        grip: Grip,
        limits: &Limits<'_>,
        opts: &BestWeaponsOpts,
    ) -> Vec<BestWeaponRow> {
        let (Some(spells), Some(worn)) = (
            self.spell_indices(limits.spells),
            self.worn(limits.defender),
        ) else {
            return Vec::new();
        };
        let ask = best::Ask {
            infusion,
            sl: u32::from(sl),
            objective,
            grip,
            spells: &spells,
            class: limits.class,
            floors: limits.floors,
            against: Against {
                worn,
                reply: limits.reply,
            },
            window: window_seconds(opts.window_s),
            rank: opts.rank,
        };
        self.best_weapons_of(&ask, opts.weapon_class.as_deref())
    }

    fn armor_pieces(&self, slot: usize) -> Vec<(String, String)> {
        let Some(pieces) = self.wearable.get(slot) else {
            return Vec::new();
        };
        let naked = pieces.iter().filter(|piece| piece.key == "Naked");
        let rest = pieces.iter().filter(|piece| piece.key != "Naked");
        naked
            .chain(rest)
            .map(|piece| (piece.key.clone(), piece.name.clone()))
            .collect()
    }

    fn generate_build(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        allow_naked: bool,
        grip: Grip,
        limits: &Limits<'_>,
    ) -> Option<GeneratedBuild> {
        let primary = self.weapon_by_key(weapon)?;
        let spells = self.spell_indices(limits.spells)?;
        let defender = Against {
            worn: self.worn(limits.defender)?,
            reply: limits.reply,
        };
        let (_, class, two_handed, stats, worn) = self.optimize_build(
            primary,
            infusion,
            u32::from(sl),
            objective,
            grip,
            &spells,
            limits.class,
            limits.floors,
            defender,
        )?;
        // The weapons, the catalysts and the slots are read at what the worn rings give.
        let eff = self.gear_stats(&stats, &worn);
        let slots_used: i32 = spells.iter().map(|&spell| self.spells[spell].slots).sum();
        let slots = self.slots_of(&eff) + self.ring_slots(&worn);
        debug_assert!(slots_used <= slots, "the optimizer fits the spells' slots");
        let catalysts = self.best_catalysts(&spells, &eff);
        let lift_floors =
            self.lift_floors(self.bracket(u32::from(sl)), primary, limits.floors, &spells);
        let lift_require = Self::grip_require_pairs(primary, grip.two_handed());
        let ring_trades = self.ring_trades(class, &lift_floors, &lift_require, &spells, &worn);
        let mut ring_lowered = [false; STAT_COUNT];
        let base = &self.classes[class].base;
        if let (Some(with), without) = (
            self.ring_lift(base, &lift_floors, &lift_require, &spells, &worn),
            self.ring_lift(base, &lift_floors, &lift_require, &spells, &[]),
        ) {
            for (stat, lowered) in ring_lowered.iter_mut().enumerate() {
                // No ATT alone holds the spells: the band stands in for all of it.
                *lowered = without.is_none_or(|without| with[stat] < without[stat]);
            }
        }
        let query = Query {
            one_hand: false,
            class: None,
            per_class: false,
            window: GENERATE_WINDOW,
            raw_ar: false,
            objective: Objective::Damage,
            top: usize::MAX,
            weapon: None,
            defender,
        };
        let ranked = self.rank(&eff, u32::from(sl), &query);
        let mut seen: Vec<&str> = vec![primary.name.as_str()];
        let (mut weapons_1h, mut weapons_2h_only) = (Vec::new(), Vec::new());
        for row in &ranked {
            let name = self.weapons[row.weapon].name.as_str();
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            if row.label.contains("2H only") {
                weapons_2h_only.push(self.result_row(row));
            } else {
                weapons_1h.push(self.result_row(row));
            }
        }
        weapons_1h.truncate(WEAPONS_1H_TOP);
        weapons_2h_only.truncate(WEAPONS_2H_ONLY_TOP);

        let mut suggested = worn.clone();
        suggested.extend(self.suggest_rings(
            &ring_counts(self.nearest(&stats, u32::from(sl), GENERATE_K)),
            &worn,
        ));
        let levelled = stats.map(|value| u16::try_from(value).unwrap_or(0));
        debug_assert_eq!(
            crate::model::soul_level(&levelled),
            sl,
            "the optimizer spends exactly the points `sl` has"
        );
        let name = |ring: &usize| self.rings[*ring].1.clone();
        let (armor, armor_note) = if allow_naked {
            (Vec::new(), None)
        } else {
            self.generate_armor(&stats, primary, &suggested)
        };
        Some(GeneratedBuild {
            class: self.classes[class].name.clone(),
            sl,
            stats: levelled,
            primary: (primary.key.clone(), infusion),
            two_handed,
            weapons_1h,
            weapons_2h_only,
            suggested_rings: suggested.iter().map(name).collect(),
            common_rings: self
                .common
                .iter()
                .filter(|ring| !suggested.contains(ring) && !self.no_use.contains(ring))
                .map(name)
                .collect(),
            armor,
            armor_note,
            spells: spells
                .iter()
                .map(|&spell| self.spells[spell].key.clone())
                .collect(),
            spell_names: spells
                .iter()
                .map(|&spell| self.spells[spell].name.clone())
                .collect(),
            slots_used: u16::try_from(slots_used).unwrap_or(0),
            slots: u16::try_from(slots).unwrap_or(0),
            catalysts,
            ring_trades,
            ring_lowered,
            stub: false,
        })
    }

    fn spells(&self) -> Vec<SpellRow> {
        let requirement = |spell: &Spell, stat: usize| {
            spell
                .require
                .iter()
                .find(|&&(at, _)| at == stat)
                .map_or(0, |&(_, value)| u16::try_from(value).unwrap_or(0))
        };
        self.spells
            .iter()
            .map(|spell| SpellRow {
                key: spell.key.clone(),
                name: spell.name.clone(),
                slots: u16::try_from(spell.slots).unwrap_or(0),
                intelligence: requirement(spell, INT),
                faith: requirement(spell, FTH),
            })
            .collect()
    }

    fn flexibility(
        &self,
        stats: &[u16; STAT_COUNT],
        sl: u16,
        armor: &[String],
        rings: &[String],
    ) -> Option<Flexibility> {
        self.flex(&to_stats(stats), u32::from(sl), armor, rings)
    }

    fn weapon_card(
        &self,
        weapon: &str,
        infusion: Infusion,
        stats: &[u16; STAT_COUNT],
    ) -> Option<WeaponCard> {
        let weapon = self.weapon_by_key(weapon)?;
        let stats = to_stats(stats);
        let row = weapon
            .infusion(infusion)
            .or_else(|| weapon.infusion(Infusion::None));
        let wield = if weapon.wieldable(&stats, false) {
            Wield::Both
        } else if weapon.wieldable(&stats, true) {
            Wield::TwoHandedOnly
        } else {
            Wield::Neither
        };
        let requirements = weapon
            .require
            .iter()
            .filter(|&&(_, value)| value > 0)
            .map(|&(stat, value)| Requirement {
                stat,
                value: u16::try_from(value).unwrap_or(u16::MAX),
                met: Weapon::meets(&stats, stat, value, true),
            })
            .collect();
        Some(WeaponCard {
            requirements,
            weight: weapon.weight as f32,
            wield,
            infusion: row.map_or(Infusion::None, |row| row.infusion),
            attack: self
                .attack_rating(row, &stats, &[])
                .map(|value| value.map(|value| value as f32)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_are_the_scripts() {
        assert_eq!(sl_bracket(1), 0);
        assert_eq!(sl_bracket(20), 0);
        assert_eq!(sl_bracket(21), 1);
        assert_eq!(sl_bracket(100), 3);
        assert_eq!(sl_bracket(251), 8);
        assert_eq!(sl_bracket(838), 8);
        assert_eq!(stat_bracket(10), 0);
        assert_eq!(stat_bracket(11), 1);
        assert_eq!(stat_bracket(99), 9);
        assert_eq!(stat_bracket(120), 9);
    }

    #[test]
    fn agility_is_the_planners() {
        assert_eq!(agility(6, 6), 86);
        assert_eq!(agility(99, 99), 120);
        assert_eq!(agility(40, 0), 110);
        assert_eq!(agility(1, 1), 85);
    }

    /// The script's selftest cases for `hit_points`, over the same made-up columns.
    #[test]
    fn hit_points_is_the_scripts() {
        let mut backend = CorpusBackend::default();
        let ten = [10; STAT_COUNT];
        let with = |stat: usize, v: i32| {
            let mut st = ten;
            st[stat] = v;
            st
        };
        assert_eq!(
            backend.hit_points(&with(VIG, 30)),
            1300.0,
            "getHP without the columns"
        );
        backend.tables.hp_max = Table(
            (0..100)
                .map(|v| if v == 0 { 0.0 } else { 1000.0 + f64::from(v) })
                .collect(),
        );
        backend.tables.additional_hp = Table((0..100).map(|v| f64::from(2 * v)).collect());
        assert_eq!(backend.hit_points(&with(VIG, 30)), 1030.0 + 8.0 * 20.0);
        assert_eq!(
            backend.hit_points(&with(FTH, 0)),
            1010.0 + 7.0 * 20.0 + 2.0,
            "FTH 0 reads row 1"
        );
    }

    /// `itertools.combinations(range(4), 2)`.
    #[test]
    fn combinations_are_pythons_order() {
        let mut out = Vec::new();
        combinations(4, 2, &mut out);
        assert_eq!(
            out,
            [[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]].map(|pair| pair.to_vec())
        );
    }

    #[test]
    fn a_typed_window_is_the_decimal_typed() {
        assert_eq!(window_seconds(1.1), 1.1);
        assert_eq!(window_seconds(1.5), 1.5);
    }

    #[test]
    fn a_foreign_file_is_refused() {
        assert!(CorpusBackend::parse("something else\n").is_err());
        assert!(CorpusBackend::parse(FORMAT).is_err(), "no tables");
    }
}
