#!/usr/bin/env bash
# Cupcake signal: last_assistant_mergeable_claim
#
# Consumed by:
#   * no_mergeable_without_green_ci (Stop): halts a turn that closed by calling the pull request
#     mergeable while CI is not passing.
#
# Ported from er-mods-rs on 2026-09-25, and changed in two places on the way in:
#
#   * The er copy never ran to completion. Its classifier read `turn.text(turn.last_text_index)`,
#     but `Turn.text` in scripts/cupcake_turn_scan.py is a property, so the call raised TypeError,
#     `2>/dev/null || true` swallowed it, and the signal printed nothing on every turn -- the guard
#     was inert there with a green `opa test`, because the policy was only ever handed typed facts.
#     This reads the closing prose run the way every other Stop signal does, and
#     .cupcake/tests/fixtures/mergeable_claim*.jsonl drive it through the real hook.
#   * The CI read happens only when the prose makes the claim. The er copy ran `gh pr checks` on
#     every Stop, which is a network call per turn to decide a question nobody asked.
#
# Why the rule exists, from er-mods-rs 2026-09-11: a turn closed "PR #426 is **MERGEABLE** now --
# the conflicts are gone." while the `check` job was `in_progress` and had never passed. `mergeable`
# is GitHub's three-way merge result -- does the branch apply without a textual conflict -- and knows
# nothing about builds or tests. This repo has CI to be green or not: .github/workflows/release.yml
# builds the shipped DLLs on every pull request.
#
# Emits  MERGEABLECLAIM:<claimed>:<verdict>  with verdict PASS / PENDING / FAIL / UNKNOWN, and
# nothing when the transcript cannot be read or the closing prose makes no claim.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT

claimed="$(python3 - <<'PY' 2>/dev/null
import os, re, sys

sys.path.insert(0, os.path.join(os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", "."), "scripts"))
try:
    import cupcake_turn_scan as scan
except Exception:
    sys.exit(0)  # fail open: a missing helper must never wedge a session

path = scan.latest_transcript()
if not path:
    sys.exit(0)
turn = scan.last_text_turn(scan.split_turns(scan.load_events(path)))
if turn is None:
    sys.exit(0)

# A turn that kept working past its last prose did not end on that prose.
if turn.last_text_index < 0 or turn.tool_after(turn.last_text_index):
    sys.exit(0)
runs = turn.text_runs
if not runs:
    sys.exit(0)
text = runs[-1]

# Quoted spans and code fences are how a policy's own wording, a log line, or `gh` output gets
# reproduced. Reporting what a tool printed is not the same as adopting it as the verdict.
scrubbed = re.sub(r"```.*?```", " ", text, flags=re.DOTALL)
scrubbed = re.sub(r"`[^`]*`", " ", scrubbed)
scrubbed = re.sub(r'"[^"]*"', " ", scrubbed)

CLAIM_RE = re.compile(
    r"(?<!not\s)(?<!isn't\s)(?<!is\snot\s)\bmerge-?able\b"
    r"|\bready\s+to\s+merge\b"
    r"|\bsafe\s+to\s+merge\b"
    r"|\bthe\s+conflicts?\s+(?:are|is)\s+gone\b",
    re.IGNORECASE,
)
NEGATED_RE = re.compile(
    r"\b(?:not|never|isn't|is\s+not|no\s+longer|un)\s*merge-?able\b"
    r"|\bmerge-?able\s+(?:is|was)\s+(?:false|no)\b",
    re.IGNORECASE,
)
if CLAIM_RE.search(scrubbed) and not NEGATED_RE.search(scrubbed):
    print("1")
PY
)"
[ "$claimed" = "1" ] || exit 0

# The regression tests pin the verdict the same way CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE pins the
# runtime signal, so no case depends on the network or on which PR the checkout happens to have.
verdict="${CUPCAKE_MERGEABLE_CI_VERDICT_OVERRIDE:-}"
if [ -z "$verdict" ]; then
    verdict="UNKNOWN"
    if command -v gh >/dev/null 2>&1 && command -v git >/dev/null 2>&1; then
        branch="$(git branch --show-current 2>/dev/null || true)"
        if [ -n "$branch" ]; then
            # `gh pr checks` exits non-zero whenever anything is failing or pending, so the exit
            # code is ignored and the verdict comes from the rows.
            rows="$(timeout 8 gh pr checks "$branch" --json name,state 2>/dev/null || true)"
            if [ -n "$rows" ]; then
                verdict="$(printf '%s' "$rows" | python3 -c '
import json, sys
try:
    rows = json.load(sys.stdin)
except Exception:
    print("UNKNOWN"); raise SystemExit(0)
states = [str(r.get("state", "")).upper() for r in rows]
live = [s for s in states if s not in ("SKIPPED", "NEUTRAL")]
if not live:
    print("UNKNOWN")
elif any(s in ("FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED") for s in live):
    print("FAIL")
elif any(s in ("PENDING", "QUEUED", "IN_PROGRESS", "WAITING", "REQUESTED", "EXPECTED") for s in live):
    print("PENDING")
elif all(s == "SUCCESS" for s in live):
    print("PASS")
else:
    print("UNKNOWN")
' 2>/dev/null || true)"
            fi
        fi
    fi
fi
[ -n "$verdict" ] || verdict="UNKNOWN"
printf 'MERGEABLECLAIM:1:%s\n' "$verdict"
exit 0
