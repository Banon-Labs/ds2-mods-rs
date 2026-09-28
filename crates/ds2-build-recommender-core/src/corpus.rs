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
//!   weapon's R1 hit timeline, hyperarmor and counter multipliers.
//! * **Computed here**, because they depend on the question: attack rating and damage for any
//!   stats, the ranking, the stat optimizer, the minimum-build search, and the nearest builds.
//!
//! The corpus is carried as what nearest-build search reads and nothing else: each build's SL
//! bracket, its nine stat brackets, its rings and its weapons with their infusions.

use std::path::Path;

use ds2_build_import_core::Infusion;

use crate::backend::{
    Calibration, GeneratedBuild, OptimizedBuild, Outcome, RecommenderBackend, ResultRow,
    WEAPONS_1H_TOP, WEAPONS_2H_ONLY_TOP,
};
use crate::model::{Objective, STAT_COUNT, StatusFilter, WeaponsForOpts};
use crate::weapons;

/// What the file is called beside `DarkSoulsII.exe`.
pub const DATA_FILE_NAME: &str = "ds2-build-recommender.dat";

/// The file's first line. A different one is a file this port does not read.
pub const FORMAT: &str = "ds2-build-recommender-data 2";

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

/// HP from VGR alone, as the script counts it.
fn hit_points(vgr: i32) -> i32 {
    500 + 30 * vgr.min(20) + 20 * (vgr.min(50) - 20).max(0) + 5 * (vgr - 50).max(0)
}

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
}

/// A starting class.
#[derive(Clone, Debug)]
struct Class {
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
    infusions: Vec<InfusionRow>,
    /// The R1 chain one- and two-handed, empty where there is no timing.
    timeline: [Vec<Hit>; 2],
}

impl Weapon {
    fn infusion(&self, infusion: Infusion) -> Option<&InfusionRow> {
        self.infusions.iter().find(|row| row.infusion == infusion)
    }

    /// The script's `weapon_ok` over these stats, with no armour.
    fn wieldable(&self, stats: &Stats, two_hand: bool) -> bool {
        self.require.iter().all(|&(stat, value)| {
            let need = if two_hand && stat == STR {
                value.div_euclid(2)
            } else {
                value
            };
            need <= stats[stat]
        })
    }
}

/// physical, magic, fire, lightning, dark, slash, strike, thrust.
type Defense = [f64; 8];

/// One SL bracket's floors and average defender.
#[derive(Clone, Debug)]
struct Bracket {
    /// VIG, VIT, ADP, ATT, then END for a high-stamina weapon.
    floors: [i32; 5],
    defense: Defense,
}

/// A ring the minimum search may wear.
#[derive(Clone, Debug)]
struct RingEffect {
    key: String,
    weight: f64,
    load_mul: f64,
    add: Stats,
}

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
    /// How many rows to keep when not `per_class`.
    top: usize,
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
    ring_effects: Vec<RingEffect>,
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
    corpus: Vec<CorpusBuild>,
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
                next("class key")?;
                let name = next("class name")?.to_owned();
                let level = int(Some(next("level")?), line)?;
                let base = stats(&mut fields, line)?;
                self.classes.push(Class { name, level, base });
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
                    infusions: Vec::new(),
                    timeline: [Vec::new(), Vec::new()],
                });
            }
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
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "an infusion before any weapon"))?
                    .infusions
                    .push(InfusionRow {
                        infusion,
                        atk,
                        scale,
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
                        Ok(Hit {
                            at,
                            motion_value,
                            defense,
                            lower,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.weapons
                    .last_mut()
                    .ok_or_else(|| bad(line, "a timeline before any weapon"))?
                    .timeline[grip] = hits;
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
                self.brackets.push(Bracket { floors, defense });
            }
            "R" => {
                let key = next("ring key")?.to_owned();
                let name = next("ring name")?.to_owned();
                let weight = float(Some(next("weight")?), line)?;
                ring_index.insert(key.clone(), self.rings.len());
                self.rings.push((key, name, weight));
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
                self.wearable[slot].push(Wearable {
                    key,
                    name,
                    weight,
                    defense,
                    require,
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
                self.corpus.push(CorpusBuild {
                    bracket,
                    stat_brackets,
                    rings,
                    weapons: carried,
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

    /// The script's `attack_rating`: per-type attack rating at full upgrade.
    fn attack_rating(&self, row: Option<&InfusionRow>, eff: &Stats) -> Ar {
        let mut out = [None; 5];
        let Some(row) = row else {
            return out;
        };
        let (atk, sc) = (&row.atk, &row.scale);
        let t = &self.tables;
        if atk[ATK_PHYSICAL] != 0.0 {
            let mut v = atk[ATK_PHYSICAL]
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
            if base != 0.0 {
                out[1 + element] = Some((base + sc[2 + element] * table.at(index)).trunc());
            }
        }
        out
    }

    /// The script's `damage`, summed over the types: one hit against the bracket's defender,
    /// motion value 1.
    fn damage(ar: &Ar, defense: &Defense) -> f64 {
        let mut total = 0.0;
        for (kind, value) in ar.iter().enumerate() {
            let Some(value) = *value else {
                continue;
            };
            total += if value == 0.0 {
                0.0
            } else if kind == 0 {
                py_max(0.0, (value * 10.0 - defense[0]) / 12.0)
            } else {
                value * (1.0 - py_min(0.99, (defense[kind] + 100.0) / 1000.0))
            };
        }
        total
    }

    /// The script's `hit_damage`: one timed hit against the bracket's defender.
    fn hit_damage(ar: &Ar, defense: &Defense, hit: &Hit) -> f64 {
        let mut total = 0.0;
        for (kind, value) in ar.iter().enumerate() {
            let Some(value) = *value else {
                continue;
            };
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
            top,
        } = *query;
        let bracket = self.bracket(sl);
        let defense = &bracket.defense;
        let class = class.map(norm);
        let mut rows: Vec<Ranked> = Vec::new();
        for (index, weapon) in self.weapons.iter().enumerate() {
            if EMPTY.contains(&weapon.key.as_str()) || weapon.catalyst || weapon.shield {
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
            if weapon.high_stamina && stats[END] < bracket.floors[4] {
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
                let ar = self.attack_rating(Some(row), stats);
                if ar.iter().all(Option::is_none) {
                    continue;
                }
                let (damage, label) = if window == 0.0 {
                    let damage = if raw_ar {
                        ar.iter().flatten().fold(0.0, |total, value| total + value)
                    } else {
                        Self::damage(&ar, defense)
                    };
                    (damage, if one { "1H" } else { "2H only" }.to_owned())
                } else {
                    // The grip whose hits deal the most; the first on a tie, as `max` keeps it.
                    let mut best: Option<(&str, usize, f64)> = None;
                    for (grip, hits) in &lines {
                        let damage = hits
                            .iter()
                            .map(|hit| Self::hit_damage(&ar, defense, hit))
                            .fold(0.0, |total, damage| total + damage);
                        if best.is_none_or(|(_, _, top)| damage > top) {
                            best = Some((grip, hits.len(), damage));
                        }
                    }
                    let Some((grip, hits, damage)) = best else {
                        continue;
                    };
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
                    label,
                });
            }
            scored.sort_by(|a, b| b.damage.total_cmp(&a.damage));
            let Some(best) = scored.first().map(|row| row.damage) else {
                continue;
            };
            rows.extend(
                scored
                    .into_iter()
                    .take(3)
                    .filter(|row| row.damage >= best * (1.0 - WITHIN)),
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

    /// The script's `objective_value`.
    fn objective_value(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        stats: &Stats,
        objective: Objective,
        defense: &Defense,
    ) -> f64 {
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
            Objective::Damage => Self::damage(&self.attack_rating(row, stats), defense),
        }
    }

    /// The script's `optimize_build`: `(value, class, two-handed, stats)`, or `None` when no class
    /// fits the floors and requirements into `sl`.
    fn optimize_build(
        &self,
        weapon: &Weapon,
        infusion: Infusion,
        sl: u32,
        objective: Objective,
    ) -> Option<(f64, usize, bool, Stats)> {
        #[derive(Clone, Copy, PartialEq)]
        enum Curve {
            Objective,
            Agility,
            HitPoints,
        }
        let bracket = self.bracket(sl);
        let defense = &bracket.defense;
        let adaptability = if objective == Objective::Poison {
            Curve::Objective
        } else {
            Curve::Agility
        };
        // The script's `curves`, in its order.
        let curves = [
            (STR, Curve::Objective),
            (DEX, Curve::Objective),
            (INT, Curve::Objective),
            (FTH, Curve::Objective),
            (ADP, adaptability),
            (VIG, Curve::HitPoints),
            (ATT, Curve::Agility),
        ];
        let value = |curve: Curve, st: &Stats| -> f64 {
            match curve {
                Curve::Objective => self.objective_value(weapon, infusion, st, objective, defense),
                Curve::Agility => f64::from(agility(st[ADP], st[ATT])),
                Curve::HitPoints => f64::from(hit_points(st[VIG])),
            }
        };
        let with = |st: &Stats, stat: usize, to: i32| {
            let mut next = *st;
            next[stat] = to;
            next
        };
        let sl = i32::try_from(sl).unwrap_or(i32::MAX - 53);
        let mut best: Option<(f64, usize, bool, Stats)> = None;
        for (class_index, class) in self.classes.iter().enumerate() {
            for two in [false, true] {
                let mut st = class.base;
                for (at, stat) in FLOOR_STATS.into_iter().enumerate() {
                    st[stat] = st[stat].max(bracket.floors[at]);
                }
                if weapon.high_stamina {
                    st[END] = st[END].max(bracket.floors[4]);
                }
                for &(stat, need) in &weapon.require {
                    let need = if two && stat == STR {
                        (need + 1).div_euclid(2)
                    } else {
                        need
                    };
                    st[stat] = st[stat].max(need);
                }
                let mut free = sl + 53 - st.iter().sum::<i32>();
                if free < 0 {
                    continue;
                }
                // Each stat weighted by its own curve's early rate, from 5 to 25.
                let mut peak = [0.0; 7];
                for (at, &(stat, curve)) in curves.iter().enumerate() {
                    peak[at] = py_max(
                        (value(curve, &with(&st, stat, 25)) - value(curve, &with(&st, stat, 5)))
                            / 20.0,
                        1e-9,
                    );
                }
                if adaptability == Curve::Agility {
                    peak[6] = peak[4];
                }
                while free > 0 {
                    let mut pick: Option<(usize, i32)> = None;
                    let mut best_w = 0.0;
                    for (at, &(stat, curve)) in curves.iter().enumerate() {
                        let current = value(curve, &st);
                        for n in 1..=free.min(99 - st[stat]).min(8) {
                            let w = (value(curve, &with(&st, stat, st[stat] + n)) - current)
                                / f64::from(n)
                                / peak[at];
                            if w > best_w + 1e-12 {
                                pick = Some((stat, n));
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
                    st[stat] += n;
                    free -= n;
                }
                let val = self.objective_value(weapon, infusion, &st, objective, defense);
                if best.is_none_or(|(top, ..)| val > top) {
                    best = Some((val, class_index, two, st));
                }
                // One-handed works: never prefer the two-handed variant.
                break;
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

impl CorpusBackend {
    /// The script's `best_armor` with `top=1`: the equip-load cap, what the weapon and rings
    /// carry, and the set, head to legs, whose defense weighted by the corpus threat mix is
    /// highest among those these stats can wear under the cap. `None` when nothing fits, which
    /// only happens when the weapon and rings alone are over it: `Naked` is a piece in every slot.
    fn best_armor(
        &self,
        stats: &Stats,
        weapon: &Weapon,
        rings: &[usize],
    ) -> (f64, f64, Option<[&Wearable; 4]>) {
        let cap = self.tables.equip_load.at(stats[VIT]) * EQUIP_CAP;
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
        let mut top: Option<(f64, f64, [&Wearable; 4])> = None;
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
                        let set = (h.1 + c.1 + g.1 + l.1, weight, [h.2, c.2, g.2, l.2]);
                        // Python's descending sort of `(value, weight, keys)`, first place.
                        let better = top.as_ref().is_none_or(|best| {
                            set.0
                                .total_cmp(&best.0)
                                .then(set.1.total_cmp(&best.1))
                                .then_with(|| keys(&set.2).cmp(&keys(&best.2)))
                                .is_gt()
                        });
                        if better {
                            top = Some(set);
                        }
                    }
                }
            }
        }
        (cap, carried, top.map(|(_, _, set)| set))
    }

    /// The script's `generate_armor`: the pieces' names head to legs, `Naked` for a slot left
    /// bare, and the note that says why a slot or the whole set is bare.
    fn generate_armor(
        &self,
        stats: &Stats,
        weapon: &Weapon,
        rings: &[usize],
    ) -> (Vec<String>, Option<String>) {
        let (cap, carried, set) = self.best_armor(stats, weapon, rings);
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
        let query = Query {
            one_hand: opts.one_hand,
            class: opts.class.as_deref(),
            per_class: opts.per_class,
            window: window_seconds(opts.window_s),
            raw_ar: opts.raw_ar,
            top: WEAPONS_FOR_TOP,
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
    ) -> Option<OptimizedBuild> {
        let weapon = self.weapon_by_key(weapon)?;
        let (value, class, two_handed, stats) =
            self.optimize_build(weapon, infusion, u32::from(sl), objective)?;
        Some(OptimizedBuild {
            class: self.classes[class].name.clone(),
            sl,
            stats: stats.map(|value| u16::try_from(value).unwrap_or(0)),
            two_handed,
            value: value as f32,
            gear: Vec::new(),
        })
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

    fn generate_build(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        allow_naked: bool,
    ) -> Option<GeneratedBuild> {
        let primary = self.weapon_by_key(weapon)?;
        let (_, class, two_handed, stats) =
            self.optimize_build(primary, infusion, u32::from(sl), objective)?;
        let query = Query {
            one_hand: false,
            class: None,
            per_class: false,
            window: GENERATE_WINDOW,
            raw_ar: false,
            top: usize::MAX,
        };
        let ranked = self.rank(&stats, u32::from(sl), &query);
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

        // The rings the nearest builds wear, a build counted once per ring, first seen first.
        let mut worn: Vec<(usize, u32)> = Vec::new();
        for build in self.nearest(&stats, u32::from(sl), GENERATE_K) {
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
        let suggested: Vec<usize> = worn
            .iter()
            .take(crate::backend::SUGGESTED_RINGS)
            .map(|&(ring, _)| ring)
            .collect();
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
                .filter(|ring| !suggested.contains(ring))
                .map(name)
                .collect(),
            armor,
            armor_note,
            stub: false,
        })
    }

    /// The backend has no game to read; the panel falls back to the typed stats.
    fn current_character_stats(&self) -> Option<[u16; STAT_COUNT]> {
        None
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
