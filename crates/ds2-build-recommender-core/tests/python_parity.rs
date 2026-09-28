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
use ds2_build_recommender_core::backend::{self, Outcome, RecommenderBackend, ResultRow};
use ds2_build_recommender_core::corpus::CorpusBackend;
use ds2_build_recommender_core::model::{
    Mode, Objective, PanelState, STAT_COUNT, StatusFilter, WeaponsForOpts,
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
type WeaponsForCases = &'static [(
    &'static [u16],
    u16,
    bool,
    &'static str,
    bool,
    f64,
    bool,
    &'static [ForRow],
)];
type OptimizeCases = &'static [(
    &'static str,
    &'static str,
    u16,
    &'static str,
    Option<(&'static str, bool, &'static [u16], f64)>,
)];
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
);
type GenerateCases = &'static [(
    &'static str,
    &'static str,
    u16,
    &'static str,
    bool,
    Option<Generated>,
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
        "bleed" => Objective::Bleed,
        "poison" => Objective::Poison,
        other => panic!("objective {other}"),
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

#[test]
fn weapons_for_is_the_scripts() {
    for (case, &(st, sl, one_hand, class, per_class, window, raw_ar, want)) in
        expected::WEAPONS_FOR.iter().enumerate()
    {
        let opts = WeaponsForOpts {
            one_hand,
            class: (!class.is_empty()).then(|| class.to_owned()),
            per_class,
            window_s: window as f32,
            raw_ar,
        };
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
}

#[test]
fn optimize_is_the_scripts() {
    for &(weapon, code, sl, goal, want) in expected::OPTIMIZE {
        let got = backend().optimize(weapon, infusion(code), sl, objective(goal));
        match (got, want) {
            (None, None) => {}
            (Some(got), Some((class, two, st, value))) => {
                assert_eq!(got.class, class, "{weapon} SL {sl}");
                assert_eq!(got.two_handed, two, "{weapon} SL {sl}");
                assert_eq!(got.stats, stats(st), "{weapon} SL {sl}");
                assert_eq!(got.sl, sl);
                assert_eq!(got.value, value as f32, "{weapon} SL {sl}");
            }
            (got, want) => panic!("{weapon} SL {sl}: {got:?} vs {want:?}"),
        }
    }
}

#[test]
fn generate_build_is_the_scripts() {
    let (mut requirements_bound, mut load_bound, mut armored) = (false, false, 0);
    for &(weapon, code, sl, goal, naked, want) in expected::GENERATE {
        let got = backend().generate_build(weapon, infusion(code), sl, objective(goal), naked);
        let (got, want) = match (got, want) {
            (None, None) => continue,
            (Some(got), Some(want)) => (got, want),
            (got, want) => panic!("{weapon} SL {sl}: {got:?} vs {want:?}"),
        };
        let (class, two, st, one_rows, two_rows, suggested, common, armor, note, by_req, by_load) =
            want;
        assert_eq!(got.class, class, "{weapon} SL {sl}");
        assert_eq!(got.sl, sl, "the build is at the soul level asked for");
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
}

#[test]
fn minimum_is_the_scripts() {
    for &(weapon, two, (class, sl, st, gear)) in expected::MINIMUM {
        let got = backend()
            .minimum(weapon, Infusion::None, two)
            .unwrap_or_else(|| panic!("{weapon}: no minimum"));
        assert_eq!(got.class, class, "{weapon}");
        assert_eq!(got.sl, sl, "{weapon}");
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
    let build = backend::generate(backend(), &state).expect("a SL 100 build");
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
    let naked = backend::generate(backend(), &state).expect("a SL 100 build");
    assert!(naked.armor.is_empty() && naked.armor_note.is_none());
}

/// Everything Apply would grant from a real build resolves in the item catalogue.
#[test]
fn every_generated_grant_names_a_real_item() {
    use ds2_build_import_core::{ItemError, id_for, is_empty_slot};
    for &(weapon, code, sl, goal, naked, _) in expected::GENERATE {
        let Some(build) =
            backend().generate_build(weapon, infusion(code), sl, objective(goal), naked)
        else {
            continue;
        };
        let (import, extras) = backend::to_import(&build);
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
                Err(error) => panic!("{weapon} SL {sl}: {name:?}: {error}"),
            }
        }
    }
}
