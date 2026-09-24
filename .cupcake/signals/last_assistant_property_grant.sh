#!/usr/bin/env bash
# Cupcake signal: last_assistant_property_grant
#
# Consumed by:
#   * no_user_property_grant (Stop): halts a turn that ends by handing the user permission over
#     something that was already theirs.
#
# WHY THIS EXISTS (user directive 2026-09-23). The instance, verbatim, from a turn that had just
# finished building a DLL and had not relaunched the game:
#
#     "the DLL is built but I have not relaunched, so your session is still yours to end when you
#     want the next run."
#
# WHAT IS ACTUALLY BEING BANNED. The clause that stages a handover of something the agent was never
# holding. Nobody granted the agent authority over when the user's own session ends, so "yours to
# end" is not restraint, it is ceremony -- ceremony that costs the user nothing to read once but
# reads as condescension the second time. What belongs in that sentence is the factual half only:
# the DLL is built, the agent has not relaunched. No clause about what is theirs.
#
# THE WAY OUT IS TO DROP THE CLAUSE, not to hedge it differently. A plain statement of what the
# agent did and did not do carries the same information with none of the ceremony.
#
# Emits  PROPERTYGRANT|phrase=<the offending phrase>
# and nothing at all when the turn is clean.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, sys

sys.path.insert(0, os.path.join(os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", "."), "scripts"))
try:
    import cupcake_turn_scan as scan
    import cupcake_property_grant as grant
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

# The closing run only. A grant mid-turn with work after it is the agent narrating between calls,
# and the turn it closes on is the one the user is left holding.
phrase = grant.first_offence(turn.text_runs[-1])
if not phrase:
    sys.exit(0)
print("PROPERTYGRANT|phrase=%s" % phrase)
PY
