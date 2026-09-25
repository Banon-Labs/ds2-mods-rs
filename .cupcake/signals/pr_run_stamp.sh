#!/usr/bin/env bash
# Cupcake signal: pr_run_stamp
#
# Consumed by:
#   * pr_requires_run_stamp (PreToolUse/Bash): refuses `gh pr create` and `gh pr ready` unless the PR
#     body carries a `Run-Stamp:` line for the commit the PR is about. Format, rules and reasons are in
#     scripts/cupcake_run_stamp.py, which does all of the deciding; this wrapper only feeds it the
#     hook event and guarantees an exit status of 0, because a non-zero signal is replaced by a failure
#     record and the policy reading it would see nothing. The policy fails CLOSED on nothing when the
#     command names `gh pr create`/`gh pr ready`, so a crash here refuses rather than waves through.
#
# Emits one line:  RUNSTAMP|verb=<none|create|ready>[|ok=<0|1>|why=<code>|sha=<40hex>]
#
# The overrides the tests use (CUPCAKE_PR_STAMP_HEAD_OVERRIDE, _VIEW_OVERRIDE, _NOW_OVERRIDE) are read
# in the Python module. Same standing as CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE: an agent setting them to
# pass the gate is writing the evidence itself, which is the failure the gate exists to refuse.
set -uo pipefail

CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT

event="$(cat)"
# Cheap exit for the overwhelming majority of Bash calls, which never name gh at all.
case "$event" in
    *gh*) ;;
    *) echo "RUNSTAMP|verb=none"; exit 0 ;;
esac

printf '%s' "$event" | python3 -c '
import json, os, sys
sys.path.insert(0, os.path.join(os.environ["CUPCAKE_SIGNAL_REPO_ROOT"], "scripts"))
import cupcake_run_stamp as rs
try:
    event = json.load(sys.stdin)
except ValueError:
    sys.exit(0)
print(rs.verdict(event))
' 2>/dev/null || true
exit 0
