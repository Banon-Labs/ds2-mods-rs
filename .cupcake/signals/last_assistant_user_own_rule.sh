#!/usr/bin/env bash
# Cupcake signal: last_assistant_user_own_rule
#
# Scans the most recently completed assistant turn and emits one facts line when that turn ended by
# handing the user back a rule the user themselves authored:
#
#   OWNRULEFACTS|clause=..|ruleid=..|command=..|guardevent=..
#
# A clean turn emits nothing (fail-open, like every neighbouring signal).
#
# The failure this exists to refuse
# ---------------------------------
# 2026-09-23, closing a turn that had just opened a draft pull request:
#
#   It stays draft -- undrafting is yours.
#
# The user wrote that rule. It lives in one of their memories and in the global policy that denies
# `gh pr ready`. Their reply is the specification:
#
#   "If undrafting is mine, because I told you it was mine, does it need to be stated? Is there a
#    way to guard against you telling me details that I provided to you through rego policies and
#    agent instructions that I already knew?"
#
# Why the neighbouring Stop guards do not catch it
#   * `no_described_next_step` needs a next step the agent could have started. Undrafting is one it
#     may never start, so that guard correctly stays silent.
#   * `no_narrated_action` needs a present-participle announcement of an action in flight.
#   * `wall_of_text` charges length, and this sentence is six words.
#
# None of them ask the question that matters here: does the reader already own this fact because
# they are the one who decided it?
#
# What it deliberately does not flag, via the two exemption fields
#   command    -- the message carries a pasteable `git push` / `gh pr ready`. `AGENTS.md` positively
#                 requires handing that over for work the agent may not do, so charging for it would
#                 delete one instruction to satisfy another.
#   guardevent -- the message reports a guard that actually fired this turn. That is an event the
#                 user was not watching for, not a standing rule they already hold.
#
# The shared half (transcript discovery, turn bucketing) comes from `scripts/cupcake_turn_scan.py`
# and the classification from `scripts/cupcake_user_own_rule.py`, so the guards cannot drift into
# disagreeing about the same turn. Quoted spans are stripped before matching, so a message that
# quotes one of these sentences -- a report about this guard included -- cannot trip it.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, sys

sys.path.insert(0, os.path.join(os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", "."), "scripts"))
try:
    import cupcake_turn_scan as scan
    import cupcake_user_own_rule as own
except Exception:
    sys.exit(0)  # fail open: a missing helper must never wedge a session

path = scan.latest_transcript()
if not path:
    sys.exit(0)
events = scan.load_events(path)
turns = scan.split_turns(events)
turn = scan.last_text_turn(turns)
if turn is None:
    sys.exit(0)

# A turn that kept going past its last prose did not end on that prose, so the clause sat between
# two tool calls and is exactly the shape this rule must never touch.
if turn.last_text_index < 0 or turn.tool_after(turn.last_text_index):
    sys.exit(0)

runs = turn.text_runs
if not runs:
    sys.exit(0)
closing = runs[-1]

found = own.restated_user_rule(closing)
if not found:
    sys.exit(0)
clause, rule_id = found

print(
    "OWNRULEFACTS|clause={}|ruleid={}|command={}|guardevent={}".format(
        clause,
        rule_id,
        int(own.hands_over_command(closing)),
        int(own.reports_guard_event(closing)),
    )
)
PY
