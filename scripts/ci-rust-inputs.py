#!/usr/bin/env python3
"""Decide which CI legs a change needs: did it touch a Rust input, a cupcake policy input, neither?

    python3 scripts/ci-rust-inputs.py --event pull_request --base <sha>   # CI, writes $GITHUB_OUTPUT
    python3 scripts/ci-rust-inputs.py --event pull_request --base origin/main
    python3 scripts/ci-rust-inputs.py --selftest

WHAT IT DECIDES. `.github/workflows/gate.yml` runs the `host-tests` leg, and `release.yml` its
DLL build, only when this prints `rust=true`. The `check` leg always runs; it passes check.sh
`--no-rust` when `rust=false`, and `--no-policy` as well when `policy=false`. So a pull request
that changes only docs gets commit messages, the PR template and the script selftests, and not the
~20 runner-minutes of cargo or the cupcake suites, neither of which can see it. MEASURED
2026-11-02: on PR #364 (scripts only) `host-tests` ran 15:57:34 -> 16:10:10 and clippy/rustdoc
took most of `check`'s seven minutes; on PR #366 (docs only) `check --no-rust` still took 5m40s,
189 s of it (local) the cupcake section.

RUST INPUTS ARE WHAT CARGO READS, checked against the tree rather than guessed: the workspace
(`crates/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`,
`.cargo/`), `vendor/` (ds2-hook's build.rs compiles `vendor/minhook/src`), and `.github/` (three
crates `include_str!` `.github/dist-ds2-mods.toml`, release.yml packages the other `dist-*`
files, and a workflow change must prove itself on every leg). No crate reads `scripts/`.

POLICY INPUTS ARE WHAT THE CUPCAKE SECTION TESTS: everything under `.cupcake/`, the scripts named
by the section's own tests (`scripts/cupcake*`, `scripts/test-*`, `scripts/check-cupcake-*`,
`scripts/monitor-throttle.py`), and every `scripts/...` path that a file under `.cupcake/`
mentions, read from the tree at decision time, so a signal that starts calling a new script is
covered without an edit here.

`scripts/check.sh` and this file are inputs to both: a change to the gate or to the decision runs
the whole gate.

IT FAILS TOWARD RUNNING EVERYTHING. Any event but `pull_request` (a push to main, a dispatch) runs
every leg without looking, so every main commit keeps a full verdict. A base that git cannot diff
against runs every leg too: a skip needs a positive answer, never a missing one.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GATE_FILES = ("scripts/check.sh", "scripts/ci-rust-inputs.py")
RUST_INPUT_PREFIXES = ("crates/", "vendor/", ".cargo/", ".github/")
RUST_INPUT_FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "clippy.toml",
    "rustfmt.toml",
    *GATE_FILES,
)
POLICY_INPUT_PREFIXES = (
    ".cupcake/",
    "scripts/cupcake",
    "scripts/test-",
    "scripts/check-cupcake-",
)
POLICY_INPUT_FILES = ("scripts/monitor-throttle.py", *GATE_FILES)


def rust_inputs(paths: list[str]) -> list[str]:
    """The changed paths cargo or the Rust legs read."""
    return [p for p in paths if p in RUST_INPUT_FILES or p.startswith(RUST_INPUT_PREFIXES)]


def scripts_named_by_policies(root: Path = ROOT) -> set[str]:
    """Every `scripts/<file>` a file under `.cupcake/` mentions."""
    named: set[str] = set()
    for path in (root / ".cupcake").rglob("*"):
        if path.is_file():
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            named.update(re.findall(r"scripts/[A-Za-z0-9_.-]+", text))
    return named


def policy_inputs(paths: list[str], named: set[str]) -> list[str]:
    """The changed paths the cupcake section's suites test."""
    return [
        p
        for p in paths
        if p in POLICY_INPUT_FILES or p in named or p.startswith(POLICY_INPUT_PREFIXES)
    ]


def check_args(rust: bool, policy: bool) -> str:
    """gate.yml's `check` job's check.sh arguments: what the change cannot reach is skipped."""
    if rust:
        return ""
    return "--no-rust" if policy else "--no-rust --no-policy"


def shown(hits: list[str]) -> str:
    return ", ".join(hits[:5]) + (f" and {len(hits) - 5} more" if len(hits) > 5 else "")


def decide(event: str, base: str | None, head: str) -> tuple[bool, bool, str]:
    """(run the Rust legs, run the policy suites, why)."""
    if event != "pull_request":
        return True, True, f"event {event!r} always runs every leg"
    if not base:
        return True, True, "no base to diff against"
    run = subprocess.run(
        ["git", "diff", "--name-only", f"{base}...{head}"], capture_output=True, text=True
    )
    if run.returncode:
        return True, True, f"git diff {base}...{head} failed: {run.stderr.strip()}"
    paths = [p for p in run.stdout.splitlines() if p]
    rust = rust_inputs(paths)
    policy = policy_inputs(paths, scripts_named_by_policies())
    why = [f"Rust inputs changed: {shown(rust)}" if rust else "no Rust input changed"]
    why.append(f"policy inputs changed: {shown(policy)}" if policy else "no policy input changed")
    return bool(rust), bool(policy), f"{'; '.join(why)} ({len(paths)} path(s) in the diff)"


def selftest() -> int:
    failed = 0

    def expect(what: str, got, want) -> None:
        nonlocal failed
        if got != want:
            print(f"FAIL {what}: got {got!r}, want {want!r}")
            failed += 1

    rust_cases = [
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
    for paths, want in rust_cases:
        expect(f"rust_inputs({paths})", rust_inputs(paths), want)
    named = {"scripts/ds2-frida-evidence.py"}
    policy_cases = [
        (["docs/X.md", "README.md"], []),
        (["scripts/ds2-msb.py"], []),
        ([".cupcake/policies/claude/x.rego"], [".cupcake/policies/claude/x.rego"]),
        (["scripts/cupcake_game_alive.py"], ["scripts/cupcake_game_alive.py"]),
        (["scripts/test-run-stamp.py"], ["scripts/test-run-stamp.py"]),
        (["scripts/ds2-frida-evidence.py"], ["scripts/ds2-frida-evidence.py"]),
        (["scripts/monitor-throttle.py"], ["scripts/monitor-throttle.py"]),
        (["scripts/check.sh"], ["scripts/check.sh"]),
        (["crates/x/src/lib.rs"], []),
    ]
    for paths, want in policy_cases:
        expect(f"policy_inputs({paths})", policy_inputs(paths, named), want)
    # The tree's own policies name scripts, so the set read from it is not empty.
    expect("scripts named by .cupcake/ is non-empty", bool(scripts_named_by_policies()), True)
    for event in ("push", "workflow_dispatch"):
        expect(f"{event} runs every leg", decide(event, "anything", "HEAD")[:2], (True, True))
    expect("no base runs every leg", decide("pull_request", None, "HEAD")[:2], (True, True))
    expect(
        "an undiffable base runs every leg",
        decide("pull_request", "no-such-ref-for-the-selftest", "HEAD")[:2],
        (True, True),
    )
    expect("Rust: check runs in full", check_args(True, False), "")
    expect("policy only", check_args(False, True), "--no-rust")
    expect("neither", check_args(False, False), "--no-rust --no-policy")
    total = len(rust_cases) + len(policy_cases) + 8
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
    rust, policy, why = decide(a.event, a.base, a.head)
    args = check_args(rust, policy)
    lines = [
        f"rust={'true' if rust else 'false'}",
        f"policy={'true' if policy else 'false'}",
        f"reason={why}",
        f"check_args={args}",
    ]
    print("\n".join(lines))
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        if rust:
            said = "every leg runs"
        else:
            said = f"host-tests and the release build are SKIPPED; check runs {args}"
        with open(summary, "a", encoding="utf-8") as f:
            f.write(f"### Which legs this change needs\n\n{why}. {said}.\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
