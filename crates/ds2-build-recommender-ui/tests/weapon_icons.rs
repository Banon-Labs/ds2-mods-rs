//! Every weapon the picker lists has an icon, on a machine with DARK SOULS II installed, so no row
//! falls back to its class's name. The item ids behind them are checked without an install, in
//! `ds2-build-recommender-core`'s own tests.

use std::path::PathBuf;

use ds2_build_recommender_core::weapons;
use ds2_overlay::ebl::Archive;

fn game() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home)
        .join(".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game");
    dir.join("GameDataEbl.bhd").is_file().then_some(dir)
}

#[test]
fn every_weapon_in_the_picker_has_an_icon() {
    let Some(game) = game() else {
        eprintln!("no DARK SOULS II install here; skipped");
        return;
    };
    let archive = Archive::open(&game).expect("the archive opens");
    let missing: Vec<String> = weapons::all()
        .iter()
        .filter_map(|row| {
            let Some(id) = weapons::item_id(row.key) else {
                return Some(format!("{}: no item id", row.key));
            };
            ds2_overlay::item_icon::load(&archive, id)
                .err()
                .map(|error| format!("{} ({id}): {error}", row.key))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "{} of {} weapons have no icon: {missing:#?}",
        missing.len(),
        weapons::all().len()
    );
}
