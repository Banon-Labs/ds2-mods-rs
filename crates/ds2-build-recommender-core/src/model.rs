//! The panel's state: what the player has typed and chosen, and the soul level it makes.

use ds2_build_import_core::Infusion;

/// Nine levelled stats.
pub const STAT_COUNT: usize = 9;

/// The stats' labels, in the order [`PanelState::stats`] holds them.
///
/// That is the planner's order, `ds2_build_import_core::Stats::each`, with ADP seventh. The game
/// keeps ADP last (`Stats::in_game_order`); `backend::to_import` goes through the named fields, so
/// neither order is ever assumed to be the other.
pub const STAT_LABELS: [&str; STAT_COUNT] = [
    "VIG", "END", "VIT", "ATT", "STR", "DEX", "ADP", "INT", "FTH",
];

/// The lowest a stat can be typed as. A class never starts a stat below 1.
pub const STAT_MIN: u16 = 1;

/// The highest a stat can be typed as.
pub const STAT_MAX: u16 = 99;

/// The highest soul level the override accepts.
pub const SL_MAX: u16 = 838;

/// The soul level nine stats make: their total less 53, never below 1.
///
/// The game's own formula, which `ds2_build_import_core::level::soul_level` already carries; this
/// is that function at the panel's type.
pub fn soul_level(stats: &[u16; STAT_COUNT]) -> u16 {
    u16::try_from(ds2_build_import_core::level::soul_level(stats)).unwrap_or(u16::MAX)
}

/// What the panel is asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// The weapons these stats wield best, ranked by damage.
    #[default]
    WeaponsForStats,
    /// The stats that make one weapon and infusion hit hardest at a soul level.
    OptimizeForWeapon,
    /// The least a character needs to wield one weapon.
    MinimumForWeapon,
    /// The weapons builds with stats like these carry.
    SimilarBuilds,
}

impl Mode {
    /// Every mode, in tab order.
    pub const ALL: [Mode; 4] = [
        Mode::WeaponsForStats,
        Mode::OptimizeForWeapon,
        Mode::MinimumForWeapon,
        Mode::SimilarBuilds,
    ];

    /// The tab's caption: one word, the game's way (docs/DS2-UI-DESIGN.md).
    pub const fn label(self) -> &'static str {
        match self {
            Mode::WeaponsForStats => "Weapons",
            Mode::OptimizeForWeapon => "Optimize",
            Mode::MinimumForWeapon => "Minimum",
            Mode::SimilarBuilds => "Similar",
        }
    }

    /// Whether this mode needs a weapon chosen before it can run.
    pub const fn needs_weapon(self) -> bool {
        matches!(self, Mode::OptimizeForWeapon | Mode::MinimumForWeapon)
    }
}

/// What an optimized build is optimized for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Objective {
    /// Expected damage against the bracket's average defender.
    #[default]
    Damage,
    /// Attack rating summed over the damage types, the worn rings' attack adds counted, before
    /// any defence: the number the game's menu shows and the community compares. It ranks a split
    /// infusion by both halves, where [`Objective::Damage`] lets the defender's elemental
    /// resistance discount one of them -- a Raw Black Dragon Greataxe's 471 against Lightning's
    /// 287 + 287 is a loss on this goal and nearly a tie on that one.
    Ar,
    /// Bleed build-up per hit.
    Bleed,
    /// Poison build-up per hit.
    Poison,
}

impl Objective {
    /// Every objective, in the order the panel offers them.
    pub const ALL: [Objective; 4] = [
        Objective::Damage,
        Objective::Ar,
        Objective::Bleed,
        Objective::Poison,
    ];

    /// The option's caption.
    pub const fn label(self) -> &'static str {
        match self {
            Objective::Damage => "Damage",
            Objective::Ar => "AR",
            Objective::Bleed => "Bleed",
            Objective::Poison => "Poison",
        }
    }
}

/// The armour slots a [`Defender::Armor`] fills, in its order.
pub const ARMOR_SLOTS: [&str; 4] = ["Head", "Chest", "Hands", "Legs"];

/// Who [`Objective::Damage`] is scored against: the script's `--defender`.
///
/// The damage a weapon deals depends on who it hits, and which infusion is best can turn on it:
/// the average defender's lightning resistance is its lowest, so Lightning edges out the other
/// split infusions against it, while Havel's set over the bracket's median stats (a low INT and
/// FTH base) makes Magic and Dark lead. Without a choice the panel could only answer for the
/// average player at the soul level, and never said which numbers that player had.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Defender {
    /// The soul level's average defender: the mean defense of the corpus builds in its bracket.
    #[default]
    Average,
    /// These pieces, head, chest, hands, legs, by soulsplanner key (`Naked` for a bare slot),
    /// worn at the bracket's median stats. A piece's physical defense grows with END + VIT + STR +
    /// DEX and the elemental base with INT and FTH, so a set has no defense without stats under
    /// it; the median is the bracket's typical player, as the average defender is its typical set.
    Armor([String; 4]),
}

impl Defender {
    /// A bare defender, every slot `Naked`: where choosing the first piece starts from.
    pub fn naked() -> Self {
        Defender::Armor(std::array::from_fn(|_| "Naked".to_owned()))
    }

    /// The pieces, or `None` for the average defender.
    pub fn pieces(&self) -> Option<&[String; 4]> {
        match self {
            Defender::Average => None,
            Defender::Armor(pieces) => Some(pieces),
        }
    }
}

/// How the defender answers the weapon it is hit by: the script's `--static-defender` and
/// `--defender-buff`.
///
/// A player who meets a weapon wears the ring that cuts its damage most, so damage scored against
/// rings worn for nobody in particular flatters a weapon whose damage type one ring counters. The
/// adaptive answers are the script's `AdaptiveDefense`: each defender swaps one ring slot for the
/// counter ring (Ring of Steel Protection+2, a Quartz Ring+3 or Dispelling Ring+1) that leaves the
/// least of this attack's damage, and with a buff also takes the defense buff that cuts it most.
/// AR is defense-free and ignores all of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Reply {
    /// The defenders' rings as worn: the script's `--static-defender`.
    Static,
    /// Each defender swaps one ring slot for the counter ring: the script's default.
    #[default]
    Ring,
    /// The counter ring, and the consumable defense buff that cuts the damage most (the Burrs, Dark
    /// Troches): `--defender-buff item`.
    RingAndItem,
    /// The counter ring, and the item or spell defense buff that cuts the damage most, whatever the
    /// defender's stats: `--defender-buff any`.
    RingAndAnyBuff,
}

impl Reply {
    /// Every reply, in the order the panel offers them.
    pub const ALL: [Reply; 4] = [
        Reply::Ring,
        Reply::RingAndItem,
        Reply::RingAndAnyBuff,
        Reply::Static,
    ];

    /// The option's caption.
    pub const fn label(self) -> &'static str {
        match self {
            Reply::Static => "Static",
            Reply::Ring => "Ring",
            Reply::RingAndItem => "Ring+item",
            Reply::RingAndAnyBuff => "Ring+buff",
        }
    }

    /// The script's `--defender-buff` name, `none` with no buff.
    pub const fn buff(self) -> &'static str {
        match self {
            Reply::Static | Reply::Ring => "none",
            Reply::RingAndItem => "item",
            Reply::RingAndAnyBuff => "any",
        }
    }

    /// Whether the defender answers at all.
    pub const fn adapts(self) -> bool {
        !matches!(self, Reply::Static)
    }
}

/// Which grip Optimize for weapon and Generate Build build for: the script's `--grip`.
///
/// Two-handed halves the STR requirement even when one-handing would fit; one-handed needs it in
/// full. Damage is scored the same for either: that two-handing multiplies the STR attack rating
/// scales from is not proven, so no multiplier is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Grip {
    /// The default. Trying two-handed first is the same as always two-handing, because a halved
    /// requirement fits whenever the full one does.
    #[default]
    TwoHanded,
    /// Meet the full STR requirement.
    OneHanded,
}

impl Grip {
    /// Every grip, in the order the panel offers them.
    pub const ALL: [Grip; 2] = [Grip::OneHanded, Grip::TwoHanded];

    /// The button's caption.
    pub const fn label(self) -> &'static str {
        match self {
            Grip::OneHanded => "1H",
            Grip::TwoHanded => "2H",
        }
    }

    /// Whether this grip is two-handed.
    pub const fn two_handed(self) -> bool {
        matches!(self, Grip::TwoHanded)
    }
}

/// Options for [`Mode::WeaponsForStats`], after `weapons_for` in `scripts/ds2-builds-recommend.py`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct WeaponsForOpts {
    /// Only weapons these stats wield one-handed.
    pub one_hand: bool,
    /// Only this weapon class, or every class.
    pub class: Option<String>,
    /// Only each class's best row.
    pub per_class: bool,
    /// Rank by the damage of R1 hits landing within this many seconds; `0` ranks one hit.
    pub window_s: f32,
    /// Rank by attack rating rather than damage against the bracket's defence.
    pub raw_ar: bool,
    /// What to rank by. Bleed and poison rank by build-up per hit times the hits of the weapon's
    /// best R1 or R2 (within `window_s` when it is set), as the script's `--objective` does.
    /// [`crate::backend::ask`] fills it from [`PanelState::objective`].
    pub objective: Objective,
    /// Who damage is scored against. [`crate::backend::ask`] fills it from
    /// [`PanelState::defender`].
    pub defender: Defender,
    /// How that defender answers each weapon. [`crate::backend::ask`] fills it from
    /// [`PanelState::reply`].
    pub reply: Reply,
}

/// What a Best weapons row's score is for [`Objective::Damage`] with a window: the script's
/// `--rank`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rank {
    /// The R1 hits landed within the window.
    #[default]
    Window,
    /// Those hits' damage over the stamina their attacks cost.
    PerStamina,
    /// The damage of the R1 chain a full bar of the build's own max stamina pays for.
    Bar,
}

/// Options for Best weapons, the script's `--best-weapons`: every weapon an infusion goes on, each
/// at the build Optimize for weapon makes for it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BestWeaponsOpts {
    /// Only this weapon class, or every class.
    pub weapon_class: Option<String>,
    /// Score the optimized build by the R1 hits landing within this many seconds (bleed and
    /// poison: build-up per hit times the hits); `0` scores the optimizer's own one-hit value. A
    /// launcher's row is one shot either way.
    pub window_s: f32,
    /// What the score is, for damage with a window.
    pub rank: Rank,
    /// Rank damage by the window's damage plus the poison and bleed its hits deal through their
    /// procs; the score stays the damage alone. The script's `--with-status`.
    pub with_status: bool,
}

/// Which status a similar build's weapon must deal to be counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StatusFilter {
    /// Only weapons that build bleed.
    pub bleed: bool,
    /// Only weapons that build poison.
    pub poison: bool,
}

impl StatusFilter {
    /// Whether any status is asked for.
    pub const fn any(self) -> bool {
        self.bleed || self.poison
    }
}

/// The most spells the panel lets Generate Build ask for. Every spell costs at least one slot and
/// ATT gives at most ten (`PhysicalStatsPerLevelStatValuesParam.spellSlot`), so an eleventh could
/// never fit.
pub const MAX_SPELLS: usize = 10;

/// The default neighbourhood for [`Mode::SimilarBuilds`], as the script's `k`.
pub const SIMILAR_K_DEFAULT: u16 = 50;

/// Everything the panel holds between frames that is not drawing state.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelState {
    /// The nine stats, in [`STAT_LABELS`] order.
    pub stats: [u16; STAT_COUNT],
    /// A soul level typed over the one the stats make, for the modes that take a level.
    pub sl_override: Option<u16>,
    /// What is being asked.
    pub mode: Mode,
    /// [`Mode::WeaponsForStats`]'s options.
    pub weapons_for: WeaponsForOpts,
    /// The weapon the weapon modes and Generate Build use, by soulsplanner key.
    pub weapon: Option<&'static str>,
    /// Its infusion. Kept to one the weapon takes by [`PanelState::choose_weapon`].
    pub infusion: Infusion,
    /// What [`Mode::OptimizeForWeapon`] and Generate Build optimize for.
    pub objective: Objective,
    /// Who [`Objective::Damage`] is scored against, on every tab that scores it.
    pub defender: Defender,
    /// How that defender answers each weapon; the script's default, a counter ring, unless chosen.
    pub reply: Reply,
    /// The grip [`Mode::OptimizeForWeapon`] and Generate Build build for.
    pub grip: Grip,
    /// Whether [`Mode::MinimumForWeapon`] may two-hand to meet strength.
    pub two_hand: bool,
    /// Whether Generate Build may leave the armour off. Off by default: a generated build wears
    /// the best set its stats can carry under 70% load.
    pub allow_naked: bool,
    /// How many neighbours [`Mode::SimilarBuilds`] reads.
    pub similar_k: u16,
    /// Which statuses [`Mode::SimilarBuilds`] insists on.
    pub status: StatusFilter,
    /// The spells Generate Build must be able to attune and cast, by soulsplanner key, in the
    /// order they were chosen. Empty by default: a build with no spells.
    pub spells: Vec<String>,
    /// Whether Optimize for weapon and Generate Build drop the soul level's floors: they are the
    /// medians of real builds, not a game rule. Off by default; a refusal's "ignore typical-build
    /// minimums" fix turns it on.
    pub ignore_floors: bool,
}

impl Default for PanelState {
    /// A fresh Deprived: every stat 6, soul level 1.
    fn default() -> Self {
        Self {
            stats: [6; STAT_COUNT],
            sl_override: None,
            mode: Mode::default(),
            weapons_for: WeaponsForOpts::default(),
            weapon: None,
            infusion: Infusion::None,
            objective: Objective::default(),
            defender: Defender::default(),
            reply: Reply::default(),
            grip: Grip::default(),
            two_hand: false,
            allow_naked: false,
            similar_k: SIMILAR_K_DEFAULT,
            status: StatusFilter::default(),
            spells: Vec::new(),
            ignore_floors: false,
        }
    }
}

impl PanelState {
    /// The soul level the stats make.
    pub fn computed_sl(&self) -> u16 {
        soul_level(&self.stats)
    }

    /// The soul level the modes run at: the override when one is typed, the stats' otherwise.
    pub fn sl(&self) -> u16 {
        self.sl_override.unwrap_or_else(|| self.computed_sl())
    }

    /// Set one stat, clamped to what a character can have.
    pub fn set_stat(&mut self, index: usize, value: u16) {
        if let Some(stat) = self.stats.get_mut(index) {
            *stat = value.clamp(STAT_MIN, STAT_MAX);
        }
    }

    /// Set or clear the soul-level override. `Some(0)` is read as clearing it.
    pub fn set_sl_override(&mut self, value: Option<u16>) {
        self.sl_override = value.filter(|&sl| sl > 0).map(|sl| sl.min(SL_MAX));
    }

    /// Choose a weapon, keeping the infusion when the new weapon takes it and falling back to
    /// uninfused when it does not. A key the table does not have clears the choice.
    pub fn choose_weapon(&mut self, key: &str) {
        match crate::weapons::by_key(key) {
            Some(row) => {
                self.weapon = Some(row.key);
                if !row.takes(self.infusion) {
                    self.infusion = Infusion::None;
                }
            }
            None => {
                self.weapon = None;
                self.infusion = Infusion::None;
            }
        }
    }

    /// Choose an infusion. Refused, returning `false`, when the chosen weapon does not take it.
    pub fn choose_infusion(&mut self, infusion: Infusion) -> bool {
        let allowed = self
            .weapon
            .and_then(crate::weapons::by_key)
            .is_none_or(|row| row.takes(infusion));
        if allowed {
            self.infusion = infusion;
        }
        allowed
    }

    /// Add `key` to the spells Generate Build casts, or take it out when it is already there.
    /// Returns whether it is chosen now. Once [`MAX_SPELLS`] are chosen, adding is refused.
    pub fn toggle_spell(&mut self, key: &str) -> bool {
        if let Some(at) = self.spells.iter().position(|chosen| chosen == key) {
            self.spells.remove(at);
            false
        } else if self.spells.len() < MAX_SPELLS {
            self.spells.push(key.to_owned());
            true
        } else {
            false
        }
    }

    /// Put the piece `key` (a soulsplanner key, `Naked` for none) on the defender's `slot`, an
    /// index into [`ARMOR_SLOTS`]. An average defender becomes a bare one first, so choosing a
    /// helmet does not leave the other three slots meaning "average". A slot past the four is
    /// ignored.
    pub fn set_defender_piece(&mut self, slot: usize, key: &str) {
        if slot >= ARMOR_SLOTS.len() {
            return;
        }
        if self.defender == Defender::Average {
            self.defender = Defender::naked();
        }
        if let Defender::Armor(pieces) = &mut self.defender {
            key.clone_into(&mut pieces[slot]);
        }
    }

    /// Whether the current mode has what it needs to run.
    pub fn ready(&self) -> bool {
        !self.mode.needs_weapon() || self.weapon.is_some()
    }

    /// The heading of a results table's score column: what the number under it measures.
    ///
    /// Here rather than in the panel so the host tests can hold it to what
    /// `CorpusBackend::rank` scores. The goal decides it first, since Best infusion and the
    /// Weapons tab both rank by it; the Weapons tab's own raw-AR toggle only when the goal is
    /// damage and no R1 window is set, because with a window `rank` scores the window's hits of
    /// damage whatever the toggle says (the script's `--weapons-for` heads that column `dmg/Ns`).
    pub fn score_heading(&self) -> &'static str {
        match (self.mode, self.objective) {
            (Mode::SimilarBuilds, _) => "Builds",
            // Build-up per hit times hits per attack (or within the window).
            (_, Objective::Bleed) => "Bleed x hits",
            (_, Objective::Poison) => "Poison x hits",
            (_, Objective::Ar) => "AR",
            (Mode::WeaponsForStats, Objective::Damage)
                if self.weapons_for.raw_ar && self.weapons_for.window_s == 0.0 =>
            {
                "AR"
            }
            (_, Objective::Damage) => "Damage",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stub's build: nine stats totalling 86, which is soul level 33.
    #[test]
    fn soul_level_is_the_total_less_53() {
        let mut state = PanelState {
            stats: [10, 16, 7, 10, 6, 13, 8, 10, 6],
            ..PanelState::default()
        };
        assert_eq!(state.computed_sl(), 33);
        assert_eq!(state.sl(), 33);
        state.set_sl_override(Some(150));
        assert_eq!(state.sl(), 150);
        assert_eq!(
            state.computed_sl(),
            33,
            "the override does not move the stats' level"
        );
        state.set_sl_override(Some(0));
        assert_eq!(state.sl(), 33, "zero clears the override");
    }

    /// The AR goal heads Best infusion's column "AR", and the Weapons tab's raw-AR toggle does
    /// only while no window turns its score back into damage.
    #[test]
    fn the_score_heading_follows_the_goal() {
        let mut state = PanelState {
            mode: Mode::OptimizeForWeapon,
            ..PanelState::default()
        };
        assert_eq!(state.score_heading(), "Damage");
        state.objective = Objective::Ar;
        assert_eq!(state.score_heading(), "AR");
        state.objective = Objective::Bleed;
        assert_eq!(state.score_heading(), "Bleed x hits");
        state.mode = Mode::WeaponsForStats;
        state.objective = Objective::Damage;
        state.weapons_for.raw_ar = true;
        assert_eq!(state.score_heading(), "AR");
        state.weapons_for.window_s = 1.5;
        assert_eq!(state.score_heading(), "Damage");
        state.objective = Objective::Ar;
        assert_eq!(state.score_heading(), "AR", "the AR goal drops the window");
        state.mode = Mode::SimilarBuilds;
        assert_eq!(state.score_heading(), "Builds");
    }

    /// The panel's Goal control cycles through every objective, AR among them.
    #[test]
    fn the_goal_offers_ar() {
        assert!(Objective::ALL.contains(&Objective::Ar));
        assert_eq!(Objective::Ar.label(), "AR");
    }

    /// The defender answers each weapon by default, as the script's does; only Static does not, and
    /// each reply is the script's `--defender-buff` setting.
    #[test]
    fn the_defender_answers_by_default() {
        assert_eq!(PanelState::default().reply, Reply::Ring);
        assert!(
            Reply::ALL
                .iter()
                .all(|reply| reply.adapts() != (*reply == Reply::Static))
        );
        assert_eq!(
            Reply::ALL.map(Reply::buff),
            ["none", "item", "any", "none"],
            "Ring, Ring+item, Ring+buff, Static"
        );
    }

    /// The defender starts as the average, and choosing one piece makes a bare defender wearing it.
    #[test]
    fn choosing_a_defender_piece_starts_from_bare() {
        let mut state = PanelState::default();
        assert_eq!(state.defender, Defender::Average);
        assert_eq!(state.defender.pieces(), None);
        state.set_defender_piece(1, "Havels_Armor");
        assert_eq!(
            state.defender.pieces().cloned(),
            Some(["Naked", "Havels_Armor", "Naked", "Naked"].map(str::to_owned))
        );
        state.set_defender_piece(0, "Havels_Helm");
        state.set_defender_piece(4, "Not_A_Slot");
        assert_eq!(
            state.defender.pieces().map(|p| p[0].as_str()),
            Some("Havels_Helm")
        );
    }

    #[test]
    fn a_fresh_panel_is_level_one() {
        assert_eq!(PanelState::default().computed_sl(), 1);
        // Below the bias still reads level 1, never 0 or a wrapped number.
        assert_eq!(soul_level(&[1; STAT_COUNT]), 1);
    }

    #[test]
    fn stats_are_clamped() {
        let mut state = PanelState::default();
        state.set_stat(0, 0);
        state.set_stat(1, 500);
        state.set_stat(99, 40);
        assert_eq!(state.stats[0], STAT_MIN);
        assert_eq!(state.stats[1], STAT_MAX);
        state.set_sl_override(Some(9999));
        assert_eq!(state.sl(), SL_MAX);
    }

    /// The infusion follows the weapon: kept when the new weapon takes it, dropped when not.
    #[test]
    fn the_infusion_filter_follows_the_weapon() {
        let mut state = PanelState::default();
        assert!(
            state.choose_infusion(Infusion::Dark),
            "no weapon: anything goes"
        );
        state.choose_weapon("Moonlight_Greatsword");
        assert_eq!(state.weapon, Some("Moonlight_Greatsword"));
        assert_eq!(state.infusion, Infusion::Dark);
        let limited = crate::weapons::all()
            .iter()
            .find(|row| !row.takes(Infusion::Dark))
            .expect("a weapon that does not take Dark");
        state.choose_weapon(limited.key);
        assert_eq!(state.infusion, Infusion::None);
        assert!(!state.choose_infusion(Infusion::Dark));
        assert_eq!(state.infusion, Infusion::None);
        state.choose_weapon("Not_A_Weapon");
        assert_eq!(state.weapon, None);
    }

    #[test]
    fn a_spell_toggles_in_and_out_and_stops_at_ten() {
        let mut state = PanelState::default();
        assert!(state.toggle_spell("Soul_Spear"));
        assert!(state.toggle_spell("Heal"));
        assert_eq!(state.spells, ["Soul_Spear", "Heal"]);
        assert!(
            !state.toggle_spell("Soul_Spear"),
            "chosen again is taken out"
        );
        assert_eq!(state.spells, ["Heal"]);
        for n in 0..20 {
            state.toggle_spell(&format!("Spell_{n}"));
        }
        assert_eq!(state.spells.len(), MAX_SPELLS);
    }

    #[test]
    fn the_weapon_modes_wait_for_a_weapon() {
        let mut state = PanelState {
            mode: Mode::OptimizeForWeapon,
            ..PanelState::default()
        };
        assert!(!state.ready());
        state.choose_weapon("Moonlight_Greatsword");
        assert!(state.ready());
        state.mode = Mode::SimilarBuilds;
        state.weapon = None;
        assert!(state.ready());
    }
}
