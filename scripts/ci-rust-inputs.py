#!/usr/bin/env python3
"""Decide whether a CI run has to build and test the Rust: did the change touch a Rust input?

    python3 scripts/ci-rust-inputs.py --event pull_request --base <sha>   # CI, writes $GITHUB_OUTPUT
    python3 scripts/ci-rust-inputs.py --event pull_request --base origin/main
    python3 scripts/ci-rust-inputs.py --selftest

WHAT IT DECIDES. `.github/workflows/gate.yml` runs the `host-tests` leg, and `release.yml` its
DLL build, only when this prints `rust=true`; the `check` leg always runs and passes check.sh
`--no-rust` when it prints `rust=false`. A pull request that changes only Python scripts, docs or
cupcake policies then gets the checks that read those files (commit messages, PR template, cupcake
suites, script selftests) and not the ~20 runner-minutes of cargo that cannot see them. MEASURED
2026-11-02 on PR #364 (scripts only): `host-tests` 15:57:34 -> 16:10:10, clippy/rustdoc most of
`check`'s seven minutes, and no crate reads anything under scripts/.

THE SET IS WHAT CARGO READS, checked against the tree rather than guessed: the workspace
(`crates/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`,
`.cargo/`), `vendor/` (ds2-hook's build.rs compiles `vendor/minhook/src`), and `.github/` (three
crates `include_str!` `.github/dist-ds2-mods.toml`, release.yml packages the other `dist-*`
files, and a workflow change must prove itself on every leg). `scripts/check.sh` and this file
are in it too: a change to the gate or to the decision runs the whole gate.

IT FAILS TOWARD RUNNING EVERYTHING. Any event but `pull_request` (a push to main, a dispatch) is
`rust=true` without looking, so every main commit keeps a full verdict. A base that git cannot
diff against is `rust=true` too: a skip needs a positive answer, never a missing one.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

RUST_INPUT_PREFIXES = ("crates/", "vendor/", ".cargo/", ".github/")
RUST_INPUT_FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "clippy.toml",
    "rustfmt.toml",
    "scripts/check.sh",
    "scripts/ci-rust-inputs.py",
)


def rust_inputs(paths: list[str]) -> list[str]:
    """The changed paths cargo or the Rust legs read."""
    return [p for p in paths if p in RUST_INPUT_FILES or p.startswith(RUST_INPUT_PREFIXES)]


def gate_legs(rust: bool) -> list[dict[str, str]]:
    """gate.yml's matrix: both legs, or `check` alone with its cargo steps skipped."""
    if rust:
        return [{"leg": "check", "args": ""}, {"leg": "host-tests", "args": "--host-tests"}]
    return [{"leg": "check", "args": "--no-rust"}]


def decide(event: str, base: str | None, head: str) -> tuple[bool, str]:
    """(run the Rust legs, why)."""
    if event != "pull_request":
        return True, f"event {event!r} always runs every leg"
    if not base:
        return True, "no base to diff against"
    run = subprocess.run(
        ["git", "diff", "--name-only", f"{base}...{head}"], capture_output=True, text=True
    )
    if run.returncode:
        return True, f"git diff {base}...{head} failed: {run.stderr.strip()}"
    paths = [p for p in run.stdout.splitlines() if p]
    hits = rust_inputs(paths)
    if hits:
        shown = ", ".join(hits[:5]) + (f" and {len(hits) - 5} more" if len(hits) > 5 else "")
        return True, f"Rust inputs changed: {shown}"
    return False, f"no Rust input among the {len(paths)} changed path(s)"


def selftest() -> int:
    cases = [
        (["scripts/ds2-msb.py", "docs/X.md"], []),
        (["crates/ds2-loader/src/lib.rs"], ["crates/ds2-loader/src/lib.rs"]),
        (["Cargo.lock", "README.md"], ["Cargo.lock"]),
        ([".github/dist-ds2-mods.toml"], [".github/dist-ds2-mods.toml"]),
        ([".github/workflows/gate.yml"], [".github/workflows/gate.yml"]),
        (["vendor/minhook/src/hook.c"], ["vendor/minhook/src/hook.c"]),
        (["scripts/check.sh"], ["scripts/check.sh"]),
        (["scripts/ci-rust-inputs.py"], ["scripts/ci-rust-inputs.py"]),
        ([".cupcake/policies/claude/x.rego"], []),
        (["cratesX/a.rs", "data/a.tsv"], []),
        ([], []),
    ]
    failed = 0
    for paths, want in cases:
        got = rust_inputs(paths)
        if got != want:
            print(f"FAIL rust_inputs({paths}) = {got}, want {want}")
            failed += 1
    for event in ("push", "workflow_dispatch"):
        if decide(event, "anything", "HEAD")[0] is not True:
            print(f"FAIL {event} must run every leg")
            failed += 1
    if decide("pull_request", None, "HEAD")[0] is not True:
        print("FAIL a pull request with no base must run every leg")
        failed += 1
    if decide("pull_request", "no-such-ref-for-the-selftest", "HEAD")[0] is not True:
        print("FAIL an undiffable base must run every leg")
        failed += 1
    if [leg["leg"] for leg in gate_legs(True)] != ["check", "host-tests"]:
        print("FAIL a Rust change must run both legs")
        failed += 1
    if gate_legs(False) != [{"leg": "check", "args": "--no-rust"}]:
        print("FAIL a change with no Rust input runs check alone, with --no-rust")
        failed += 1
    total = len(cases) + 6
    print(f"ci-rust-inputs selftest: {total - failed}/{total} pass")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--event", default="pull_request", help="github.event_name")
    ap.add_argument("--base", help="the pull request's base commit")
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    rust, why = decide(a.event, a.base, a.head)
    legs = json.dumps(gate_legs(rust), separators=(",", ":"))
    print(f"rust={'true' if rust else 'false'}")
    print(f"reason={why}")
    print(f"legs={legs}")
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a", encoding="utf-8") as f:
            f.write(f"rust={'true' if rust else 'false'}\nreason={why}\nlegs={legs}\n")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            said = "every leg runs" if rust else "host-tests and the release build are SKIPPED; check runs with --no-rust"
            f.write(f"### Rust inputs: {'changed' if rust else 'unchanged'}\n\n{why}. {said}.\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
