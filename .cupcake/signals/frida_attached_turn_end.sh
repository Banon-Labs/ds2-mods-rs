#!/usr/bin/env bash
# Cupcake signal: frida_attached_turn_end
#
# Consumed by:
#   * no_stop_while_frida_attached (Stop): halts a turn end while DarkSoulsII.exe is up, a
#     ds2-frida-watch.py this session started is attached to it, and the closing message hands the
#     user nothing -- no question, no in-game request, no blocker -- and nothing was torn down.
#
# Why this exists. User, 2026-11-02: "I didn't ask you to do anything except for ensure that I never
# have to ask you to keep going when a project isn't completed while the game is up and Frida is
# attached". The turn it answers ended on a finding ("it's invisible for some reason other than where
# it is") with the summon-sign tracer attached and the investigation open.
#
# The facts and the decision live in scripts/cupcake_frida_attached.py (`--selftest` pins it). The
# event is passed through to it because its `transcript_path` names the session whose turn is ending;
# the newest transcript in the project directory may be another session's.
#
# Emits one line, or nothing:
#   FRIDAATTACHED|game=<pid>|watcher=<agent file name>
#
# CUPCAKE_FRIDA_ATTACHED_OVERRIDE, when SET (even to empty), replaces the whole measurement.
# CUPCAKE_FRIDA_ATTACHED_MACHINE_OVERRIDE=dead|<game pid>|<agent file names or -> replaces only
# pgrep and /proc, so the Stop guard tests read the transcript half for real whatever is running on
# the test machine.
set -uo pipefail
# Read the whole event before printing anything (the cupcake stdin deadlock; see repo_paths.sh).
event="$(cat)"
if [ -n "${CUPCAKE_FRIDA_ATTACHED_OVERRIDE+set}" ]; then
    [ -n "$CUPCAKE_FRIDA_ATTACHED_OVERRIDE" ] && printf '%s\n' "$CUPCAKE_FRIDA_ATTACHED_OVERRIDE"
    exit 0
fi
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
printf '%s' "$event" | python3 "$root/scripts/cupcake_frida_attached.py" 2>/dev/null || true
