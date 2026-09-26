#!/usr/bin/env bash
# Cupcake signal: stack_merge_dependents
#
# Consumed by:
#   * no_delete_branch_under_stack (PreToolUse/Bash): refuses `gh pr merge <N> --delete-branch` (or
#     `-d`) while another open pull request uses PR <N>'s head branch as its base, because deleting
#     that branch makes GitHub close the dependents instead of retargeting them (incident 2026-09-26,
#     #107 #108 #115 #129 closed, #131 merged into #129's branch, recreated as #139-#143).
#
# All deciding is in scripts/cupcake_stack_merge.py, which asks `gh pr view` and `gh pr list`. This
# wrapper feeds it the hook event and always exits 0: a non-zero signal is replaced by a failure
# record, and the policy fails CLOSED on silence for a command that deletes a merged branch.
#
# Emits one line: STACKMERGE|verb=<none|merge>[|delete=..|ok=..|why=..|pr=..|head=..|base=..|deps=..]
set -uo pipefail

CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"

event="$(cat)"
# Cheap exit for nearly every Bash call, which never names gh and merge together.
case "$event" in
    *gh*merge*) ;;
    *) echo "STACKMERGE|verb=none"; exit 0 ;;
esac

printf '%s' "$event" | python3 "$CUPCAKE_SIGNAL_REPO_ROOT/scripts/cupcake_stack_merge.py" 2>/dev/null || true
exit 0
