#!/usr/bin/env bash
# Cupcake signal: tmp_copy_sources
#
# Consumed by:
#   * edit_no_tmp_scripts_guard (PreToolUse/Bash): a `cp`/`install` into /tmp is a copy, not
#     authoring, when every source it names is a TRACKED file under this repo's scripts/ tree.
#     Rego cannot ask git that; scripts/cupcake_tmp_copy_sources.py does, and documents the output.
#
# Why (2026-11-02): `cp scripts/frida/summon-sign-trace.js <scratchpad>/summon-sign-trace-frozen.js`,
# the hot-reload publish step for scripts/ds2-frida-watch.py, was denied as authoring a script.
#
# Failure direction: silence (a crash, a timeout) removes the carve-out, so the copy is denied as
# before. Always exits 0: a non-zero signal is replaced by a failure record.
set -uo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"

event="$(cat)"
# Cheap exit for every event that cannot contain a copy out of scripts/ into /tmp.
case "$event" in
    *scripts/*) ;;
    *) exit 0 ;;
esac
case "$event" in
    *cp\ *|*install\ *) ;;
    *) exit 0 ;;
esac

printf '%s' "$event" | python3 "$repo_root/scripts/cupcake_tmp_copy_sources.py" 2>/dev/null || true
exit 0
