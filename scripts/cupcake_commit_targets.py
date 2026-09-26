#!/usr/bin/env python3
"""The branch each directory a pending `git commit` could target is on, resolved THERE.

Read by `.cupcake/signals/commit_target_branches.sh` for DS2-MODS-BLOCK-MAIN-COMMIT
(`.cupcake/policies/claude/git_block_main_commit.rego`).

# The miss this closes (bd ds2-mods-rs-qzd, 2026-09-26)

The guard's `current_branch` and `worktree_branches` signals describe the SESSION checkout, and the
session checkout is this repository. With it on `main`, a commit in a different repository's
feature-branch worktree was denied:

    git -C /home/banon/projects/fromsoftware-rs-ds2-paramdefs commit -q -F <msgfile>
    cd /home/banon/projects/fromsoftware-rs-ds2-paramdefs && git commit -q -F <msgfile>

The target was on `ds2-paramdefs`. The only escape the policy knew was a path listed by THIS
repository's `git worktree list`, which another repository's worktree never is, so the work was
committed with `git write-tree`/`commit-tree`/`update-ref` to get past the guard -- the plumbing
route the guard cannot see at all.

# What it emits

One line per `git -C <path>` or `cd <path>` operand in the command whose path is absolute (or
`~`-spelled) and names a git checkout with a branch checked out:

    <operand as the policy extracts it>\\t<branch>

The operand key is the token with surrounding quotes and trailing slashes removed, which is exactly
the key the policy computes, so the policy looks the branch up by the text it already has. A
detached HEAD, a path that is not a checkout, a relative path, and anything with a `$` or backtick
in it emit nothing, and the policy fails closed on nothing. A relative path is refused rather than
resolved because it is relative to whatever directory the shell is in when it gets there, which an
earlier `cd` in the same command can change and the event's cwd does not know about.

The policy decides which commits these branches may vouch for; this module only answers "what
branch is that directory on".
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys

OPERAND = re.compile(r"""(?:\bgit[ \t]+-C|(?:^|[\s;&|(])cd)[ \t]+("[^"\n]*"|'[^'\n]*'|[^\s;&|()]+)""")


def operand_key(token: str) -> str:
    return token.strip("\"'").rstrip("/")


def branch_at(path: str) -> str:
    try:
        r = subprocess.run(
            ["git", "-C", path, "symbolic-ref", "--quiet", "--short", "HEAD"],
            capture_output=True, text=True, timeout=2,
        )
    except (OSError, subprocess.SubprocessError):
        return ""
    return r.stdout.strip() if r.returncode == 0 else ""


def targets(command: str) -> list[tuple[str, str]]:
    if "commit" not in command.lower():
        return []
    out: list[tuple[str, str]] = []
    seen: set[str] = set()
    for m in OPERAND.finditer(command):
        key = operand_key(m.group(1))
        if not key or key in seen or "$" in key or "`" in key:
            continue
        seen.add(key)
        path = os.path.expanduser(key) if key.startswith("~") else key
        if not os.path.isabs(path) or not os.path.isdir(path):
            continue
        branch = branch_at(path)
        if branch and "\t" not in key and "\n" not in key:
            out.append((key, branch))
    return out


def render(command: str) -> str:
    return "\n".join(f"{key}\t{branch}" for key, branch in targets(command))


def main() -> int:
    try:
        event = json.load(sys.stdin)
    except ValueError:
        return 0
    ti = event.get("tool_input") if isinstance(event, dict) else None
    cmd = ti.get("command") if isinstance(ti, dict) else None
    if isinstance(cmd, str):
        text = render(cmd)
        if text:
            print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
