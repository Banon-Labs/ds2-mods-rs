#!/usr/bin/env bash
# Cupcake signal: last_assistant_unwatched_wait
#
# Prints `UNWATCHED:<phrase>` when the last turn's closing prose says it is waiting or blocked and no
# backgrounded command, async subagent or Monitor is live to wake the agent; empty otherwise. The
# decision, its carve-outs (a wait on the user, quoted text, mid-turn narration) and its selftest live
# in scripts/cupcake_unwatched_wait.py. Consumed by no_unwatched_wait (Stop).
#
# CUPCAKE_UNWATCHED_WAIT_OVERRIDE, when set, is printed instead, so policy tests can pin a verdict.
# Fails open (empty output) on any error so a transcript hiccup cannot wedge the session.
set -uo pipefail

# Drain the event cupcake writes to stdin before printing anything: the engine reads output only
# after the whole event is written, so a signal that skips this can deadlock the hook (see
# repo_paths.sh, 2026-10-19). Enforced by run_signal_stdin_drain_checks.
cat >/dev/null
if [ -n "${CUPCAKE_UNWATCHED_WAIT_OVERRIDE+set}" ]; then
    [ -n "$CUPCAKE_UNWATCHED_WAIT_OVERRIDE" ] && printf '%s\n' "$CUPCAKE_UNWATCHED_WAIT_OVERRIDE"
    exit 0
fi
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
python3 "$root/scripts/cupcake_unwatched_wait.py" 2>/dev/null || true
