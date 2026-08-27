//! Single cc-compile of the vendored MinHook C source, so every DLL that wants a detour links
//! one already-built `libminhook.a` instead of carrying its own build script.
//!
//! WHY THIS IS ~20 LINES WHERE `er-mods-rs/crates/er-hook/build.rs` IS ~130. That repo
//! GITIGNORES its `vendor/` drop, so the MinHook source exists only inside whichever checkout
//! happened to populate it: a linked git worktree -- which is how that repo is built, one agent
//! per worktree -- has no `vendor/` above it at all and no way to reach the one that does. Its
//! build script therefore carries an `ER_MINHOOK_SRC_DIR` env override, a second legacy override,
//! an ancestor walk, a `.git`-file parse to find a worktree's main tree, and a
//! `git rev-parse --path-format=absolute --git-common-dir` subprocess, all to answer "where did
//! the source go".
//!
//! THIS REPO COMMITS `vendor/minhook/` TO GIT. Every clone and every linked worktree has the C
//! source checked out at one fixed path relative to this manifest, so the entire search collapses
//! to a single join and there is nothing left to configure.
//!
//! DO NOT REINTRODUCE THE SEARCH. If this path ever fails to resolve, the bug is that `vendor/`
//! left git -- put it back. A resolver that goes hunting would turn that into a silent success
//! against some other checkout's copy, which is strictly worse than a build error naming the
//! missing directory.

use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // The MinHook C is Win32 (VirtualAlloc, thread suspension, SEH-adjacent code). A host
    // `cargo check` still RUNS build scripts, so a non-Windows target has to skip the compile
    // rather than fail it -- the crate's Rust half is host-parseable and the tests below
    // `#[cfg(test)]` are meant to run there.
    let target = env::var("TARGET").expect("cargo sets TARGET for build scripts");
    if !target.contains("windows") {
        return;
    }

    let arch = target.split('-').next().unwrap_or_default();
    let hde = match arch {
        "i686" => "hde/hde32.c",
        "x86_64" => "hde/hde64.c",
        _ => panic!("Architecture '{arch}' not supported by bundled MinHook"),
    };

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let mh_src_dir = manifest_dir.join("../../vendor/minhook/src");

    cc::Build::new()
        .file(mh_src_dir.join("buffer.c"))
        .file(mh_src_dir.join("hook.c"))
        .file(mh_src_dir.join("trampoline.c"))
        .file(mh_src_dir.join(hde))
        .compile("minhook");

    println!("cargo:rerun-if-changed={}", mh_src_dir.display());
}
