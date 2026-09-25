#!/usr/bin/env bash
# Has a Frida session actually observed something since the last commit?
#
# Read by `.cupcake/policies/claude/no_rust_edit_without_frida_proof.rego`, which refuses a Rust
# edit under `crates/` until this prints `PROVEN`. The judgement lives in
# `scripts/ds2-frida-evidence.py --check`, not here, so the rule and its selftest are one thing.
#
# Fails closed, unlike the other signals in this directory. Printing nothing means the policy sees
# no proof and denies, which is the whole point: a broken evidence reader must not quietly hand out
# permission to edit. Absence here means "no measurement was taken". Ported from er-mods-rs on
# 2026-09-25 with the reader renamed to `ds2-frida-evidence.py`.
set -uo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)" || exit 0
reader="$repo_root/scripts/ds2-frida-evidence.py"
[ -f "$reader" ] || exit 0

timeout 15 python3 "$reader" --check 2>/dev/null || true
exit 0
