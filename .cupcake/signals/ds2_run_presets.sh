#!/usr/bin/env bash
# Cupcake signal: ds2_run_presets
#
# Consumed by:
#   * ds2_run_preset_guard (PreToolUse/Bash): whether each ds2-run.py the pending command launches
#     carries ENGINE_SHIPPED_PRESETS, the check that keeps presets saved from the lighting engine's
#     F1 menu. All resolving is in scripts/cupcake_ds2_run_presets.py, which documents the format.
#
# Why (2026-09-28): launches of ds2-run.py from worktrees cut before d694947 reinstalled Second
# Sin's atmospheres_extended.ini over the user's own presets, twice.
#
# Failure direction: when the command names ds2-run.py and this prints nothing (a crash, a missing
# helper), the policy refuses. Always exits 0: a non-zero signal is replaced by a failure record.
set -uo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"

event="$(cat)"
# Cheap exit for the Bash calls that do not name the launcher at all.
case "$event" in
    *ds2-run.py*) ;;
    *) exit 0 ;;
esac

printf '%s' "$event" | python3 "$repo_root/scripts/cupcake_ds2_run_presets.py" 2>/dev/null || true
exit 0
