//! `CorpusBackend` against `scripts/ds2-builds-recommend.py`, question for question.
//!
//! `fixtures/corpus-sample.dat` is the script's `--export-backend` over every fourth corpus build,
//! and `fixtures/python_expected.rs` is the script's own answers over that same sample, written by
//! the same run:
//!
//! ```text
//! python3 scripts/ds2-builds-recommend.py --corpus-every 4 \
//!     --export-backend crates/ds2-build-recommender-core/tests/fixtures/corpus-sample.dat \
//!     --expect crates/ds2-build-recommender-core/tests/fixtures/python_expected.rs
//! ```
//!
//! Damage is compared exactly, at the `f32` the panel shows: the port computes in the same `f64`
//! operations in the same order, so a difference is a porting bug, not rounding.

use std::sync::OnceLock;

use ds2_build_import_core::Infusion;
use ds2_build_recommender_core::backend::{
    self, Change, Limits, Outcome, RecommenderBackend, RefusalKind, ResultRow,
};
use ds2_build_recommender_core::corpus::CorpusBackend;
use ds2_build_recommender_core::model::{
    BestWeaponsOpts, Defender, Grip, Mode, Objective, PanelState, Rank, Reply, STAT_COUNT,
    StatusFilter, WeaponsForOpts, soul_level,
};
use ds2_build_recommender_core::weapons;

/// name, infusion code, damage, AR by type, grip, hyperarmor, counter, class.
type ForRow = (
    &'static str,
    &'static str,
    f64,
    &'static [f64],
    &'static str,
    f64,
    f64,
    &'static str,
);
type WeaponsForCase = (
    &'static [u16],
    u16,
    bool,
    &'static str,
    bool,
    f64,
    bool,
    &'static str,
    &'static [ForRow],
);
type WeaponsForCases = &'static [WeaponsForCase];
/// A defender's armour keys head to legs, then a case asked against that defender.
type Against<T> = &'static [(&'static [&'static str], T)];
type DefenderWeaponsForCases = Against<WeaponsForCase>;
type OptimizeCase = (
    &'static str,
    &'static str,
    u16,
    &'static str,
    Option<(&'static str, bool, &'static [u16], f64)>,
);
type OptimizeCases = &'static [OptimizeCase];
type DefenderOptimizeCases = Against<OptimizeCase>;
/// sl, armour keys (`[]` for the average defender) -> defense by type, builds it was taken from.
type DefenseCases = &'static [(u16, &'static [&'static str], &'static [f64], u32)];
/// name, infusion code, rounded damage.
type GenRow = (&'static str, &'static str, i64);
type Generated = (
    &'static str,
    bool,
    &'static [u16],
    &'static [GenRow],
    &'static [GenRow],
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    Option<&'static str>,
    bool,
    bool,
    &'static [&'static str],
);
type GenerateCase = (
    &'static str,
    &'static str,
    u16,
    &'static str,
    bool,
    Option<Generated>,
);
type GenerateCases = &'static [GenerateCase];
type DefenderGenerateCases = Against<GenerateCase>;
/// A class key, and a generate case asked of that starting class alone.
type ClassGenerateCases = &'static [(&'static str, GenerateCase)];
/// school, catalyst, cast power, the catalyst passed over for its requirements (`""` for none).
type CatalystRow = (&'static str, &'static str, f64, &'static str);
/// class, two-handed, stats, spell names, slots used, slots given, catalysts, ring trades.
type SpellBuild = (
    &'static str,
    bool,
    &'static [u16],
    &'static [&'static str],
    u16,
    u16,
    &'static [CatalystRow],
    &'static [&'static str],
);
type SpellCases = &'static [(
    &'static str,
    &'static str,
    u16,
    &'static str,
    &'static [&'static str],
    Option<SpellBuild>,
)];
type OptimizeSpellCases = &'static [(
    &'static str,
    &'static str,
    u16,
    &'static str,
    &'static [&'static str],
    bool,
    Option<(
        &'static str,
        bool,
        &'static [u16],
        f64,
        &'static [&'static str],
    )>,
)];
/// kind, closest class, points short, reason lines, fixes: what, value, label.
type RefusalAnswer = (
    &'static str,
    &'static str,
    i32,
    &'static [&'static str],
    &'static [(&'static str, &'static str, &'static str)],
);
type RefusalCases = &'static [(
    &'static str,
    &'static str,
    u16,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    bool,
    Option<RefusalAnswer>,
)];
/// weapon, stats -> load scarcity, armour keys at scarcity 0, the same at that scarcity.
type ArmorScarcityCases = &'static [(
    &'static str,
    &'static [u16],
    f64,
    &'static [&'static str],
    &'static [&'static str],
)];
type MinimumCases = &'static [(
    &'static str,
    bool,
    (&'static str, u16, &'static [u16], &'static [&'static str]),
)];
/// count, weapon, top infusions with counts, grip.
type SimilarRow = (
    u32,
    &'static str,
    &'static [(&'static str, u32)],
    &'static str,
);
type SimilarCases = &'static [(
    &'static [u16],
    u16,
    u16,
    &'static [&'static str],
    &'static [SimilarRow],
)];
type FloorCases = &'static [(u16, &'static [u16])];
/// 1H, 2H, total, below, equal, neighbours, percentile, spare load, fits.
type FlexAnswer = (u32, u32, u32, u32, u32, u32, f64, f64, u32);
type FlexCases = &'static [(
    &'static [u16],
    u16,
    &'static [&'static str],
    &'static [&'static str],
    FlexAnswer,
)];
/// infusion code, score, AR by type, grip.
type InfusionRow = (&'static str, f64, &'static [f64], &'static str);
type BestInfusionCase = (
    &'static str,
    &'static [u16],
    u16,
    f64,
    bool,
    &'static str,
    &'static [InfusionRow],
);
type BestInfusionCases = &'static [BestInfusionCase];
type DefenderBestInfusionCases = Against<BestInfusionCase>;
/// The adaptive defender's buff setting (`none`, `item`, `any`) and armour keys, then a case.
type Adaptive<T> = &'static [(&'static str, &'static [&'static str], T)];
type AdaptiveWeaponsForCases = Adaptive<WeaponsForCase>;
type AdaptiveOptimizeCases = Adaptive<OptimizeCase>;
type AdaptiveGenerateCases = Adaptive<GenerateCase>;
type AdaptiveBestInfusionCases = Adaptive<BestInfusionCase>;
/// R1 reach, startup, recovery, first hit, 5 s damage.
type R1Metrics = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);
/// Hyperarmor share, its rate, the counter-hits it holds, poise damage per hit, armorBreak, hits
/// to stagger, defender poise.
type PoiseMetrics = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<i64>,
    Option<i64>,
    f64,
);
/// Status hits, per status (name, build-up per hit, hits to proc), damage per window, in the
/// first window.
type StatusMetrics = Option<(i64, &'static [(&'static str, f64, i64)], f64, f64)>;
/// Stamina per attack, per window, damage per stamina, max stamina, a full bar's attacks, damage
/// and seconds.
type StaminaMetrics = Option<(
    &'static [f64],
    f64,
    f64,
    Option<f64>,
    Option<i64>,
    Option<f64>,
    Option<f64>,
)>;
/// score, weapon key, value, class, two-handed, stats, rings, label, (ammo, shot), then metrics.
type BestRow = (
    f64,
    &'static str,
    f64,
    &'static str,
    bool,
    &'static [u16],
    &'static [&'static str],
    &'static str,
    Option<(&'static str, &'static str)>,
    R1Metrics,
    PoiseMetrics,
    StatusMetrics,
    Option<f64>,
    StaminaMetrics,
);
/// infusion code, sl, objective, grip, weapon class, window, rank, with status, defender keys,
/// who answers (`static` or a buff setting), rows.
type BestWeaponsCase = (
    &'static str,
    u16,
    &'static str,
    &'static str,
    &'static str,
    f64,
    &'static str,
    bool,
    &'static [&'static str],
    &'static str,
    &'static [BestRow],
);
type BestWeaponsCases = &'static [BestWeaponsCase];

/// The questions most fixtures were asked: the script's `--expect` writes them with a static
/// defender, its rings as worn.
const STATIC: Limits<'static> = Limits {
    reply: Reply::Static,
    ..Limits::NONE
};

/// The adaptive defender a fixture's buff setting names.
fn reply(buff: &str) -> Reply {
    match buff {
        "none" => Reply::Ring,
        "item" => Reply::RingAndItem,
        "any" => Reply::RingAndAnyBuff,
        other => panic!("buff {other}"),
    }
}

mod expected {
    use super::*;
    include!("fixtures/python_expected.rs");
}

fn backend() -> &'static CorpusBackend {
    static BACKEND: OnceLock<CorpusBackend> = OnceLock::new();
    BACKEND.get_or_init(|| {
        CorpusBackend::parse(include_str!("fixtures/corpus-sample.dat"))
            .expect("the fixture parses")
    })
}

fn stats(values: &[u16]) -> [u16; STAT_COUNT] {
    values.try_into().expect("nine stats")
}

fn infusion(code: &str) -> Infusion {
    weapons::infusion_for_code(code.chars().next().expect("a code")).expect("a known code")
}

fn objective(name: &str) -> Objective {
    match name {
        "damage" => Objective::Damage,
        "ar" => Objective::Ar,
        "bleed" => Objective::Bleed,
        "poison" => Objective::Poison,
        other => panic!("objective {other}"),
    }
}

/// The script's `--defender`: `[]` is the average defender.
fn defender(keys: &[&str]) -> Defender {
    match keys {
        [] => Defender::Average,
        keys => Defender::Armor(
            <[&str; 4]>::try_from(keys)
                .expect("four pieces")
                .map(str::to_owned),
        ),
    }
}

fn rows(outcome: Outcome) -> Vec<ResultRow> {
    match outcome {
        Outcome::Rows(rows) => rows,
        Outcome::FloorViolations(lines) => panic!("floor violations {lines:?}"),
    }
}

#[test]
fn the_fixture_is_a_whole_file() {
    let backend = backend();
    assert!(!backend.is_stub());
    assert!(backend.corpus_len() > 2000, "{}", backend.corpus_len());
}

#[test]
fn floors_are_the_scripts() {
    for &(sl, floors) in expected::FLOORS {
        let got = backend().floors(sl);
        // The script's VIG, VIT, ADP, ATT.
        assert_eq!([got[0], got[2], got[6], got[3]], floors, "SL {sl}");
    }
}

#[test]
fn calibration_is_the_scripts() {
    let (n, top1, top2) = expected::CALIBRATION;
    let got = backend().calibration();
    assert_eq!(got.n, n);
    assert_eq!(got.top1, top1 as f32);
    assert_eq!(got.top2, top2 as f32);
}

/// The defense the damage objective is scored against: the bracket's average defender, and a
/// chosen set at the bracket's median stats, the script's `defender_defense`.
#[test]
fn defense_is_the_scripts() {
    let mut chosen = 0;
    for &(sl, keys, want, builds) in expected::DEFENSE {
        let asked = defender(keys);
        let got = backend()
            .defense(sl, &asked, Reply::Static)
            .unwrap_or_else(|| panic!("SL {sl} {keys:?}: no defense"));
        let want: Vec<f32> = want.iter().map(|&value| value as f32).collect();
        assert_eq!(got.defense[..], want[..], "SL {sl} {keys:?}");
        assert_eq!(got.builds, builds, "SL {sl} {keys:?}");
        assert_eq!(got.stats.is_some(), !keys.is_empty(), "SL {sl} {keys:?}");
        assert!(got.counters.is_empty(), "a static defender swaps nothing");
        // An adaptive defender puts up the same numbers until an attack is in hand.
        let adaptive = backend()
            .defense(sl, &asked, Reply::Ring)
            .expect("the same defender");
        assert_eq!(adaptive.defense, got.defense, "SL {sl} {keys:?}");
        assert_eq!(adaptive.counters, expected::COUNTER_RINGS);
        chosen += usize::from(!keys.is_empty());
    }
    assert!(chosen >= 9, "{chosen} chosen defenders");
    let unknown = defender(&["Not_A_Helm", "Naked", "Naked", "Naked"]);
    assert_eq!(
        backend().defense(100, &unknown, Reply::Ring),
        None,
        "an unknown piece"
    );
}

/// The defender line says when the defender answers the weapon, and with what.
#[test]
fn the_defender_line_says_how_the_defender_answers() {
    let at = |reply| {
        let defense = backend()
            .defense(150, &Defender::Average, reply)
            .expect("the average");
        backend::defender_line(&defense, 150)
    };
    assert!(!at(Reply::Static).contains("adapts"));
    let ring = at(Reply::Ring);
    assert!(
        ring.contains("adapts: swaps a ring slot to Ring of Steel Protection + 2,"),
        "{ring}"
    );
    assert!(!ring.contains("buff"), "{ring}");
    assert!(at(Reply::RingAndItem).contains("plus the best item defense buff"));
    assert!(at(Reply::RingAndAnyBuff).contains("plus the best item or spell defense buff"));
}

/// The panel's Best infusion and Weapons tab are asked against the panel's defender, and its line
/// leads with the numbers that defender puts up.
#[test]
fn the_panel_asks_against_its_defender() {
    let mut state = PanelState {
        mode: Mode::OptimizeForWeapon,
        sl_override: Some(150),
        ..PanelState::default()
    };
    state.choose_weapon("Black_Dragon_Greataxe");
    let ranked = |state: &PanelState| match backend::best_infusion(backend(), state) {
        backend::Answer::Rows(rows) => rows,
        other => panic!("{other:?}"),
    };
    let average = ranked(&state);
    for (slot, piece) in [
        "Havels_Helm",
        "Havels_Armor",
        "Havels_Gauntlets",
        "Havels_Leggings",
    ]
    .into_iter()
    .enumerate()
    {
        state.set_defender_piece(slot, piece);
    }
    let havel = ranked(&state);
    assert_ne!(average, havel, "the defender changes the scores");
    let limits = Limits::of(&state);
    for row in &havel {
        let build = backend()
            .optimize(
                "Black_Dragon_Greataxe",
                row.infusion,
                150,
                Objective::Damage,
                Grip::TwoHanded,
                &limits,
            )
            .expect("a build");
        assert_eq!(build.value, row.damage, "{:?}", row.infusion);
    }
    let defense = backend()
        .defense(150, &state.defender, state.reply)
        .expect("Havel's set");
    let line = backend::defender_line(&defense, 150);
    assert!(
        line.starts_with(&format!("Defender: phys {:.0}  slash", defense.defense[0])),
        "{line}"
    );
    assert!(line.contains("chosen set at the SL 150 median"), "{line}");
    let average = backend()
        .defense(150, &Defender::Average, state.reply)
        .expect("the average");
    assert!(
        backend::defender_line(&average, 150).contains("SL 150 average defender"),
        "the average says it is the average"
    );
}

/// Weapons for Stats, against the static defender, a chosen set, and the adaptive defender whose
/// answer to each weapon ends its row's grip (` vs Dark Quartz Ring + 3`).
#[test]
fn weapons_for_is_the_scripts() {
    let cases = expected::WEAPONS_FOR
        .iter()
        .map(|case| (case, Defender::Average, Reply::Static))
        .chain(
            expected::DEFENDER_WEAPONS_FOR
                .iter()
                .map(|(keys, case)| (case, defender(keys), Reply::Static)),
        )
        .chain(
            expected::ADAPTIVE_WEAPONS_FOR
                .iter()
                .map(|(buff, keys, case)| (case, defender(keys), reply(buff))),
        );
    let mut answered = 0;
    for (
        case,
        (&(st, sl, one_hand, class, per_class, window, raw_ar, goal, want), defender, reply),
    ) in cases.enumerate()
    {
        let opts = WeaponsForOpts {
            one_hand,
            class: (!class.is_empty()).then(|| class.to_owned()),
            per_class,
            window_s: window as f32,
            raw_ar,
            objective: objective(goal),
            defender,
            reply,
        };
        answered += want.iter().filter(|row| row.4.contains(" vs ")).count();
        let got = rows(backend().weapons_for(&stats(st), sl, &opts));
        assert_eq!(got.len(), want.len(), "case {case}: row count");
        for (at, (row, &(name, code, damage, ar, grip, ha, ctr, wclass))) in
            got.iter().zip(want).enumerate()
        {
            let at = format!("case {case} row {at} ({name})");
            assert_eq!(row.weapon, name, "{at}");
            assert_eq!(row.infusion, infusion(code), "{at}");
            assert_eq!(row.damage, damage as f32, "{at}: damage");
            let ar: Vec<f32> = ar.iter().map(|&value| value as f32).collect();
            assert_eq!(row.ar_by_type[..], ar[..], "{at}: AR");
            assert_eq!(row.class, wclass, "{at}");
            if !per_class {
                assert_eq!(row.grip, grip, "{at}");
            }
            assert_eq!(row.hyperarmor, (ha != 0.0).then_some(ha as f32), "{at}");
            assert_eq!(row.counter, (ctr != 0.0).then_some(ctr as f32), "{at}");
        }
    }
    assert!(
        answered > 20,
        "{answered} rows the adaptive defender answered"
    );
}

#[test]
fn best_infusion_is_the_scripts() {
    let mut ranked_some = 0;
    let cases = expected::BEST_INFUSION
        .iter()
        .map(|case| (case, &[][..], Reply::Static))
        .chain(
            expected::DEFENDER_BEST_INFUSION
                .iter()
                .map(|(keys, case)| (case, *keys, Reply::Static)),
        )
        .chain(
            expected::ADAPTIVE_BEST_INFUSION
                .iter()
                .map(|(buff, keys, case)| (case, *keys, reply(buff))),
        );
    for (&(weapon, st, sl, window, raw_ar, goal, want), keys, reply) in cases {
        let opts = WeaponsForOpts {
            window_s: window as f32,
            raw_ar,
            objective: objective(goal),
            defender: defender(keys),
            reply,
            ..WeaponsForOpts::default()
        };
        let got = rows(backend().best_infusion(weapon, &stats(st), sl, &opts));
        let case = format!("{weapon} SL {sl} {goal} window {window} against {keys:?} {reply:?}");
        assert_eq!(got.len(), want.len(), "{case}: row count");
        for (row, &(code, score, ar, grip)) in got.iter().zip(want) {
            assert_eq!(row.infusion, infusion(code), "{case}");
            assert_eq!(row.damage, score as f32, "{case} {code}: score");
            let ar: Vec<f32> = ar.iter().map(|&value| value as f32).collect();
            assert_eq!(row.ar_by_type[..], ar[..], "{case} {code}: AR");
            assert_eq!(row.grip, grip, "{case} {code}");
        }
        if want.len() > 1 {
            ranked_some += 1;
            let margin = backend::infusion_margin(&got).expect("a runner-up");
            let want_margin = want[0].1 / want[1].1 - 1.0;
            assert!(
                (f64::from(margin) - want_margin).abs() < 1e-5,
                "{case}: margin {margin} vs {want_margin}"
            );
        }
    }
    assert!(ranked_some >= 4, "{ranked_some} cases ranked two or more");
}

#[test]
fn optimize_is_the_scripts() {
    let cases = expected::OPTIMIZE
        .iter()
        .map(|case| (case, Grip::TwoHanded, &[][..], Reply::Static))
        .chain(
            expected::OPTIMIZE_ONE_HANDED
                .iter()
                .map(|case| (case, Grip::OneHanded, &[][..], Reply::Static)),
        )
        .chain(
            expected::DEFENDER_OPTIMIZE
                .iter()
                .map(|(keys, case)| (case, Grip::TwoHanded, *keys, Reply::Static)),
        )
        .chain(
            expected::ADAPTIVE_OPTIMIZE
                .iter()
                .map(|(buff, keys, case)| (case, Grip::TwoHanded, *keys, reply(buff))),
        );
    for (&(weapon, code, sl, goal, want), grip, keys, reply) in cases {
        let got = backend().optimize(
            weapon,
            infusion(code),
            sl,
            objective(goal),
            grip,
            &Limits {
                defender: &defender(keys),
                reply,
                ..STATIC
            },
        );
        match (got, want) {
            (None, None) => {}
            (Some(got), Some((class, two, st, value))) => {
                assert_eq!(got.class, class, "{weapon} SL {sl} {grip:?}");
                assert_eq!(got.two_handed, two, "{weapon} SL {sl}");
                assert_eq!(got.two_handed, grip.two_handed(), "the grip asked for");
                assert_eq!(got.stats, stats(st), "{weapon} SL {sl}");
                assert_eq!(got.sl, sl);
                assert_eq!(
                    soul_level(&got.stats),
                    sl,
                    "{weapon}: the stats make the SL"
                );
                assert_eq!(got.value, value as f32, "{weapon} SL {sl}");
            }
            (got, want) => panic!("{weapon} SL {sl}: {got:?} vs {want:?}"),
        }
    }
}

/// The report that asked for the grip: at SL 74 a Dark Murakumo was always optimized one-handed,
/// because one-handing fit and the optimizer only two-handed when it did not.
#[test]
fn a_weapon_the_stats_can_one_hand_still_optimizes_two_handed() {
    let ask = |grip| {
        backend()
            .optimize(
                "Murakumo",
                Infusion::Dark,
                74,
                Objective::Damage,
                grip,
                &STATIC,
            )
            .unwrap_or_else(|| panic!("{grip:?}: no build"))
    };
    let (two, one) = (ask(Grip::TwoHanded), ask(Grip::OneHanded));
    assert!(
        two.two_handed,
        "two-handed when asked, though one-handing fits"
    );
    assert!(!one.two_handed);
    assert_eq!(Grip::default(), Grip::TwoHanded, "two-handed first");
}

#[test]
fn generate_build_is_the_scripts() {
    let (mut requirements_bound, mut load_bound, mut armored) = (false, false, 0);
    let mut traded = false;
    let cases = expected::GENERATE
        .iter()
        .map(|case| (case, Grip::TwoHanded, None))
        .chain(
            expected::GENERATE_ONE_HANDED
                .iter()
                .map(|case| (case, Grip::OneHanded, None)),
        )
        .chain(
            expected::GENERATE_AS_CLASS
                .iter()
                .map(|(class, case)| (case, Grip::TwoHanded, Some(*class))),
        );
    let cases = cases
        .map(|(case, grip, only)| (case, grip, only, &[][..], Reply::Static))
        .chain(
            expected::DEFENDER_GENERATE
                .iter()
                .map(|(keys, case)| (case, Grip::TwoHanded, None, *keys, Reply::Static)),
        )
        .chain(
            expected::ADAPTIVE_GENERATE
                .iter()
                .map(|(buff, keys, case)| (case, Grip::TwoHanded, None, *keys, reply(buff))),
        );
    for (&(weapon, code, sl, goal, naked, want), grip, only, keys, reply) in cases {
        let got = backend().generate_build(
            weapon,
            infusion(code),
            sl,
            objective(goal),
            naked,
            grip,
            &Limits {
                class: only,
                defender: &defender(keys),
                reply,
                ..STATIC
            },
        );
        if let (Some(only), Some(got)) = (only, &got) {
            assert!(
                got.class.eq_ignore_ascii_case(only),
                "{weapon} SL {sl}: asked for a {only}, got a {}",
                got.class
            );
        }
        let (got, want) = match (got, want) {
            (None, None) => continue,
            (Some(got), Some(want)) => (got, want),
            (got, want) => panic!("{weapon} SL {sl}: {got:?} vs {want:?}"),
        };
        let (
            class,
            two,
            st,
            one_rows,
            two_rows,
            suggested,
            common,
            armor,
            note,
            by_req,
            by_load,
            trades,
        ) = want;
        assert_eq!(got.ring_trades, trades, "{weapon} SL {sl}");
        traded |= !trades.is_empty();
        assert_eq!(got.class, class, "{weapon} SL {sl}");
        assert_eq!(got.sl, sl, "the build is at the soul level asked for");
        assert_eq!(
            soul_level(&got.stats),
            sl,
            "{weapon}: the stats make the SL"
        );
        assert_eq!(got.two_handed, two, "{weapon} SL {sl}");
        assert_eq!(got.stats, stats(st), "{weapon} SL {sl}");
        assert_eq!(got.primary, (weapon.to_owned(), infusion(code)));
        assert!(!got.stub);
        for (label, got_rows, want_rows) in [
            ("1H", &got.weapons_1h, one_rows),
            ("2H only", &got.weapons_2h_only, two_rows),
        ] {
            let names: Vec<(&str, Infusion)> = got_rows
                .iter()
                .map(|row| (row.weapon.as_str(), row.infusion))
                .collect();
            let want_names: Vec<(&str, Infusion)> = want_rows
                .iter()
                .map(|&(name, code, _)| (name, infusion(code)))
                .collect();
            assert_eq!(names, want_names, "{weapon} SL {sl} {label}");
            for (row, &(name, _, damage)) in got_rows.iter().zip(want_rows) {
                // The script rounds these for printing; the row keeps the float.
                assert!(
                    (f64::from(row.damage) - damage as f64).abs() <= 0.5,
                    "{weapon} SL {sl} {name}: {} vs {damage}",
                    row.damage
                );
            }
        }
        assert_eq!(got.suggested_rings, suggested, "{weapon} SL {sl}");
        assert_eq!(got.common_rings, common, "{weapon} SL {sl}");
        // Agape Ring is common enough to top the nearest builds' rings; its place is taken, by a
        // ring the build does not already wear in any upgrade.
        assert!(
            !got.suggested_rings
                .iter()
                .chain(&got.common_rings)
                .any(|ring| ring == "Agape Ring"),
            "{weapon} SL {sl}: {:?} {:?}",
            got.suggested_rings,
            got.common_rings
        );
        assert_eq!(
            got.suggested_rings.len(),
            4,
            "{weapon} SL {sl}: a ring per slot"
        );
        assert_eq!(got.armor, armor, "{weapon} SL {sl} naked={naked}");
        assert_eq!(got.armor_note.as_deref(), note, "{weapon} SL {sl}");
        if naked {
            assert!(got.armor.is_empty(), "allow naked wears nothing");
        } else {
            assert_eq!(
                got.armor.len(),
                4,
                "{weapon} SL {sl}: a piece or Naked per slot"
            );
            armored += 1;
        }
        requirements_bound |= by_req;
        load_bound |= by_load;
    }
    // The armour search was exercised where it matters: the script's own check says the pieces'
    // stat requirements and the 70% load cap each changed the set it chose, and Rust chose the same.
    assert!(armored >= 5, "{armored} armoured builds");
    assert!(
        requirements_bound,
        "no case where armour requirements bound the choice"
    );
    assert!(
        load_bound,
        "no case where the equip-load cap bound the choice"
    );
    assert!(traded, "no case wore a ring in place of stat points");
}

/// Spells constrain a generated build as a weapon's requirements do: their INT/FTH are met, their
/// summed slot cost fits the ATT's slots, and a build that cannot fit them is refused. Each school
/// of them gets the wieldable catalyst with the most cast power, and one that would cast harder but
/// whose requirements are unmet is passed over, not picked.
#[test]
fn generate_build_with_spells_is_the_scripts() {
    let (mut raised, mut forced_att, mut refused, mut passed_over, mut schools) =
        (false, false, 0, 0, Vec::new());
    let mut banded = false;
    let spell_rows = backend().spells();
    for &(weapon, code, sl, goal, spells, want) in expected::GENERATE_SPELLS {
        let asked: Vec<String> = spells.iter().map(|&key| key.to_owned()).collect();
        let case = format!("{weapon} SL {sl} {spells:?}");
        let got = backend().generate_build(
            weapon,
            infusion(code),
            sl,
            objective(goal),
            false,
            Grip::TwoHanded,
            &Limits {
                spells: &asked,
                ..STATIC
            },
        );
        let (got, want) = match (got, want) {
            (None, None) => {
                refused += 1;
                continue;
            }
            (Some(got), Some(want)) => (got, want),
            (got, want) => panic!("{case}: {got:?} vs {want:?}"),
        };
        let (class, two, st, names, used, slots, catalysts, trades) = want;
        assert_eq!(got.ring_trades, trades, "{case}");
        banded |= trades.iter().any(|t| t.contains("attunement slots"));
        assert_eq!(got.class, class, "{case}");
        assert_eq!(got.two_handed, two, "{case}");
        assert_eq!(got.stats, stats(st), "{case}");
        assert_eq!(soul_level(&got.stats), sl, "{case}: the stats make the SL");
        assert_eq!(got.spells, asked, "{case}");
        assert_eq!(got.spell_names, names, "{case}");
        assert_eq!((got.slots_used, got.slots), (used, slots), "{case}");
        assert!(got.slots_used <= got.slots, "{case}: the spells fit");
        let picks: Vec<(&str, &str, f32, &str)> = got
            .catalysts
            .iter()
            .map(|pick| {
                (
                    pick.school.as_str(),
                    pick.name.as_str(),
                    pick.power,
                    pick.passed_over.as_deref().unwrap_or(""),
                )
            })
            .collect();
        let want_picks: Vec<(&str, &str, f32, &str)> = catalysts
            .iter()
            .map(|&(school, name, power, over)| (school, name, power as f32, over))
            .collect();
        assert_eq!(picks, want_picks, "{case}");
        passed_over += picks.iter().filter(|pick| !pick.3.is_empty()).count();
        schools.extend(picks.iter().map(|pick| pick.0.to_owned()));
        // Every requirement met, from the spell table the panel's picker lists.
        for key in &asked {
            let row = spell_rows
                .iter()
                .find(|row| row.key == *key)
                .unwrap_or_else(|| panic!("{case}: {key} is not in the spell table"));
            // A worn ring's bonus counts toward the requirement, and its trade line says so.
            let ring_for = |label: &str| got.ring_trades.iter().any(|t| t.contains(label));
            assert!(
                got.stats[7] >= row.intelligence || ring_for("INT +"),
                "{case}: {key} INT"
            );
            assert!(
                got.stats[8] >= row.faith || ring_for("FTH +"),
                "{case}: {key} FTH"
            );
            raised |= got.stats[7] == row.intelligence || got.stats[8] == row.faith;
        }
        forced_att |= used > 1 && got.stats[3] > backend().floors(sl)[3];
        // Apply attunes the spells and grants the catalysts.
        let (import, extras) = backend::to_import(&got);
        assert_eq!(import.spells, asked, "{case}");
        for pick in &got.catalysts {
            assert!(
                extras.contains(&(pick.name.clone(), Infusion::None)),
                "{case}: {} granted",
                pick.name
            );
        }
    }
    assert!(raised, "no case where a spell's requirement set INT or FTH");
    assert!(forced_att, "no case where the spells' slots raised ATT");
    assert!(refused >= 1, "{refused} refused cases");
    assert!(
        banded,
        "no case wore a Southern Ritual Band for the spells' slots"
    );
    assert!(passed_over >= 2, "{passed_over} catalysts passed over");
    for school in ["sorcery", "miracle", "pyromancy", "hex"] {
        assert!(
            schools.iter().any(|seen| seen == school),
            "no {school} catalyst"
        );
    }
}

/// An unknown spell key gets no build rather than one that ignores it.
#[test]
fn an_unknown_spell_gets_no_build() {
    let got = backend().generate_build(
        "Demons_Great_Hammer",
        Infusion::Raw,
        100,
        Objective::Damage,
        false,
        Grip::TwoHanded,
        &Limits {
            spells: &["Not_A_Spell".to_owned()],
            ..STATIC
        },
    );
    assert!(got.is_none());
}

/// A build that wields few weapons one-handed takes lighter armour of similar defense: the
/// script's `load_scarcity` and its `best_armor` trade, set for set.
#[test]
fn armor_trades_defense_for_weight_as_the_script_does() {
    let mut changed = 0;
    for &(weapon, st, scarcity, plain, scarce) in expected::ARMOR_SCARCITY {
        let stats = stats(st);
        let at = |scarcity| {
            backend()
                .armor_for(weapon, &stats, scarcity)
                .expect("a set fits")
        };
        assert_eq!(at(Some(0.0)), plain, "{weapon} {st:?} at 0");
        assert_eq!(at(Some(scarcity)), scarce, "{weapon} {st:?} at {scarcity}");
        assert_eq!(
            at(None),
            scarce,
            "{weapon} {st:?}: the build's own scarcity is {scarcity}"
        );
        changed += usize::from(plain != scarce);
    }
    assert!(changed >= 1, "a case where the trade picks a lighter set");
}

fn keys(spells: &[&str]) -> Vec<String> {
    spells.iter().map(|&key| key.to_owned()).collect()
}

/// Optimize for weapon honours the spells (and the floors setting) exactly as the script's
/// `optimize_build` does: measured 2026-09-28, it passed none, so at SL 120 with Climax chosen it
/// showed a Sorcerer build that Generate Build then refused.
#[test]
fn optimize_with_spells_is_the_scripts() {
    for &(weapon, code, sl, goal, spells, floors, want) in expected::OPTIMIZE_SPELLS {
        let asked = keys(spells);
        let limits = Limits {
            spells: &asked,
            floors,
            ..STATIC
        };
        let got = backend().optimize(
            weapon,
            infusion(code),
            sl,
            objective(goal),
            Grip::TwoHanded,
            &limits,
        );
        let case = format!("{weapon} SL {sl} {spells:?} floors {floors}");
        match (got, want) {
            (None, None) => {}
            (Some(got), Some((class, two, st, value, rings))) => {
                assert_eq!(got.class, class, "{case}");
                assert_eq!(got.two_handed, two, "{case}");
                assert_eq!(got.stats, stats(st), "{case}");
                assert_eq!(got.value, value as f32, "{case}");
                assert_eq!(got.gear, rings, "{case}: the rings worn in place of points");
            }
            (got, want) => panic!("{case}: {got:?} vs {want:?}"),
        }
    }
}

/// The refusal's words, arithmetic and fixes are the script's, and every fix it offers builds:
/// each one made, `generate_build` returns a build.
#[test]
fn refusal_is_the_scripts_and_every_fix_builds() {
    let mut fixed = 0;
    for &(weapon, code, sl, goal, grip, spells, class, floors, want) in expected::REFUSALS {
        let asked = keys(spells);
        let grip = if grip == "one" {
            Grip::OneHanded
        } else {
            Grip::TwoHanded
        };
        let class = (!class.is_empty()).then_some(class);
        let limits = Limits {
            spells: &asked,
            class,
            floors,
            ..STATIC
        };
        let (infusion, objective) = (infusion(code), objective(goal));
        let case = format!("{weapon} SL {sl} {spells:?} {class:?}");
        let got = backend().refusal(weapon, infusion, sl, objective, grip, &limits);
        let (got, (kind, closest, short, lines, fixes)) = match (got, want) {
            (None, None) => {
                assert!(
                    backend()
                        .optimize(weapon, infusion, sl, objective, grip, &limits)
                        .is_some(),
                    "{case}: no refusal, so a build"
                );
                continue;
            }
            (Some(got), Some(want)) => (got, want),
            (got, want) => panic!("{case}: {got:?} vs {want:?}"),
        };
        assert_eq!(got.kind, RefusalKind::from_name(kind).unwrap(), "{case}");
        assert_eq!(got.class.as_deref().unwrap_or(""), closest, "{case}");
        assert_eq!(got.short, short, "{case}");
        assert_eq!(got.lines, lines, "{case}");
        let offered: Vec<(&str, String, &str)> = got
            .fixes
            .iter()
            .map(|fix| match &fix.change {
                Change::RaiseSl(sl) => ("sl", sl.to_string(), fix.label.as_str()),
                Change::RemoveSpell(key) => ("spell", key.clone(), fix.label.as_str()),
                Change::IgnoreFloors => ("floors", String::new(), fix.label.as_str()),
                Change::Class(key) => ("class", key.clone(), fix.label.as_str()),
            })
            .collect();
        let want: Vec<(&str, String, &str)> = fixes
            .iter()
            .map(|&(what, value, label)| (what, value.to_owned(), label))
            .collect();
        assert_eq!(offered, want, "{case}");
        for fix in &got.fixes {
            let (mut sl, mut spells, mut class, mut floors) = (sl, asked.clone(), class, floors);
            match &fix.change {
                Change::RaiseSl(to) => sl = *to,
                Change::RemoveSpell(key) => {
                    let at = spells.iter().position(|spell| spell == key).unwrap();
                    spells.remove(at);
                }
                Change::IgnoreFloors => floors = false,
                Change::Class(key) => class = Some(key.as_str()),
            }
            let limits = Limits {
                spells: &spells,
                class,
                floors,
                ..STATIC
            };
            let build =
                backend().generate_build(weapon, infusion, sl, objective, false, grip, &limits);
            assert!(build.is_some(), "{case}: {} builds nothing", fix.label);
            fixed += 1;
        }
    }
    assert!(fixed >= 8, "the cases exercise the fixes: {fixed}");
}

/// The user's panel: Climax chosen, a Dagger and then a Roaring Halberd. At SL 120, where both were
/// refused before rings counted, both build now, wearing rings in place of points (the Dagger a
/// Southern Ritual Band for Climax's slots; the Halberd, since END and VIT have curves, an Embedded
/// and a Third Dragon Ring for VIG and VIT, its ATT holding Climax itself) and passing the floor
/// check the rings stand in for. At SL 60 Optimize for weapon and Generate
/// Build refuse alike, with the arithmetic, and each fix button's change made to the panel makes
/// both of them build.
#[test]
fn the_climax_panel_refuses_alike_and_every_fix_builds() {
    for weapon in ["Dagger", "Roaring_Halberd"] {
        let mut state = PanelState {
            mode: Mode::OptimizeForWeapon,
            spells: vec!["Climax".to_owned()],
            ..PanelState::default()
        };
        state.choose_weapon(weapon);
        state.set_sl_override(Some(120));
        let built = backend::generate(backend(), &state, None)
            .unwrap_or_else(|why| panic!("{weapon} SL 120 with rings: {why:?}"));
        let band = weapon == "Dagger";
        assert!(
            !built.ring_trades.is_empty()
                && (!band
                    || built
                        .ring_trades
                        .iter()
                        .any(|t| t.starts_with("Southern Ritual Band + 2: attunement slots +3"))),
            "{weapon}: {:?}",
            built.ring_trades
        );
        assert!(
            built.slots_used <= built.slots,
            "{weapon}: the band's slots count"
        );
        state.set_sl_override(Some(60));
        let refused = backend::generate(backend(), &state, None).expect_err("no class fits");
        assert_eq!(refused.kind, RefusalKind::Spells, "{weapon}");
        assert!(
            refused.lines[1].starts_with("SL 60 is "),
            "{:?}",
            refused.lines
        );
        assert_eq!(
            backend::ask(backend(), &state),
            backend::Answer::Refused(refused.clone()),
            "Optimize for weapon refuses as Generate Build does"
        );
        let labels: Vec<&str> = refused.fixes.iter().map(|fix| fix.label.as_str()).collect();
        assert!(labels.len() >= 2, "{weapon}: {labels:?}");
        for fix in &refused.fixes {
            let mut fixed = state.clone();
            assert!(fix.change.apply(&mut fixed), "{}", fix.label);
            let build = backend::generate(backend(), &fixed, None)
                .unwrap_or_else(|why| panic!("{weapon}: {} still refused: {why:?}", fix.label));
            assert!(
                matches!(backend::ask(backend(), &fixed), backend::Answer::Build(_)),
                "{weapon}: {} optimizes",
                fix.label
            );
            if let Change::RaiseSl(sl) = fix.change {
                assert_eq!(build.sl, sl);
            }
        }
    }
}

#[test]
fn minimum_is_the_scripts() {
    for &(weapon, two, (class, sl, st, gear)) in expected::MINIMUM {
        let got = backend()
            .minimum(weapon, Infusion::None, two)
            .unwrap_or_else(|| panic!("{weapon}: no minimum"));
        assert_eq!(got.class, class, "{weapon}");
        assert_eq!(got.sl, sl, "{weapon}");
        assert_eq!(
            soul_level(&got.stats),
            sl,
            "{weapon}: the stats make the SL"
        );
        assert_eq!(got.stats, stats(st), "{weapon}");
        assert_eq!(got.two_handed, two, "{weapon}");
        assert_eq!(got.gear, gear, "{weapon}");
    }
}

#[test]
fn similar_is_the_scripts() {
    for &(st, sl, k, status, want) in expected::SIMILAR {
        let filter = StatusFilter {
            bleed: status.contains(&"bleed"),
            poison: status.contains(&"poison"),
        };
        let got = rows(backend().similar(&stats(st), sl, k, filter));
        assert_eq!(got.len(), want.len(), "SL {sl} k {k}");
        for (row, &(count, name, infusions, grip)) in got.iter().zip(want) {
            assert_eq!(row.weapon, name, "SL {sl}");
            assert_eq!(row.damage, count as f32, "SL {sl} {name}");
            assert_eq!(row.grip, grip, "SL {sl} {name}");
            // The row holds the script's most carried recorded infusion.
            let first_known = infusions
                .iter()
                .find(|(code, _)| *code != "?")
                .map_or(Infusion::None, |(code, _)| infusion(code));
            assert_eq!(row.infusion, first_known, "SL {sl} {name}");
        }
    }
}

/// The panel's own path: the SL override drives the build, and the chosen weapon is its primary.
#[test]
fn the_panel_generates_at_its_override_for_its_weapon() {
    let mut state = PanelState {
        mode: Mode::OptimizeForWeapon,
        ..PanelState::default()
    };
    state.choose_weapon("Demons_Great_Hammer");
    assert!(state.choose_infusion(Infusion::Raw));
    state.set_sl_override(Some(100));
    let build = backend::generate(backend(), &state, None).expect("a SL 100 build");
    assert_eq!(build.sl, 100);
    assert_eq!(ds2_build_import_core::level::soul_level(&build.stats), 100);
    assert_eq!(
        build.primary,
        ("Demons_Great_Hammer".to_owned(), Infusion::Raw)
    );
    assert!(!build.stub);

    // Armour by default, worn by the import in its four slots; none only when asked.
    assert!(!state.allow_naked, "the panel starts with armour required");
    assert_eq!(build.armor.len(), 4);
    let (import, _) = backend::to_import(&build);
    assert_eq!(import.armor, build.armor);
    assert!(
        import
            .armor
            .iter()
            .all(|piece| !ds2_build_import_core::is_empty_slot(piece)),
        "{:?}",
        import.armor
    );
    state.allow_naked = true;
    let naked = backend::generate(backend(), &state, None).expect("a SL 100 build");
    assert!(naked.armor.is_empty() && naked.armor_note.is_none());
}

/// Everything Apply would grant from a real build resolves in the item catalogue.
#[test]
fn every_generated_grant_names_a_real_item() {
    use ds2_build_import_core::{ItemError, id_for, is_empty_slot};
    let cases = expected::GENERATE
        .iter()
        .map(|&(weapon, code, sl, goal, naked, _)| (weapon, code, sl, goal, naked, &[][..]))
        .chain(
            expected::GENERATE_SPELLS
                .iter()
                .map(|&(weapon, code, sl, goal, spells, _)| {
                    (weapon, code, sl, goal, false, spells)
                }),
        );
    for (weapon, code, sl, goal, naked, spells) in cases {
        let spells: Vec<String> = spells.iter().map(|&key| key.to_owned()).collect();
        let Some(build) = backend().generate_build(
            weapon,
            infusion(code),
            sl,
            objective(goal),
            naked,
            Grip::TwoHanded,
            &Limits {
                spells: &spells,
                ..STATIC
            },
        ) else {
            continue;
        };
        let (import, extras) = backend::to_import(&build);
        let names = import
            .weapons
            .chunks(2)
            .map(|pair| pair[0].clone())
            .chain(import.armor.iter().cloned())
            .chain(import.rings.iter().cloned())
            .chain(import.spells.iter().cloned())
            .chain(extras.into_iter().map(|(name, _)| name));
        for name in names {
            if is_empty_slot(&name) {
                continue;
            }
            match id_for(&name) {
                Ok(_) | Err(ItemError::Ambiguous { .. }) => {}
                Err(error) => panic!("{weapon} SL {sl}: {name:?}: {error}"),
            }
        }
    }
}

/// Weapon flexibility, for hand-picked stat lines and for every build the script generated, asked
/// with the names the panel holds: counts, the neighbour ranking, the spare load, and the words.
#[test]
fn flexibility_is_the_scripts() {
    use ds2_build_recommender_core::flex::{flex_line, flex_load_line};
    assert_eq!(expected::FLEXIBILITY.len(), expected::FLEX_LINES.len());
    for (&(st, sl, armor, rings, want), &(line, load)) in
        expected::FLEXIBILITY.iter().zip(expected::FLEX_LINES)
    {
        let armor: Vec<String> = armor.iter().map(|name| (*name).to_owned()).collect();
        let rings: Vec<String> = rings.iter().map(|name| (*name).to_owned()).collect();
        let got = backend()
            .flexibility(&stats(st), sl, &armor, &rings)
            .expect("the fixture carries neighbour counts");
        let (one, two, total, below, equal, n, percentile, spare, fits) = want;
        assert_eq!(
            (got.one_handed, got.two_handed, got.total),
            (one, two, total),
            "{st:?}"
        );
        assert_eq!(
            (got.below, got.equal, got.neighbours),
            (below, equal, n),
            "{st:?}"
        );
        assert_eq!(got.percentile.to_bits(), percentile.to_bits(), "{st:?}");
        assert_eq!(
            got.spare_load.to_bits(),
            spare.to_bits(),
            "{st:?} {armor:?} {rings:?}"
        );
        assert_eq!(got.fits, fits, "{st:?}");
        assert_eq!(flex_line(&got), line);
        assert_eq!(flex_load_line(&got), load);
    }
}

/// The SL 35 DEX/FTH character the metric was written for wields 28 of 324 weapons one-handed and
/// 110 two-handed, as measured on the full corpus: the counts do not depend on the sample.
#[test]
fn the_sl35_character_wields_what_was_measured() {
    let got = backend()
        .flexibility(&[10, 6, 7, 6, 6, 20, 9, 6, 18], 35, &[], &[])
        .expect("neighbour counts");
    assert_eq!((got.one_handed, got.two_handed, got.total), (28, 110, 324));
}

/// A data file written before the script exported neighbour counts still reads, and says it has
/// no flexibility to rank rather than ranking against nothing.
#[test]
fn a_file_without_neighbour_counts_has_no_flexibility() {
    // A build's record up to its weapons, the trailing fields from the neighbour counts on cut.
    let old: String = include_str!("fixtures/corpus-sample.dat")
        .lines()
        .map(|line| {
            if line.starts_with("X\t") {
                line.split('\t').take(5).collect::<Vec<_>>().join("\t")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let backend = CorpusBackend::parse(&old).expect("an older file parses");
    assert!(
        backend
            .flexibility(&[10, 6, 7, 6, 6, 20, 9, 6, 18], 35, &[], &[])
            .is_none()
    );
}

/// soulsplanner's class table in the data file is the game's `PlayerStatusParam`, which is what
/// `StartingClass` carries: two sources for the same eight spreads, held together.
#[test]
fn the_data_files_classes_are_the_games() {
    use ds2_build_import_core::StartingClass;
    let mut seen = 0;
    for line in include_str!("fixtures/corpus-sample.dat").lines() {
        let Some(rest) = line.strip_prefix("C\t") else {
            continue;
        };
        let fields: Vec<&str> = rest.split('\t').collect();
        let class = StartingClass::from_key(fields[0])
            .unwrap_or_else(|| panic!("{:?} is no starting class", fields[0]));
        let level: u32 = fields[2].parse().expect("a level");
        let spread: Vec<u16> = fields[3..12]
            .iter()
            .map(|field| field.parse().expect("a stat"))
            .collect();
        let spread: [u16; STAT_COUNT] = spread.try_into().expect("nine stats");
        assert_eq!(level, class.level(), "{class}");
        assert_eq!(backend::game_order(&spread), class.base(), "{class}");
        seen += 1;
    }
    assert_eq!(seen, StartingClass::ALL.len());
}

/// The measured case, end to end on the corpus: a Sorcerer's SL 90 panel. Unrestricted, another
/// class wins and the build is under the Sorcerer's base -- the shape of the build that was
/// applied. Asked for the character's class, every stat is at or above it.
#[test]
fn a_sorcerers_build_is_a_sorcerers() {
    use ds2_build_import_core::{StartingClass, check_build};
    let state = PanelState {
        weapon: Some("Demons_Great_Hammer"),
        infusion: Infusion::Raw,
        sl_override: Some(90),
        ..PanelState::default()
    };
    let any = backend::generate(backend(), &state, None).expect("some class wields it");
    assert_ne!(
        any.class, "Sorcerer",
        "the case only means something if another class wins"
    );
    assert!(
        check_build(
            "sorcerer",
            StartingClass::Sorcerer,
            &backend::game_order(&any.stats)
        )
        .is_err()
    );
    let own = backend::generate(backend(), &state, Some(StartingClass::Sorcerer))
        .expect("a Sorcerer wields it at SL 90");
    assert_eq!(own.class, "Sorcerer");
    assert_eq!(own.sl, 90);
    assert_eq!(
        check_build(
            &own.class,
            StartingClass::Sorcerer,
            &backend::game_order(&own.stats)
        ),
        Ok(())
    );
    let (import, _) = backend::to_import(&own);
    assert_eq!(
        import.class, "sorcerer",
        "the import sees the class it checks"
    );
}

/// The weapon picker's card says what the ranking says: for every row the script ranked, the card
/// for that weapon, infusion and stats carries the row's attack by type, and it is two-handed only
/// exactly when the row is.
#[test]
fn a_weapons_card_is_its_ranked_row() {
    use ds2_build_recommender_core::backend::Wield;
    let mut checked = 0;
    for &(st, sl, one_hand, class, per_class, window, raw_ar, goal, _) in expected::WEAPONS_FOR {
        let opts = WeaponsForOpts {
            one_hand,
            class: (!class.is_empty()).then(|| class.to_owned()),
            per_class,
            window_s: window as f32,
            raw_ar,
            objective: objective(goal),
            ..WeaponsForOpts::default()
        };
        let stats = stats(st);
        for row in rows(backend().weapons_for(&stats, sl, &opts)) {
            // A bow, greatbow or crossbow's row is one shot: its attack is the launcher's plus the
            // ammunition's, times the hand scale, which is not what the card shows the launcher at.
            if row.grip.contains("1 shot") {
                continue;
            }
            let key = weapons::all()
                .iter()
                .find(|weapon| weapon.name == row.weapon)
                .map(|weapon| weapon.key)
                .expect("a ranked weapon is in the table");
            let card = backend()
                .weapon_card(key, row.infusion, &stats)
                .expect("a ranked weapon has a card");
            assert_eq!(card.infusion, row.infusion, "{key}");
            assert_eq!(
                card.attack.map(|value| value.unwrap_or(0.0)),
                row.ar_by_type,
                "{key} {:?} at {st:?}",
                row.infusion
            );
            let wield = if row.two_hand_only {
                Wield::TwoHandedOnly
            } else {
                Wield::Both
            };
            assert_eq!(card.wield, wield, "{key} at {st:?}: {}", row.grip);
            assert!(card.requirements.iter().all(|req| req.met), "{key}");
            checked += 1;
        }
    }
    assert!(checked > 100, "only {checked} rows checked");
}

/// Stats that cannot hold a weapon mark the requirements that stop them, and an infusion the
/// weapon does not take falls back to uninfused, as choosing the weapon does.
#[test]
fn a_card_marks_what_the_stats_miss() {
    use ds2_build_recommender_core::backend::Wield;
    let low = backend()
        .weapon_card("Greatsword", Infusion::None, &[1; STAT_COUNT])
        .expect("the Greatsword has a card");
    assert_eq!(low.wield, Wield::Neither);
    assert!(!low.requirements.is_empty());
    assert!(low.requirements.iter().all(|req| req.value > 0));
    assert!(
        low.requirements
            .iter()
            .filter(|req| req.value > 1)
            .all(|req| !req.met),
        "{:?}",
        low.requirements
    );
    let high = backend()
        .weapon_card("Greatsword", Infusion::Raw, &[99; STAT_COUNT])
        .expect("the Greatsword has a card");
    assert_eq!(high.wield, Wield::Both);
    assert!(high.requirements.iter().all(|req| req.met));
    assert!(high.weight > 0.0);
    let takes_raw = weapons::by_key("Greatsword").is_some_and(|row| row.takes(Infusion::Raw));
    assert_eq!(
        high.infusion,
        if takes_raw {
            Infusion::Raw
        } else {
            Infusion::None
        }
    );
    assert!(
        backend()
            .weapon_card("No_Such_Weapon", Infusion::None, &[99; STAT_COUNT])
            .is_none()
    );
    assert!(
        backend::StubBackend
            .weapon_card("Greatsword", Infusion::None, &[99; STAT_COUNT])
            .is_none()
    );
}

/// The script's `--rank` name.
fn rank(name: &str) -> Rank {
    match name {
        "window" => Rank::Window,
        "per-stamina" => Rank::PerStamina,
        "bar" => Rank::Bar,
        other => panic!("rank {other}"),
    }
}

/// Best weapons, the script's `--best-weapons` with its `--json` metrics: every weapon an
/// infusion goes on, at the build the optimizer makes for it, ranked; the rows' builds, labels,
/// launchers' ammunition and the R1's reach, timing and five-second damage.
#[test]
fn best_weapons_is_the_scripts() {
    let (mut compared, mut launchers, mut timed, mut poised) = (0, 0, 0, 0);
    for &(code, sl, goal, grip, class, window, by, with_status, keys, who, want) in
        expected::BEST_WEAPONS
    {
        // Not ported yet: the stamina ranks and the status ranking.
        if by != "window" || with_status {
            continue;
        }
        let opts = BestWeaponsOpts {
            weapon_class: (!class.is_empty()).then(|| class.to_owned()),
            window_s: window as f32,
            rank: rank(by),
            with_status,
        };
        let limits = Limits {
            defender: &defender(keys),
            reply: if who == "static" {
                Reply::Static
            } else {
                reply(who)
            },
            ..STATIC
        };
        let grip = if grip == "one" {
            Grip::OneHanded
        } else {
            Grip::TwoHanded
        };
        let got = backend().best_weapons(infusion(code), sl, objective(goal), grip, &limits, &opts);
        let case = format!("{code} SL {sl} {goal} {class} window {window} {by} {who}");
        assert_eq!(got.len(), want.len(), "{case}: rows");
        for (row, want) in got.iter().zip(want) {
            let (score, key, value, class, two, st, rings, label, ammo, r1, poise, ..) = *want;
            let at = format!("{case}: {key}");
            assert_eq!(row.weapon, key, "{case}");
            assert_eq!(row.score as f32, score as f32, "{at}: score");
            assert_eq!(row.value as f32, value as f32, "{at}: value");
            assert_eq!(row.class, class, "{at}");
            assert_eq!(row.two_handed, two, "{at}");
            assert_eq!(row.stats, stats(st), "{at}");
            assert_eq!(row.rings, rings, "{at}");
            assert_eq!(row.label, label, "{at}");
            assert_eq!(
                row.ammo
                    .as_ref()
                    .map(|(ammo, shot)| (ammo.as_str(), shot.as_str())),
                ammo,
                "{at}"
            );
            let m = &row.metrics;
            assert_eq!(
                (
                    m.reach_m,
                    m.startup_s,
                    m.recovery_s,
                    m.time_to_first_hit_s,
                    m.damage_per_5s
                ),
                r1,
                "{at}: R1 metrics"
            );
            assert_eq!(
                (
                    m.hyperarmor,
                    m.hyperarmor_rate,
                    m.hyperarmor_holds,
                    m.poise_damage_per_hit,
                    m.armor_break.map(i64::from),
                    m.hits_to_stagger.map(i64::from),
                    m.defender_poise
                ),
                poise,
                "{at}: poise metrics"
            );
            compared += 1;
            launchers += usize::from(ammo.is_some());
            timed += usize::from(r1.4.is_some());
            poised += usize::from(poise.3.is_some());
        }
    }
    assert!(
        compared >= 20 && launchers >= 4 && timed >= 10 && poised >= 10,
        "{compared} rows, {launchers} launchers, {timed} with 5 s damage, {poised} with poise data"
    );
}
