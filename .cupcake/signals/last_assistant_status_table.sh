#!/usr/bin/env bash
# Cupcake signal: last_assistant_status_table
#
# Consumed by:
#   * no_status_table_at_turn_end (Stop): halts a turn that closes on a table of its own PROGRESS.
#
# WHY THIS EXISTS (user directive 2026-09-23, their words): "Maybe we can add a rego stop policy for
# tidy tables. I don't think you've ever produced one when I asked for one, you only produce them
# when you're trying to stop early."
#
# THE INSTANCE, from the turn immediately before that directive. The work in flight was a save
# picker. One piece of it landed, and the turn ended:
#
#     | piece | state |
#     |---|---|
#     | name/state/stats per slot, in Rust | done, host-testable |
#     | soul level | deliberately absent |
#     | picker rows over those slots | next |
#     | load one chosen character instead of the whole container | after that |
#
# Nothing in that table is information the user asked for. Two of its four rows are work that was not
# done, written out in the format that makes not-doing-it look like a deliverable. The agent's own
# account of why, from the following turn: "ending a turn with a tidy table reads as progress and
# costs me nothing".
#
# WHAT SEPARATES THIS FROM A TABLE THAT EARNS ITS PLACE. Not the table -- the CONTENT. A table of
# measurements (offset to value, file to hash, flag to effect) is the right shape for data the user
# has to scan, and the repo's own one-paragraph rule actively pushes structure like that. A table
# whose CELLS are `done` / `next` / `missing` / `after that` is not data at all: it is a status report
# on the agent, in a grid, at the exact moment the agent stopped working. So the test is a lexicon of
# progress words appearing in the table's cells, TWICE, which is what stops one stray "done" in a
# measurement table from arming it.
#
# THREE WAYS OUT, each a turn shape that is allowed to end on a progress table:
#   1. the user asked for one -- "table", "compare", "side by side", "matrix", "checklist";
#   2. the turn is genuinely waiting on the user (`cupcake_turn_scan.blocked_on_user`);
#   3. the table is not in the turn's FINAL prose run -- a table mid-turn, with work after it, is
#      the agent showing findings and carrying on, which is the shape this guard wants more of.
#
# Emits  STATUSTABLE|hits=<n>|asked=<0|1>|blocked=<0|1>|sample=<first offending row>
# and nothing at all when the turn is clean.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, sys

sys.path.insert(0, os.path.join(os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", "."), "scripts"))
try:
    import cupcake_turn_scan as scan
    import cupcake_status_table as table
except Exception:
    sys.exit(0)  # fail open: a missing helper must never wedge a session

path = scan.latest_transcript()
if not path:
    sys.exit(0)
events = scan.load_events(path)
turns = scan.split_turns(events)
turn = scan.last_text_turn(turns)
if turn is None or not turn.text_runs:
    sys.exit(0)

closing = turn.text_runs[-1]
hits, sample = table.progress_table_hits(closing)
if hits < table.MIN_HITS:
    sys.exit(0)

asked = 1 if table.user_asked_for_a_table(table.last_user_prompt(events)) else 0
blocked = 1 if scan.blocked_on_user(closing) else 0
print("STATUSTABLE|hits=%d|asked=%d|blocked=%d|sample=%s" % (hits, asked, blocked, sample))
PY
