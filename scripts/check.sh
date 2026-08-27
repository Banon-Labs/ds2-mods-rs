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

if (( run_host_tests )); then
  echo "== host tests =="
  cargo test --workspace

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
  # `opa test` proves the RULES are right; it does not prove cupcake can LOAD them. Verify
  # compiles the project tree to WASM, which is what the live PreToolUse hook actually evaluates.
  cupcake verify --harness claude --log-level error >/dev/null
  echo "  wasm: compiled"
  # Third layer: drive the real `cupcake eval` binary with real PreToolUse events. `opa test`
  # can be green while the deployed pipeline still says something else -- different event shape,
  # a signal that never fires, a policy that compiles but never routes. This is the only step
  # that exercises what the live PreToolUse hook actually runs.
  python3 scripts/test-cupcake-policies.py
fi

echo "== launcher selftest =="
# scripts/ds2-run.py decides whether a runtime run is reported as evidence or as silence, and it
# is the one part of this repo that can turn a HEALTHY run into a reported failure. It did exactly
# that on 2026-08-27: `await_testimony` returned from the middle of a read chunk and discarded the
# probe's install lines, so the first real M1 run reported "the probe never installed" over a log
# that plainly contained the install. The script had a selftest covering that class of bug; the
# selftest was simply never wired into the gate. It is now.
python3 scripts/ds2-run.py --selftest >/dev/null
echo "  ds2-run.py: OK"

echo "== OK =="
