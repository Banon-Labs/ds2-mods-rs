#!/usr/bin/env bash
# Cupcake signal: last_assistant_fix_claim_without_runtime_evidence
#
# Scans the most recently completed assistant turn and emits one facts line when that turn changed
# code that ships inside the game, called the change a fix, and never opened anything a run wrote:
#
#   FIXFACTS|claim=..|changed=..|evidence=..|hedged=..|blocked=..|hostobject=..
#
# A clean turn emits nothing (fail-open, like every neighbouring signal).
#
# The failure this exists to refuse
# ---------------------------------
# User, 2026-09-11: "If someone ever says real fix to me and doesn't put it in airquotes, I look at
# them like this" -- and then, when nothing had stopped it: "We are supposed to have a rego policy
# that stops you from saying 'fix' without runtime evidence". The turn had edited a crate that ships
# inside a loaded DLL, built it, never launched the game, and closed on "the real fix". It failed
# live on the next launch, which is the cost the word had already spent.
#
# Ported from er-mods-rs on 2026-09-25. The history in this header is that repo's; the artifact
# names in the list below are this one's, and scripts/cupcake_fix_claim.py carries the measurement
# behind them.
#
# What counts as runtime evidence, and what does not
#   counts     -- the turn opened something a run of the game produced, at or after the edit:
#                 `<Game>/ds2-loader.log` or its `.prev`, a `ds2-crash-<name>.txt`,
#                 `scripts/ds2-teardown.py --status`, an attached `scripts/ds2-frida-watch.py` or
#                 `scripts/ds2-stop-works.py`, or a live read through `scripts/ds2-player-kind.py`,
#                 `scripts/ds2-io-sample.py` or `scripts/ds2-harness-liveness.py`.
#   never       -- a build, however green. `cargo xwin build`, `cargo test`, `cargo check`,
#                 `scripts/check.sh` and a `sha256sum` of the artifact all say the code compiles.
#                 A launch says the game starts: `scripts/ds2-run.py` is not in the allowlist
#                 either, because a DLL attaching is not the change being observed.
#   never       -- arming a watch on one, which is the 2026-09-13 miss. A `Monitor`, a
#                 `run_in_background` call, and a `tail -f`/`nohup`/`setsid` command all name the
#                 artifact and have read none of it. That turn closed on "Fixed and relaunched as
#                 07f2729b" with its last call a `Monitor` on `tail -F
#                 er-quickload-autoload-debug.log` -- a log the process it had just started had not
#                 written a line of -- and the filename inside the tool input read as evidence. The
#                 same exclusion applies to the prose: "the monitor will tell me if `muted=true`
#                 appears" is the promise of a measurement, not one.
#
# Ordering is half the rule. Evidence counts only from the first edit onward, because a log read
# before the change describes the code that was there before it. Measured from the first such edit
# rather than the last, so a turn that reads the log and then makes one more small edit still counts
# as having looked -- the lenient reading of a rule whose false positive gags an honest report. The
# one place the anchor does move forward is a rebuild: a measurement taken before the artifact was
# built again describes the previous artifact, and the claim is about the one that is loaded now.
# A `cargo check` or a test run produces no artifact and moves nothing.
#
# The exemptions, each a case where the word is being used honestly
#   hedged     -- the closing prose says somewhere that the change is unverified, untested, has not
#                 run, needs a run, or is ready for the next one. An honest hedge is the behaviour
#                 being asked for and must never be punished. Scanned over the whole closing run,
#                 not the matched sentence, so a claim in one paragraph and its hedge in the next
#                 still passes.
#   blocked    -- a dependency the agent cannot dissolve: sudo, a credential, an observation only
#                 the user can make, or a decision handed back to them.
#   hostobject -- the sentence says a gate, a check, a test or a lint was fixed and names nothing
#                 that lives inside the game. A green gate is the proof of host work, and demanding
#                 a game launch for a clippy lint is how a guard earns a reputation for being wrong.
#   changed=0  -- the turn edited no crate that can reach a DLL. Host work under `scripts/`, `docs/`
#                 or `.cupcake/`, and the two host-only crate directories, are all outside the rule:
#                 a run cannot show any of them working or failing.
#
# Why the neighbouring Stop guards do not catch it
#   * `last_assistant_proof_without_observation` reads one word, `proven`, measured against the
#     corpus. A turn that says "the real fix" never says "proven", which is the whole reason the
#     stronger word gets avoided.
#   * `last_assistant_unbacked_claim` asks whether an artifact the turn claims to have built exists.
#     Here it does exist -- the edit was real, and only the claim about its effect is unbacked.
#   * `last_assistant_diagnosis_without_fix` turns on whether a file changed. One did.
#   * `git_require_runtime_evidence` gates `git push`, not prose, so a turn that says it and never
#     pushes is invisible to it.
#
# Fenced code, backtick spans and double-quoted spans are stripped before matching, so quoting this
# file, the policy, or the user's own words cannot trip it -- and a scare-quoted "fix" stops being a
# claim for free, which is the airquotes the directive asks for. The shared half (transcript
# discovery, turn bucketing, prose runs) comes from `scripts/cupcake_turn_scan.py` and the
# classification from `scripts/cupcake_fix_claim.py`, so the guards cannot drift into disagreeing
# about the same turn. Fail-open (empty output) on any error.
#
# Measured before it shipped, and re-measured 2026-09-13 after the widening, over the real closing
# turns in ~/.claude/projects/-home-banon-projects-er-mods-rs*/*.jsonl. Now: 2,820 closing turns,
# 299 of them call something a fix, three halt -- the hook change this rule shipped for, and two
# from the session that prompted the widening. The first pass of that widening halted five, and the
# two it should not have are pinned as negatives in `scripts/test-fix-claim-classifier.py`: a turn
# that said "This build doesn't fix that case -- it makes it legible", and one that settled a
# compile against a pinned upstream revision. Both corrections were made to the vocabulary rather
# than to the conjunction, and re-measured rather than argued.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, sys

repo_root = os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", ".")
sys.path.insert(0, os.path.join(repo_root, "scripts"))
try:
    import cupcake_turn_scan as scan
    import cupcake_fix_claim as fc
except Exception:
    sys.exit(0)  # fail open: a missing helper must never wedge a session

path = scan.latest_transcript()
if not path:
    sys.exit(0)
turn = scan.last_text_turn(scan.split_turns(scan.load_events(path)))
if turn is None:
    sys.exit(0)

# A turn that kept going past its last prose did not end on that prose, so the sentence was a
# working note between two tool calls rather than a claim delivered to anyone.
if turn.last_text_index < 0 or turn.tool_after(turn.last_text_index):
    sys.exit(0)

runs = turn.text_runs
if not runs:
    sys.exit(0)
closing = runs[-1]

claim = fc.fix_claim(closing)
if not claim:
    sys.exit(0)

changed, evidence = fc.runtime_evidence(turn.blocks, fc.runtime_crates(repo_root))
# The prose citing a run artifact counts as evidence on its own. Searched unscrubbed over the whole
# turn, the way the proof-without-observation guard searches its own evidence half: a run id most
# often arrives inside a fenced block, and a generous evidence search fails toward silence.
cited = evidence or fc.cites_run_artifact(turn.text)

print(
    "FIXFACTS|claim={}|changed={}|evidence={}|hedged={}|blocked={}|hostobject={}".format(
        claim,
        int(changed),
        int(cited),
        int(fc.hedged(closing)),
        int(fc.externally_blocked(closing)),
        int(fc.host_object(claim)),
    )
)
PY
