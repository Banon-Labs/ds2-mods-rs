//! The parser against a page the site actually served.
//!
//! The unit tests in `saved_build` use a fixture reduced to the fields this crate reads, which
//! proves the grammar and proves nothing about the page. THE REAL PAGE IS 28 KB OF SCRIPT,
//! STYLESHEET AND MARKUP with the literal somewhere in the middle of it, and every simplification
//! the fixture makes is a way this could pass a unit test and fail on the site.
//!
//! Skipped, loudly, when the page is not there -- the same shape `er-gfx`'s corpus tests use. Save
//! one with:
//!
//! ```text
//! curl -A Mozilla/5.0 https://soulsplanner.com/darksouls2/253 > /tmp/build253.html
//! DS2_SOULSPLANNER_PAGE=/tmp/build253.html cargo test -p ds2-build-import-core
//! ```

/// The environment variable naming a saved page.
const PAGE_VAR: &str = "DS2_SOULSPLANNER_PAGE";

/// The build the saved page is expected to be.
const BUILD_ID: u32 = 253;

/// The real page yields the same build the reference python script reports.
///
/// Values pinned from `scripts/ds2-soulsplanner.py 253` on 2026-08-28. If the site edits this
/// build the test fails, which is the point: it would mean the fixture and the site have parted
/// company and the pinned numbers below are no longer evidence of anything.
#[test]
fn the_page_the_site_serves_parses_into_the_build_the_script_reports() {
    let Ok(path) = std::env::var(PAGE_VAR) else {
        eprintln!("skipping: set {PAGE_VAR} to a saved soulsplanner page to run this");
        return;
    };
    let html =
        std::fs::read_to_string(&path).expect("the page named by the variable must be readable");

    let build = ds2_build_import_core::saved_build::parse(&html, BUILD_ID)
        .expect("a saved page of a real build must parse");

    assert_eq!(build.class, "swordsman");
    assert_eq!(build.covenant, "Brotherhood_of_Blood");
    assert_eq!(build.gender, 0);
    assert_eq!(build.grip, 0);
    assert_eq!(
        build.armor,
        [
            "Black_Hood",
            "Armor_of_the_Forlorn",
            "Shadow_Gauntlets",
            "Leggings_of_the_Forlorn"
        ]
    );
    assert_eq!(
        build.rings,
        [
            "Ring_of_Binding",
            "Flynns_Ring",
            "Crest_of_Blood",
            "Third_Dragon_Ring"
        ]
    );
    assert_eq!(build.weapons.len(), 12);
    assert_eq!(build.spells.len(), 14);
    assert_eq!(build.items.len(), 10);
    assert_eq!(build.stats.vigor, 50);
    assert_eq!(build.stats.endurance, 20);
    assert_eq!(build.stats.vitality, 4);
    assert_eq!(build.stats.attunement, 16);
    assert_eq!(build.stats.strength, 25);
    assert_eq!(build.stats.dexterity, 16);
    assert_eq!(build.stats.adaptability, 16);
    assert_eq!(build.stats.intelligence, 28);
    assert_eq!(build.stats.faith, 28);
}

/// The link the player will paste reaches the id this page is.
#[test]
fn the_link_the_player_will_paste_reaches_this_build() {
    assert_eq!(
        ds2_build_import_core::build_id_from_url(&format!(
            "{}{BUILD_ID}",
            ds2_build_import_core::BUILD_URL_PREFIX
        )),
        Ok(BUILD_ID)
    );
}
