#!/usr/bin/env bash
# Cupcake signal: push_target_branches
#
# Consumed by:
#   * git_block_main_push (PreToolUse/Bash): the remote branch each `git push` in the pending
#     command would update, judged in the directory the push runs in (`cd <dir> &&`, `git -C <dir>`,
#     `bash -lc 'cd ...'`) rather than the hook's own directory (bd ds2-mods-rs-zmep). All resolving
#     is in scripts/cupcake_push_target_branch.py.
#
# Emits `DEST <ref>` per destination, or one `UNKNOWN <reason>` line. The policy only uses this to
# ALLOW a push whose every destination it names as non-main, so silence (a crash, a timeout) leaves
# the deny standing. Always exits 0: a non-zero signal is replaced by a failure record.
set -uo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"

event="$(cat)"
# Cheap exit for the overwhelming majority of Bash calls, which never push.
case "$event" in
    *push*|*PUSH*|*Push*) ;;
    *) exit 0 ;;
esac

printf '%s' "$event" | python3 "$repo_root/scripts/cupcake_push_target_branch.py" 2>/dev/null || true
exit 0
