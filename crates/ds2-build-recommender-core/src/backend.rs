//! What the panel asks, who answers, and the rules that hold whoever answers.
//!
//! [`RecommenderBackend`] is the contract. [`crate::corpus::CorpusBackend`] answers it with the
//! ranking in `scripts/ds2-builds-recommend.py`, ported, over the data file the script exports.
//! [`StubBackend`] answers it with fixed data, for a game folder with no data file; its numbers are
//! placeholders and the panel says so, from [`RecommenderBackend::is_stub`].
//!
//! The rules live OUTSIDE the trait, in [`ask`], [`generate`], [`split_weapons`], [`ring_grants`]
//! and [`to_import`], so a real backend cannot forget them:
//!
//! * **No result is shown for a build under its floors.** [`ask`] checks the stats against
//!   [`RecommenderBackend::floors`] before the backend is asked at all, and [`generate`] checks the
//!   build that comes back. A floor is the bracket's median VIG, VIT, ADP and ATT; a build under it
//!   is one the script would not recommend, so a ranking for it would be advice nobody should take.
//! * **A generated build carries the 15 best one-handable and the 5 best two-hand-only weapons**,
//!   one row per weapon, never the primary again.
//! * **Rings are granted three to a suggestion**, plus one of every ring at least a tenth of all
//!   builds wear that is not already suggested.
//! * **A generated build never carries an Agape Ring.** Its place among the suggestions goes to
//!   the ring the nearest builds wear most that the build does not already wear in any upgrade.

use ds2_build_import_core::{Build, Infusion, StartingClass, Stats, check_build};

use crate::model::{
    Defender, Grip, Mode, Objective, PanelState, Reply, STAT_COUNT, STAT_LABELS, StatusFilter,
    WeaponsForOpts,
};
use crate::weapons;

/// How many one-handable weapons a generated build lists.
pub const WEAPONS_1H_TOP: usize = 15;

/// How many two-hand-only weapons a generated build lists.
pub const WEAPONS_2H_ONLY_TOP: usize = 5;

/// How many rings a build suggests: one per ring slot.
pub const SUGGESTED_RINGS: usize = 4;

/// Copies granted of each suggested ring: one to wear and two for the +1 and +2 a player would
/// otherwise go and find.
pub const SUGGESTED_RING_COPIES: usize = 3;

/// A ring worn by at least this percentage of every build is common to all builds.
pub const COMMON_RING_PERCENT: u32 = 10;

/// The stats a floor applies to, by index into [`STAT_LABELS`]: VIG, VIT, ATT, ADP.
pub const FLOOR_STATS: [usize; 4] = [0, 2, 3, 6];

/// Spells a build may attune and still be held to the no-spell floors ([`floor_stats`]), by
/// soulsplanner key: the script's `NO_FLOOR_SPELLS`.
pub const NO_FLOOR_SPELLS: [&str; 1] = ["Warmth"];

/// Whether `spells` (soulsplanner keys) hold anything but [`NO_FLOOR_SPELLS`].
pub fn casts_for_floors<'a>(mut spells: impl Iterator<Item = &'a str>) -> bool {
    spells.any(|spell| !NO_FLOOR_SPELLS.contains(&spell))
}

/// The [`FLOOR_STATS`] a build casting `spells` is held to: ATT only when it casts something
/// other than [`NO_FLOOR_SPELLS`].
///
/// The script's `floor_stats`. Without spells ATT buys nothing but agility, which ADP buys three
/// times as fast, so the median ATT of real builds is no floor for one that casts nothing. Warmth
/// alone counts as nothing: a melee build attunes it for the heal, and the spells' own floor
/// already raises ATT to the slot it needs.
#[must_use]
pub fn floor_stats(spells: &[String]) -> &'static [usize] {
    const NO_SPELLS: [usize; 3] = [0, 2, 6];
    if !casts_for_floors(spells.iter().map(String::as_str)) {
        &NO_SPELLS
    } else {
        &FLOOR_STATS
    }
}

/// The damage types an attack rating is split into, in [`ResultRow::ar_by_type`] order.
pub const DAMAGE_TYPES: [&str; 5] = ["phys", "magic", "fire", "light", "dark"];

/// The defense types a [`DefenderDefense`] lists, in its order: the script's `DMG + PHYS_TYPES`.
pub const DEFENSE_TYPES: [&str; 8] = [
    "phys", "magic", "fire", "light", "dark", "slash", "strike", "thrust",
];

/// What [`Objective::Damage`] is scored against at one soul level: the panel's defender line.
#[derive(Clone, Debug, PartialEq)]
pub struct DefenderDefense {
    /// Defense per type, in [`DEFENSE_TYPES`] order. A hit's physical damage is read against its
    /// slash, strike or thrust defense when it has that type, the general physical one otherwise.
    pub defense: [f32; 8],
    /// How many corpus builds it was taken from: the bracket's builds the average defender is the
    /// mean of, or the median stats a chosen set is worn at are the median of.
    pub builds: u32,
    /// The median stats a chosen set is worn at, in [`STAT_LABELS`] order; `None` for the average
    /// defender, which wears every build's own stats.
    pub stats: Option<[u16; STAT_COUNT]>,
    /// How the defender answers each weapon. `defense` is before any answer: the numbers a score
    /// without an attack in hand reads.
    pub reply: Reply,
    /// The rings the defender may swap in against a weapon, by name; empty for a static defender.
    pub counters: Vec<String>,
}

/// The panel's defender line: what a Damage column is scored against.
///
/// The numbers come first, physical and its three types then the elements, so a line clipped to
/// the panel's width keeps them, then who they belong to. A chosen set says the median stats it is
/// worn at, the ones its defense
/// reads (END, VIT, STR and DEX feed physical; INT and FTH the elements), since without them
/// "Havel's set" would not say why its magic defense is below the average player's.
pub fn defender_line(defense: &DefenderDefense, sl: u16) -> String {
    const SHOWN: [usize; 8] = [0, 5, 6, 7, 1, 2, 3, 4];
    let numbers = SHOWN
        .iter()
        .map(|&kind| format!("{} {:.0}", DEFENSE_TYPES[kind], defense.defense[kind]))
        .collect::<Vec<_>>()
        .join("  ");
    let whose = match defense.stats {
        None => format!("the SL {sl} average defender ({} builds)", defense.builds),
        Some(stats) => {
            let read = [1, 2, 4, 5, 7, 8]
                .iter()
                .map(|&stat| format!("{} {}", STAT_LABELS[stat], stats[stat]))
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "the chosen set at the SL {sl} median of {} builds, {read}",
                defense.builds
            )
        }
    };
    let adapts = if defense.counters.is_empty() {
        String::new()
    } else {
        let buff = match defense.reply {
            Reply::RingAndItem => ", plus the best item defense buff",
            Reply::RingAndAnyBuff => ", plus the best item or spell defense buff",
            Reply::Static | Reply::Ring => "",
        };
        format!(
            "; adapts: swaps a ring slot to {} against each weapon{buff} (column 'vs')",
            defense.counters.join(", ")
        )
    };
    format!("Defender: {numbers}  --  {whose}{adapts}")
}

/// One row of a ranking.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    /// The weapon's display name.
    pub weapon: String,
    /// The infusion this row is for.
    pub infusion: Infusion,
    /// The score the mode ranks by: expected damage, attack rating, or for
    /// [`Mode::SimilarBuilds`] the number of neighbouring builds that carry the weapon.
    pub damage: f32,
    /// Attack rating per type, in [`DAMAGE_TYPES`] order.
    pub ar_by_type: [f32; 5],
    /// How the row is held, as the script labels it: `1H`, `2H only`, `2H 3 hits`.
    pub grip: String,
    /// Whether these stats can only wield it two-handed.
    pub two_hand_only: bool,
    /// The R1's hyperarmor multiplier, where it has one.
    pub hyperarmor: Option<f32>,
    /// The counter-hit multiplier, where the weapon's differs from nothing.
    pub counter: Option<f32>,
    /// The weapon's class.
    pub class: String,
}

/// One row of Best weapons: a weapon at the build Optimize for weapon makes for it, scored so
/// weapons compare. The script's `best_weapon_row` and the metrics its `--json` prints.
#[derive(Clone, Debug, PartialEq)]
pub struct BestWeaponRow {
    /// What the ranking sorts by: the R1 hits landed within the window (bleed and poison: build-up
    /// per hit times the hits), a stamina metric under [`crate::model::Rank`], the optimizer's own
    /// value without a window, a launcher's one shot.
    pub score: f64,
    /// The weapon, by soulsplanner key.
    pub weapon: String,
    /// Its display name.
    pub name: String,
    /// The optimizer's own value: one hit (or one shot) of the objective.
    pub value: f64,
    /// The starting class the build is from, by display name.
    pub class: String,
    /// Whether the build holds it two-handed.
    pub two_handed: bool,
    /// The build's levelled stats, in [`STAT_LABELS`] order.
    pub stats: [u16; STAT_COUNT],
    /// The rings it wears in place of stat points, by name.
    pub rings: Vec<String>,
    /// The grip and what the score counted: `2H 3 hits`, `2H R1 2 hits`, `2H 1 shot`.
    pub label: String,
    /// A launcher's best ammunition and the shot scored, as the script notes it; `None` for any
    /// other weapon.
    pub ammo: Option<(String, String)>,
    /// Measurements beside the score.
    pub metrics: WeaponMetrics,
}

/// What a Best weapons row measures besides its score, each `None` where the data cannot say.
///
/// The script's `r1_metrics` and `poise_metrics`, at the row's own stats, rings and grip. A
/// launcher has none of them but the defender's poise.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct WeaponMetrics {
    /// Metres the R1's hitbox extends along the weapon.
    pub reach_m: Option<f64>,
    /// Seconds to the first chain attack's first live hitbox frame.
    pub startup_s: Option<f64>,
    /// Seconds from its last live hitbox frame to the end of the animation.
    pub recovery_s: Option<f64>,
    /// Seconds to the first hit of the R1 chain.
    pub time_to_first_hit_s: Option<f64>,
    /// The R1 chain repeated for five seconds, every hit landed by then, against the defender's
    /// numbers before any answer.
    pub damage_per_5s: Option<f64>,
    /// The share of the R1's windup and active frames inside its hyperarmor window; `0` when its
    /// rate is `0`, as then the window does nothing.
    pub hyperarmor: Option<f64>,
    /// `WeaponParam.uninterruptibleRate`: the factor on poise damage the attacker takes inside the
    /// window.
    pub hyperarmor_rate: Option<f64>,
    /// The share of the bracket's counter-hits (each build's one-handed R1) that do not stagger the
    /// attacker inside the window, the attacker's poise taken as the defender's; `None` without
    /// hyperarmor.
    pub hyperarmor_holds: Option<f64>,
    /// The R1's first hit's poise damage to a player.
    pub poise_damage_per_hit: Option<f64>,
    /// That hit's `DamageCtrlParam.armorBreak`: 1 and 2 stagger whatever the poise.
    pub armor_break: Option<i32>,
    /// R1 chain hits until the defender staggers, `None` if not within ten seconds.
    pub hits_to_stagger: Option<u32>,
    /// The defender's max poise: the bracket's mean, or the chosen set's.
    pub defender_poise: f64,
    /// The poison and bleed the scored hits deal through their procs, for a melee row scored over
    /// a window; `None` otherwise.
    pub status: Option<StatusMetrics>,
    /// For damage over a window, the score plus the status damage per window: what the script's
    /// `--with-status` ranks by. The score itself does not count status.
    pub damage_with_status: Option<f64>,
    /// What the scored hits cost in stamina and what a full bar of it pays for, for damage over a
    /// window; `None` otherwise, or without stamina costs or attack timing.
    pub stamina: Option<StaminaMetrics>,
}

/// The stamina side of a row's R1 chain (a launcher's: one shot), in menu points: the script's
/// `stamina_metrics` and `shot_metrics`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StaminaMetrics {
    /// The R1 chain's 1st and 2nd attack's cost; a launcher's shot's.
    pub per_attack: Vec<f64>,
    /// What the attacks whose hits land in the window cost.
    pub per_window: f64,
    /// The window's damage over that cost: what [`crate::model::Rank::PerStamina`] ranks by.
    pub damage_per_stamina: f64,
    /// The build's max stamina, its rings counted; `None` without the regulation's table.
    pub max_stamina: Option<f64>,
    /// The attacks a full bar begins: each begins while stamina is above 0.
    pub bar_attacks: Option<u32>,
    /// Their damage: what [`crate::model::Rank::Bar`] ranks by.
    pub bar_damage: Option<f64>,
    /// Seconds until the last of them begins; `None` for a launcher, whose fire rate is not read.
    pub bar_seconds: Option<f64>,
}

/// The poison and bleed a row's hits deal through their procs: the script's `row_status`, against
/// the defenders at the soul level.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StatusMetrics {
    /// The hits counted: the R1 chain's landed in the window (one per distinct hitbox), or the
    /// status ranking's own.
    pub hits: u32,
    /// Per status, poison then bleed, its build-up per hit before resistance; `None` for one the
    /// weapon does not build up.
    pub buildup_per_hit: [Option<f64>; 2],
    /// Per status, the hits the median defender takes to proc; `0` when none procs.
    pub hits_to_proc: [Option<u32>; 2],
    /// Proc damage per window of the chain repeated without pause, the lockout after a proc
    /// counted, summed over the statuses and averaged over the defenders.
    pub damage_per_window: f64,
    /// Proc damage those hits deal a defender whose gauge starts empty.
    pub damage_first_window: f64,
}

/// How some stats can hold a weapon: the weapon picker's Grip line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wield {
    /// One-handed, and so two-handed as well.
    Both,
    /// Only two-handed, which is what lets the stats meet its strength.
    TwoHandedOnly,
    /// Not at all: a requirement is unmet in either grip.
    Neither,
}

/// One of a weapon's requirements, against some stats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// The stat, by index into [`STAT_LABELS`].
    pub stat: usize,
    /// What the weapon asks for one-handed.
    pub value: u16,
    /// Whether the stats meet it in the grip that helps it most, two-handed for strength, by the
    /// same rule [`Wield`] is decided by.
    pub met: bool,
}

/// A weapon as the weapon picker's row shows it at some stats: what it asks for, what it weighs,
/// how the stats can hold it, and what it hits for.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponCard {
    /// Its requirements in the data's order, the zero ones left out.
    pub requirements: Vec<Requirement>,
    /// Its weight.
    pub weight: f32,
    /// How the stats can hold it.
    pub wield: Wield,
    /// The infusion [`Self::attack`] is for: the one asked about when the weapon takes it,
    /// uninfused otherwise.
    pub infusion: Infusion,
    /// Attack rating by type at full upgrade, in [`DAMAGE_TYPES`] order, `None` for a type it does
    /// not deal: the numbers a ranking's row carries for the same weapon, infusion and stats.
    pub attack: [Option<f32>; 5],
}

/// What a ranking came back as.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The ranking, best first.
    Rows(Vec<ResultRow>),
    /// The stats are under the bracket's floors, and no ranking is shown. Each entry reads
    /// `VIG 5 < 12`.
    FloorViolations(Vec<String>),
}

/// A stat spread the backend chose.
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizedBuild {
    /// The starting class, `Deprived`.
    pub class: String,
    /// The soul level.
    pub sl: u16,
    /// The nine stats, in [`STAT_LABELS`] order.
    pub stats: [u16; STAT_COUNT],
    /// Whether the weapon is two-handed to meet its strength.
    pub two_handed: bool,
    /// The objective's value at these stats; `0` for a minimum build, which has no objective.
    pub value: f32,
    /// Armour and rings the stats count on, by display name: a minimum build meets its
    /// requirements with their stat bonuses and cannot wield the weapon without them; an optimized
    /// build wears these rings in place of stat points.
    pub gear: Vec<String>,
}

/// How often the damage model's best infusion is the one real builds chose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    /// Builds that recorded an infusion and were counted.
    pub n: u32,
    /// Share where the model's first choice matched, `0.0..=1.0`.
    pub top1: f32,
    /// Share where the build's infusion was the model's first or second.
    pub top2: f32,
}

/// A whole build for one weapon, ready to show and to apply.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedBuild {
    /// The starting class, `Deprived`.
    pub class: String,
    /// The soul level.
    pub sl: u16,
    /// The nine stats, in [`STAT_LABELS`] order.
    pub stats: [u16; STAT_COUNT],
    /// The weapon the build is for, by soulsplanner key, and its infusion.
    pub primary: (String, Infusion),
    /// Whether the primary is two-handed.
    pub two_handed: bool,
    /// The best one-handable other weapons, at most [`WEAPONS_1H_TOP`].
    pub weapons_1h: Vec<ResultRow>,
    /// The best two-hand-only other weapons, at most [`WEAPONS_2H_ONLY_TOP`].
    pub weapons_2h_only: Vec<ResultRow>,
    /// The rings the nearest builds wear most, at most [`SUGGESTED_RINGS`].
    pub suggested_rings: Vec<String>,
    /// Rings common to all builds that are not already suggested.
    pub common_rings: Vec<String>,
    /// Head, chest, hands, legs, by item name, `Naked` for a slot left bare; empty for a build
    /// that wears none, which only [`PanelState::allow_naked`] asks for.
    pub armor: Vec<String>,
    /// Why the armour is not four pieces, when it is not: the load cap left a slot, or every slot,
    /// bare. Never silent.
    pub armor_note: Option<String>,
    /// What the armour does in R1 trades against the build's SL bracket, over every weapon it
    /// lists: `None` for a build that wears none or has nothing to trade.
    pub trade: Option<ArmorTrade>,
    /// The same for the set picked on defense and load alone, which poise outbid when it differs.
    pub trade_previous: Option<ArmorTrade>,
    /// The spells the build was asked to cast, by soulsplanner key, in the order asked: what Apply
    /// attunes. Its stats meet every one's requirements and its ATT holds their slots.
    pub spells: Vec<String>,
    /// The same spells by display name.
    pub spell_names: Vec<String>,
    /// The attunement slots the spells cost together.
    pub slots_used: u16,
    /// The attunement slots the build's ATT gives, plus a worn Southern Ritual Band's.
    pub slots: u16,
    /// The best catalyst for each school of the spells, in spell-category order.
    pub catalysts: Vec<CatalystPick>,
    /// Per ring worn in place of stat points (the first of `suggested_rings`), what it does and
    /// the stats it lowered: `Southern Ritual Band + 2: attunement slots +3 -> ATT 20 -> 10`.
    pub ring_trades: Vec<String>,
    /// Per stat, in [`STAT_LABELS`] order, whether those rings let the build level it less than
    /// its floor or requirement: a floor such a stat is under is met by the ring, not missed.
    pub ring_lowered: [bool; STAT_COUNT],
    /// Whether this came from [`StubBackend`], so the panel can say its numbers mean nothing.
    pub stub: bool,
}

/// An armour set's R1 trades against an SL bracket: the script's `armor_trade`.
///
/// Both sides press R1 at once, each with its first hit at the game play speed; the earlier hit
/// lands, and the later one too unless the first staggered its swinger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmorTrade {
    /// The build's max poise in it: the pieces (less what unmet requirements take), the stat
    /// poise and the rings'.
    pub poise: f32,
    /// The largest counter-hit poise damage that poise holds through, `0` for none.
    pub poise_target: f32,
    /// The share of trades in which the build's R1 lands, `0.0..=1.0`.
    pub trade_rate: f32,
    /// The mean, per trade, of the build's first-hit damage when it lands less the counter's
    /// when that lands.
    pub exchange: f32,
}

/// A spell a generated build can be asked to cast: the script's `--spells`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpellRow {
    /// The soulsplanner key, `Soul_Spear`: what a build names and what Apply attunes.
    pub key: String,
    /// The display name.
    pub name: String,
    /// The attunement slots it costs.
    pub slots: u16,
    /// Its intelligence requirement.
    pub intelligence: u16,
    /// Its faith requirement.
    pub faith: u16,
}

/// The catalyst a generated build is recommended for one school of its spells.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalystPick {
    /// `sorcery`, `miracle`, `pyromancy` or `hex`.
    pub school: String,
    /// The catalyst's display name.
    pub name: String,
    /// Its full-upgrade cast power in the school's element at the build's stats.
    pub power: f32,
    /// The catalyst that would have cast harder had the build met its requirements, when one would.
    pub passed_over: Option<String>,
}

/// What a build must meet besides its weapon.
///
/// [`RecommenderBackend::optimize`], [`RecommenderBackend::generate_build`] and
/// [`RecommenderBackend::refusal`] all take it, so Optimize for weapon and Generate Build answer
/// the same question and cannot disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits<'a> {
    /// Spells it must attune and cast, by soulsplanner key; a repeat is a second copy.
    pub spells: &'a [String],
    /// The starting class it must be (`sorcerer`), or `None` for whichever class does best.
    pub class: Option<&'a str>,
    /// Whether the soul level's floors apply. They are the medians of real builds, not a game
    /// rule, so the panel can drop them ("ignore typical-build minimums").
    pub floors: bool,
    /// Who the damage objective is scored against. It changes which build scores best, never
    /// whether one exists, so [`RecommenderBackend::refusal`] does not read it.
    pub defender: &'a Defender,
    /// How that defender answers the weapon. [`RecommenderBackend::refusal`] reads it only for the
    /// class it offers instead, which is the best build's.
    pub reply: Reply,
}

impl Limits<'static> {
    /// No spells, any class, the floors applied, the average defender answering as the script's
    /// default does.
    pub const NONE: Self = Self {
        spells: &[],
        class: None,
        floors: true,
        defender: &Defender::Average,
        reply: Reply::Ring,
    };
}

impl<'a> Limits<'a> {
    /// The panel's: its chosen spells, floors setting and defender, any class.
    pub fn of(state: &'a PanelState) -> Self {
        Self {
            spells: &state.spells,
            class: None,
            floors: !state.ignore_floors,
            defender: &state.defender,
            reply: state.reply,
        }
    }
}

/// Which layer of a build no class can fit into its soul level: the script's `refusal` kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalKind {
    /// No attunement holds the spells' slots at all.
    Slots,
    /// The weapon's requirements alone do not fit.
    Weapon,
    /// The weapon fits, but not with the spells.
    Spells,
    /// The weapon and spells fit, but not above the floors.
    Floors,
    /// Anything else: a build refused after it was made, or a backend that cannot say.
    Other,
}

impl RefusalKind {
    /// The script's name for it.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "slots" => Self::Slots,
            "weapon" => Self::Weapon,
            "spells" => Self::Spells,
            "floors" => Self::Floors,
            _ => return None,
        })
    }
}

/// One change to the panel that makes a refused build possible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Build at this soul level instead: the least above the asked one that fits.
    RaiseSl(u16),
    /// Take out one copy of this spell (soulsplanner key).
    RemoveSpell(String),
    /// Drop the floors.
    IgnoreFloors,
    /// Build as this starting class (key) instead of the one asked for. Only offered when a class
    /// was asked for, which the panel's own Generate never does.
    Class(String),
}

impl Change {
    /// Make the change to `state`. `false` when `state` has nothing it applies to: a class, which
    /// the panel does not hold.
    pub fn apply(&self, state: &mut PanelState) -> bool {
        match self {
            Change::RaiseSl(sl) => state.set_sl_override(Some(*sl)),
            Change::RemoveSpell(key) => {
                let Some(at) = state.spells.iter().position(|spell| spell == key) else {
                    return false;
                };
                state.spells.remove(at);
            }
            Change::IgnoreFloors => state.ignore_floors = true,
            Change::Class(_) => return false,
        }
        true
    }
}

/// A fix the backend checked: with [`Self::change`] made, its optimizer finds a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    /// What to change.
    pub change: Change,
    /// The button's caption: `Raise SL to 139`, `Remove Climax`.
    pub label: String,
}

/// Why no build came back, and what would make one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// Which layer did not fit.
    pub kind: RefusalKind,
    /// The class the arithmetic is about, the least short one; `None` when there is none.
    pub class: Option<String>,
    /// How many points that class is short; `0` when there is no class.
    pub short: i32,
    /// The reason, a line each: what does not fit, the arithmetic, the biggest contributors, and
    /// the least soul level that fits.
    pub lines: Vec<String>,
    /// Each change that was checked to produce a build, in the script's order.
    pub fixes: Vec<Fix>,
}

impl Refusal {
    /// A refusal that is only its reason lines.
    pub fn plain(lines: Vec<String>) -> Self {
        Self {
            kind: RefusalKind::Other,
            class: None,
            short: 0,
            lines,
            fixes: Vec::new(),
        }
    }
}

/// The questions the panel asks. Every answer is for the soul level the caller passes.
pub trait RecommenderBackend: Sync {
    /// Whether the answers are placeholders.
    fn is_stub(&self) -> bool;
    /// The minimum each stat may be at `sl`, in [`STAT_LABELS`] order; `0` is no floor.
    fn floors(&self, sl: u16) -> [u16; STAT_COUNT];
    /// The weapons `stats` wield best.
    fn weapons_for(&self, stats: &[u16; STAT_COUNT], sl: u16, opts: &WeaponsForOpts) -> Outcome;
    /// Every infusion `weapon` takes, best first, scored as [`Self::weapons_for`] scores a row
    /// (the script's `--best-infusion`). No rows when `stats` cannot wield it even two-handed, or
    /// for bleed/poison when no infusion deals it. The default has no data and ranks nothing.
    fn best_infusion(
        &self,
        _weapon: &str,
        _stats: &[u16; STAT_COUNT],
        _sl: u16,
        _opts: &WeaponsForOpts,
    ) -> Outcome {
        Outcome::Rows(Vec::new())
    }
    /// The stats that make `weapon` hit hardest at `sl`, held in `grip`, meeting `limits` as
    /// [`Self::generate_build`] does. `None` when no class can.
    fn optimize(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        grip: Grip,
        limits: &Limits<'_>,
    ) -> Option<OptimizedBuild>;
    /// Why [`Self::optimize`] finds nothing for these arguments, and the fixes that were checked to
    /// find something. `None` when it finds a build, or when this backend cannot say (the default).
    fn refusal(
        &self,
        _weapon: &str,
        _infusion: Infusion,
        _sl: u16,
        _objective: Objective,
        _grip: Grip,
        _limits: &Limits<'_>,
    ) -> Option<Refusal> {
        None
    }
    /// The least a character needs to wield `weapon`. `None` when the weapon is unknown.
    fn minimum(&self, weapon: &str, infusion: Infusion, two_hand: bool) -> Option<OptimizedBuild>;
    /// The weapons the `k` builds nearest `stats` carry.
    fn similar(&self, stats: &[u16; STAT_COUNT], sl: u16, k: u16, status: StatusFilter) -> Outcome;
    /// How far the damage model agrees with real builds.
    fn calibration(&self) -> Calibration;
    /// A whole build for `weapon` held in `grip`, in armour unless `allow_naked`, that can attune
    /// and cast every one of `limits.spells`. `None` when no class can wield it and meet the
    /// spells' requirements and slots (and the floors, when `limits.floors`) at `sl` that way, or a
    /// spell is unknown. The build is from the starting class `limits.class` names (`sorcerer`)
    /// or, with `None`, whichever class does best.
    // DEBT: ds2-mods-rs-59p7 -- bundle the remaining per-build options into `Limits` as well.
    #[allow(clippy::too_many_arguments)]
    fn generate_build(
        &self,
        weapon: &str,
        infusion: Infusion,
        sl: u16,
        objective: Objective,
        allow_naked: bool,
        grip: Grip,
        limits: &Limits<'_>,
    ) -> Option<GeneratedBuild>;
    /// The spells [`Self::generate_build`] can be asked for, in the data's order. The default has
    /// no spell data and offers none.
    fn spells(&self) -> Vec<SpellRow> {
        Vec::new()
    }
    /// How many weapons `stats` wield, where that sits among the builds nearest them at `sl`, and
    /// the load `armor` (head to legs) and `rings`, by name, leave for weapons. `None` when this
    /// backend cannot say, which is the default: the stub has no neighbours to rank against.
    fn flexibility(
        &self,
        _stats: &[u16; STAT_COUNT],
        _sl: u16,
        _armor: &[String],
        _rings: &[String],
    ) -> Option<crate::flex::Flexibility> {
        None
    }
    /// What `defender` puts up at `sl`, the numbers the damage column is scored against, and how it
    /// answers a weapon under `reply`. `None` when this backend cannot say, which is the default
    /// (the stub has no corpus), or when a piece is not in its armour table.
    fn defense(&self, _sl: u16, _defender: &Defender, _reply: Reply) -> Option<DefenderDefense> {
        None
    }
    /// Every weapon `infusion` goes on, each at the build [`Self::optimize`] makes for it at `sl`
    /// (its own class, stats and rings) under `limits`, best first by [`BestWeaponRow::score`]; a
    /// weapon no class wields there is left out. The script's `--best-weapons`. Empty by default:
    /// the stub has nothing to rank.
    fn best_weapons(
        &self,
        _infusion: Infusion,
        _sl: u16,
        _objective: Objective,
        _grip: Grip,
        _limits: &Limits<'_>,
        _opts: &crate::model::BestWeaponsOpts,
    ) -> Vec<BestWeaponRow> {
        Vec::new()
    }
    /// The armour a defender can wear in `slot` (an index into [`crate::model::ARMOR_SLOTS`]) as
    /// `(soulsplanner key, name)`, `Naked` first, then the data's order. Empty by default: the stub
    /// has no armour table.
    fn armor_pieces(&self, _slot: usize) -> Vec<(String, String)> {
        Vec::new()
    }
    /// `weapon`, a soulsplanner key, as the weapon picker shows it at `stats`: with `infusion` when
    /// it takes that one, which is what choosing it keeps, and uninfused otherwise. `None` when the
    /// backend has no row for it, which is the default: the stub has none for any weapon.
    fn weapon_card(
        &self,
        _weapon: &str,
        _infusion: Infusion,
        _stats: &[u16; STAT_COUNT],
    ) -> Option<WeaponCard> {
        None
    }
}

/// Each floor `stats` is under, as `VIG 5 < 12`: the [`floor_stats`] of a build casting `spells`.
pub fn floor_violations(
    stats: &[u16; STAT_COUNT],
    floors: &[u16; STAT_COUNT],
    spells: &[String],
) -> Vec<String> {
    floor_stats(spells)
        .iter()
        .filter(|&&index| stats[index] < floors[index])
        .map(|&index| {
            format!(
                "{} {} < {}",
                STAT_LABELS[index], stats[index], floors[index]
            )
        })
        .collect()
}

/// Raise every floored stat that is under its floor up to it, at the level `state` runs at, and
/// answer whether any stat moved: what Run does a second time after it answered
/// [`Answer::FloorViolations`].
///
/// Without an override, a raised stat raises the level the stats make, and the floors with it, so
/// this repeats until nothing is under a floor. Stats only rise and are capped, so it ends.
pub fn raise_to_floors(backend: &dyn RecommenderBackend, state: &mut PanelState) -> bool {
    let mut raised = false;
    loop {
        let floors = backend.floors(state.sl());
        let mut moved = false;
        for &index in floor_stats(&state.spells) {
            let before = state.stats[index];
            if before < floors[index] {
                state.set_stat(index, floors[index]);
                moved |= state.stats[index] != before;
            }
        }
        if !moved {
            return raised;
        }
        raised = true;
    }
}

/// What the panel shows for one press of Run.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// A ranking, best first.
    Rows(Vec<ResultRow>),
    /// A stat spread.
    Build(OptimizedBuild),
    /// The stats are under the floors; nothing else is shown.
    FloorViolations(Vec<String>),
    /// The mode could not run, and why.
    Nothing(&'static str),
    /// Optimize for weapon found no build: why, and the fixes that were checked to find one.
    Refused(Refusal),
}

/// Run the panel's current mode against `backend`.
///
/// **The floors are checked here, before the backend is asked**, for the two modes that rank for
/// the player's own stats. A backend that answers anyway is overruled: its [`Outcome`] is only ever
/// shown as rows when the stats clear every floor.
pub fn ask(backend: &dyn RecommenderBackend, state: &PanelState) -> Answer {
    let sl = state.sl();
    let gate = |outcome: Outcome| match outcome {
        Outcome::Rows(rows) => Answer::Rows(rows),
        Outcome::FloorViolations(lines) => Answer::FloorViolations(lines),
    };
    match state.mode {
        Mode::WeaponsForStats | Mode::SimilarBuilds => {
            let violations = floor_violations(&state.stats, &backend.floors(sl), &state.spells);
            if !violations.is_empty() {
                return Answer::FloorViolations(violations);
            }
            gate(if state.mode == Mode::WeaponsForStats {
                let opts = WeaponsForOpts {
                    objective: state.objective,
                    defender: state.defender.clone(),
                    reply: state.reply,
                    ..state.weapons_for.clone()
                };
                backend.weapons_for(&state.stats, sl, &opts)
            } else {
                backend.similar(&state.stats, sl, state.similar_k, state.status)
            })
        }
        Mode::OptimizeForWeapon | Mode::MinimumForWeapon => {
            let Some(weapon) = state.weapon else {
                return Answer::Nothing("choose a weapon first");
            };
            if state.mode == Mode::MinimumForWeapon {
                return backend
                    .minimum(weapon, state.infusion, state.two_hand)
                    .map_or(Answer::Nothing("no class can wield it here"), Answer::Build);
            }
            let limits = Limits::of(state);
            let (infusion, objective, grip) = (state.infusion, state.objective, state.grip);
            match backend.optimize(weapon, infusion, sl, objective, grip, &limits) {
                Some(build) => Answer::Build(build),
                None => backend
                    .refusal(weapon, infusion, sl, objective, grip, &limits)
                    .map_or(
                        Answer::Nothing("no class can wield it here"),
                        Answer::Refused,
                    ),
            }
        }
    }
}

/// Rank every infusion of the panel's weapon by the build Optimize for weapon makes for it: one
/// [`RecommenderBackend::optimize`] per infusion at the panel's soul level, grip and objective,
/// best first.
///
/// Measured on 5b6c304: this used to score each infusion at the stats in the panel, and those are
/// Optimize for weapon's output rather than the player's input -- so a weapon the panel's stats
/// could not wield ranked nothing ("these stats cannot wield it"). Optimizing per infusion asks the
/// question the button is for: which infusion reaches the most at this soul level. A row's class
/// column carries the optimized build, since each infusion wants different stats.
pub fn best_infusion(backend: &dyn RecommenderBackend, state: &PanelState) -> Answer {
    let Some(weapon) = state.weapon else {
        return Answer::Nothing("choose a weapon first");
    };
    let Some(row) = weapons::by_key(weapon) else {
        return Answer::Nothing("the chosen weapon is not in the weapon table");
    };
    let sl = state.sl();
    let mut rows: Vec<ResultRow> = row
        .infusions()
        .into_iter()
        .filter_map(|infusion| {
            let build = backend.optimize(
                weapon,
                infusion,
                sl,
                state.objective,
                state.grip,
                &Limits::of(state),
            )?;
            Some(ResultRow {
                weapon: row.name.to_owned(),
                infusion,
                damage: build.value,
                ar_by_type: [0.0; 5],
                grip: if build.two_handed { "2H" } else { "1H" }.to_owned(),
                two_hand_only: false,
                hyperarmor: None,
                counter: None,
                class: optimized_stats(&build),
            })
        })
        .collect();
    if rows.is_empty() {
        return Answer::Nothing("no infusion to rank: no class can wield it at this soul level");
    }
    rows.sort_by(|a, b| b.damage.total_cmp(&a.damage));
    Answer::Rows(rows)
}

/// `Deprived SL 74: STR 11 DEX 42 INT 9 FTH 9`, the stats that decide an infusion.
fn optimized_stats(build: &OptimizedBuild) -> String {
    let [.., strength, dexterity, _, intelligence, faith] = build.stats;
    format!(
        "{} SL {}: STR {strength} DEX {dexterity} INT {intelligence} FTH {faith}",
        build.class, build.sl
    )
}

/// How far the first row's score is ahead of the second's, as a fraction of the second: `0.25`
/// is 25% more. `None` without a second row or when it scores nothing. The script's
/// `infusion_margin`.
pub fn infusion_margin(rows: &[ResultRow]) -> Option<f32> {
    match rows {
        [best, second, ..] if second.damage > 0.0 => Some(best.damage / second.damage - 1.0),
        _ => None,
    }
}

/// Generate a build for the panel's weapon, refusing one under its own floors.
///
/// `class` is the live character's starting class, when there is one: the build is generated for
/// that class alone, and a build that comes back for another class, or with a stat under that
/// class's base, is refused here whatever the backend did. `None` lets any class win.
///
/// # Errors
///
/// The reason as lines for the panel: no weapon chosen, no build possible (with the backend's
/// [`RecommenderBackend::refusal`] -- its arithmetic and checked fixes -- when it has one), a
/// build for the wrong class or under its base, or each floor the backend's build is under, unless
/// the panel ignores the floors.
pub fn generate(
    backend: &dyn RecommenderBackend,
    state: &PanelState,
    class: Option<StartingClass>,
) -> Result<GeneratedBuild, Refusal> {
    let Some(weapon) = state.weapon else {
        return Err(Refusal::plain(vec!["choose a weapon first".to_owned()]));
    };
    let limits = Limits {
        class: class.map(StartingClass::key),
        ..Limits::of(state)
    };
    let (infusion, sl, objective, grip) = (state.infusion, state.sl(), state.objective, state.grip);
    let Some(build) = backend.generate_build(
        weapon,
        infusion,
        sl,
        objective,
        state.allow_naked,
        grip,
        &limits,
    ) else {
        if let Some(refusal) = backend.refusal(weapon, infusion, sl, objective, grip, &limits) {
            return Err(refusal);
        }
        let who = class.map_or_else(
            || "no class".to_owned(),
            |class| format!("a {}", class.key()),
        );
        let can = if class.is_some() { "cannot" } else { "can" };
        return Err(Refusal::plain(vec![if state.spells.is_empty() {
            format!("{who} {can} wield it at this soul level")
        } else {
            format!("{who} {can} wield it and cast the chosen spells at this soul level")
        }]));
    };
    if let Some(class) = class
        && let Err(refusal) = check_build(&build.class, class, &game_order(&build.stats))
    {
        return Err(Refusal::plain(vec![refusal.to_string()]));
    }
    // A floor a worn ring stands in for (a Life Ring's HP for VIG, ...) is met by the ring.
    let violations = if state.ignore_floors {
        Vec::new()
    } else {
        let mut floors = backend.floors(build.sl);
        for (floor, &lowered) in floors.iter_mut().zip(&build.ring_lowered) {
            if lowered {
                *floor = 0;
            }
        }
        floor_violations(&build.stats, &floors, &state.spells)
    };
    if violations.is_empty() {
        Ok(build)
    } else {
        Err(Refusal::plain(violations))
    }
}

/// Nine stats in [`STAT_LABELS`] order, in the game's order instead
/// (`ds2_build_import_core::class::GAME_ORDER_NAMES`): adaptability moves from seventh to last.
pub fn game_order(stats: &[u16; STAT_COUNT]) -> [u16; STAT_COUNT] {
    to_stats(stats).in_game_order()
}

/// Nine stats in [`STAT_LABELS`] order, in the planner [`Stats`] a [`Build`] carries.
fn to_stats(stats: &[u16; STAT_COUNT]) -> Stats {
    let [
        vigor,
        endurance,
        vitality,
        attunement,
        strength,
        dexterity,
        adaptability,
        intelligence,
        faith,
    ] = *stats;
    Stats {
        vigor,
        endurance,
        vitality,
        attunement,
        strength,
        dexterity,
        adaptability,
        intelligence,
        faith,
    }
}

/// Nine stats in the game's order, in [`STAT_LABELS`] order instead: [`game_order`] undone.
pub fn planner_order(game: &[u16; STAT_COUNT]) -> [u16; STAT_COUNT] {
    let [vig, end, vit, att, str_, dex, int, fth, adp] = *game;
    [vig, end, vit, att, str_, dex, adp, int, fth]
}

/// Split a ranking into the one-handable and two-hand-only lists a generated build carries.
///
/// One row per weapon -- the first, which is its best infusion -- and never `primary_name`.
pub fn split_weapons(ranked: &[ResultRow], primary_name: &str) -> (Vec<ResultRow>, Vec<ResultRow>) {
    let mut seen = vec![primary_name];
    let (mut one, mut two) = (Vec::new(), Vec::new());
    for row in ranked {
        if seen.contains(&row.weapon.as_str()) {
            continue;
        }
        seen.push(&row.weapon);
        if row.two_hand_only {
            two.push(row.clone());
        } else {
            one.push(row.clone());
        }
    }
    one.truncate(WEAPONS_1H_TOP);
    two.truncate(WEAPONS_2H_ONLY_TOP);
    (one, two)
}

/// The rings common to all builds: worn by at least [`COMMON_RING_PERCENT`] of `total_builds`,
/// most worn first, leaving out any already `suggested`.
pub fn common_rings(worn: &[(&str, u32)], total_builds: u32, suggested: &[String]) -> Vec<String> {
    let mut common: Vec<(&str, u32)> = worn
        .iter()
        .copied()
        .filter(|&(ring, count)| {
            u64::from(count) * 100 >= u64::from(COMMON_RING_PERCENT) * u64::from(total_builds)
                && !suggested.iter().any(|name| name == ring)
        })
        .collect();
    common.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    common
        .into_iter()
        .map(|(ring, _)| ring.to_owned())
        .collect()
}

/// Every ring a generated build grants: [`SUGGESTED_RING_COPIES`] of each suggestion, then one of
/// each common ring that is not a suggestion.
pub fn ring_grants(suggested: &[String], common: &[String]) -> Vec<String> {
    let mut grants = Vec::new();
    for ring in suggested.iter().take(SUGGESTED_RINGS) {
        grants.extend(std::iter::repeat_n(ring.clone(), SUGGESTED_RING_COPIES));
    }
    for ring in common {
        if !suggested.contains(ring) && !grants.contains(ring) {
            grants.push(ring.clone());
        }
    }
    grants
}

/// What a planner build calls an empty weapon hand.
const EMPTY_HAND: &str = "Bare_Fists";
/// What a planner build calls an empty ring slot.
const EMPTY_RING: &str = "No_Ring";
/// What a planner build calls an empty armour slot.
const EMPTY_ARMOR: &str = "Naked";
/// What a build with no covenant names.
const NO_COVENANT: &str = "No_Covenant";

/// Turn a generated build into what `ds2-build-import` applies: a planner [`Build`], plus the extra
/// grants its fixed slots cannot hold.
///
/// The build wears the suggested rings, holds the primary in the right hand, wears the armour and
/// attunes its spells. The extras are everything else the build lists: the second and third copy
/// of each suggested ring, one of each common ring, every recommended weapon at its ranked
/// infusion, and each recommended catalyst uninfused. They are
/// granted through the same per-name count as the build's own gear, so a player who already holds
/// a copy is given one fewer.
pub fn to_import(generated: &GeneratedBuild) -> (Build, Vec<(String, Infusion)>) {
    let stats = to_stats(&generated.stats);
    let (primary, infusion) = &generated.primary;
    // `LH1, RH1, LH2, RH2, LH3, RH3`, each followed by its infusion: the primary is RH1.
    let mut hands = vec![
        EMPTY_HAND.to_owned(),
        weapons::planner_name(Infusion::None).to_owned(),
    ];
    hands.push(primary.clone());
    hands.push(weapons::planner_name(*infusion).to_owned());
    while hands.len() < 12 {
        hands.push(EMPTY_HAND.to_owned());
        hands.push(weapons::planner_name(Infusion::None).to_owned());
    }
    let worn: Vec<String> = generated
        .suggested_rings
        .iter()
        .take(SUGGESTED_RINGS)
        .cloned()
        .collect();
    let mut rings = worn.clone();
    rings.resize(SUGGESTED_RINGS, EMPTY_RING.to_owned());
    let mut armor: Vec<String> = generated.armor.iter().take(4).cloned().collect();
    armor.resize(4, EMPTY_ARMOR.to_owned());
    let build = Build {
        id: 0,
        class: generated.class.to_lowercase().replace(' ', "_"),
        gender: 0,
        covenant: NO_COVENANT.to_owned(),
        grip: i64::from(generated.two_handed),
        armor,
        weapons: hands,
        rings,
        spells: generated.spells.clone(),
        items: Vec::new(),
        stats,
    };

    // The rings the build does not already wear: every grant less one copy of each worn ring.
    let mut extras: Vec<(String, Infusion)> = Vec::new();
    let mut unworn = worn;
    for ring in ring_grants(&generated.suggested_rings, &generated.common_rings) {
        if let Some(at) = unworn.iter().position(|name| *name == ring) {
            unworn.remove(at);
            continue;
        }
        extras.push((ring, Infusion::None));
    }
    for row in generated
        .weapons_1h
        .iter()
        .chain(&generated.weapons_2h_only)
    {
        extras.push((row.weapon.clone(), row.infusion));
    }
    for pick in &generated.catalysts {
        let catalyst = (pick.name.clone(), Infusion::None);
        if !extras.contains(&catalyst) {
            extras.push(catalyst);
        }
    }
    (build, extras)
}

/// Fixed answers, so the panel can be drawn and applied before the real ranking exists.
///
/// Every number it returns is a placeholder. The generated build is a real, valid one -- the
/// Moonlight Greatsword on a two-handed Deprived at soul level 33 -- because Apply puts it on a
/// character, and a stub that applied nonsense would test nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubBackend;

/// The stub build's stats: VIG 10 END 16 VIT 7 ATT 10 STR 6 DEX 13 ADP 8 INT 10 FTH 6.
pub const STUB_STATS: [u16; STAT_COUNT] = [10, 16, 7, 10, 6, 13, 8, 10, 6];

/// The stub's floors: its own build clears them, a fresh Deprived does not.
const STUB_FLOORS: [u16; STAT_COUNT] = [10, 0, 7, 10, 0, 0, 8, 0, 0];

/// The stub's ranking, best first, by soulsplanner key: infusion, placeholder damage, 2H only.
///
/// The primary and a second Falchion row are in it on purpose, so the split has something to drop.
const STUB_RANKING: [(&str, Infusion, f32, bool); 26] = [
    ("Moonlight_Greatsword", Infusion::None, 410.0, false),
    ("Giant_Stone_Axe", Infusion::Raw, 402.0, true),
    ("Falchion", Infusion::Raw, 395.0, false),
    ("Falchion", Infusion::Bleed, 390.0, false),
    ("Scimitar", Infusion::Raw, 388.0, false),
    ("Greataxe", Infusion::Raw, 385.0, true),
    ("Shotel", Infusion::Bleed, 380.0, false),
    ("Royal_Dirk", Infusion::Raw, 372.0, false),
    ("Demons_Great_Hammer", Infusion::Raw, 368.0, true),
    ("Bandits_Knife", Infusion::Bleed, 366.0, false),
    ("Blacksteel_Katana", Infusion::Raw, 361.0, false),
    ("Eleum_Loyce", Infusion::None, 355.0, false),
    ("Large_Club", Infusion::Raw, 350.0, true),
    ("Melu_Scimitar", Infusion::Raw, 347.0, false),
    ("Hand_Axe", Infusion::Raw, 342.0, false),
    ("Drakekeepers_Warpick", Infusion::Raw, 339.0, true),
    ("Infantry_Axe", Infusion::Raw, 336.0, false),
    ("Work_Hook", Infusion::Bleed, 330.0, false),
    ("Pickaxe", Infusion::Raw, 328.0, true),
    ("Retainers_Short_Sword", Infusion::Raw, 322.0, false),
    ("Whip", Infusion::Raw, 318.0, false),
    ("Drangleic_Sword", Infusion::Raw, 312.0, false),
    ("Royal_Greatsword", Infusion::Raw, 305.0, false),
    ("Black_Knight_Greatsword", Infusion::Raw, 300.0, false),
    ("Old_Mirrah_Greatsword", Infusion::Raw, 296.0, false),
    ("Helix_Halberd", Infusion::Raw, 290.0, false),
];

/// The stub's ring counts over its pretend corpus of [`STUB_CORPUS`] builds.
const STUB_RINGS_WORN: [(&str, u32); 5] = [
    ("Chloranthy Ring", 240),
    ("Dexterity Ring", 180),
    ("Ring of Blades", 130),
    ("Third Dragon Ring", 60),
    ("Flynn's Ring", 20),
];

/// How many builds the stub pretends to have read.
const STUB_CORPUS: u32 = 1000;

impl StubBackend {
    fn row(key: &str, infusion: Infusion, damage: f32, two_hand_only: bool) -> Option<ResultRow> {
        let table = weapons::by_key(key)?;
        Some(ResultRow {
            weapon: table.name.to_owned(),
            infusion,
            damage,
            ar_by_type: [damage, 0.0, 0.0, 0.0, 0.0],
            grip: if two_hand_only { "2H only" } else { "1H" }.to_owned(),
            two_hand_only,
            hyperarmor: None,
            counter: None,
            class: table.class.to_owned(),
        })
    }

    fn ranking() -> Vec<ResultRow> {
        STUB_RANKING
            .iter()
            .filter_map(|&(key, infusion, damage, two)| Self::row(key, infusion, damage, two))
            .collect()
    }

    fn stub_build(sl: u16, two_handed: bool) -> OptimizedBuild {
        OptimizedBuild {
            class: "Deprived".to_owned(),
            sl,
            stats: STUB_STATS,
            two_handed,
            value: 410.0,
            gear: Vec::new(),
        }
    }
}

impl RecommenderBackend for StubBackend {
    fn is_stub(&self) -> bool {
        true
    }

    fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
        STUB_FLOORS
    }

    fn weapons_for(&self, _stats: &[u16; STAT_COUNT], _sl: u16, opts: &WeaponsForOpts) -> Outcome {
        let mut rows: Vec<ResultRow> = Self::ranking()
            .into_iter()
            .filter(|row| !opts.one_hand || !row.two_hand_only)
            .filter(|row| opts.class.as_deref().is_none_or(|class| row.class == class))
            .collect();
        if opts.per_class {
            let mut seen: Vec<String> = Vec::new();
            rows.retain(|row| {
                let fresh = !seen.contains(&row.class);
                seen.push(row.class.clone());
                fresh
            });
        }
        Outcome::Rows(rows)
    }

    fn optimize(
        &self,
        weapon: &str,
        _infusion: Infusion,
        sl: u16,
        _objective: Objective,
        grip: Grip,
        limits: &Limits<'_>,
    ) -> Option<OptimizedBuild> {
        // As `generate_build`: no spells to offer, and a Deprived's build only.
        if !limits.spells.is_empty()
            || limits
                .class
                .is_some_and(|class| !class.eq_ignore_ascii_case("deprived"))
        {
            return None;
        }
        weapons::by_key(weapon).map(|_| Self::stub_build(sl, grip.two_handed()))
    }

    fn minimum(&self, weapon: &str, _infusion: Infusion, two_hand: bool) -> Option<OptimizedBuild> {
        weapons::by_key(weapon)
            .map(|_| Self::stub_build(crate::model::soul_level(&STUB_STATS), two_hand))
    }

    fn similar(
        &self,
        _stats: &[u16; STAT_COUNT],
        _sl: u16,
        k: u16,
        status: StatusFilter,
    ) -> Outcome {
        let rows = Self::ranking()
            .into_iter()
            .filter(|row| {
                !status.any()
                    || (status.bleed && row.infusion == Infusion::Bleed)
                    || (status.poison && row.infusion == Infusion::Poison)
            })
            .enumerate()
            .map(|(rank, mut row)| {
                // A count of neighbours, not damage, in this mode.
                row.damage = f32::from(k.saturating_sub(u16::try_from(rank * 3).unwrap_or(k)));
                row
            })
            .filter(|row| row.damage > 0.0)
            .collect();
        Outcome::Rows(rows)
    }

    fn calibration(&self) -> Calibration {
        Calibration {
            n: 0,
            top1: 0.0,
            top2: 0.0,
        }
    }

    fn generate_build(
        &self,
        _weapon: &str,
        _infusion: Infusion,
        _sl: u16,
        _objective: Objective,
        allow_naked: bool,
        grip: Grip,
        limits: &Limits<'_>,
    ) -> Option<GeneratedBuild> {
        // The stub offers no spells (`spells` is the trait's empty default), so none can be asked
        // of it; a caller that asks anyway gets no build rather than one that ignores them.
        if !limits.spells.is_empty() {
            return None;
        }
        // The stub's one build is a Deprived's; it has nothing to offer any other class.
        if limits
            .class
            .is_some_and(|class| !class.eq_ignore_ascii_case("deprived"))
        {
            return None;
        }
        let primary = weapons::by_key("Moonlight_Greatsword")?;
        let (weapons_1h, weapons_2h_only) = split_weapons(&Self::ranking(), primary.name);
        let suggested_rings: Vec<String> = ["Dexterity Ring", "Ring of Knowledge", "Strength Ring"]
            .map(str::to_owned)
            .to_vec();
        let common_rings = common_rings(&STUB_RINGS_WORN, STUB_CORPUS, &suggested_rings);
        let built = Self::stub_build(crate::model::soul_level(&STUB_STATS), grip.two_handed());
        Some(GeneratedBuild {
            class: built.class,
            sl: built.sl,
            stats: built.stats,
            primary: (primary.key.to_owned(), Infusion::None),
            two_handed: built.two_handed,
            weapons_1h,
            weapons_2h_only,
            suggested_rings,
            common_rings,
            armor: if allow_naked {
                Vec::new()
            } else {
                vec!["Desert Sorceress Hood".to_owned()]
            },
            armor_note: None,
            trade: None,
            trade_previous: None,
            spells: Vec::new(),
            spell_names: Vec::new(),
            slots_used: 0,
            slots: 1,
            catalysts: Vec::new(),
            ring_trades: Vec::new(),
            ring_lowered: [false; STAT_COUNT],
            stub: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generated() -> GeneratedBuild {
        let state = PanelState {
            weapon: Some("Moonlight_Greatsword"),
            ..PanelState::default()
        };
        generate(&StubBackend, &state, None).expect("the stub build is valid")
    }

    #[test]
    fn the_stub_build_is_the_one_asked_for() {
        let build = generated();
        assert_eq!(build.class, "Deprived");
        assert_eq!(build.sl, 33);
        assert_eq!(build.stats, [10, 16, 7, 10, 6, 13, 8, 10, 6]);
        assert!(build.two_handed);
        assert_eq!(
            build.primary,
            ("Moonlight_Greatsword".to_owned(), Infusion::None)
        );
        assert_eq!(
            build.suggested_rings,
            ["Dexterity Ring", "Ring of Knowledge", "Strength Ring"]
        );
        assert_eq!(build.armor, ["Desert Sorceress Hood"]);
        assert!(build.stub);
    }

    /// 15 one-handable and 5 two-hand-only, one row per weapon, never the primary.
    #[test]
    fn the_weapon_lists_are_fifteen_and_five() {
        let build = generated();
        assert_eq!(build.weapons_1h.len(), WEAPONS_1H_TOP);
        assert_eq!(build.weapons_2h_only.len(), WEAPONS_2H_ONLY_TOP);
        assert!(build.weapons_1h.iter().all(|row| !row.two_hand_only));
        assert!(build.weapons_2h_only.iter().all(|row| row.two_hand_only));
        let names: Vec<&str> = build
            .weapons_1h
            .iter()
            .chain(&build.weapons_2h_only)
            .map(|row| row.weapon.as_str())
            .collect();
        assert!(!names.contains(&"Moonlight Greatsword"));
        let falchions = names.iter().filter(|name| **name == "Falchion").count();
        assert_eq!(
            falchions, 1,
            "a weapon is listed once, at its best infusion"
        );
        let falchion = build.weapons_1h.iter().find(|row| row.weapon == "Falchion");
        assert_eq!(falchion.map(|row| row.infusion), Some(Infusion::Raw));
    }

    /// The script's `infusion_margin`: best over runner-up as a fraction of the runner-up.
    #[test]
    fn the_infusion_margin_is_over_the_runner_up() {
        let scored = |damage: f32| ResultRow {
            weapon: "Caestus".to_owned(),
            infusion: Infusion::None,
            damage,
            ar_by_type: [0.0; 5],
            grip: "1H".to_owned(),
            two_hand_only: false,
            hyperarmor: None,
            counter: None,
            class: String::new(),
        };
        assert_eq!(
            infusion_margin(&[scored(300.0), scored(200.0), scored(50.0)]),
            Some(0.5)
        );
        assert_eq!(infusion_margin(&[scored(300.0)]), None);
        assert_eq!(infusion_margin(&[scored(300.0), scored(0.0)]), None);
    }

    /// It waits for a weapon, then ranks one optimized build per infusion the weapon takes --
    /// whatever the panel's own stats are, since Optimize for weapon picks those.
    #[test]
    fn best_infusion_optimizes_every_infusion_whatever_the_panels_stats() {
        let mut state = PanelState::default();
        assert_eq!(
            best_infusion(&StubBackend, &state),
            Answer::Nothing("choose a weapon first")
        );
        // Measured on 5b6c304: at SL 10 stats this weapon ranked nothing.
        state.choose_weapon("Dragonslayers_Crescent_Axe");
        let Answer::Rows(rows) = best_infusion(&StubBackend, &state) else {
            panic!("no rows for a weapon the stats cannot wield");
        };
        assert_eq!(
            rows.len(),
            weapons::infusions_for("Dragonslayers_Crescent_Axe").len()
        );
        assert!(rows.windows(2).all(|pair| pair[0].damage >= pair[1].damage));
    }

    #[test]
    fn the_split_keeps_fewer_when_there_are_fewer() {
        let (one, two) = split_weapons(&[], "x");
        assert!(one.is_empty() && two.is_empty());
    }

    /// Common: at least a tenth of every build, most worn first, suggestions left out.
    #[test]
    fn common_rings_are_a_tenth_of_all_builds() {
        let suggested = vec!["Dexterity Ring".to_owned()];
        let common = common_rings(
            &[
                ("Chloranthy Ring", 240),
                ("Dexterity Ring", 180),
                ("Third Dragon Ring", 99),
                ("Ring of Blades", 100),
            ],
            1000,
            &suggested,
        );
        assert_eq!(common, ["Chloranthy Ring", "Ring of Blades"]);
    }

    /// Three of each suggestion, then one of each common ring that is not one.
    #[test]
    fn rings_are_three_copies_plus_one_common() {
        let suggested = ["A", "B"].map(str::to_owned).to_vec();
        let common = ["B", "C"].map(str::to_owned).to_vec();
        assert_eq!(
            ring_grants(&suggested, &common),
            ["A", "A", "A", "B", "B", "B", "C"]
        );
        let five = ["A", "B", "C", "D", "E"].map(str::to_owned).to_vec();
        assert_eq!(
            ring_grants(&five, &[]).len(),
            SUGGESTED_RINGS * SUGGESTED_RING_COPIES
        );
    }

    /// No rows are shown for stats under a floor -- and the backend is never asked.
    #[test]
    fn a_build_under_its_floors_gets_no_results() {
        let mut state = PanelState::default();
        for mode in [Mode::WeaponsForStats, Mode::SimilarBuilds] {
            state.mode = mode;
            match ask(&StubBackend, &state) {
                Answer::FloorViolations(lines) => {
                    // No spells chosen, so no ATT floor.
                    assert_eq!(lines, ["VIG 6 < 10", "VIT 6 < 7", "ADP 6 < 8"]);
                }
                other => panic!("{mode:?} answered {other:?} for a fresh Deprived"),
            }
            state.stats = STUB_STATS;
            assert!(matches!(ask(&StubBackend, &state), Answer::Rows(rows) if !rows.is_empty()));
            state.stats = [6; STAT_COUNT];
        }
    }

    /// Run's second press after a floor refusal: every floored stat reaches its floor, and floors
    /// that rise with the level the raise makes are chased until none is left unmet.
    #[test]
    fn raising_to_the_floors_clears_every_violation() {
        let mut state = PanelState {
            stats: [6; STAT_COUNT],
            ..PanelState::default()
        };
        let violations = |state: &PanelState| {
            floor_violations(&state.stats, &StubBackend.floors(state.sl()), &state.spells)
        };
        assert!(!violations(&state).is_empty());
        assert!(raise_to_floors(&StubBackend, &mut state));
        assert!(violations(&state).is_empty());
        // Stats already over their floors are left as they are.
        assert_eq!(state.stats[1], 6);
        assert!(!raise_to_floors(&StubBackend, &mut state));

        /// VIG's floor is a third of the level, so raising VIG raises its own floor.
        struct Rising;
        impl RecommenderBackend for Rising {
            fn is_stub(&self) -> bool {
                true
            }
            fn floors(&self, sl: u16) -> [u16; STAT_COUNT] {
                let mut floors = [0; STAT_COUNT];
                floors[0] = sl / 3;
                floors
            }
            fn weapons_for(&self, s: &[u16; STAT_COUNT], sl: u16, o: &WeaponsForOpts) -> Outcome {
                StubBackend.weapons_for(s, sl, o)
            }
            fn optimize(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<OptimizedBuild> {
                StubBackend.optimize(w, i, sl, o, g, l)
            }
            fn minimum(&self, w: &str, i: Infusion, t: bool) -> Option<OptimizedBuild> {
                StubBackend.minimum(w, i, t)
            }
            fn similar(&self, s: &[u16; STAT_COUNT], sl: u16, k: u16, f: StatusFilter) -> Outcome {
                StubBackend.similar(s, sl, k, f)
            }
            fn calibration(&self) -> Calibration {
                StubBackend.calibration()
            }
            fn generate_build(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                n: bool,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<GeneratedBuild> {
                StubBackend.generate_build(w, i, sl, o, n, g, l)
            }
        }
        let mut state = PanelState {
            stats: [20; STAT_COUNT],
            ..PanelState::default()
        };
        state.stats[0] = 1;
        assert!(raise_to_floors(&Rising, &mut state));
        assert!(
            floor_violations(&state.stats, &Rising.floors(state.sl()), &state.spells).is_empty()
        );
    }

    /// Weapons for stats ranks by the panel's objective: Bleed there reaches the backend as Bleed.
    #[test]
    fn weapons_for_stats_is_asked_for_the_panels_objective() {
        struct Echo;
        impl RecommenderBackend for Echo {
            fn is_stub(&self) -> bool {
                true
            }
            fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
                [0; STAT_COUNT]
            }
            fn weapons_for(&self, s: &[u16; STAT_COUNT], sl: u16, o: &WeaponsForOpts) -> Outcome {
                let mut rows = rows_of(StubBackend.weapons_for(s, sl, o));
                rows.truncate(1);
                rows[0].grip = o.objective.label().to_owned();
                Outcome::Rows(rows)
            }
            fn optimize(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<OptimizedBuild> {
                StubBackend.optimize(w, i, sl, o, g, l)
            }
            fn minimum(&self, w: &str, i: Infusion, t: bool) -> Option<OptimizedBuild> {
                StubBackend.minimum(w, i, t)
            }
            fn similar(&self, s: &[u16; STAT_COUNT], sl: u16, k: u16, f: StatusFilter) -> Outcome {
                StubBackend.similar(s, sl, k, f)
            }
            fn calibration(&self) -> Calibration {
                StubBackend.calibration()
            }
            fn generate_build(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                n: bool,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<GeneratedBuild> {
                StubBackend.generate_build(w, i, sl, o, n, g, l)
            }
        }
        fn rows_of(outcome: Outcome) -> Vec<ResultRow> {
            match outcome {
                Outcome::Rows(rows) => rows,
                Outcome::FloorViolations(lines) => panic!("under floors: {lines:?}"),
            }
        }
        let mut state = PanelState {
            mode: Mode::WeaponsForStats,
            ..PanelState::default()
        };
        for objective in Objective::ALL {
            state.objective = objective;
            match ask(&Echo, &state) {
                Answer::Rows(rows) => assert_eq!(rows[0].grip, objective.label()),
                other => panic!("{objective:?} answered {other:?}"),
            }
        }
    }

    /// A generated build under its own floors is refused, whatever the backend says.
    #[test]
    fn a_generated_build_under_its_floors_is_refused() {
        struct Strict;
        impl RecommenderBackend for Strict {
            fn is_stub(&self) -> bool {
                true
            }
            fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
                [20; STAT_COUNT]
            }
            fn weapons_for(&self, s: &[u16; STAT_COUNT], sl: u16, o: &WeaponsForOpts) -> Outcome {
                StubBackend.weapons_for(s, sl, o)
            }
            fn optimize(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<OptimizedBuild> {
                StubBackend.optimize(w, i, sl, o, g, l)
            }
            fn minimum(&self, w: &str, i: Infusion, t: bool) -> Option<OptimizedBuild> {
                StubBackend.minimum(w, i, t)
            }
            fn similar(&self, s: &[u16; STAT_COUNT], sl: u16, k: u16, f: StatusFilter) -> Outcome {
                StubBackend.similar(s, sl, k, f)
            }
            fn calibration(&self) -> Calibration {
                StubBackend.calibration()
            }
            fn generate_build(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                naked: bool,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<GeneratedBuild> {
                StubBackend.generate_build(w, i, sl, o, naked, g, l)
            }
        }
        let state = PanelState {
            weapon: Some("Moonlight_Greatsword"),
            ..PanelState::default()
        };
        let refused = generate(&Strict, &state, None).expect_err("under every floor");
        // No spells: VIG, VIT and ADP, never ATT.
        assert_eq!(refused.lines.len(), floor_stats(&state.spells).len());
        assert!(refused.lines.iter().all(|line| !line.starts_with("ATT")));
        let ignoring = PanelState {
            ignore_floors: true,
            ..state.clone()
        };
        assert!(
            generate(&Strict, &ignoring, None).is_ok(),
            "the floors are dropped when the panel ignores them"
        );
        assert!(
            generate(&StubBackend, &PanelState::default(), None).is_err(),
            "no weapon"
        );
    }

    /// ATT has a floor only for a build that casts something other than Warmth.
    #[test]
    fn attunement_is_floored_only_with_spells() {
        let (stats, floors) = ([1; STAT_COUNT], [10; STAT_COUNT]);
        let att = |lines: Vec<String>| lines.iter().any(|line| line.starts_with("ATT"));
        let keys = |keys: &[&str]| keys.iter().map(|&key| key.to_owned()).collect::<Vec<_>>();
        assert!(!att(floor_violations(&stats, &floors, &[])));
        assert!(!att(floor_violations(&stats, &floors, &keys(&["Warmth"]))));
        assert!(!att(floor_violations(
            &stats,
            &floors,
            &keys(&["Warmth", "Warmth"])
        )));
        assert!(att(floor_violations(&stats, &floors, &keys(&["Heal"]))));
        assert!(att(floor_violations(
            &stats,
            &floors,
            &keys(&["Warmth", "Heal"])
        )));
    }

    /// The measured case: a Sorcerer's panel generated a Warrior at SL 90 and the apply wrote it,
    /// ATT 12 -> 6 and INT 14 -> 5. A backend that ignores the class asked for is overruled here.
    #[test]
    fn a_build_for_another_class_is_refused_for_a_live_character() {
        struct Careless;
        impl RecommenderBackend for Careless {
            fn is_stub(&self) -> bool {
                false
            }
            fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
                [0; STAT_COUNT]
            }
            fn weapons_for(&self, s: &[u16; STAT_COUNT], sl: u16, o: &WeaponsForOpts) -> Outcome {
                StubBackend.weapons_for(s, sl, o)
            }
            fn optimize(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<OptimizedBuild> {
                StubBackend.optimize(w, i, sl, o, g, l)
            }
            fn minimum(&self, w: &str, i: Infusion, t: bool) -> Option<OptimizedBuild> {
                StubBackend.minimum(w, i, t)
            }
            fn similar(&self, s: &[u16; STAT_COUNT], sl: u16, k: u16, f: StatusFilter) -> Outcome {
                StubBackend.similar(s, sl, k, f)
            }
            fn calibration(&self) -> Calibration {
                StubBackend.calibration()
            }
            fn generate_build(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                naked: bool,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<GeneratedBuild> {
                let any = Limits { class: None, ..*l };
                let mut build = StubBackend.generate_build(w, i, sl, o, naked, g, &any)?;
                // The log's Warrior: game order [22, 6, 11, 6, 28, 42, 5, 5, 18].
                build.class = "Warrior".to_owned();
                build.stats = planner_order(&[22, 6, 11, 6, 28, 42, 5, 5, 18]);
                build.sl = crate::model::soul_level(&build.stats);
                Some(build)
            }
        }
        let state = PanelState {
            weapon: Some("Moonlight_Greatsword"),
            ..PanelState::default()
        };
        let generated = generate(&Careless, &state, None).expect("any class, when none is live");
        assert_eq!(generated.sl, 90);
        let refused = generate(&Careless, &state, Some(StartingClass::Sorcerer))
            .expect_err("a Warrior for a Sorcerer");
        assert!(
            refused.lines[0].contains("warrior") && refused.lines[0].contains("sorcerer"),
            "{refused:?}"
        );
        // Named a Sorcerer, the same spread is still under the Sorcerer's base.
        struct Relabelled;
        impl RecommenderBackend for Relabelled {
            fn is_stub(&self) -> bool {
                false
            }
            fn floors(&self, _sl: u16) -> [u16; STAT_COUNT] {
                [0; STAT_COUNT]
            }
            fn weapons_for(&self, s: &[u16; STAT_COUNT], sl: u16, o: &WeaponsForOpts) -> Outcome {
                StubBackend.weapons_for(s, sl, o)
            }
            fn optimize(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<OptimizedBuild> {
                StubBackend.optimize(w, i, sl, o, g, l)
            }
            fn minimum(&self, w: &str, i: Infusion, t: bool) -> Option<OptimizedBuild> {
                StubBackend.minimum(w, i, t)
            }
            fn similar(&self, s: &[u16; STAT_COUNT], sl: u16, k: u16, f: StatusFilter) -> Outcome {
                StubBackend.similar(s, sl, k, f)
            }
            fn calibration(&self) -> Calibration {
                StubBackend.calibration()
            }
            fn generate_build(
                &self,
                w: &str,
                i: Infusion,
                sl: u16,
                o: Objective,
                naked: bool,
                g: Grip,
                l: &Limits<'_>,
            ) -> Option<GeneratedBuild> {
                let mut build = Careless.generate_build(w, i, sl, o, naked, g, l)?;
                build.class = "Sorcerer".to_owned();
                Some(build)
            }
        }
        let refused = generate(&Relabelled, &state, Some(StartingClass::Sorcerer))
            .expect_err("under the Sorcerer base");
        assert!(
            refused.lines[0].contains("attunement 6 < 12")
                && refused.lines[0].contains("intelligence 5 < 14"),
            "{refused:?}"
        );
    }

    /// The stub has only a Deprived build, and offers it to no other class.
    #[test]
    fn the_stub_generates_for_a_deprived_only() {
        let state = PanelState {
            weapon: Some("Moonlight_Greatsword"),
            ..PanelState::default()
        };
        assert!(generate(&StubBackend, &state, Some(StartingClass::Deprived)).is_ok());
        let refused = generate(&StubBackend, &state, Some(StartingClass::Sorcerer))
            .expect_err("no Sorcerer build");
        assert_eq!(
            refused.lines,
            ["a sorcerer cannot wield it at this soul level"]
        );
    }

    /// Each fix makes the change its label names, to the panel state Generate reads.
    #[test]
    fn a_fix_changes_the_panel_as_its_label_says() {
        let mut state = PanelState {
            spells: ["Climax", "Heal", "Climax"].map(str::to_owned).to_vec(),
            ..PanelState::default()
        };
        assert!(Change::RaiseSl(139).apply(&mut state));
        assert_eq!(state.sl(), 139);
        assert!(Change::RemoveSpell("Climax".to_owned()).apply(&mut state));
        assert_eq!(state.spells, ["Heal", "Climax"], "one copy, the first");
        assert!(!Change::RemoveSpell("Soul_Arrow".to_owned()).apply(&mut state));
        assert!(Change::IgnoreFloors.apply(&mut state));
        assert!(state.ignore_floors);
        assert!(
            !Change::Class("knight".to_owned()).apply(&mut state),
            "the panel holds no class"
        );
    }

    /// The two stat orders are each other's inverse, and adaptability is what moves.
    #[test]
    fn the_stat_orders_round_trip() {
        let planner = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        assert_eq!(game_order(&planner), [1, 2, 3, 4, 5, 6, 8, 9, 7]);
        assert_eq!(planner_order(&game_order(&planner)), planner);
    }

    #[test]
    fn the_weapon_modes_need_a_weapon() {
        let mut state = PanelState {
            mode: Mode::OptimizeForWeapon,
            ..PanelState::default()
        };
        assert_eq!(
            ask(&StubBackend, &state),
            Answer::Nothing("choose a weapon first")
        );
        state.choose_weapon("Moonlight_Greatsword");
        assert!(matches!(ask(&StubBackend, &state), Answer::Build(_)));
    }

    /// The import: stats in the game's order, the primary in the right hand, the rings worn, and
    /// the extras bringing each suggested ring to three.
    #[test]
    fn to_import_wears_the_build_and_grants_the_rest() {
        let build = generated();
        let (import, extras) = to_import(&build);
        assert_eq!(import.class, "deprived");
        // The planner's order, which is the panel's: ADP seventh, not last as the game keeps it.
        assert_eq!(import.stats.each().map(|(_, value)| value), build.stats);
        assert_eq!(import.stats.adaptability, 8);
        assert_eq!(import.stats.in_game_order()[8], 8);
        assert_eq!(
            ds2_build_import_core::level::soul_level(&import.stats.in_game_order()),
            33
        );
        assert_eq!(import.weapons.len(), 12);
        assert_eq!(import.weapons[2], "Moonlight_Greatsword");
        assert_eq!(
            Infusion::from_name(&import.weapons[3]),
            Some(Infusion::None)
        );
        assert!(ds2_build_import_core::is_empty_slot(&import.weapons[0]));
        assert_eq!(import.rings[..3], build.suggested_rings[..]);
        assert!(ds2_build_import_core::is_empty_slot(&import.rings[3]));
        assert_eq!(import.armor[0], "Desert Sorceress Hood");
        assert!(
            import.armor[1..]
                .iter()
                .all(|piece| ds2_build_import_core::is_empty_slot(piece))
        );

        for ring in &build.suggested_rings {
            let worn = import.rings.iter().filter(|name| *name == ring).count();
            let extra = extras.iter().filter(|(name, _)| name == ring).count();
            assert_eq!(worn + extra, SUGGESTED_RING_COPIES, "{ring}");
        }
        for ring in &build.common_rings {
            assert_eq!(
                extras.iter().filter(|(name, _)| name == ring).count(),
                1,
                "{ring}"
            );
        }
        assert_eq!(build.common_rings, ["Chloranthy Ring", "Ring of Blades"]);
        let weapons = extras
            .iter()
            .filter(|(name, _)| !name.contains("Ring"))
            .count();
        assert_eq!(weapons, WEAPONS_1H_TOP + WEAPONS_2H_ONLY_TOP);
    }

    /// Everything Apply would grant resolves in the item catalogue, so the stub tests the real path.
    #[test]
    fn every_stub_grant_names_a_real_item() {
        use ds2_build_import_core::{ItemError, id_for, is_empty_slot};
        let (import, extras) = to_import(&generated());
        let names = import
            .weapons
            .chunks(2)
            .map(|pair| pair[0].clone())
            .chain(import.armor.iter().cloned())
            .chain(import.rings.iter().cloned())
            .chain(extras.into_iter().map(|(name, _)| name));
        for name in names {
            if is_empty_slot(&name) {
                continue;
            }
            match id_for(&name) {
                Ok(_) | Err(ItemError::Ambiguous { .. }) => {}
                Err(error) => panic!("{name:?}: {error}"),
            }
        }
    }
}
