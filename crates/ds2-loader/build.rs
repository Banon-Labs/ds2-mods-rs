//! Bake the commit this DLL was built from into it, so a run's log names the code it loaded.
//!
//! `scripts/ds2-run.py` prints the staged file's SHA-256, which says which bytes ran but not which
//! commit made them; the push guard had to compare times to connect the two. With `DS2_BUILD_GIT`
//! in the first log line, a run names its commit outright. `-dirty` is appended when the working
//! tree differed from that commit under `crates/`, so a build of uncommitted code cannot pass for
//! the commit it sits on.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn main() {
    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let dirty = git(&["status", "--porcelain", "--", "../../crates", "../../Cargo.toml"])
        .is_some_and(|status| !status.is_empty());
    let suffix = if dirty { "-dirty" } else { "" };
    println!("cargo:rustc-env=DS2_BUILD_GIT={sha}{suffix}");

    // Rebuild the stamp when the commit moves or the index changes (a commit, an add), and when
    // any source under crates/ changes, since that is what makes a tree dirty.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/index");
    }
    if let Some(common) = git(&["rev-parse", "--git-common-dir"]) {
        println!("cargo:rerun-if-changed={common}/refs/heads");
        println!("cargo:rerun-if-changed={common}/packed-refs");
    }
    println!("cargo:rerun-if-changed=../../crates");
}
