//! Rebind a `.sl2` to an account ID, on the host, using the same code the DLL runs.
//!
//!     cargo run -p ds2-sl2-core --example rebind -- <in.sl2> <steamid-hex-16> <out.sl2>
//!
//! Exists so the crate's behaviour can be diffed against `scripts/ds2-sl2-rebind.py` without a
//! game, a Windows target, or a runtime test. Two independent implementations agreeing byte for
//! byte is the evidence that the reseal is right.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, id, output] = args.as_slice() else {
        eprintln!("usage: rebind <in.sl2> <steamid-hex-16> <out.sl2>");
        std::process::exit(2);
    };
    let mut save = std::fs::read(input).expect("read input");
    let report = ds2_sl2_core::rebind(&mut save, id).expect("rebind");
    std::fs::write(output, &save).expect("write output");
    println!(
        "replaced={} previous={:?} bytes={}",
        report.replaced,
        report.previous,
        save.len()
    );
}
