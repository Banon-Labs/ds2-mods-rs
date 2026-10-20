#!/usr/bin/env bash
# Cupcake signal: idle_game_after_evidence
#
# Consumed by:
#   * no_idle_game_after_evidence (Stop): halts a turn end while DarkSoulsII.exe, launched by
#     scripts/ds2-run.py in this session, is still running after its ds2-loader.log evidence was read
#     and nobody gave a reason to keep it up.
#
# Why this exists. User, 2026-10-19: "Is there a reason the game is still up? If not, we lack a rego
# policy to tear it down as soon as evidence is collected". The push teardown hook and
# DS2-MODS-NO-UNSPENT-RUN-EVIDENCE both key on an unpushed commit; an amended commit, a non-game-code
# branch or a run that is not push evidence at all left the game up with neither of them speaking.
#
# The facts and the decision live in scripts/cupcake_idle_game.py (`--selftest` pins it).
#
# Emits one line, or nothing:
#   IDLEGAME|pid=<pid>|launched=<iso timestamp>
#
# CUPCAKE_IDLE_GAME_OVERRIDE, when SET (even to empty), replaces the whole measurement.
# CUPCAKE_IDLE_GAME_MACHINE_OVERRIDE=dead|<pid>|<start epoch> replaces only pgrep and /proc, so the
# Stop guard tests read the transcript half for real whatever is running on the test machine.
set -uo pipefail
if [ -n "${CUPCAKE_IDLE_GAME_OVERRIDE+set}" ]; then
    [ -n "$CUPCAKE_IDLE_GAME_OVERRIDE" ] && printf '%s\n' "$CUPCAKE_IDLE_GAME_OVERRIDE"
    exit 0
fi
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
python3 "$root/scripts/cupcake_idle_game.py" 2>/dev/null || true
