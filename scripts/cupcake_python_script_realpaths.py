#!/usr/bin/env python3
"""Where each `.py` path in a pending Bash command really points, symlinks followed.

Read by `.cupcake/signals/python_script_realpaths.sh` for DS2-MODS-BASH-NO-PYTHON-FILE-WRITE
(`.cupcake/policies/claude/bash_no_python_file_write.rego`).

# The miss this closes (2026-09-27)

    python3 /home/banon/DS2/ds2-run.py

was DENIED as a python file write. `/home/banon/DS2/ds2-run.py` is a symlink to this repo's
committed `scripts/ds2-run.py`, and the refusal text itself says a committed script is allowed "BY
ANY SPELLING THAT RESOLVES THERE". The policy only compares path TEXT against the repo root, and Rego
cannot stat the filesystem, so a symlink never resolved anywhere.

The same blindness ran the other way: `scripts/evil.py -> /tmp/patch.py` has the spelling of a
committed script and was exempt by that spelling alone, while python runs the file in /tmp.

# What it emits

    path\t<key>\t<realpath>     one per `.py` operand that names an existing file
    root\t<realpath>            the real location of the event cwd and of this checkout

The key is the path token as the policy extracts it (leading quotes removed), so the policy looks
the target up by the text it already has. `~/` is expanded against $HOME, a relative path against
the event's cwd. Anything with `$` or a backtick in it, and anything that is not an existing file,
emits nothing. The root lines exist so a checkout that sits behind a symlinked directory still
matches its own scripts' real paths.

The policy decides what these targets may vouch for; this module only answers "what file is that".
"""
from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path

# The same shape the policy's script_file_pattern captures after the interpreter: a token that does
# not start with `-`, runs to `.py`, and is followed by a non-path character or the end. Any
# whitespace-delimited token is considered, not only the one right after `python`; the policy only
# looks up the keys it extracted itself, so an extra line here is inert.
PY_TOKEN = re.compile(r"(?:^|(?<=\s))([^\s-][^\s]*\.py)(?=$|[^A-Za-z0-9_.-])")

REPO_ROOT = Path(__file__).resolve().parents[1]


def resolve(key: str, cwd: str) -> str:
    if not key or "$" in key or "`" in key or "\t" in key or "\n" in key:
        return ""
    path = os.path.expanduser(key) if key.startswith("~/") else key
    if not os.path.isabs(path):
        if not os.path.isabs(cwd):
            return ""
        path = os.path.join(cwd, path)
    if not os.path.isfile(path):
        return ""
    return os.path.realpath(path)


def render(command: str, cwd: str) -> str:
    lines: list[str] = []
    seen: set[str] = set()
    for m in PY_TOKEN.finditer(command):
        key = m.group(1).lstrip("\"'")
        if key in seen:
            continue
        seen.add(key)
        real = resolve(key, cwd)
        if real:
            lines.append(f"path\t{key}\t{real}")
    if not lines:
        return ""
    roots = {str(REPO_ROOT)}
    if os.path.isabs(cwd) and os.path.isdir(cwd):
        roots.add(os.path.realpath(cwd))
    lines.extend(f"root\t{root}" for root in sorted(roots))
    return "\n".join(lines)


def main() -> int:
    try:
        event = json.load(sys.stdin)
    except ValueError:
        return 0
    if not isinstance(event, dict):
        return 0
    ti = event.get("tool_input")
    cmd = ti.get("command") if isinstance(ti, dict) else None
    cwd = event.get("cwd")
    if isinstance(cmd, str):
        text = render(cmd, cwd if isinstance(cwd, str) else "")
        if text:
            print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
