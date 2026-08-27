#!/usr/bin/env bash
# Repo gate. Run before pushing a branch; CI runs the same thing.
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
fi

echo "== OK =="
