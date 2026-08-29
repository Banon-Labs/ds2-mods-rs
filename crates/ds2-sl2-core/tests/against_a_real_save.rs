//! [`ds2_sl2_core::validate`] against a save the game actually wrote.
//!
//! The unit tests build BND4 headers by hand, which proves the parser's arithmetic and proves
//! nothing about DARK SOULS II. A real `DS2SOFS0000.sl2` is the only thing that can say the entry
//! table is walked the way the game lays it out -- and getting that wrong is exactly the kind of
//! mistake a hand-built fixture agrees with, because the same person wrote both.
//!
//! Skipped, loudly, when no save is named. Run it with:
//!
//! ```text
//! DS2_SAVE_FILE="$HOME/.local/share/Steam/steamapps/compatdata/335300/pfx/drive_c/users/\
//! steamuser/AppData/Roaming/DarkSoulsII/<steamid>/DS2SOFS0000.sl2" \
//!   cargo test -p ds2-sl2-core
//! ```

/// The environment variable naming a save to read.
const SAVE_VAR: &str = "DS2_SAVE_FILE";

/// A real save is a BND4 container holding at least one payload.
#[test]
fn the_save_the_game_wrote_validates() {
    let Ok(path) = std::env::var(SAVE_VAR) else {
        eprintln!("skipping: set {SAVE_VAR} to a DS2SOFS0000.sl2 to run this");
        return;
    };
    let bytes = std::fs::read(&path).expect("the save named by the variable must be readable");
    assert!(bytes.len() > 0x40, "a save is bigger than its own header");
    assert_eq!(
        &bytes[..4],
        b"BND4",
        "a save starts with the container magic"
    );

    let entries = ds2_sl2_core::validate(&bytes).expect("a real save must validate");
    // A DS2 save carries one payload per character slot plus the system data, so "more than none"
    // is the honest bound. Pinning the exact count would pin this machine's save file, not the
    // format.
    assert!(entries > 0, "a real save holds at least one payload");
    eprintln!("{path}: {} bytes, {entries} entries", bytes.len());
}

/// Truncating a real save is caught, rather than read as a shorter one.
#[test]
fn half_a_real_save_is_refused() {
    let Ok(path) = std::env::var(SAVE_VAR) else {
        return;
    };
    let bytes = std::fs::read(&path).expect("readable");
    let half = &bytes[..bytes.len() / 2];
    assert!(
        ds2_sl2_core::validate(half).is_err(),
        "a save cut in half still has the magic and must not pass on that alone"
    );
}
