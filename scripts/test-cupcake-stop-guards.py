#!/usr/bin/env python3
"""End-to-end proof that the Stop-event guard actually halts a turn.

WHY THIS TEST EXISTS. In the sibling repo (er-mods-rs) every Stop guard was inert for 36 days -- from
the day the first one landed until the day this was written -- and the suite stayed green the entire
time, because the coverage was split in a way that left the real path untested:

  * scripts/test-*-signal.py           tested the SIGNAL shell scripts alone (no policy, no cupcake);
  * .cupcake/tests/*_test.rego         tested the POLICIES in the OPA INTERPRETER (`opa test`);
  * scripts/test-cupcake-policies.py   ran the real `cupcake eval` binary -- PreToolUse events ONLY.

`cupcake eval` does not use the OPA interpreter. It compiles the policies to WASM and runs them in
its own embedded runtime, where an unimplemented host builtin (`sprintf`) silently yields undefined
and the rule never fires. So the signal passed, the policy passed, the interpreter passed, and the
thing that actually runs at turn-end returned `{}` -- a clean ALLOW -- every single time.

This repo's first Stop guard is `no_unexecuted_promise`, and it lands with that hole already closed:
this file drives the WHOLE path exactly as Claude Code does -- the real transcript on disk, the real
signal script, the real hook command read out of .claude/settings.json, and an assertion on the
verdict that comes back. Both directions are asserted -- a guard that cannot halt is useless, and a
guard that halts on a clean turn wedges every session.

Fixtures live in .cupcake/tests/fixtures/*.jsonl and are ordinary Claude Code transcripts.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
FIXTURES = REPO_ROOT / ".cupcake" / "tests" / "fixtures"
SETTINGS = REPO_ROOT / ".claude" / "settings.json"


@dataclass(frozen=True)
class Case:
    fixture: str
    # Distinctive substring of the expected halt reason, or None when the turn must be allowed.
    # Matched on the reason rather than the rule_id because cupcake renders a lone decision as a
    # bare reason string with no [rule_id] prefix.
    expect_halt_text: str | None
    why: str


CASES = [
    Case(
        "unexecuted_promise.jsonl",
        "promise nothing is going to keep",
        "turn ends on 'I'll re-run the gate...' with no tool call, no background work, no handoff",
    ),
    Case(
        "clean.jsonl",
        None,
        "substantive work, no banned prose -- must NOT halt, or every turn wedges",
    ),
]


def hook_command(event: str) -> list[str]:
    """The hook command Claude Code actually runs for `event`, read from settings.json so this test
    follows the real configuration instead of a copy that can drift away from it."""
    settings = json.loads(SETTINGS.read_text(encoding="utf-8"))
    for group in settings.get("hooks", {}).get(event, []):
        for hook in group.get("hooks", []):
            cmd = hook.get("command", "")
            if "cupcake" in cmd:
                return cmd.replace("$CLAUDE_PROJECT_DIR", str(REPO_ROOT)).split()
    raise SystemExit(
        f"test-cupcake-stop-guards: no cupcake {event} hook found in .claude/settings.json"
    )


def run_hook(fixture_name: str, event_name: str, argv: list[str]) -> tuple[dict, str] | str:
    """Drive one fixture through a real cupcake hook invocation. Returns (decision, raw stdout), or
    a failure message string."""
    fixture = FIXTURES / fixture_name
    if not fixture.is_file():
        return f"missing fixture {fixture}"

    with tempfile.TemporaryDirectory(prefix="cupcake-stop-guard-") as tmp:
        # Signals discover the transcript via ~/.claude/projects/<cwd-with-slashes-as-dashes>/*.jsonl
        # (scripts/cupcake_turn_scan.latest_transcript). Point HOME at a throwaway tree holding only
        # this fixture, so the test never reads the live session transcript.
        slug = str(REPO_ROOT).replace("/", "-")
        tdir = Path(tmp) / ".claude" / "projects" / slug
        tdir.mkdir(parents=True)
        shutil.copy(fixture, tdir / "session.jsonl")

        env = {**os.environ, "HOME": tmp, "CLAUDE_PROJECT_DIR": str(REPO_ROOT)}
        payload = {
            "session_id": f"stop-guard-{fixture_name}",
            "transcript_path": str(tdir / "session.jsonl"),
            "cwd": str(REPO_ROOT),
            "hook_event_name": event_name,
            "stop_hook_active": False,
        }
        proc = subprocess.run(
            argv,
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            env=env,
            cwd=REPO_ROOT,
            timeout=25,
        )

    raw = proc.stdout.strip()
    try:
        return (json.loads(raw) if raw else {}), raw
    except ValueError:
        return f"unparseable cupcake output: {raw[:200]!r}"


def run_case(case: Case, argv: list[str]) -> str | None:
    """Returns None on pass, or a failure message."""
    outcome = run_hook(case.fixture, "Stop", argv)
    if isinstance(outcome, str):
        return outcome
    decision, raw = outcome

    reason = decision.get("reason", "")
    blocked = decision.get("decision") == "block"

    if case.expect_halt_text is None:
        if blocked:
            return f"expected NO halt (clean turn) but cupcake blocked: {reason[:160]!r}"
        return None

    if not blocked:
        return (
            f"expected a HALT but cupcake returned {raw or '{}'!r}.\n"
            f"      The guard is INERT -- the exact failure described in this file's docstring. Check\n"
            f"      that no rule in its path uses a builtin Cupcake's WASM runtime cannot execute\n"
            f"      (run: python3 scripts/check-cupcake-wasm-builtins.py)."
        )
    if case.expect_halt_text not in reason:
        return f"halted, but reason lacked {case.expect_halt_text!r}: {reason[:200]!r}"
    return None


def main() -> int:
    if shutil.which("cupcake") is None:
        print("test-cupcake-stop-guards: SKIP (cupcake not installed)")
        return 0

    stop_argv = hook_command("Stop")
    failures = 0
    for case in CASES:
        err = run_case(case, stop_argv)
        verdict = "halt" if case.expect_halt_text else "allow"
        if err:
            failures += 1
            print(f"FAIL [{verdict}] {case.fixture}: {case.why}\n      {err}", file=sys.stderr)
        else:
            print(f"ok   [{verdict}] {case.fixture}: {case.why}")

    if failures:
        print(f"\ntest-cupcake-stop-guards: {failures} failure(s)", file=sys.stderr)
        return 1
    print(
        f"test-cupcake-stop-guards: OK ({len(CASES)} Stop cases, through the real hook command)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
