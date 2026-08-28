//! Rebind a real donor save and check the result is internally coherent.
//!
//! WHY THIS DOES NOT DIFF AGAINST A STORED "KNOWN GOOD" FILE. It did, once, against the save
//! `scripts/ds2-sl2-rebind.py` produced and the game loaded -- and the test failed with 630872
//! differing bytes for a reason that had nothing to do with this code: the redirect points the
//! game at the staged copy, so DARK SOULS II WROTE TO IT on the first load and the reference
//! stopped being the reference. A fixture that the system under test mutates is not a fixture.
//!
//! The byte-for-byte agreement was still established, out of band and recorded here: rebinding the
//! same donor with `scripts/ds2-sl2-rebind.py` (openssl) and with `examples/rebind.rs` (the `aes`
//! crate) produces identical files, 6728 bytes differing from the donor. Two independent
//! implementations, one of them already validated by the game loading its output. To redo it:
//!
//! ```text
//! python3 scripts/ds2-sl2-rebind.py <donor>.sl2 --to <id> -o /tmp/a.sl2
//! cargo run -p ds2-sl2-core --example rebind -- <donor>.sl2 <id> /tmp/b.sl2
//! cmp /tmp/a.sl2 /tmp/b.sl2
//! ```
//!
//! What remains here is what can be asserted from the donor alone, and it is skipped rather than
//! failed where that file does not exist -- it is someone's save data, not a repo fixture.

use std::path::Path;

const DONOR: &str = "/home/banon/DS2/DarkSoulsII/01100001526d6d84/DS2SOFS0000.sl2";
const ACCOUNT: &str = "01100001018fa4be";
const DONOR_ID: &str = "01100001526d6d84";

fn donor() -> Option<Vec<u8>> {
    Path::new(DONOR)
        .is_file()
        .then(|| std::fs::read(DONOR).expect("read donor"))
}

#[test]
fn rebind_replaces_exactly_one_id_and_preserves_length() {
    let Some(original) = donor() else {
        eprintln!("skipped: donor save not present on this machine");
        return;
    };
    let mut save = original.clone();
    let report = ds2_sl2_core::rebind(&mut save, ACCOUNT).expect("rebind");

    assert_eq!(report.replaced, 1, "exactly one Steam ID should have moved");
    assert_eq!(report.previous.as_deref(), Some(DONOR_ID));
    assert_eq!(save.len(), original.len(), "the patch must not resize");

    // The reseal re-encrypts one entry, so the change is confined but not tiny: CBC propagates
    // from the patched block to the end of that entry. It must not touch the whole file.
    let differing = save.iter().zip(&original).filter(|(a, b)| a != b).count();
    assert!(differing > 0, "nothing changed");
    assert!(
        differing < original.len() / 10,
        "{differing} bytes changed; the edit should be confined to one entry"
    );
}

/// Rebinding is idempotent. Every launch re-stages, so a save that already belongs to this account
/// must come out untouched rather than being re-encrypted each time.
#[test]
fn rebinding_twice_is_a_no_op_the_second_time() {
    let Some(original) = donor() else {
        eprintln!("skipped: donor save not present on this machine");
        return;
    };
    let mut once = original.clone();
    ds2_sl2_core::rebind(&mut once, ACCOUNT).expect("first rebind");
    let mut twice = once.clone();
    let report = ds2_sl2_core::rebind(&mut twice, ACCOUNT).expect("second rebind");
    assert_eq!(report.replaced, 0);
    assert_eq!(report.previous.as_deref(), Some(ACCOUNT));
    assert_eq!(twice, once, "a second rebind must change nothing");
}

/// A save already belonging to this account must be recognised, whichever direction it came from.
#[test]
fn a_foreign_id_is_reported_before_it_is_replaced() {
    let Some(original) = donor() else {
        eprintln!("skipped: donor save not present on this machine");
        return;
    };
    let mut save = original;
    let report = ds2_sl2_core::rebind(&mut save, DONOR_ID).expect("rebind to its own id");
    assert_eq!(
        report.replaced, 0,
        "rebinding to its own id changes nothing"
    );
}
