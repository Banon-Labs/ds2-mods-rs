#!/usr/bin/env python3
"""Which path tokens in a pending Bash command name a TRACKED file under this repo's `scripts/`.

Read by `.cupcake/signals/tmp_copy_sources.sh` for DS2-MODS-NO-TMP-SCRIPTS-GUARD
(`.cupcake/policies/claude/edit_no_tmp_scripts_guard.rego`).

# The miss this closes (2026-11-02)

    cp scripts/frida/summon-sign-trace.js $SCRATCHPAD/summon-sign-trace-frozen.js

with the scratchpad spelled out literally was DENIED as "authoring a script into /tmp". Nothing was
authored: the source is a committed repo script, copied so a running `scripts/ds2-frida-watch.py`
reloads it. The guard's own text asks for exactly that shape (script in the repo, copies and
artifacts in /tmp). Rego cannot ask git whether a file is tracked, so the copy carve-out needs this.

# What it emits

    tracked\t<token>     one per token that resolves to a tracked file under <worktree>/scripts/

The token is spelled as the policy extracts it: a run of characters between whitespace and the
shell separators `;&|<>`, with every quote character removed. `~/` is expanded against $HOME,
`$CLAUDE_PROJECT_DIR` against the environment, a relative path against the event cwd -- but only
when the command contains no `cd`/`pushd` to anywhere else, since that moves what a relative path
names. Symlinks are followed, and the REAL file must be the tracked one, so `scripts/x.js ->
/tmp/x.js` vouches for nothing. Any other `$`, a backtick, or a path that is not an existing
regular file emits nothing.

Failure direction: silence. A crash or an empty answer removes the carve-out and the copy is denied
exactly as it was before this helper existed.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys

REPO_ROOT = os.path.realpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

TOKEN_SPLIT = re.compile(r"[\s;&|<>]+")
# A `cd`/`pushd` command word and its operand, if any. A bare `cd` goes to $HOME.
CD_PATTERN = re.compile(r"(?:^|[\s;&|(])(?:cd|pushd)(?:\s+([^\s;&|<>)]+))?(?=$|[\s;&|)])")
PROJECT_VARS = ("$CLAUDE_PROJECT_DIR/", "${CLAUDE_PROJECT_DIR}/")


def git(cwd: str, *args: str) -> str | None:
    try:
        result = subprocess.run(
            ["git", "-C", cwd, *args],
            capture_output=True,
            text=True,
            timeout=2,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def relative_paths_resolve_against_cwd(command: str, cwd: str) -> bool:
    for m in CD_PATTERN.finditer(command):
        target = (m.group(1) or "").replace('"', "").replace("'", "")
        if not target:
            return False
        if target.startswith("~/") or target == "~":
            target = os.path.expanduser(target)
        if not os.path.isabs(target) or os.path.realpath(target) != os.path.realpath(cwd):
            return False
    return True


def resolve(token: str, cwd: str, relative_ok: bool) -> str:
    path = token
    for var in PROJECT_VARS:
        if path.startswith(var):
            project = os.environ.get("CLAUDE_PROJECT_DIR", "")
            if not os.path.isabs(project):
                return ""
            path = os.path.join(project, path[len(var):])
            break
    if "$" in path or "`" in path:
        return ""
    if path.startswith("~/"):
        path = os.path.expanduser(path)
    if not os.path.isabs(path):
        if not relative_ok:
            return ""
        path = os.path.join(cwd, path)
    if not os.path.isfile(path):
        return ""
    return os.path.realpath(path)


def tracked_under_scripts(real: str, common_dir: str) -> bool:
    directory = os.path.dirname(real)
    top = git(directory, "rev-parse", "--show-toplevel")
    other_common = git(directory, "rev-parse", "--path-format=absolute", "--git-common-dir")
    if not top or not other_common or os.path.realpath(other_common) != common_dir:
        return False
    rel = os.path.relpath(real, os.path.realpath(top))
    if not rel.startswith("scripts/") or ".." in rel.split("/"):
        return False
    return git(top, "ls-files", "--error-unmatch", "--", rel) is not None


def render(command: str, cwd: str) -> str:
    if not os.path.isabs(cwd):
        cwd = ""
    common_dir = git(REPO_ROOT, "rev-parse", "--path-format=absolute", "--git-common-dir")
    if not common_dir:
        return ""
    common_dir = os.path.realpath(common_dir)
    relative_ok = bool(cwd) and relative_paths_resolve_against_cwd(command, cwd)
    lines: list[str] = []
    seen: set[str] = set()
    for raw in TOKEN_SPLIT.split(command):
        token = raw.replace('"', "").replace("'", "")
        if "scripts/" not in token or token in seen or "\t" in token:
            continue
        seen.add(token)
        real = resolve(token, cwd, relative_ok)
        if real and tracked_under_scripts(real, common_dir):
            lines.append(f"tracked\t{token}")
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
