#!/usr/bin/env python3
"""Show how `.cupcake/system/commands.rego` decomposes a command, for debugging a guard verdict.

A guard denial names the RULE that fired, never the text the rule matched against. When a denial
looks wrong, the question is always "what did the helper actually hand the guard?", and the answer
is a `commands.executed_texts` / `executed_unquoted_texts` value nothing prints. This prints it.

    python3 scripts/explain-cupcake-text.py <file-containing-the-command>
    python3 scripts/explain-cupcake-text.py --case <name-from-test-cupcake-policies.py>

The command is read from a FILE and never from argv, for the same reason the fixtures in
`scripts/test-cupcake-policies.py` live in a file: a command that reproduces a guard denial
necessarily contains the text that guard blocks, so typing it into an agent Bash command is itself
intercepted -- correctly. Measured 2026-08-26 while debugging ds2-mods-rs-1tc: two attempts to probe
this inline were denied, once by the launch guard and once by the main-push guard.

Prints, for each public view, the text with newlines made visible, so a heredoc body that survived
blanking is obvious rather than something to be inferred.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
COMMANDS_REGO = REPO_ROOT / ".cupcake/system/commands.rego"

#: The public entry points a policy is allowed to scan, plus the two decisions that most often
#: explain a surprising verdict.
EXPRESSIONS = (
    "executed_texts(input.cmd)",
    "executed_unquoted_texts(input.cmd)",
    "quotes_removed(input.cmd)",
    "heredoc_resolved(input.cmd)",
    "unparsed_shell_payload(input.cmd)",
    "git_commit_message_only(input.cmd)",
)


def evaluate(expression: str, command: str) -> str:
    """One `opa eval` against the real helper, with the command supplied as input."""
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump({"cmd": command}, handle)
        input_path = handle.name
    try:
        result = subprocess.run(
            [
                "opa",
                "eval",
                "-d",
                str(COMMANDS_REGO),
                "-i",
                input_path,
                "-f",
                "pretty",
                f"data.cupcake.system.commands.{expression}",
            ],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
    finally:
        Path(input_path).unlink(missing_ok=True)
    return (result.stdout or result.stderr).strip()


def command_from_case(name: str) -> str:
    """The command text of a named case in `scripts/test-cupcake-policies.py`.

    Reading it from there rather than retyping it is the whole point: a fixture that reproduces a
    denial contains the text the guard blocks, so retyping it into an agent Bash command -- even to
    write it to a file -- is itself denied. That happened twice while debugging ds2-mods-rs-1tc.
    """
    # The module name has hyphens, so it cannot be imported by name.
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "cupcake_cases", REPO_ROOT / "scripts" / "test-cupcake-policies.py"
    )
    module = importlib.util.module_from_spec(spec)
    # Registered before exec: dataclasses resolves annotations through sys.modules, and a module
    # that is not there yet fails on the first @dataclass.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    for case in module.cases():
        if case.name == name:
            return case.command
    raise SystemExit(f"no case named {name!r}")


def main() -> int:
    if len(sys.argv) == 3 and sys.argv[1] == "--case":
        command = command_from_case(sys.argv[2])
        print(f"=== case {sys.argv[2]} ===")
        for expression in EXPRESSIONS:
            print(f"\n=== {expression} ===")
            print(evaluate(expression, command).replace("\n", "\\n"))
        return 0
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    command = Path(sys.argv[1]).read_text(encoding="utf-8")
    # A trailing newline from the file is not part of the command the tool would send.
    command = command.rstrip("\n")

    print("=== command, newlines shown as \\n ===")
    print(command.replace("\n", "\\n"))
    for expression in EXPRESSIONS:
        print(f"\n=== {expression} ===")
        print(evaluate(expression, command).replace("\n", "\\n"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
