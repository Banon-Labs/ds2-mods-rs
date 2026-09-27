#!/usr/bin/env bash
# Cupcake signal: unspent_run_evidence
#
# Consumed by:
#   * no_unspent_run_evidence (Stop): halts a turn end while the running DARK SOULS II build is an
#     unpushed game-code commit whose run evidence is already in ds2-loader.log.
#
# Why this exists. User directive 2026-09-26: "as soon as you collect your commit sha evidence for
# runtime testing that's required for a PR, you're obligated to tear down". The teardown hook
# (scripts/ds2-teardown-after-evidence-push.py) fires on the push, so an agent that read the
# `build git=<sha>` line and then did not push kept the game up indefinitely. It happened with
# 2f2d0a5 on estus-max-default-on the same day.
#
# The facts and the decision live in scripts/cupcake_unspent_evidence.py (`--selftest` pins it).
#
# Emits one line, or nothing:
#   UNSPENTEVIDENCE|branch=<branch>|sha=<sha>
#
# CUPCAKE_UNSPENT_EVIDENCE_OVERRIDE, when SET (even to empty), replaces the measurement, so the Stop
# guard tests never depend on whether a game happens to be running on the machine that runs them.
# CUPCAKE_UNSPENT_EVIDENCE_MACHINE_OVERRIDE=<sha>|<branch>|<worktree> replaces only the machine facts
# and takes precedence, so the hand-test exemption (the user wrote after a launch from that worktree)
# is still decided from the real transcript.
set -uo pipefail
if [ -z "${CUPCAKE_UNSPENT_EVIDENCE_MACHINE_OVERRIDE:-}" ] && [ -n "${CUPCAKE_UNSPENT_EVIDENCE_OVERRIDE+set}" ]; then
    [ -n "$CUPCAKE_UNSPENT_EVIDENCE_OVERRIDE" ] && printf '%s\n' "$CUPCAKE_UNSPENT_EVIDENCE_OVERRIDE"
    exit 0
fi
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
python3 "$root/scripts/cupcake_unspent_evidence.py" 2>/dev/null || true
