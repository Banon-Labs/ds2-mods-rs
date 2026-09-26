#!/usr/bin/env bash
# Repo gate. Run before pushing a branch.
#
# NOTHING ELSE RUNS THIS. There is no `.github/` in this repo, so every green gate here has been
# someone running it by hand, and a pull request can be merged without any of it (`ds2-mods-rs-e60`).
# This line used to claim CI ran the same thing, which was never true and is the sort of claim that
# makes a reviewer skip the check themselves.
#
# Everything in `crates/` ships as a Windows DLL or is linked into one, so the MSVC target is
# the gate that matters and it is the one clippy runs against. Host `cargo test` is a second,
# narrower pass for the crates that are game-free by construction -- pass `--host-tests` to
# include it. It is not run by default because a workspace-wide host build fails the moment a
# `cfg(windows)`-only crate is added, which is not a lint failure and should not read as one.
set -euo pipefail

cd "$(dirname "$0")/.."
TARGET=x86_64-pc-windows-msvc
run_host_tests=0
[[ "${1:-}" == "--host-tests" ]] && run_host_tests=1

# RECORD THE RUN, pass or fail, so `scripts/pr-run-stamp.py --from-last-check` can print the PR
# footer's `Run-Stamp:` line from what actually ran instead of from what an agent types. One file per
# checkout (`git rev-parse --git-path` resolves inside a worktree's own git dir), overwritten each run.
# `tree=dirty` means uncommitted changes were in the tree, so the run did not test the commit it names
# and the helper refuses to stamp from it. Written by the EXIT trap so a failure is recorded as well.
check_record=$(git rev-parse --path-format=absolute --git-path ds2-last-check)
check_sha=$(git rev-parse HEAD)
check_tree=clean
[[ -n "$(git status --porcelain --untracked-files=no)" ]] && check_tree=dirty
check_gate=check.sh
(( run_host_tests )) && check_gate=check.sh+host-tests
record_check_run() {
  local rc=$? result=fail
  (( rc == 0 )) && result=pass
  printf 'sha=%s at=%s gate=%s result=%s tree=%s\n' \
    "$check_sha" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$check_gate" "$result" "$check_tree" >"$check_record"
}
trap record_check_run EXIT

echo "== commit messages =="
# Conventional commits, checked in two places because one is not enough. `.beads/hooks/commit-msg`
# catches a message as it is written, and catches nothing at all in a checkout where no hooks are
# installed -- a fresh clone has none until beads or a human wires them up. This catches the branch
# either way, which is the check that actually gates a push. docs/COMMITS.md is the convention.
# First in the gate because it costs a second and clippy costs minutes.
python3 scripts/check-commit-message.py --selftest
if git rev-parse --verify --quiet origin/main >/dev/null; then
  commit_base=$(git merge-base origin/main HEAD)
  python3 scripts/check-commit-message.py --range "$commit_base..HEAD"
else
  echo "  no origin/main in this checkout -- the branch's own commits were not checked"
fi

# The pull request template is not this repo's to shape freely. A global policy guard refuses a PR
# body that lacks these three headings, spelled exactly, or that runs past 2500 characters -- it
# refused the one that opened this feature, because the headings here were written from taste
# instead of read off the guard. A template carrying the wrong headings would hand that same block
# to everyone who fills it in, so the gate pins it.
pr_template=.github/pull_request_template.md
for heading in '## What changed' '## Why' '## Evidence'; do
  if ! grep -qxF "$heading" "$pr_template"; then
    echo "  $pr_template is missing the heading '$heading' -- a PR filled from it will be blocked" >&2
    exit 1
  fi
done
template_size=$(wc -c <"$pr_template")
if [[ "$template_size" -ge 2500 ]]; then
  echo "  $pr_template is at or over the 2500-character body cap before anyone fills it in" >&2
  exit 1
fi
echo "  $pr_template: OK"

echo "== lint allows =="
# Every `#[allow]` in `crates/` names the issue that will remove it. An allow switches off a lint
# the root manifest denies on purpose, so it is a hole in this gate, and a hole nobody is tracking
# cannot be told from one nobody noticed. docs/COMMENTS.md is the convention. Cheap, so it runs
# before clippy.
python3 scripts/check-allow-debt.py --selftest
python3 scripts/check-allow-debt.py

echo "== fresh-run logs =="
# A log describes exactly one process run. ds2-game-base::log has always said so and said, too,
# that nothing here enforced it. Every appending opener in crates/ must route through its one-shot
# truncation or be exempt with a reason. Selftest first, so the gate is never trusted on its own
# say-so.
python3 scripts/check-fresh-run-logs.py --selftest
python3 scripts/check-fresh-run-logs.py

echo "== rustfmt =="
# NOT `cargo fmt --all`. `--all` is documented as "format all packages, AND ALSO THEIR LOCAL
# PATH-BASED DEPENDENCIES", so the moment a crate here depended on `../dearxan` the gate started
# checking someone else's checkout and failing on their brace style. Same principle as `--no-deps`
# on the clippy line below: a path dependency outside this workspace does not get a vote on our
# gate. `cargo metadata --no-deps` lists workspace members only, so this keeps working with the
# members glob in Cargo.toml -- a new crate needs no edit here.
members=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print("\n".join(p["name"] for p in json.load(sys.stdin)["packages"]))')
# shellcheck disable=SC2086  # deliberate word splitting: one -p per member.
cargo fmt $(printf -- '-p %s ' $members) -- --check

echo "== clippy ($TARGET) =="
# `--all-targets` so tests and examples are linted too, `--no-deps` so a warning in a path
# dependency outside this workspace does not fail our gate.
cargo xwin clippy --workspace --all-targets --no-deps --target "$TARGET"

echo "== docs ($TARGET) =="
# THIS STEP EXISTS FOR ONE LINT. `docs/COMMENTS.md` says a comment naming an address in
# `DarkSoulsII.exe` names its `ds2_rva` constant as an intra-doc link, so that deleting or
# renaming the entry breaks the build instead of leaving a comment quietly describing an address
# that moved. `rustdoc::broken_intra_doc_links` is what breaks it, and clippy never runs rustdoc
# -- without this line the rule in the root manifest is denied and never evaluated.
#
# MEASURED: a doc comment containing `[notes](../../../docs/GONE.md)` documents CLEAN under
# `#![deny(warnings)]`. rustdoc has no lint for a dead relative path, which is why the convention
# spends a plain-text filename there and checks it below rather than writing a link that rots in
# silence.
cargo xwin doc $(printf -- '-p %s ' $members) --no-deps --target "$TARGET" --quiet

echo "== docs/ references =="
# Every `docs/<name>.md` named from a comment must name a file that exists. This is the other
# half of the same rule: the markdown filename is plain text precisely because rustdoc will not
# check it, so something has to.
missing=0
while read -r ref; do
  [[ -f "$ref" ]] && continue
  echo "  $ref is named in a comment and does not exist" >&2
  missing=1
done < <(grep -rhoE 'docs/[A-Za-z0-9._-]+\.md' crates --include='*.rs' | sort -u)
(( missing )) && exit 1
echo "  OK"

if (( run_host_tests )); then
  echo "== host tests =="
  # SCOPED, NOT `--workspace`, and this is the failure the block comment above predicted actually
  # happening: a crate whose `mod` declarations are `#[cfg(windows)]` does not compile for the host
  # at all, so `cargo test --workspace` fails with `E0433` on `ds2-continue` before running a single
  # test. That is not a lint failure and it is not a regression -- it is the workspace containing
  # crates that are Windows by construction, which is the normal state here.
  #
  # So the host pass names the crates that are game-free BY CONSTRUCTION and can therefore be
  # exercised without Windows. A crate belongs on this line only if it has no `cfg(windows)` gate;
  # everything else is covered by the wine pass below.
  cargo test -p ds2-sl2-core -p ds2-hotkey-config -p ds2-safe-input -p ds2-crash-logging-core \
    -p ds2-build-import-core -p ds2-save-file-core -p ds2-save-picker-core

  echo "== windows-target tests (wine) =="
  # THE CRATES THAT MATTER MOST WERE THE ONES WITH NO EXECUTABLE TESTS. `ds2-loader` is
  # `#![cfg(windows)]` at the crate root, so `cargo test --workspace` above compiles NONE of it and
  # reports `0 passed` -- a green line that means "nothing ran", which reads identically to
  # "everything passed". `arxan_probe` shipped with zero tests under that cover and nobody could
  # have noticed from the gate output.
  #
  # The fix is not to relax the cfg. These DLLs genuinely are Windows-only. It is to RUN the
  # Windows test binaries, which wine does perfectly well for pure-logic tests -- config parsing,
  # classification, formatting -- none of which touch the game.
  #
  # Same contract as the opa step below: a missing runner is a HARD FAILURE with the fix printed,
  # never a silent skip, because a check that quietly does nothing still looks enforced. Opt out
  # deliberately and visibly with WINE_SKIP=1.
  if [[ -n "${WINE_SKIP:-}" ]]; then
    echo "  SKIPPED (WINE_SKIP set)"
  else
    command -v wine >/dev/null 2>&1 || {
      echo "wine not found -- needed to execute the cfg(windows) crates' tests" >&2
      echo "install it, or re-run with WINE_SKIP=1 to deliberately skip them." >&2
      exit 1
    }
    cargo xwin build --workspace --tests --target "$TARGET" 2>/dev/null
    ran=0
    for exe in target/"$TARGET"/debug/deps/*.exe; do
      # cargo leaves older hashed binaries behind; only run what this build just produced.
      [[ "$exe" -nt Cargo.toml ]] || continue
      name=$(basename "$exe")
      out=$(WINEDEBUG=-all wine "$exe" 2>/dev/null) || {
        echo "$out" >&2
        echo "  FAILED: $name" >&2
        exit 1
      }
      result=$(echo "$out" | grep -oE 'test result: ok\. [0-9]+ passed' | head -1)
      # A binary with no tests prints nothing useful; say so rather than implying it passed.
      echo "  ${name%%-*}: ${result:-no tests}"
      ran=$((ran + 1))
    done
    (( ran > 0 )) || { echo "  no windows test binaries found -- did the build succeed?" >&2; exit 1; }
  fi
fi

echo "== cupcake policies =="
# The .cupcake/ tree is executable enforcement, so it gets a gate like any other code.
#
# NOT `command -v opa >/dev/null && opa test ...`. A guard that silently does nothing when its
# runner is absent is worse than no guard, because it still LOOKS enforced -- which is exactly
# how `rulebook_security_guardrails` sat configured-but-uncompiled in the sibling repo without
# anyone noticing (it needs an explicit `enabled: true`; configuring it is not enough in cupcake
# 0.5.2). So a missing opa is a hard failure with the fix printed, and CUPCAKE_SKIP=1 is the
# deliberate, visible opt-out.
if [[ -n "${CUPCAKE_SKIP:-}" ]]; then
  echo "  SKIPPED (CUPCAKE_SKIP set)"
else
  command -v opa >/dev/null 2>&1 || {
    echo "opa not found -- install from https://github.com/open-policy-agent/opa/releases" >&2
    echo "or re-run with CUPCAKE_SKIP=1 to deliberately skip the policy gate." >&2
    exit 1
  }
  # Each test runs against ONLY its own policy plus the shared commands.rego helper, never the
  # whole tree at once: `opa test` over every package together lets one policy's rules satisfy
  # another's assertions, which turns a red test green for the wrong reason. Tests are discovered
  # from the filesystem so a new policy needs no edit here -- same reasoning as the members glob
  # above. `commands_test.rego` tests the helper itself and has no policy of its own.
  shopt -s nullglob
  for t in .cupcake/tests/*_test.rego; do
    name=$(basename "$t" _test.rego)
    policy=""
    for candidate in ".cupcake/policies/claude/$name.rego" ".cupcake/policies/claude/builtins/$name.rego"; do
      [[ -f "$candidate" ]] && policy="$candidate" && break
    done
    # shellcheck disable=SC2086  # $policy is one path or deliberately empty (commands_test).
    result=$(opa test .cupcake/system/commands.rego $policy "$t" 2>&1) || {
      echo "$result" >&2; exit 1
    }
    echo "  $name: $(echo "$result" | grep -oE 'PASS: [0-9]+/[0-9]+' | head -1)"
  done
  # A SIGNAL THAT IS NOT EXECUTABLE IS A GUARD THAT IS NOT ENFORCED, and it looks exactly like a
  # clean turn. Measured 2026-09-23: last_assistant_status_table.sh shipped mode 644, so cupcake
  # could not run it, the policy saw an empty signal, and the Stop hook returned `{}` -- an ALLOW --
  # on the very transcript the guard was written for, while `opa test` stayed green at 20/20. Same
  # silence as the 36-day episode the block below describes, one chmod wide.
  nonexec=$(find .cupcake/signals -name '*.sh' ! -perm -u+x -print)
  if [[ -n "$nonexec" ]]; then
    echo "cupcake signal scripts are not executable -- the policies reading them are INERT:" >&2
    echo "$nonexec" >&2
    echo "fix: chmod +x <file>" >&2
    exit 1
  fi
  echo "  signals: executable"
  # `opa test` proves the RULES are right; it does not prove cupcake can LOAD them. Verify
  # compiles the project tree to WASM, which is what the live PreToolUse hook actually evaluates.
  cupcake verify --harness claude --log-level error >/dev/null
  echo "  wasm: compiled"
  # Third layer: drive the real `cupcake eval` binary with real PreToolUse events. `opa test`
  # can be green while the deployed pipeline still says something else -- different event shape,
  # a signal that never fires, a policy that compiles but never routes. This is the only step
  # that exercises what the live PreToolUse hook actually runs.
  python3 scripts/test-cupcake-policies.py
  # Same third layer for the STOP hook, which has its own way of being silently dead: `cupcake
  # eval` runs the policies as WASM, not in the OPA interpreter, and a builtin the WASM runtime
  # cannot execute yields undefined -- the rule never fires and the verdict is an ordinary ALLOW.
  # That is how every Stop guard in the sibling repo sat inert for 36 days with a green suite.
  # These two steps are what make "the Stop guard is enforced" a measurement rather than a claim:
  # the first refuses any builtin it has not watched survive the real runtime, the second drives
  # a real transcript through the hook command out of .claude/settings.json.
  python3 scripts/check-cupcake-wasm-builtins.py
  # A policy whose verb evaluate.rego does not name is INERT: it loads, routes, evaluates, and its
  # decision is discarded. Measured 2026-09-23 on no_unchecked_game_alive_claim, which shipped with
  # 8/8 opa tests, a passing signal test, `cupcake verify` green and its name in the routing map,
  # and could not halt anything. evaluate.rego's own comment claimed the builtins check above caught
  # this; it did not, so this is the check that comment describes.
  python3 scripts/check-cupcake-routed-verbs.py
  python3 scripts/test-cupcake-stop-guards.py
  # The signal is the half that decides WHICH turns are violations, and it is where every
  # false-positive carve-out lives. `opa test` above only pins the policy's tag -> halt mapping.
  python3 scripts/test-unexecuted-promise-signal.py
  # Same half for the status-table guard: scripts/cupcake_status_table.py owns what counts as a
  # table at all (a separator row is required, so a shell pipeline in prose is not one), which
  # cells are progress words, and which prompts asked for a grid. Every false positive this guard
  # can have is a decision made in that module, and `opa test` above only pins hits -> halt.
  python3 scripts/test-status-table-signal.py
  # And the same half for the shouting guard, which has one extra way to go wrong: the offence is
  # defined twice, in Rego for the edit about to hit disk and in Python for the prose about to end
  # a turn. Two definitions that drift apart are worse than one, because the disagreement is
  # invisible until somebody is refused by one arm and waved through by the other, so this compares
  # the patterns across all three files byte for byte as well as pinning the classification.
  python3 scripts/test-shouting-signal.py
  # And the same half for the live-game guard, whose entire decision is a tense: "I launched it" is
  # provable and allowed, "the game is up" is a claim about now and needs a check in the same turn.
  # The lexicon and the liveness-command list live in scripts/cupcake_game_alive.py; `opa test`
  # above only pins that a spoken signal halts.
  python3 scripts/test-game-alive-signal.py
  # And the same half for the property-grant guard, whose whole decision is idiom recognition: a
  # closing sentence that stages the agent handing the user control over something already theirs.
  # The lexicon and the quoting carve-out live in scripts/cupcake_property_grant.py; `opa test`
  # above only pins that a spoken signal halts.
  python3 scripts/test-property-grant-signal.py
  # And the PR Run-Stamp guard's deciding half: which body earns which verdict at `gh pr create` and
  # `gh pr ready`, and the helper that prints the stamp. `opa test` above pins verdict -> denial.
  python3 scripts/test-run-stamp.py
  # The runtime push guard's deciding half, against throwaway repositories and the real engine. The
  # policy was right on 2026-09-25 and the signal handed it `game_code=0` for a push of the
  # launcher, because `git commit ... && git push` was one command and the commit did not exist yet
  # when the hook looked. `opa test` above cannot see that; this can.
  python3 scripts/cupcake_push_scope.py --selftest | tail -1
  python3 scripts/test-runtime-evidence-signal.py
  # The fix-claim guard's deciding half. Its Rego suite pins what the policy does with a facts line;
  # this pins where the facts line comes from -- which sentences are claims, which artifacts count
  # as a run, and which crates reach a DLL. The crate walk is the part that can go silently inert.
  python3 scripts/test-fix-claim-classifier.py | grep -v '^  ok '
  # The own-rule guard's classifier, which is where its one divergence from er-mods-rs lives:
  # `push` is not a user-owned action here, and the selftest pins that saying so never halts.
  python3 scripts/cupcake_user_own_rule.py
  # The hook shim is the fourth place this layer can be silently dead, and the one no `.rego` file
  # can reach. scripts/cupcake-hook.sh sits between Claude Code and the engine and repairs three
  # things the engine gets wrong before any policy runs: a permission mode cupcake does not know
  # (fatal -- every hook goes inert), the unquoted newlines the engine erases (line 2 of a Bash
  # command arrives with no separator in front of it, so every command-position anchor misses it),
  # and a --global-config given a file path where the engine wants a directory (loads project-only
  # and reports success at DEBUG, which --log-level error hides). Each is invisible to `opa test`,
  # because the text the policies are tested against is not the text the engine delivers.
  python3 scripts/test-cupcake-hook-shim.py
  # And the DELIVERED shape: the engine rewrites a command before any policy sees it (whitespace
  # normalised, unquoted newlines erased, a heredoc body welded onto its reader), so a test written
  # against the text a human typed can be green while the rule never fires on what arrives. This
  # gate drives the real engine and pins the difference.
  python3 scripts/test-cupcake-delivered-shape.py
fi
# The Monitor guard refuses any unthrottled stream and names scripts/monitor-throttle.py as the fix.
# Until 2026-09-25 that file existed only in er-mods-rs, so the guard's one sanctioned shape could
# not run here and the workaround was an absolute path into the other repository. Pure python, so
# it runs whether or not the engine is installed.
python3 scripts/monitor-throttle.py --selftest >/dev/null
echo "  monitor-throttle.py: OK"

echo "== launcher selftest =="
# scripts/ds2-run.py decides whether a runtime run is reported as evidence or as silence, and it
# is the one part of this repo that can turn a HEALTHY run into a reported failure. It did exactly
# that on 2026-08-27: `await_testimony` returned from the middle of a read chunk and discarded the
# probe's install lines, so the first real M1 run reported "the probe never installed" over a log
# that plainly contained the install. The script had a selftest covering that class of bug; the
# selftest was simply never wired into the gate. It is now.
python3 scripts/ds2-run.py --selftest >/dev/null
echo "  ds2-run.py: OK"
# And the injector's own entry point. `crates/ds2-launcher` decides which DLLs go into the game
# and in what order, and gets exactly one attempt per launch -- a plan that silently came out
# empty would produce a session with no mods in it and no error saying so. The Windows half
# cannot run here; this is the planning half, which is the half that can be wrong quietly.
cargo run --quiet -p ds2-launcher -- --selftest >/dev/null
echo "  ds2-launcher: OK"
# The live roster reader. `ds2-invasion-path` decides who to draw a path to by reading a name out
# of a `std::wstring`, and this script reads the same fields the same way -- so a wrong reader
# here is a wrong reader in the crate, discovered against a fixture instead of against a session.
# Its first draft failed on its own fixture because `ctypes.create_unicode_buffer` is UCS-4 on
# Linux; that is exactly the class of mistake this line catches before a run is spent on it.
python3 scripts/ds2-player-kind.py --selftest >/dev/null
echo "  ds2-player-kind.py: OK"
# The Frida watcher, which is how a crate change gets its measurement. Its selftest was never wired
# in here, and it had been failing since the port: it defaulted to er-mods-rs's session agent, a
# file this repository never had.
python3 scripts/ds2-frida-watch.py --selftest >/dev/null
echo "  ds2-frida-watch.py: OK"

echo "== OK =="
