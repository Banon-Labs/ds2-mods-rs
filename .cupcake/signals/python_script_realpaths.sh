#!/usr/bin/env bash
# Cupcake signal: python_script_realpaths
#
# Consumed by:
#   * bash_no_python_file_write (PreToolUse/Bash): the real file behind each `.py` operand in the
#     pending command, symlinks followed, so the committed-script exemption is judged by the file
#     python will run rather than by the spelling of its path. All resolving is in
#     scripts/cupcake_python_script_realpaths.py, which documents the output format.
#
# Why (2026-09-27): `python3 /home/banon/DS2/ds2-run.py`, a symlink to this repo's
# scripts/ds2-run.py, was denied; and `scripts/evil.py -> /tmp/patch.py` was exempt by its spelling.
#
# Failure direction: silence (a crash, a timeout) removes the symlink allow and leaves the spelling
# rules exactly as they were before this signal existed. Always exits 0: a non-zero signal is
# replaced by a failure record.
set -uo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"

event="$(cat)"
# Cheap exit for the Bash calls that name no python file at all.
case "$event" in
    *.py*) ;;
    *) exit 0 ;;
esac

printf '%s' "$event" | python3 "$repo_root/scripts/cupcake_python_script_realpaths.py" 2>/dev/null || true
exit 0
