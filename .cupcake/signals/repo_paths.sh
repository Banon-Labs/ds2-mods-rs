#!/usr/bin/env bash
# Cupcake signal: repo_paths
#
# Consumed by:
#   * bash_no_python_file_write (PreToolUse/Bash): the committed-script exemption has to decide
#     whether an ABSOLUTE or `~`-spelled `.py` path names a file under this checkout's `scripts/`
#     tree. A regex over the command text cannot answer that -- nothing in the text says where the
#     repo is -- and guessing from the shape of the path is how `/tmp/ds2-mods-rs/scripts/x.py`
#     becomes an escape hatch.
#
# Why it exists (2026-09-23)
#
# The exemption recognised exactly one spelling, `python3 scripts/<name>.py`, while the user's
# global AGENTS.md carries a standing directive for launch commands: "use ABSOLUTE paths in every
# launch command -- never `./script`, because cwd is not yours and resets between calls". Those two
# rules contradicted each other. Measured that day, through scripts/cupcake-hook.sh:
#
#     cd /home/banon/projects/ds2-mods-rs; python3 \
#       /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py --rows load-character-from-file,...
#
# was DENIED as a "python file write" -- a launch of a committed, reviewed script, refused for the
# only spelling the instructions allow. An agent obeying the instructions hit a deny; an agent
# satisfying the guard disobeyed the instructions. Widening the regex to "any path ending in
# /scripts/*.py" would have traded that conflict for a bypass, so the policy needs the real root.
#
# What it emits: exactly two lines, in this order, no labels.
#
#     <absolute path of the checkout that owns these policies>
#     <absolute path of $HOME>
#
# Line 1 is derived from THIS FILE'S OWN LOCATION (`.cupcake/signals/` -> repo root), not from the
# process cwd and not from git. That is the point: the policy set being evaluated and the scripts/
# tree being exempted then provably come from the same checkout, so a second clone, a git worktree
# or a session started in a subdirectory each resolve to their own root rather than to whichever
# one happened to be current. `git rev-parse --show-toplevel` would answer a different question
# (the repo containing the CWD) and would go silent outside a work tree.
#
# Line 2 is $HOME, which is the only way to expand a `~/...` spelling. Without it the policy cannot
# tell `~/projects/ds2-mods-rs/scripts/x.py` (this repo) from `~/ds2-mods-rs/scripts/x.py` (a
# lookalike directory an agent could create), so it refuses both.
#
# FAIL CLOSED, in the direction that matters. This signal only ever feeds the EXEMPTION. If it is
# missing, times out, or emits something that is not an absolute path, the policy loses spellings
# and denies more -- it never gains an allow. Measured against cupcake 0.5.2 the same day: a
# `required_signals` entry naming a signal that does not exist logs `Signal '<name>' failed` and the
# policy still evaluates with the signal absent, so a broken signal cannot take the guard down.
#
# Safe to run on every PreToolUse: no subprocess, no network, no filesystem write -- one `cd` in a
# subshell and a `printf`.
set -uo pipefail

# The overrides exist for the policy regression tests and for another machine's layout, mirroring
# CUPCAKE_CURRENT_BRANCH_OVERRIDE in .cupcake/rulebook.yml.
repo_root="${CUPCAKE_REPO_ROOT_OVERRIDE:-}"
if [ -z "$repo_root" ]; then
	repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd) || repo_root=""
fi

printf '%s\n%s' "$repo_root" "${CUPCAKE_HOME_DIR_OVERRIDE:-${HOME:-}}"

# Lines 3 and on: every other git worktree of this same repository. A worktree is this repo's own
# committed scripts on another branch, exactly as the main checkout is on whichever branch it has
# out, and refusing its launcher sent a run through a copy-the-DLL workaround (2026-09-26). The
# list comes from git's own record of the repository's worktrees, which the command being judged
# cannot write, and a failure here only drops these lines -- the exemption narrows, never widens.
if [ -z "${CUPCAKE_REPO_ROOT_OVERRIDE:-}" ] && [ -n "$repo_root" ]; then
	git -C "$repo_root" worktree list --porcelain 2>/dev/null \
		| sed -n 's/^worktree //p' \
		| while IFS= read -r tree; do
			[ "$tree" = "$repo_root" ] || printf '\n%s' "$tree"
		done
fi
