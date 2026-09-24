#!/usr/bin/env bash
# Cupcake signal: last_assistant_game_alive
#
# Consumed by:
#   * no_unchecked_game_alive_claim (Stop): halts a turn that tells the user the game is running
#     without anything in that turn having looked.
#
# WHY THIS EXISTS (user directive 2026-09-23, their words): "Wait. The game is up? Prove it. If you
# can't prove it, rego policy for bad behavior."
#
# THE INSTANCE, from the turn immediately before that directive. The agent launched the game, read
# the launcher's attach block, and then across four turns kept telling the user the game was up and
# to go press a row in it -- "The game is up on the new DLL with the rows armed -- press Load
# Character from File." `ds2-teardown.py --status` answered `nothing running`. The process had
# crashed on its own several minutes earlier, and the agent's own teardown that turn had reported
# `0 session process(es) to remove`, which it read past.
#
# WHAT IS ACTUALLY BEING BANNED. Not saying the game was launched -- that is a past-tense fact the
# launcher proves properly, with the DLL's own log line. The offence is the PRESENT TENSE: telling
# the user a process is alive NOW on the strength of having started one earlier. A game that
# attached, ran and died leaves the attach block in the scrollback and nothing on the system, and
# the user acts on the claim -- they turn to the screen and there is nothing there.
#
# THE WAY OUT IS TO LOOK, in the same turn: `ds2-teardown.py --status`, a `ds2-run.py` launch, or a
# `pgrep -x DarkSoulsII.exe`. Any one of those in the turn's tool calls and this stays silent.
#
# Emits  GAMEALIVE|phrase=<the offending phrase>
# and nothing at all when the turn is clean.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, sys

sys.path.insert(0, os.path.join(os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", "."), "scripts"))
try:
    import cupcake_turn_scan as scan
    import cupcake_game_alive as alive
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

# Every Bash command the turn ran, which is where a liveness check would be.
commands = []
for block in turn.tools("Bash"):
    command = (block.get("input") or {}).get("command")
    if isinstance(command, str):
        commands.append(command)

# The closing run only. A claim mid-turn with work after it is the agent narrating between calls,
# and the turn it closes on is the one the user is left holding.
phrase = alive.first_offence(turn.text_runs[-1], commands)
if not phrase:
    sys.exit(0)
print("GAMEALIVE|phrase=%s" % phrase)
PY
