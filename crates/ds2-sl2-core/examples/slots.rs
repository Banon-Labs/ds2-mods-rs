//! List a `.sl2`'s ten character slots, on the host, using the same code the DLL runs.
//!
//!     cargo run -p ds2-sl2-core --example slots -- <save.sl2>
//!
//! Exists for the reason [`rebind`](../rebind.rs) does: so the crate's answer can be diffed
//! against `scripts/ds2-sl2.py --slots` without a game, a Windows target or a runtime test. The
//! output is that script's format character for character, so the check is a `diff` and not a
//! reading. Two independent implementations agreeing on a real save is the evidence that the
//! section walk, the stat block and the name decode are right.
//!
//! The crate's own real-save test asserts SHAPE -- ten slots, in order, loadable ones carrying
//! stats -- deliberately, so it does not pin one machine's characters. That leaves the values
//! unchecked against anything, and the values are what a picker's rows would show. This closes
//! that gap without putting somebody's save into an assertion.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, input] = args.as_slice() else {
        eprintln!("usage: slots <save.sl2>");
        std::process::exit(2);
    };
    let save = std::fs::read(input).expect("read save");
    let slots = ds2_sl2_core::slots(&save).expect("read the character list");
    for slot in &slots {
        // `scripts/ds2-sl2.py` prints the state left-aligned in eleven columns and appends the
        // stat total and name only for a slot that holds something. Matching it exactly is the
        // whole point of this example.
        let state = match slot.state {
            ds2_sl2_core::SlotState::Empty => "empty",
            ds2_sl2_core::SlotState::Blank => "blank",
            ds2_sl2_core::SlotState::Occupied => "occupied",
        };
        let extra = if slot.state == ds2_sl2_core::SlotState::Empty {
            String::new()
        } else {
            format!("  stats={:<4} name={}", slot.stat_total(), slot.name)
        };
        println!("slot {} {state:<11}{extra}", slot.slot);
    }
}
