#!/usr/bin/env bash
# Cupcake signal: runtime_evidence_for_head
#
# Consumed by:
#   * git_require_runtime_test_before_push (PreToolUse/Bash): denies a push of game code that has
#     never been run.
#
# WHY THIS EXISTS (user directive 2026-09-23, their words): "I thought your agent instructions were
# clear: no push until runtime tests." They were not -- neither AGENTS.md carried that rule, and the
# repo's own session-completion section says the opposite in capitals. Four commits of DLL code went
# to the remote on the strength of a green local gate. This is the rule made executable instead.
#
# WHAT A GREEN GATE DOES NOT PROVE. `scripts/check.sh` compiles for MSVC and runs the pure-logic
# tests under wine. Everything that makes this repo hard -- Arxan's stubs, a detour surviving at a
# redirected entry, a `.flo` child-count substitution, an item vector's inline storage -- is invisible
# to all of it. A crate can be green and still panic the game's allocator on the first menu open.
#
# WHAT COUNTS AS EVIDENCE, and why it is this and not a marker file the agent writes. The game's own
# loader writes `ds2-loader.log` beside the executable, and its first line is `ds2-loader: attach`,
# printed from `DllMain` after the DLL is mapped into the game's address space. An agent cannot
# produce that line by claiming to have run the game; only the game produces it. The check is that
# the line exists AND the log is newer than HEAD's commit time -- a run that predates the code under
# review says nothing about the code under review.
#
# WHY A CLOCK COMPARISON WAS NOT ENOUGH, measured 2026-09-23 in this repo rather than reasoned.
# The first version of this signal asked only `log_mtime >= head_commit_time`. Within one hour of it
# being written its verdict on an unchanged branch flipped from DENY to ALLOW -- HEAD 1790178382, log
# 1790175624 (deny), then log 1790179044 (allow) -- because a PARALLEL SESSION launched the game.
# Nothing about the branch changed. There is one game install and one `ds2-loader.log`, and AGENTS.md
# has agents working in separate worktrees, so the log is shared state that any session can freshen.
#
# Three shapes a clock cannot tell apart, all of which satisfy `log >= head`:
#   1. another worktree's run, with another worktree's DLL staged;
#   2. a plain Steam launch of whatever DLL happened to already be in the game directory;
#   3. commit, then launch WITHOUT rebuilding -- `git commit` does not touch working-tree mtimes, so
#      a DLL built before the commit is still sitting there and the log still postdates HEAD.
# In all three the bytes that ran are not the bytes under review, and the clock says they are.
#
# So the run is tied to the BINARY as well as to the clock, by comparing bytes rather than times:
# `dll_match` is sha256(the DLL staged in the game directory) == sha256(the DLL this checkout built),
# and `fresh` additionally requires the log to postdate the STAGED file, so a restage after a run
# invalidates that run instead of inheriting it. Costs one sha256 of each copy per evaluation,
# measured at ~9ms for the pair.
#
# WHY NOT HAVE THE DLL LOG ITS OWN DIGEST, which is the obvious stronger design. It would have to
# either hash a multi-megabyte file from `DllMain` with the loader lock held -- a startup stall and a
# new crash surface in the one code path whose failure takes the game's startup with it -- or echo
# back a digest `scripts/ds2-run.py` wrote into `ds2-mods.toml`, in which case the log line carries
# the LAUNCHER'S claim rather than the DLL's observation and is barely stronger than reading the two
# files here. Either way it couples this guard to `crates/ds2-loader` and to the launcher's staging
# layout, for a gain over the two-file comparison that is confined to one shape: a log written by a
# binary that has since been replaced by an identical-looking one. The stage-mtime ordering above
# already refuses that. Filed rather than built.
#
# WHAT THIS DELIBERATELY DOES NOT CLAIM. That the run exercised the feature. It proves a launch
# happened after the code was written, with the bytes this checkout built, and that the DLL loaded;
# it cannot prove which row was pressed. That is the honest ceiling of a filesystem signal, and the
# alternative -- an agent-written marker -- has no ceiling at all because it is just prose in a file.
#
# Emits  RUNTIME|game_code=<0|1>|attached=<0|1>|fresh=<0|1>|dll_match=<0|1>|head=<epoch>|log=<epoch>
# and nothing when it cannot tell (the policy fails closed on silence).
set -uo pipefail

# The regression tests need to drive every combination of the three fields without a game install
# and without a commit, the same way `CUPCAKE_CURRENT_BRANCH_OVERRIDE` drives `current_branch`.
# An override that only the tests set cannot weaken the guard in a session: an agent setting it to
# fake a run would be writing the evidence itself, which is the failure this whole signal exists to
# make impossible -- so the guard's value rests on the agent not doing that, exactly as it rests on
# the agent not deleting the policy.
if [ -n "${CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE:-}" ]; then
    printf '%s\n' "$CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE"
    exit 0
fi

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
cd "$REPO" || exit 0

GAME_DIR="$HOME/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
GAME_LOG="$GAME_DIR/ds2-loader.log"

# Mirrors `STAGED_DLL_NAME` and `BUILT_DLL` in scripts/ds2-run.py. Both halves of that contract have
# to change together: rename the artifact there and `dll_match` silently goes to 0 forever, which
# fails closed but denies every game-code push until someone reads this line.
STAGED_DLL="$GAME_DIR/dinput8.dll"
BUILT_DLL="$REPO/target/x86_64-pc-windows-msvc/release/dinput8.dll"

# Which paths make a push a GAME-CODE push. `crates/` ships inside the DLL; the launcher decides what
# the DLL is handed. Policies, docs and the beads export are not game code and are not gated -- a
# guard that demanded a game launch before a Rego fix could be pushed would be gamed within the hour.
head_time="$(git log -1 --format=%ct 2>/dev/null)" || exit 0
[ -n "$head_time" ] || exit 0

upstream="$(git rev-parse --verify --quiet origin/main)" || upstream=""
if [ -n "$upstream" ]; then
    changed="$(git diff --name-only origin/main...HEAD 2>/dev/null)"
else
    changed="$(git diff --name-only HEAD~1..HEAD 2>/dev/null)"
fi

game_code=0
case "$changed" in
    *crates/*|*scripts/ds2-run.py*) game_code=1 ;;
esac

# The staged copy's mtime is the floor `fresh` has to clear as well as HEAD's commit time: a run can
# only vouch for the binary that was in place WHEN IT RAN, so a restage after the run makes the log
# describe a binary that is no longer there. Missing staged file -> epoch 0, which no log can predate,
# and `dll_match` is what refuses that case.
staged_time=0
[ -r "$STAGED_DLL" ] && staged_time="$(stat -c %Y "$STAGED_DLL" 2>/dev/null || echo 0)"

floor="$head_time"
[ "$staged_time" -gt "$floor" ] 2>/dev/null && floor="$staged_time"

attached=0
fresh=0
if [ -r "$GAME_LOG" ]; then
    grep -q "ds2-loader: attach" "$GAME_LOG" 2>/dev/null && attached=1
    log_time="$(stat -c %Y "$GAME_LOG" 2>/dev/null || echo 0)"
    [ "$log_time" -ge "$floor" ] 2>/dev/null && fresh=1
else
    log_time=0
fi

# Bytes, not times. Either file unreadable -> 0, which is the fail-closed direction: a checkout that
# has not built the DLL cannot have run it, and a game directory with no DLL staged cannot have loaded
# one. `cut` rather than awk so the two hashes are compared and nothing else is parsed.
dll_match=0
if [ -r "$STAGED_DLL" ] && [ -r "$BUILT_DLL" ]; then
    staged_sha="$(sha256sum -- "$STAGED_DLL" 2>/dev/null | cut -d' ' -f1)"
    built_sha="$(sha256sum -- "$BUILT_DLL" 2>/dev/null | cut -d' ' -f1)"
    if [ -n "$staged_sha" ] && [ "$staged_sha" = "$built_sha" ]; then
        dll_match=1
    fi
fi

printf 'RUNTIME|game_code=%d|attached=%d|fresh=%d|dll_match=%d|head=%s|log=%s\n' \
    "$game_code" "$attached" "$fresh" "$dll_match" "$head_time" "$log_time"
