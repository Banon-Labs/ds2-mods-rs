#!/usr/bin/env python3
"""Regression for the fix-claim classifier, in both directions.

Ported from `../er-mods-rs/scripts/test-fix-claim-classifier.py` on 2026-09-25. The prose cases are
verbatim, because the classifier's English half is unchanged. The tool-call fixtures, citations and
crate assertions are DARK SOULS II shapes: `ds2-loader.log` and its `.prev`, the `ds2-crash-*.txt`
family, `ds2-teardown.py --status`, `ds2-frida-watch.py`, and the workspace-dependency walk that
reaches `dinput8.dll`.

The Rego suite pins what the policy does with a facts line; this pins where the facts line comes
from. Both halves matter, and the split is deliberate: a policy can be green while the classifier
that feeds it has drifted into calling every mention of the word a claim, which is the failure mode
that makes a guard get switched off.

It imports `scripts/cupcake_fix_claim.py`, the single definition that
`.cupcake/signals/last_assistant_fix_claim_without_runtime_evidence.sh` also uses, so the test
cannot pass against a classifier production does not run.

Six things are pinned:
  * fix_claim          -- the assertive shapes convict, and a path, a slug, a fixture, a quoted or
                          backticked word, a table cell and a modal never do;
  * hedged             -- an honest admission that the change has not run exempts;
  * externally_blocked -- a dependency the agent cannot dissolve, including a decision handed back;
  * host_object        -- a sentence that fixes a gate and names nothing inside the game exempts,
                          while one that leans on a passing test does not;
  * runtime_crates     -- the walk over the workspace manifests still separates the crates that can
                          reach a DLL from the host-only ones, so the guard cannot go silently inert
                          by finding nothing to convict;
  * runtime_evidence   -- a build is not evidence, a launch is not evidence, and a log read before
                          the edit is not evidence for it.
"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from cupcake_fix_claim import (  # noqa: E402  (path set above)
    cites_run_artifact,
    externally_blocked,
    fix_claim,
    hedged,
    host_object,
    runtime_crates,
    runtime_evidence,
)

REPO_ROOT = Path(__file__).resolve().parents[1]

# The phrase the user quoted, twice, on 2026-09-11.
REAL_FIX = "That is the real fix."

CLAIM_CASES = [
    ("the verbatim phrase", REAL_FIX, True),
    ("a demonstrative copula", "This is the fix: the row was cloned before the menu existed.", True),
    ("the contracted form", "That's the fix.", True),
    ("an assertion that it works", "The fix works.", True),
    ("a transitive present tense", "This fixes the crash on the second load.", True),
    ("a bare object", "That fixes it.", True),
    ("a first-person past tense", "I fixed the ordering in the row cloner.", True),
    ("a passive report", "The ordering is fixed.", True),
    ("a sentence-initial participle", "Fixed the ordering in the row cloner.", True),
    ("a one-word report", "Fixed.", True),
    # The 2026-09-13 escape, verbatim. It closed a turn that had edited three feature-gate
    # predicates, cross-compiled and launched, in the same message as the launch and before the
    # game had written a line. The user: "Does the word 'fix' to you mean that its proven?"
    (
        "the sentence-initial report beside a launch",
        "Fixed and relaunched as `07f2729b`: `splash_skip_required`, `online_disable_required` and "
        "`pre_world_audio_mute_required` now each take a `boot_autoload` term.",
        True,
    ),
    # Three more from the same session, none of which any branch read before 2026-09-13.
    ("a synonym", "Solved and user-confirmed: the orphan title window is reaped on the first frame.", True),
    ("the mechanism named as the answer", "So the fix is `CloseAsFailed`, which the native path uses.", True),
    (
        "an edit asserted to have produced an effect",
        "This single edit restored BOTH the `05_010` chrome and the `02_040` six-cell Quit grid.",
        True,
    ),
    ("a synonym in the passive", "The softlock on the second load is resolved.", True),
    ("behaviour asserted to have arrived", "The second load now works.", True),
    # And the negatives those four branches must not take with them. A plan is the shape the
    # correction asks for, and the infinitive is what separates it from a claim.
    ("a plan, not a claim", "The fix is to gate it on boot_autoload, which I have not done yet.", False),
    ("a judgement about difficulty", "The fix is simple, but it changes behaviour.", False),
    ("remaining work named", "The fix is needed in the loader too.", False),
    ("a file put back", "I restored the file from git after the bad sed.", False),
    ("someone else's code", "The fix in the upstream crate is to widen the offset; we pin ours.", False),
    # Both from the corpus audit that followed the 2026-09-13 widening, and both the reason two
    # branches were pulled back in. A turn naming what it did not fix is honest reporting, and the
    # first-person synonym was the only branch convicting a turn whose object was a compile.
    (
        "a fix denied outright",
        "This build doesn't fix that case -- it makes it legible, and the next rejection names the field.",
        False,
    ),
    (
        "a compile settled in this repo",
        "I resolved it in this repo per AGENTS.md by reading the singleton out of the global, and "
        "moved that constant into the shared crate since a second one now needs it.",
        False,
    ),
    # ... while the same session's other unproven claim still convicts.
    (
        "a report that names the built artifact",
        "Fixed and built -- that blank row now has a third place-name source that does not depend "
        "on your save holding a second character.",
        True,
    ),
    # The negatives are the important half. A guard that convicts these takes ordinary reporting
    # away, which costs far more than the unproven claim it would catch.
    ("a branch name", "The branch is fix/harness-repl-reach and it is already pushed.", False),
    ("the prefix and suffix stems", "The prefix and suffix are both stripped before matching.", False),
    ("a test fixture", "A fixture of that turn left every signal silent.", False),
    (
        "the gerund the promissory closer owns",
        "Fixing both: bypass the union so the naked capture is the detour entry.",
        False,
    ),
    ("naming remaining work", "This needs a fix in the loader too, which I have not started.", False),
    ("the user's own words, quoted", 'The user said "this is the real fix" and it was not.', False),
    ("the word in backticks", "The `fix` word is what the guard reads.", False),
    ("a table cell", "| status | fixed |", False),
    ("a modal", "It should fix the second load, but nothing has run yet.", False),
    ("a document path", "docs/fix.md records the earlier attempt.", False),
    (
        "a policy filename",
        "no_fix_claim_without_runtime_evidence.rego is the new policy.",
        False,
    ),
    ("a kebab slug", "The fix-the-loader branch was abandoned.", False),
]

HEDGE_CASES = [
    ("unverified", "That is the real fix. Unverified -- nothing has run.", True),
    ("not yet proven", "That is the fix, not yet proven against a run.", True),
    ("needs a run", "That is the fix; it needs a run before anyone believes it.", True),
    ("first person absence", "That is the fix. I have not run it.", True),
    ("an attempt", "An attempt at the fix, in the row cloner.", True),
    # Measured, not invented: the one corpus turn whose closing line already said the change had not
    # run, in this repo's idiom rather than the dictionary's.
    ("ready for the next run", "The table now exists once (DLL 798414f3, ready for the next run).", True),
    # Naming what would settle it is the behaviour the correction asks for and must pass.
    (
        "what would prove it",
        "That is the fix. What would prove it is ds2-loader.log showing the added rows greyed "
        "after a summon sign is touched.",
        True,
    ),
    ("work described but not done", "The fix is to gate it, which I have not done yet.", True),
    ("a fix denied outright", "This build doesn't fix that case -- it makes it legible.", True),
    ("a plain claim is not a hedge", REAL_FIX, False),
    # A watcher armed on a log is a promise of evidence, not a hedge, and the 2026-09-13 escape
    # closed on exactly this sentence. Reading it as a hedge would re-open the hole from the prose
    # side the moment the evidence side was shut.
    (
        "a watcher armed is not a hedge",
        "The monitor will tell me if `splash-skip: patched` or `muted=true` appears anyway.",
        False,
    ),
    ("a green build is not a hedge", "That is the fix, and cargo xwin build is clean.", False),
]

BLOCKED_CASES = [
    ("the game is down", "That is the real fix; reading the log needs the game running.", True),
    ("sudo", "That is the fix, but starting the daemon needs sudo.", True),
    # From the corpus: a real fix deliberately not made, handed back because it changes behaviour.
    (
        "a decision handed back",
        "The real fix is validating readability, which I have left for you to call.",
        True,
    ),
    ("an observation only the user can make", "That is the fix -- tell me what you saw.", True),
    ("a plain claim is not a blocker", REAL_FIX, False),
]

HOST_OBJECT_CASES = [
    # Both verbatim from the corpus, and both the reason this exemption exists.
    ("gates fixed", "Two gates broke on the way and I fixed them rather than shaving around them.", True),
    (
        "gate failures fixed",
        "The integration gate came back red and I have fixed all four failures.",
        True,
    ),
    ("a lint", "I fixed the clippy warning in the same pass.", True),
    # Needed by the `resolved` vocabulary added on 2026-09-13: a merge conflict is host work, and
    # the stem match reads the filename `rows.rs` as the game noun `row`, so the ordinary host test
    # is defeated by the very file the conflict was in.
    ("a merge conflict in a game file", "Resolved the merge conflict in rows.rs and kept both hunks.", True),
    ("a softlock is not host machinery", "Resolved the softlock on the second load.", False),
    # A gate list elsewhere in the message is not the object of the sentence. Measured: widening
    # either host list to the whole closing prose was tried, and "Gates: `fmt` 0, comment-caps 0"
    # two sentences later bought silence for a claim about a row in the game.
    (
        "a claim about a game row is not host machinery",
        "Fixed and built -- that blank row now has a third place-name source.",
        False,
    ),
    # A passing test is exactly the substitution the directive refuses, so naming one alongside
    # something that lives in the game must not exempt.
    ("a test leaned on beside a game noun", "The fix works and the tests pass on the load path.", False),
    ("a runtime object", "I fixed the crash on the second load.", False),
    ("a plain claim names no host machinery", REAL_FIX, False),
]


def tool(name: str, **inputs) -> tuple:
    return ("tool", {"type": "tool_use", "name": name, "id": "t", "input": inputs})


GAME = "/home/banon/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"

# `ds2-menu-row` reaches `dinput8.dll` through `ds2-loader`'s `ds2-menu-row.workspace = true`, so
# an edit to it is an edit to the DLL -- the crate commit 686200f changed without a run.
EDIT = tool("Edit", file_path=str(REPO_ROOT / "crates/ds2-menu-row/src/lib.rs"))
HOST_EDIT = tool("Edit", file_path=str(REPO_ROOT / "scripts/ds2-flo.py"))
TEST_EDIT = tool("Edit", file_path=str(REPO_ROOT / "crates/ds2-menu-row/tests/rows.rs"))
BUILD = tool("Bash", command="cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader")
UNIT_TEST = tool("Bash", command="cargo test -p ds2-save-file-core")
LAUNCH = tool("Bash", command="python3 scripts/ds2-run.py --all-menu-rows")
READ_LOG = tool("Bash", command=f'tail -n 40 "{GAME}/ds2-loader.log"')
READ_PREV = tool("Read", file_path=f"{GAME}/ds2-loader.log.prev")
TEARDOWN = tool("Bash", command="python3 scripts/ds2-teardown.py --status")
FRIDA = tool("Bash", command="uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/rows.js")
HEREDOC_READ = tool(
    "Bash",
    command="python3 - <<'PY'\nprint(open('crates/ds2-menu-row/src/lib.rs').read())\nPY",
)
SED_WRITE = tool("Bash", command="sed -i 's/a/b/' crates/ds2-menu-row/src/lib.rs")

# The er 2026-09-13 escape, reduced to the tool call that let it through there: a `Monitor` armed
# on a log the just-launched process had not written a line of. The filename is in the input, and
# an allowlist that only asks "was the name mentioned" calls that a read.
ARM_MONITOR = tool(
    "Monitor",
    command=f'L="{GAME}/ds2-loader.log"; tail -F "$L" | grep -E --line-buffered "roster:"',
)
ARM_BACKGROUND_TAIL = tool(
    "Bash",
    command=f'tail -F "{GAME}/ds2-loader.log"',
    run_in_background=True,
)
ARM_DETACHED_WATCH = tool(
    "Bash",
    command="setsid nohup python3 scripts/ds2-frida-watch.py --agent scripts/frida/rows.js &",
)
READ_CRASH = tool("Read", file_path=f"{GAME}/ds2-crash-latest.txt")
# A teardown without --status ends a session and reads nothing.
TEARDOWN_KILL = tool("Bash", command="python3 scripts/ds2-teardown.py")
# The files the game reads rather than writes.
HARNESS_CMD = tool("Read", file_path=f"{GAME}/ds2-input-harness-cmd.txt")

EVIDENCE_CASES = [
    ("an edit and a build", [EDIT, BUILD], (True, False)),
    ("an edit and a passing unit test", [EDIT, UNIT_TEST], (True, False)),
    ("an edit and a launch", [EDIT, BUILD, LAUNCH], (True, False)),
    ("an edit and the log the run wrote", [EDIT, BUILD, LAUNCH, READ_LOG], (True, True)),
    ("an edit and the rotated log of the run", [EDIT, READ_PREV], (True, True)),
    ("an edit and the live session state", [EDIT, TEARDOWN], (True, True)),
    ("an edit and an attached frida watch", [EDIT, FRIDA], (True, True)),
    ("a teardown is not a read", [EDIT, BUILD, LAUNCH, TEARDOWN_KILL], (True, False)),
    ("a file the game reads is not a run artifact", [EDIT, HARNESS_CMD], (True, False)),
    ("a log read before the edit describes the old code", [READ_LOG, EDIT], (True, False)),
    ("a host-only edit", [HOST_EDIT, BUILD], (False, False)),
    ("a crate test file is host work", [TEST_EDIT, UNIT_TEST], (False, False)),
    ("a heredoc that only reads is not a write", [HEREDOC_READ, BUILD], (False, False)),
    ("an in-place stream edit is a write", [SED_WRITE, BUILD], (True, False)),
    # The 2026-09-13 miss, in the three shapes an arming can take. Each names a real run artifact
    # and each has read none of it, which is the whole correction: a launch is not proof, and
    # neither is subscribing to what a launch might later write.
    ("a monitor armed on a log is not a read", [EDIT, BUILD, LAUNCH, ARM_MONITOR], (True, False)),
    ("a backgrounded tail is not a read", [EDIT, BUILD, LAUNCH, ARM_BACKGROUND_TAIL], (True, False)),
    ("a detached watcher is not a read", [EDIT, BUILD, LAUNCH, ARM_DETACHED_WATCH], (True, False)),
    # ... while the ordinary foreground read of the same file still counts, or reading the evidence
    # would be punished identically to skipping it.
    ("a foreground tail of the same log is a read", [EDIT, BUILD, LAUNCH, READ_LOG], (True, True)),
    ("the crash dump a run left", [EDIT, BUILD, LAUNCH, READ_CRASH], (True, True)),
    # A measurement taken before the artifact was rebuilt describes the previous artifact. The
    # claim is about the DLL that is loaded now.
    ("a log read then a rebuild stales the read", [EDIT, BUILD, LAUNCH, READ_LOG, EDIT, BUILD], (True, False)),
    ("a rebuild then the log read again", [EDIT, BUILD, LAUNCH, READ_LOG, EDIT, BUILD, READ_LOG], (True, True)),
    # A `cargo check` produces no artifact, so it stales nothing.
    ("a check after the read does not stale it", [EDIT, BUILD, LAUNCH, READ_LOG, UNIT_TEST], (True, True)),
]

# `cites_run_artifact`: the prose half of the same distinction. Reporting what a log said is
# evidence; saying a watcher will report it later is the promise of evidence.
CITATION_CASES = [
    (
        "a log reported",
        "ds2-loader.log carries `ds2-menu-row: locked 3 rows (multiplayer)` after the summon.",
        True,
    ),
    ("a crash dump reported", "ds2-crash-latest.txt names ds2-menu-row as the faulting module.", True),
    ("a status read reported", "ds2-teardown.py --status lists no survivors.", True),
    # The er 2026-09-13 closing sentence, one word away from silencing the guard from the prose side.
    (
        "a watcher promised",
        "The monitor on ds2-loader.log will tell me if `locked 3 rows` appears.",
        False,
    ),
    (
        "a watcher promised, with a report elsewhere in the turn",
        "The monitor on ds2-loader.log will tell me if it appears.\n"
        "Meanwhile ds2-loader.log.prev shows the rows unlocked on the previous run.",
        True,
    ),
    ("no artifact named at all", "The three predicates now take a boot_autoload term.", False),
]


def main() -> int:
    bad = 0

    def check(label: str, name: str, got, want) -> None:
        nonlocal bad
        ok = got == want
        bad += 0 if ok else 1
        print(f"  {'ok  ' if ok else 'FAIL'} [{label}] {name}: expected {want}, got {got}")

    for name, text, want in CLAIM_CASES:
        check("claim", name, bool(fix_claim(text)), want)
    for name, text, want in HEDGE_CASES:
        check("hedged", name, hedged(text), want)
    for name, text, want in BLOCKED_CASES:
        check("blocked", name, externally_blocked(text), want)
    for name, text, want in HOST_OBJECT_CASES:
        check("hostobject", name, host_object(fix_claim(text) or text), want)

    crates = runtime_crates(REPO_ROOT)
    # The walk is what decides whether the guard can convict anything at all. An empty or collapsed
    # result makes every turn `changed=0` and the rule silently inert, which is the defect this
    # repo's Stop guards have shipped with twice.
    check("crates", "the dinput8.dll shell is reachable", "ds2-loader" in crates, True)
    check("crates", "the crash-logging shell is reachable", "ds2-crash-logging" in crates, True)
    check("crates", "a workspace dependency of the loader is reachable", "ds2-menu-row" in crates, True)
    check("crates", "a dependency of a dependency is reachable", "ds2-game-base" in crates, True)
    check("crates", "the injector exe is not", "ds2-launcher" in crates, False)
    check("crates", "the host-only save picker is not", "ds2-save-picker-core" in crates, False)

    for name, blocks, want in EVIDENCE_CASES:
        check("evidence", name, runtime_evidence(blocks, crates), want)
    for name, text, want in CITATION_CASES:
        check("citation", name, cites_run_artifact(text), want)

    total = (
        len(CLAIM_CASES)
        + len(HEDGE_CASES)
        + len(BLOCKED_CASES)
        + len(HOST_OBJECT_CASES)
        + 6
        + len(EVIDENCE_CASES)
        + len(CITATION_CASES)
    )
    if bad:
        print(f"fix-claim classifier: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"fix-claim classifier: all {total} cases passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
