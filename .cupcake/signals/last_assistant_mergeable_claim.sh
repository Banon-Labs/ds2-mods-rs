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
# Which PR is measured (2026-09-26): the one the claim names, not the one on the branch the
# session's cwd happens to be on. The instance: a closer said "PR #216 is ready to merge once you
# run `gh pr ready 216`" with #216's build, check and host-tests all SUCCESS, from a worktree on
# another branch whose PR #217 was still pending -- and the halt fired on #217's CI. So the PR
# numbers written in the claiming sentence(s) (#N, PR N, pull request N, `gh pr <verb> N`) are
# measured and every one of them must be PASS; a number elsewhere in the closer is next; the cwd
# branch's PR is the fallback only when the closing prose names no number at all. Numbers are read
# with backtick spans intact, because the `gh pr ready 216` that names the PR is exactly the kind
# of span the claim scrub removes.
#
# Emits  MERGEABLECLAIM:<claimed>:<verdict>:<targets>  with verdict PASS / PENDING / FAIL / UNKNOWN
# (the worst across the targets) and targets a comma list of PR numbers or `branch`, and nothing
# when the transcript cannot be read or the closing prose makes no claim.
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
def scrub(t):
    t = re.sub(r"```.*?```", " ", t, flags=re.DOTALL)
    t = re.sub(r"`[^`]*`", " ", t)
    return re.sub(r'"[^"]*"', " ", t)


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
PR_RE = re.compile(
    r"(?<![\w/&])#(\d+)\b"
    r"|\b(?:PR|pull\s+request)\s+#?(\d+)\b"
    r"|\bgh\s+pr\s+(?:ready|merge|checks|view)\s+#?(\d+)\b",
    re.IGNORECASE,
)

scrubbed = scrub(text)
if not CLAIM_RE.search(scrubbed) or NEGATED_RE.search(scrubbed):
    sys.exit(0)


def numbers(t):
    seen = []
    for m in PR_RE.finditer(t):
        n = next(g for g in m.groups() if g)
        if n not in seen:
            seen.append(n)
    return seen


# Split into sentences with code fences dropped but inline backtick spans kept, so
# `gh pr ready 216` stays in the sentence that makes the claim.
unfenced = re.sub(r"```.*?```", " ", text, flags=re.DOTALL)
sentences = [s for s in re.split(r"(?<=[.!?])\s+|\n+", unfenced) if s.strip()]
claiming = [s for s in sentences if CLAIM_RE.search(scrub(s))]
targets = numbers(" ".join(claiming)) or numbers(unfenced)
print(" ".join(["1", *targets]))
PY
)"
case "$claimed" in
    1|"1 "*) ;;
    *) exit 0 ;;
esac
read -r -a targets <<<"${claimed#1}"
[ "${#targets[@]}" -gt 0 ] || targets=(branch)

# The regression tests pin verdicts the same way CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE pins the runtime
# signal, so no case depends on the network or on which PR the checkout happens to have:
#   CUPCAKE_MERGEABLE_CI_VERDICTS_OVERRIDE="216=PASS,217=PENDING,branch=PENDING" answers per target;
#   CUPCAKE_MERGEABLE_CI_VERDICT_OVERRIDE=<verdict> answers every target the map does not name.
lookup_override() {
    local want="$1" pair
    local IFS=','
    for pair in ${CUPCAKE_MERGEABLE_CI_VERDICTS_OVERRIDE:-}; do
        pair="${pair// /}"
        if [ "${pair%%=*}" = "$want" ] && [ -n "${pair#*=}" ]; then
            printf '%s' "${pair#*=}"
            return 0
        fi
    done
    if [ -n "${CUPCAKE_MERGEABLE_CI_VERDICT_OVERRIDE:-}" ]; then
        printf '%s' "$CUPCAKE_MERGEABLE_CI_VERDICT_OVERRIDE"
        return 0
    fi
    return 1
}

measure() {
    local target="$1" ref rows
    command -v gh >/dev/null 2>&1 || { echo UNKNOWN; return; }
    if [ "$target" = "branch" ]; then
        ref="$(git branch --show-current 2>/dev/null || true)"
        [ -n "$ref" ] || { echo UNKNOWN; return; }
    else
        ref="$target"
    fi
    # `gh pr checks` exits non-zero whenever anything is failing or pending, so the exit code is
    # ignored and the verdict comes from the rows.
    rows="$(timeout 8 gh pr checks "$ref" --json name,state 2>/dev/null || true)"
    [ -n "$rows" ] || { echo UNKNOWN; return; }
    printf '%s' "$rows" | python3 -c '
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
' 2>/dev/null || echo UNKNOWN
}

# Every named PR must pass; the reported verdict is the worst one, FAIL > PENDING > UNKNOWN > PASS.
# An unrecognised verdict (NOPR, say) ranks with UNKNOWN: not a pass.
rank() { case "$1" in PASS) echo 0 ;; PENDING) echo 2 ;; FAIL) echo 3 ;; *) echo 1 ;; esac; }
worst="PASS"
for t in "${targets[@]}"; do
    v="$(lookup_override "$t" || measure "$t")"
    [ -n "$v" ] || v="UNKNOWN"
    if [ "$(rank "$v")" -gt "$(rank "$worst")" ]; then
        worst="$v"
    fi
done
joined="$(IFS=','; printf '%s' "${targets[*]}")"
printf 'MERGEABLECLAIM:1:%s:%s\n' "$worst" "$joined"
exit 0
