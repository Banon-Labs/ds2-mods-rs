#!/usr/bin/env bash
# Cupcake signal: commit_target_branches
#
# Consumed by:
#   * git_block_main_commit (PreToolUse/Bash): the branch of each absolute `git -C <path>` / `cd <path>`
#     operand in the pending command, resolved in THAT directory rather than in the session checkout,
#     so a commit into another repository's feature branch is judged by that branch (bd
#     ds2-mods-rs-qzd). All resolving is in scripts/cupcake_commit_targets.py.
#
# Emits one `<operand>\t<branch>` line per resolvable operand and nothing otherwise. The policy only
# ever uses this to ALLOW a commit whose target it can name, so silence (a crash, a timeout, a detached
# HEAD) leaves the deny standing. Always exits 0: a non-zero signal is replaced by a failure record.
set -uo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"

event="$(cat)"
# Cheap exit for the overwhelming majority of Bash calls, which never commit.
case "$event" in
    *commit*|*COMMIT*|*Commit*) ;;
    *) exit 0 ;;
esac

printf '%s' "$event" | python3 "$repo_root/scripts/cupcake_commit_targets.py" 2>/dev/null || true
exit 0
