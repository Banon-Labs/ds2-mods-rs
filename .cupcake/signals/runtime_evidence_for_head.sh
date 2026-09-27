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
# Emits  RUNTIME|game_code=<0|1>|attached=<0|1>|fresh=<0|1>|dll_match=<0|1>|pending=<0|1>|head=<epoch>|log=<epoch>
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

# THE CHECKOUT BEING PUSHED IS NOT ALWAYS THE ONE THIS SCRIPT LIVES IN, and resolving it from
# `BASH_SOURCE` alone was wrong in the direction that matters. AGENTS.md has agents working in
# `git worktree`s under `.claude/worktrees/`, and the hook always runs the MAIN checkout's copy of
# this file -- so every field below described the main checkout's branch while the push being judged
# was a worktree's. Measured 2026-09-24: a worktree whose build was staged and launched, staged and
# built sha256 byte-identical, and this signal answered `dll_match=0` because it hashed a different
# checkout's `target/`. The dangerous direction is the other one: `game_code` read off the main
# checkout's diff is 0 whenever THAT branch happens to touch no `crates/`, which would have let an
# unrun worktree push of pure game code straight through.
#
# So: the invoking directory wins when it is a checkout of this same repository -- same
# `--git-common-dir`, which is what a worktree shares with its main checkout and what an unrelated
# repo does not. Anything else falls back to this script's own root, including a cwd outside any
# repository, which is the pre-existing behaviour and the fail-closed one.
SCRIPT_REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"

# The pending event. Cupcake pipes the whole PreToolUse event to every signal on stdin (measured in
# er-mods-rs against cupcake 0.5.2, and it is how `scripts/cupcake_push_target_repo.py` there reads
# the command). Guarded on a pipe so a hand run from a terminal does not wait on the keyboard.
event=""
if [ ! -t 0 ]; then
    event="$(cat)"
fi

REPO="$SCRIPT_REPO"
script_common="$(git -C "$SCRIPT_REPO" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || script_common=""

# `<dir>` -> its toplevel when it is a checkout of this same repository, else nothing.
same_repo_root() {
    local dir="$1" common
    [ -n "$dir" ] && [ -n "$script_common" ] && [ -d "$dir" ] || return 1
    common="$(git -C "$dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || return 1
    [ "$(realpath -- "$common" 2>/dev/null)" = "$(realpath -- "$script_common" 2>/dev/null)" ] || return 1
    git -C "$dir" rev-parse --show-toplevel 2>/dev/null
}

# The shell's directory as the event reports it wins over the hook process's own cwd: the event's
# `cwd` is where the Bash tool's shell sits, which is where a bare `git push` would run.
event_cwd=""
if [ -n "$event" ]; then
    event_cwd="$(printf '%s' "$event" | python3 -c 'import json,sys
try:
    v = json.load(sys.stdin).get("cwd")
except Exception:
    v = None
print(v if isinstance(v, str) else "")' 2>/dev/null)" || event_cwd=""
fi
# `base` is the directory a relative `cd` in the command resolves against, so it keeps a subdirectory
# rather than collapsing to the toplevel.
base="$SCRIPT_REPO"
if root="$(same_repo_root "$event_cwd")" && [ -n "$root" ]; then
    REPO="$root"
    base="$event_cwd"
elif root="$(same_repo_root "$PWD")" && [ -n "$root" ]; then
    REPO="$root"
    base="$PWD"
fi

# The `cd` in the command itself (2026-09-26). Resolving from the cwd alone still got this wrong:
# with the hook sitting in the main checkout,
#
#     cd /home/banon/projects/ds2-mods-rs/.claude/worktrees/load-hang && git push -u origin save-file-load-hang
#
# was refused with `dll_match=0`, although the staged dinput8.dll and that worktree's built one had
# the same sha256 and a launch of it had attached. Every field was measured in the main checkout,
# because the `cd` had not run yet when the hook fired. `head`, `game_code` and `pending` were read
# from the wrong tree too, and `game_code` is the dangerous one: a docs-only main checkout makes an
# unrun worktree push of crates/ read as out of jurisdiction.
#
# scripts/cupcake_push_target_repo.py walks a leading `cd <dir>`, `git -C <dir>` and `bash -c`
# wrapper to the directory the push runs in, and only answers `REPO` for a working tree whose
# git-common-dir is this repository's. `UNKNOWN` (unlexable, a subshell, a heredoc, a foreign or
# missing directory, two pushes at two trees) and a resolver that is missing or crashes all print
# nothing here, which the policy refuses: this is the fail-closed direction.
if [ -n "$event" ]; then
    target="$(printf '%s' "$event" | python3 "$SCRIPT_REPO/scripts/cupcake_push_target_repo.py" --cwd "$base" 2>/dev/null)" || exit 0
    case "$target" in
        SELF) ;;
        "REPO "*)
            root="$(same_repo_root "${target#REPO }")" || exit 0
            [ -n "$root" ] || exit 0
            REPO="$root"
            ;;
        *) exit 0 ;;
    esac
fi
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
#
# WHAT IS BEING PUSHED IS READ OFF THE COMMAND, NOT OFF `HEAD` (2026-09-25). This used to diff
# `origin/main...HEAD` as `HEAD` stood when the hook fired, which is before the command runs. The
# command that walked through was
#
#     git commit -qam "..." && git push -u origin launcher-drop-flag
#
# on a branch cut from `origin/main` with `scripts/ds2-run.py` edited and uncommitted. At hook time
# `HEAD` WAS `origin/main`, the diff was empty, `game_code` was 0, and the launcher change went out
# with `ds2-loader.log` last written 330 seconds before its commit. `scripts/cupcake_push_scope.py`
# now says which refs the command pushes and whether it commits first:
#
#   * `pending=1` -- a `git commit` runs before the push. The working tree's tracked changes and
#     untracked files count toward `game_code`, because the commit is made of them, and the policy refuses
#     outright: the commit that would be pushed does not exist yet, so no run can have tested it.
#     Commit in one command, run the game, push in another.
#   * each `REF` is diffed against `origin/main` on its own, and `head` is the newest of their
#     commit times, so `git push origin other-branch` is judged on `other-branch`.
#   * `ALL 1` (`--all`, `--mirror`) is game code: it pushes every branch and none was measured.
scope=""
if [ -n "$event" ] && [ -f "$SCRIPT_REPO/scripts/cupcake_push_scope.py" ]; then
    scope="$(printf '%s' "$event" | python3 "$SCRIPT_REPO/scripts/cupcake_push_scope.py" 2>/dev/null)" || scope=""
fi
pending=0
case "$scope" in *"COMMIT 1"*) pending=1 ;; esac
refs="$(printf '%s\n' "$scope" | sed -n 's/^REF //p')"
[ -n "$refs" ] || refs="HEAD"

#
# A committed ref whose game-path changes are comments only is not game code (2026-09-26).
# `scripts/cupcake_comment_only.py` lexes both sides of every changed Rust file and answers `GAME 0`
# only when the token streams match with comments removed; an added, deleted or non-`.rs` game file,
# a lex failure, a git error, or the script crashing all count as game code. A pending commit's
# working-tree changes stay on the plain path test: the policy refuses that shape anyway.
upstream="$(git rev-parse --verify --quiet origin/main)" || upstream=""
game_code=0
head_time=""
while IFS= read -r ref; do
    [ -n "$ref" ] || continue
    # An unknown ref makes the push fail on its own; measure HEAD rather than nothing.
    git rev-parse --verify --quiet "$ref^{commit}" >/dev/null 2>&1 || ref="HEAD"
    # The newest commit on the branch that touches game code, not the ref's own tip: a scripts-only
    # commit on top of a run does not change what the run loaded, and flooring on the tip refused
    # that push until a second, pointless launch (2026-09-26). No such commit -> the tip, as before.
    t=""
    if [ -n "$upstream" ]; then
        t="$(git log -1 --format=%ct "origin/main..$ref" -- crates scripts/ds2-run.py 2>/dev/null)" || t=""
    fi
    [ -n "$t" ] || t="$(git log -1 --format=%ct "$ref" 2>/dev/null)" || t=""
    [ -n "$t" ] || continue
    if [ -z "$head_time" ] || [ "$t" -gt "$head_time" ]; then head_time="$t"; fi
    if [ -n "$upstream" ]; then diff_base="origin/main"; else diff_base="$ref~1"; fi
    ref_changed="$(git diff --name-only "$diff_base...$ref" 2>/dev/null)"
    case "$ref_changed" in
        *crates/*|*scripts/ds2-run.py*)
            verdict="$(python3 "$SCRIPT_REPO/scripts/cupcake_comment_only.py" --base "$diff_base" --ref "$ref" 2>/dev/null)" || verdict=""
            [ "$verdict" = "GAME 0" ] || game_code=1
            ;;
    esac
done <<EOF_REFS
$refs
EOF_REFS
[ -n "$head_time" ] || exit 0

if [ "$pending" = 1 ]; then
    case "$(git diff --name-only HEAD 2>/dev/null; git ls-files --others --exclude-standard 2>/dev/null)" in
        *crates/*|*scripts/ds2-run.py*) game_code=1 ;;
    esac
fi
case "$scope" in *"ALL 1"*) game_code=1 ;; esac

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
    # WHEN THE RUN STARTED, not when it last wrote. The DLL rotates the log to `.prev` and creates
    # it afresh on its first write (crates/ds2-loader/src/lib.rs), so the file's BIRTH time is the
    # run's start. Its mtime keeps moving for as long as the game is up, so a session launched
    # before a commit and still logging after it read as fresh, and vouched for code it never
    # loaded. Birth time is what the filesystem reports as `%W`; where it cannot (0 or `-`), fall
    # back to mtime, which is the old behaviour and no looser than it was.
    log_time="$(stat -c %W "$GAME_LOG" 2>/dev/null || echo 0)"
    case "$log_time" in ''|0|-|*[!0-9]*) log_time="$(stat -c %Y "$GAME_LOG" 2>/dev/null || echo 0)" ;; esac
    [ "$log_time" -ge "$floor" ] 2>/dev/null && fresh=1
    # Exact provenance when the log has it. ds2-loader's first line names the commit it was built
    # from (`build git=<sha>`, `-dirty` when crates/ differed). When it is there, times are not
    # asked at all: the run covers a ref when its commit is an ancestor of the ref and no commit
    # between them touches `crates/`. A dirty build covers nothing.
    #
    # The stamp names the DLL's commit, not the launcher's (2026-09-27). `build git=` is what
    # `crates/ds2-loader/build.rs` saw when the DLL was built, and a commit that changes only
    # `scripts/ds2-run.py` changes no DLL byte, so the stamp can name its parent while the run was in
    # fact launched by it. Measured: branch ds2-run-builds-launcher, HEAD 4bb7302 (12:16:16, launcher
    # only) on e73635e; staged DLL 12:16:22, sha256 equal to the worktree's build, stamped e73635e;
    # log born 12:16:28 with `ds2-loader: attach`. The time test said fresh, and this arm overrode it
    # with fresh=0 because 4bb7302 touches the launcher. The launcher is not in the stamp, so for
    # launcher-only commits after it the clock is the only evidence there is: the run's birth must
    # postdate the newest of them, which is what the time test demands anyway.
    run_sha="$(head -1 "$GAME_LOG" 2>/dev/null | sed -n 's/.* build git=\([0-9a-f]\{40\}\)\(-dirty\)\{0,1\} .*/\1\2/p')"
    if [ -n "$run_sha" ]; then
        fresh=1
        case "$run_sha" in *-dirty) fresh=0 ;; esac
        sha="${run_sha%-dirty}"
        while IFS= read -r ref; do
            [ -n "$ref" ] || continue
            git rev-parse --verify --quiet "$ref^{commit}" >/dev/null 2>&1 || ref="HEAD"
            if ! git merge-base --is-ancestor "$sha" "$ref" 2>/dev/null; then fresh=0; fi
            if [ -n "$(git log --format=%H "$sha..$ref" -- crates 2>/dev/null | head -1)" ]; then fresh=0; fi
            launcher_time="$(git log -1 --format=%ct "$sha..$ref" -- scripts/ds2-run.py 2>/dev/null)" || launcher_time=""
            if [ -n "$launcher_time" ] && ! [ "$log_time" -ge "$launcher_time" ] 2>/dev/null; then fresh=0; fi
        done <<EOF_RUN_REFS
$refs
EOF_RUN_REFS
    fi
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

printf 'RUNTIME|game_code=%d|attached=%d|fresh=%d|dll_match=%d|pending=%d|head=%s|log=%s\n' \
    "$game_code" "$attached" "$fresh" "$dll_match" "$pending" "$head_time" "$log_time"
