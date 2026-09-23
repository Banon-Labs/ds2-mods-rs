#!/usr/bin/env bash
# Cupcake signal: last_assistant_shouting
#
# Consumed by:
#   * no_shouting_at_turn_end (Stop): halts a turn whose closing prose puts its emphasis in the
#     capitalisation instead of in the sentence.
#
# Why this exists (user directive 2026-09-23, written in the style it is complaining about): "When
# I TALK LIKE THIS everyone thinks the WORDS ARE IMPORTANT but really it distracts from the
# SUBSTANCE OF THE MESSAGE which is burried by font style and EXCESSIVE PROSE that is supposed to
# be caught by a REGO POLICY that prevents you from writing in ALL CAPS."
#
# What separates a shout from a name, and it is the whole design. A capitalised token is very often
# the right thing to write: DLL, RVA, AES, TOML, CPU and OK are names, SAVE_DIR_BUILD and
# FUN_1402e67f0 are identifiers, DS2SOFS0000.sl2 is a file. None of those is volume. What is volume
# is capitalising an ordinary sentence, and scripts/cupcake_shouting.py draws the line in two
# places, both tuned against this repo's own text: four capitalised words in a row, or one word
# taken from a closed list of function words that could never be a name. That module owns the
# patterns, the verbatim-span stripping and the threshold, so the policy, its unit test and any
# audit read one answer instead of three.
#
# Three things this deliberately cannot see:
#   1. anything in backticks, in a fenced block or in quotation marks -- a log line, a menu string,
#      a register dump or the user's own words quoted back are not the agent shouting;
#   2. anything that is not the turn's FINAL prose run, since `turn.text_runs[-1]` is what is read.
#      A shout mid-turn with work after it is not the behaviour this guard is aimed at, and this is
#      also what keeps it clear of the launch banner AGENTS.md requires, which by that rule sits
#      immediately before the launch call and therefore mid-turn;
#   3. an interrupted turn, which fires no Stop event at all.
#
# Emits  SHOUTING|score=<n>|sample=<the loudest line>
# and nothing at all when the closing prose is clean.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, sys

sys.path.insert(0, os.path.join(os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", "."), "scripts"))
try:
    import cupcake_turn_scan as scan
    import cupcake_shouting as shouting
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

score, sample = shouting.shouting_score(turn.text_runs[-1])
if score < shouting.MIN_RUN or not sample:
    sys.exit(0)
print("SHOUTING|score=%d|sample=%s" % (score, sample))
PY
