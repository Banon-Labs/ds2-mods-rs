#!/usr/bin/env python3
"""Live-eval regression tests for the repo-local Cupcake policies.

`opa test .cupcake/tests/*_test.rego` (run by scripts/check.sh) proves the RULES are right in the
OPA interpreter. It does not prove that the deployed pipeline -- cupcake's WASM build, the harness
event shape, the signal wiring in .cupcake/rulebook.yml -- reaches the same verdict. This file
closes that gap by driving the real `cupcake eval` binary with real PreToolUse events, which is
exactly what the global PreToolUse hook does on every tool call.

WHY THIS IS A FILE AND NOT A SHELL ONE-LINER
Several of the deny cases below must contain the literal text of a command the policies exist to
block (`git push origin main`, `--no-verify`). Typing that into an agent Bash command is itself
intercepted -- correctly. Measured while porting these policies on 2026-08-26: an attempt to build
the same fixtures inline was denied with "This command wraps a shell payload the guard cannot read
(an unquoted or substituted `-c`/`eval` argument) while naming git and push, so it cannot be shown
not to push to main." That is the guard working, not a false positive. A committed file on disk is
never an agent Bash command, so it is the sanctioned place for this text -- the same reasoning the
sibling repo's pgrep policy uses to sanction `scripts/steam-running.sh`.

Signals are pinned per case through the CUPCAKE_*_OVERRIDE env vars that .cupcake/rulebook.yml
reads, so a case that models "on main" does not depend on which branch the checkout is actually on.
"""
from __future__ import annotations

import json
import os
import subprocess
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# Claude Code sends a `timeout` alongside every Bash command; include it so the event shape
# matches production rather than a stripped-down approximation.
DEFAULT_BASH_TIMEOUT_MS = 30000

# Two identical OIDs mean "origin/main is fresh". The fresh-origin-main guard fails CLOSED, so
# every case that is not specifically testing staleness must pin a matching pair or it will deny
# for the wrong reason and the test will pass while proving nothing.
FRESH_OIDS = "a" * 40 + " " + "a" * 40
STALE_OIDS = "a" * 40 + " " + "b" * 40


@dataclass(frozen=True)
class PolicyCase:
    name: str
    should_allow: bool
    command: str = ""
    tool_name: str = "Bash"
    expected_text: str | None = None
    tool_input: dict[str, object] = field(default_factory=dict)
    current_branch: str = "cupcake-policies"
    origin_main_oids: str = FRESH_OIDS


def run_case(case: PolicyCase) -> None:
    if case.tool_name == "Bash":
        tool_input: dict[str, object] = {
            "command": case.command,
            "timeout": DEFAULT_BASH_TIMEOUT_MS,
        }
    else:
        tool_input = {}
    tool_input.update(case.tool_input)

    event = {
        "session_id": f"cupcake-policy-regression-{case.name}",
        "transcript_path": f"/tmp/cupcake-policy-regression-{case.name}.jsonl",
        "cwd": str(REPO_ROOT),
        "hook_event_name": "PreToolUse",
        "tool_name": case.tool_name,
        "tool_input": tool_input,
        "permission_mode": "default",
    }

    env = {
        **os.environ,
        "CUPCAKE_CURRENT_BRANCH_OVERRIDE": case.current_branch,
        "CUPCAKE_WORKTREE_BRANCHES_OVERRIDE": "",
        "CUPCAKE_ORIGIN_MAIN_OIDS_OVERRIDE": case.origin_main_oids,
    }

    result = subprocess.run(
        ["cupcake", "eval", "--harness", "claude", "--strict", "--log-level", "error"],
        cwd=REPO_ROOT,
        input=json.dumps(event),
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
        env=env,
    )
    output = result.stdout + result.stderr
    allowed = result.returncode == 0
    if allowed != case.should_allow:
        raise AssertionError(
            f"{case.name}: expected allow={case.should_allow}, "
            f"got returncode={result.returncode}\n{output}"
        )
    if case.expected_text and case.expected_text not in output:
        raise AssertionError(f"{case.name}: missing {case.expected_text!r}\n{output}")


def main() -> int:
    cases = [
        # --- control: an ordinary command must survive the whole policy set -------------------
        PolicyCase("allow-cargo-build", True, "cargo build --workspace"),
        PolicyCase("allow-ds2-run-dry", True, "python3 scripts/ds2-run.py --dry-run"),
        # --- git_block_main_commit -------------------------------------------------------------
        PolicyCase(
            "deny-commit-on-main",
            False,
            'git commit -m "wip"',
            current_branch="main",
        ),
        PolicyCase(
            "allow-commit-on-feature-branch",
            True,
            'git commit -m "wip"',
        ),
        # --- git_block_main_push ---------------------------------------------------------------
        PolicyCase("deny-push-origin-main", False, "git push origin main"),
        PolicyCase(
            "deny-push-head-while-on-main",
            False,
            "git push origin HEAD",
            current_branch="main",
        ),
        PolicyCase("allow-push-feature-branch", True, "git push origin cupcake-policies"),
        # --- git_require_fresh_origin_main -----------------------------------------------------
        # Force-pushing a PR branch is allowed only when origin/main was just fetched. The two
        # cases differ ONLY in the pinned OIDs, so a pass here is attributable to that guard and
        # not to some other rule catching the command first.
        PolicyCase(
            "deny-force-push-with-stale-origin-main",
            False,
            "git push --force-with-lease origin cupcake-policies",
            origin_main_oids=STALE_OIDS,
        ),
        PolicyCase(
            "allow-force-push-with-fresh-origin-main",
            True,
            "git push --force-with-lease origin cupcake-policies",
        ),
        # --- git_block_no_verify (builtin) -----------------------------------------------------
        PolicyCase("deny-commit-no-verify", False, 'git commit --no-verify -m "wip"'),
        # --- protected_paths (builtin) ---------------------------------------------------------
        # Read allowed, write blocked -- that asymmetry is the whole point of this builtin, so
        # both halves are asserted.
        PolicyCase("allow-read-protected-path", True, "cat /etc/hostname"),
        PolicyCase("deny-write-protected-path", False, "rm -f /etc/hostname"),
        # --- edit_no_tmp_scripts_guard ---------------------------------------------------------
        # Scripts belong in the repo where they are reviewable and survive the session; DATA in
        # /tmp is fine and intended. The .json case guards against the rule over-reaching into
        # artifacts, which is the failure mode that would make agents fight the guard.
        PolicyCase(
            "deny-write-python-script-to-tmp",
            False,
            tool_name="Write",
            tool_input={"file_path": "/tmp/helper.py", "content": "print(1)\n"},
        ),
        PolicyCase(
            "allow-write-json-artifact-to-tmp",
            True,
            tool_name="Write",
            tool_input={"file_path": "/tmp/measurements.json", "content": "{}\n"},
        ),
        PolicyCase(
            "allow-write-python-script-to-repo",
            True,
            tool_name="Write",
            tool_input={
                "file_path": str(REPO_ROOT / "scripts" / "example.py"),
                "content": "print(1)\n",
            },
        ),
    ]

    max_workers = min(8, max(1, len(cases)))
    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        futures = {pool.submit(run_case, case): case for case in cases}
        for future in as_completed(futures):
            future.result()
    print(f"cupcake live-eval regression tests passed ({len(cases)} cases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
