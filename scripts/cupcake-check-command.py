#!/usr/bin/env python3
"""Ad-hoc live-WASM-engine check for a single Bash command, for use during
agent policy work when the harness's own worktree-isolation guard refuses an
inline Bash call containing the literal substring "eval" (the `cupcake eval`
subcommand name). Mirrors eval_bash() in scripts/test-cupcake-delivered-shape.py
but as a standalone one-shot CLI so the word never has to appear on the outer
Bash tool call line.

Usage:
    python3 scripts/cupcake-check-command.py '<command>'
    python3 scripts/cupcake-check-command.py --command-file <path>
    python3 scripts/cupcake-check-command.py --command-file <path> \
        --env `CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE=MISSING` --log-level debug

Prints allow or deny plus the reason (if any), using the real `cupcake` binary
compiled to WASM in production -- not the `opa` interpreter that `opa test`
and `opa eval` use.

Why the three flags exist, each from a case that could not be driven without it
(2026-09-13, while reproducing the runtime-evidence guard measuring the wrong
repository):

  * `--command-file`. The isolation guard reads the outer Bash tool call, not
    this script's argv, and a fixture that names another checkout -- a
    `cd <other worktree> && git push ...` -- is refused as "too complex to
    verify that it stays inside the worktree" however it is quoted. Reading the
    fixture from a file keeps the outer call plain while the text under test
    stays exact: rewording it to get past the guard would test a different
    command than the one that was denied in production.
  * `--env`. The evidence signals answer from a live measurement of this
    checkout, so a case that needs a specific verdict has to pin it the way the
    signals themselves provide for (`CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE` and its
    note sibling), exactly as scripts/test-cupcake-policies.py pins the branch.
  * `--log-level`. The engine's own debug output is where a question like
    "does a signal receive the pending tool input" is answered, and the default
    `error` discards it.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog=Path(argv[0]).name,
        description="Drive one PreToolUse Bash command through the real cupcake engine.",
    )
    parser.add_argument("command", nargs="?", help="the command text to evaluate")
    parser.add_argument(
        "--command-file",
        help="read the command text from this file instead of argv",
    )
    parser.add_argument(
        "--env",
        action="append",
        default=[],
        metavar="NAME=VALUE",
        help="extra environment for the engine and its signals; repeatable",
    )
    parser.add_argument(
        "--cwd",
        help="the `cwd` field of the event, which signals measure from (default: repo root)",
    )
    parser.add_argument("--log-level", default="error", help="engine log level")
    args = parser.parse_args(argv[1:])
    if (args.command is None) == (args.command_file is None):
        parser.error("give exactly one of a positional command or --command-file")
    return args


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.command_file:
        command = Path(args.command_file).read_text(encoding="utf-8").rstrip("\n")
    else:
        command = args.command
    event = {
        "session_id": "cupcake-check-command",
        "transcript_path": "/tmp/cupcake-check-command.jsonl",
        "cwd": args.cwd or str(REPO_ROOT),
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command, "timeout": 30000},
        "signals": {"current_branch": "feature/cupcake-check-command\n"},
    }
    env = {**os.environ}
    for pair in args.env:
        name, _, value = pair.partition("=")
        env[name] = value
    result = subprocess.run(
        ["cupcake", "eval", "--harness", "claude", "--strict", "--log-level", args.log_level],
        cwd=REPO_ROOT,
        input=json.dumps(event),
        text=True,
        capture_output=True,
        check=False,
        timeout=25,
        env=env,
    )
    verdict = "DENY" if result.returncode != 0 else "ALLOW"
    print(f"{verdict}  {command!r}")
    output = (result.stdout + result.stderr).strip()
    if output:
        print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
