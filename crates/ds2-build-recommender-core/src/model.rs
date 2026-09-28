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

    /// The tab's caption.
    pub const fn label(self) -> &'static str {
        match self {
            Mode::WeaponsForStats => "Weapons for stats",
            Mode::OptimizeForWeapon => "Optimize for weapon",
            Mode::MinimumForWeapon => "Minimum for weapon",
            Mode::SimilarBuilds => "Similar builds",
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
    /// Bleed build-up per hit.
    Bleed,
    /// Poison build-up per hit.
    Poison,
}

impl Objective {
    /// Every objective, in the order the panel offers them.
    pub const ALL: [Objective; 3] = [Objective::Damage, Objective::Bleed, Objective::Poison];

    /// The option's caption.
    pub const fn label(self) -> &'static str {
        match self {
            Objective::Damage => "Damage",
            Objective::Bleed => "Bleed",
            Objective::Poison => "Poison",
        }
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
    /// Whether [`Mode::MinimumForWeapon`] may two-hand to meet strength.
    pub two_hand: bool,
    /// How many neighbours [`Mode::SimilarBuilds`] reads.
    pub similar_k: u16,
    /// Which statuses [`Mode::SimilarBuilds`] insists on.
    pub status: StatusFilter,
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
            two_hand: false,
            similar_k: SIMILAR_K_DEFAULT,
            status: StatusFilter::default(),
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

    /// Whether the current mode has what it needs to run.
    pub fn ready(&self) -> bool {
        !self.mode.needs_weapon() || self.weapon.is_some()
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
