#!/usr/bin/env python3
"""Live-eval regression tests for the repo-local Cupcake policies.

`opa test .cupcake/tests/*_test.rego` (run by scripts/check.sh) proves the RULES are right in the
OPA interpreter. It does not prove that the deployed pipeline -- cupcake's WASM build, the harness
event shape, the signal wiring in .cupcake/rulebook.yml -- reaches the same verdict. This file
closes that gap by driving the real `cupcake eval` binary with real PreToolUse events, which is
exactly what the global PreToolUse hook does on every tool call.

WHY THIS IS A FILE AND NOT A SHELL ONE-LINER
Several of the deny cases below must contain the literal text of a command the policies exist to
block (`git push origin main`, `--no-verify`). Typing that into an agent Bash command is itself
intercepted -- correctly. Measured while porting these policies on 2026-08-26: an attempt to build
the same fixtures inline was denied with "This command wraps a shell payload the guard cannot read
(an unquoted or substituted `-c`/`eval` argument) while naming git and push, so it cannot be shown
not to push to main." That is the guard working, not a false positive. A committed file on disk is
never an agent Bash command, so it is the sanctioned place for this text -- the same reasoning the
sibling repo's pgrep policy uses to sanction `scripts/steam-running.sh`.

Signals are pinned per case through the CUPCAKE_*_OVERRIDE env vars that .cupcake/rulebook.yml
reads, so a case that models "on main" does not depend on which branch the checkout is actually on.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# Claude Code sends a `timeout` alongside every Bash command; include it so the event shape
# matches production rather than a stripped-down approximation.
DEFAULT_BASH_TIMEOUT_MS = 30000

# Two identical OIDs mean "origin/main is fresh". The fresh-origin-main guard fails CLOSED, so
# every case that is not specifically testing staleness must pin a matching pair or it will deny
# for the wrong reason and the test will pass while proving nothing.
FRESH_OIDS = "a" * 40 + " " + "a" * 40
STALE_OIDS = "a" * 40 + " " + "b" * 40

# The runtime-evidence signal, in the format .cupcake/signals/runtime_evidence_for_head.sh emits.
# This one has to be pinned for the same reason the OIDs above do, and the failure it prevents is
# worse than flakiness: unpinned, every `git push` case in this file reads the REAL game log, so
# whether the suite is green depends on whether somebody happened to launch Dark Souls II recently.
# A gate whose verdict changes when nothing in the repo changed teaches agents to re-run it until it
# passes.
RUNTIME_PROVEN = "RUNTIME|game_code=1|attached=1|fresh=1|dll_match=1|head=1758600000|log=1758600900"
RUNTIME_NEVER_RAN = "RUNTIME|game_code=1|attached=0|fresh=0|dll_match=1|head=1758600000|log=0"
RUNTIME_STALE_LOG = (
    "RUNTIME|game_code=1|attached=1|fresh=0|dll_match=1|head=1758600000|log=1758500000"
)
# A launch that happened after the commit, with a DLL this checkout did not build: the clock says the
# run is current and the bytes say it was somebody else's. The clock-only version of the guard allowed
# this, and it is the shape a parallel worktree or a plain Steam launch produces.
RUNTIME_FOREIGN_BINARY = (
    "RUNTIME|game_code=1|attached=1|fresh=1|dll_match=0|head=1758600000|log=1758600900"
)
RUNTIME_NO_GAME_CODE = "RUNTIME|game_code=0|attached=0|fresh=0|dll_match=0|head=1758600000|log=0"


@dataclass(frozen=True)
class PolicyCase:
    name: str
    should_allow: bool
    command: str = ""
    tool_name: str = "Bash"
    expected_text: str | None = None
    tool_input: dict[str, object] = field(default_factory=dict)
    current_branch: str = "cupcake-policies"
    origin_main_oids: str = FRESH_OIDS
    runtime_evidence: str = RUNTIME_PROVEN


def run_case(case: PolicyCase) -> None:
    if case.tool_name == "Bash":
        tool_input: dict[str, object] = {
            "command": case.command,
            "timeout": DEFAULT_BASH_TIMEOUT_MS,
        }
    else:
        tool_input = {}
    tool_input.update(case.tool_input)

    event = {
        "session_id": f"cupcake-policy-regression-{case.name}",
        "transcript_path": f"/tmp/cupcake-policy-regression-{case.name}.jsonl",
        "cwd": str(REPO_ROOT),
        "hook_event_name": "PreToolUse",
        "tool_name": case.tool_name,
        "tool_input": tool_input,
        "permission_mode": "default",
    }

    env = {
        **os.environ,
        "CUPCAKE_CURRENT_BRANCH_OVERRIDE": case.current_branch,
        "CUPCAKE_WORKTREE_BRANCHES_OVERRIDE": "",
        "CUPCAKE_ORIGIN_MAIN_OIDS_OVERRIDE": case.origin_main_oids,
        "CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE": case.runtime_evidence,
    }

    result = subprocess.run(
        ["cupcake", "eval", "--harness", "claude", "--strict", "--log-level", "error"],
        cwd=REPO_ROOT,
        input=json.dumps(event),
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
        env=env,
    )
    output = result.stdout + result.stderr
    allowed = result.returncode == 0
    if allowed != case.should_allow:
        raise AssertionError(
            f"{case.name}: expected allow={case.should_allow}, "
            f"got returncode={result.returncode}\n{output}"
        )
    if case.expected_text and case.expected_text not in output:
        raise AssertionError(f"{case.name}: missing {case.expected_text!r}\n{output}")


def cases() -> list[PolicyCase]:
    """The fixture set, exposed so `scripts/explain-cupcake-text.py` can reuse it by name.

    A fixture that reproduces a denial necessarily contains the text the guard blocks, so it can
    never be retyped into an agent Bash command -- not even to write it to a file. Sharing the list
    is what lets a denial be DEBUGGED rather than only detected.
    """
    return [
        # --- control: an ordinary command must survive the whole policy set -------------------
        #
        # This case used to be `cargo build --workspace`, allowed. It is not any more, and the
        # change is the point rather than a casualty: `require_scoped_cargo` (ported from
        # er-mods-rs 2026-09-21) denies unscoped cargo, and this workspace is the shape that
        # policy was written for -- `members = ["crates/*"]`, 19 crates, built by parallel agents
        # in separate worktrees with unshared target/ dirs, which is exactly what the root
        # Cargo.toml says out loud. A control case has to be a command the rules really do allow,
        # so the control is now the scoped spelling and the unscoped one is pinned as a deny.
        PolicyCase("allow-cargo-build-scoped", True, "cargo build -p ds2-inventory-sort"),
        PolicyCase("deny-cargo-build-workspace", False, "cargo build --workspace"),
        PolicyCase("allow-cargo-fmt", True, "cargo fmt --check"),
        PolicyCase("allow-ds2-run-dry", True, "python3 scripts/ds2-run.py --dry-run"),
        # --- ds2_launch_guard -------------------------------------------------------------
        # The bare launch this guard exists to stop, and the sanctioned launcher it must
        # never touch (`allow-ds2-run-dry` above is the other half of that pair).
        PolicyCase("deny-bare-steam-applaunch", False, "steam -applaunch 335300"),
        PolicyCase("deny-steam-run-url", False, "xdg-open steam://run/335300"),
        # A different appid is somebody else's game, and static RE on our binary is the
        # most encouraged activity in this repo -- neither may read as a launch.
        PolicyCase("allow-applaunch-other-appid", True, "steam -applaunch 1245620"),
        PolicyCase("allow-objdump-on-the-exe", True, "objdump -d Game/DarkSoulsII.exe | head"),
        # A COMMIT MESSAGE IS NOT A COMMAND, and a heredoc body is where commit messages live.
        # Measured 2026-08-26 (ds2-mods-rs-1tc): `git commit -F -` with a heredoc explaining that
        # check.sh now runs the Windows test binaries was DENIED by the launch guard, because the
        # message named the executable and wine in the same body. The message had to be made less
        # precise to land -- a guard about launching degrading the record of what was built.
        PolicyCase(
            "allow-commit-message-describing-the-launch-rule",
            True,
            "git commit -q -F - <<'EOF'\n"
            "Wire crash logging into the loader\n"
            "\n"
            "check.sh now builds the Windows test binaries and runs them under wine.\n"
            "Nothing imports ds2_crash_logging.dll, and DarkSoulsII.exe imports only\n"
            "dinput8, so the library is linked in instead.\n"
            "EOF",
        ),
        # The same shape for the pgrep guard: prose explaining why -f is banned must not be
        # denied by the rule it explains.
        PolicyCase(
            "allow-commit-message-describing-the-pgrep-rule",
            True,
            "git commit -q -F - <<'EOF'\n"
            "Document the pgrep guard\n"
            "\n"
            "pgrep -f DarkSoulsII.exe matched the agent's own command line and\n"
            "produced a fabricated ALIVE claim; use pgrep -x instead.\n"
            "EOF",
        ),
        # The EXACT command that was still denied after the first fix: a compound
        # `git add && git status && git commit -F -` whose message uses backticks for code
        # formatting. Kept verbatim rather than minimised, because the minimised form passed
        # The guard must still bite when a heredoc genuinely FEEDS A SHELL -- that body is a
        # program, not prose, and commands.rego deliberately leaves it raw.
        PolicyCase(
            "deny-launch-hidden-in-a-shell-heredoc",
            False,
            "bash <<'EOF'\nsteam -applaunch 335300\nEOF",
        ),
        # --- block_pgrep_full_match ---------------------------------------------------------
        # -f matched the agent's own command line twice in one session: once producing a
        # fabricated ALIVE claim, once killing the agent's own shell. -x cannot self-match.
        PolicyCase("deny-pgrep-full-match", False, "pgrep -f DarkSoulsII.exe"),
        PolicyCase("deny-pkill-full-match", False, "pkill -f ds2-run.py"),
        PolicyCase("allow-pgrep-exact-name", True, "pgrep -x DarkSoulsII.exe"),
        PolicyCase("allow-unrelated-grep-f", True, "grep -f patterns.txt haystack.txt"),
        # --- git_block_main_commit -------------------------------------------------------------
        PolicyCase(
            "deny-commit-on-main",
            False,
            'git commit -m "wip"',
            current_branch="main",
        ),
        PolicyCase(
            "allow-commit-on-feature-branch",
            True,
            'git commit -m "wip"',
        ),
        # --- git_block_main_push ---------------------------------------------------------------
        PolicyCase("deny-push-origin-main", False, "git push origin main"),
        PolicyCase(
            "deny-push-head-while-on-main",
            False,
            "git push origin HEAD",
            current_branch="main",
        ),
        PolicyCase("allow-push-feature-branch", True, "git push origin cupcake-policies"),
        # --- git_require_fresh_origin_main -----------------------------------------------------
        # Force-pushing a PR branch is allowed only when origin/main was just fetched. The two
        # cases differ ONLY in the pinned OIDs, so a pass here is attributable to that guard and
        # not to some other rule catching the command first.
        PolicyCase(
            "deny-force-push-with-stale-origin-main",
            False,
            "git push --force-with-lease origin cupcake-policies",
            origin_main_oids=STALE_OIDS,
        ),
        PolicyCase(
            "allow-force-push-with-fresh-origin-main",
            True,
            "git push --force-with-lease origin cupcake-policies",
        ),
        # --- git_require_runtime_test_before_push ----------------------------------------------
        # The guard the user believed already existed: "no push until runtime tests". Four of these
        # five cases differ ONLY in the pinned runtime signal, so a verdict here is attributable to
        # this guard rather than to another rule catching the command first.
        #
        # This is also the step that would have caught the guard being inert. `opa test` proves the
        # rule; it cannot prove cupcake compiled it to WASM, routed it through evaluate.rego and ran
        # the signal -- and every one of those failures returns an ordinary allow.
        PolicyCase(
            "deny-push-of-game-code-never-run",
            False,
            "git push origin cupcake-policies",
            runtime_evidence=RUNTIME_NEVER_RAN,
            expected_text="ds2-loader: attach",
        ),
        PolicyCase(
            "deny-push-of-game-code-whose-run-predates-head",
            False,
            "git push origin cupcake-policies",
            runtime_evidence=RUNTIME_STALE_LOG,
            expected_text="predates HEAD",
        ),
        PolicyCase(
            "deny-push-of-game-code-run-with-a-foreign-binary",
            False,
            "git push origin cupcake-policies",
            runtime_evidence=RUNTIME_FOREIGN_BINARY,
            expected_text="not the one this checkout built",
        ),
        # Prefix present, not one field readable. Fail-closed means this reaches the same verdict as
        # a missing signal, which a bare `startswith("RUNTIME|")` test did not.
        PolicyCase(
            "deny-push-when-runtime-signal-is-unreadable",
            False,
            "git push origin cupcake-policies",
            runtime_evidence="RUNTIME|game_code=|attached=|fresh=",
            expected_text="No runtime evidence could be read",
        ),
        # The carve-out that stops the guard being routed around: a policies/docs/beads-export push
        # is not gated, asserted with the worst runtime evidence the signal can emit.
        PolicyCase(
            "allow-push-with-no-game-code-and-no-runtime-evidence",
            True,
            "git push origin cupcake-policies",
            runtime_evidence=RUNTIME_NO_GAME_CODE,
        ),
        # Jurisdiction: only a push. An unrun branch must still be able to inspect and commit.
        PolicyCase(
            "allow-non-push-git-on-unrun-game-code",
            True,
            "git status --short --branch && git log --oneline -3",
            runtime_evidence=RUNTIME_NEVER_RAN,
        ),
        # --- bash_no_python_file_write ---------------------------------------------------------
        # The committed-script exemption resolves a path against the REAL repo root, which the
        # repo_paths signal supplies from .cupcake/signals/'s own location. That makes it exactly
        # the kind of rule `opa test` cannot vouch for: the interpreter test hands the policy a
        # fixture root, and only this layer proves the signal is wired, executable, and read.
        #
        # Measured 2026-09-23, before the widening: the absolute case below was DENIED as a python
        # file write, while the relative `allow-ds2-run-dry` above was allowed -- the same file,
        # the same launcher, refused for the spelling the user's global AGENTS.md requires of every
        # launch command.
        PolicyCase(
            "allow-ds2-run-dry-absolute",
            True,
            f"python3 {REPO_ROOT}/scripts/ds2-run.py --dry-run",
        ),
        PolicyCase(
            "allow-ds2-run-dry-project-dir-var",
            True,
            "python3 $CLAUDE_PROJECT_DIR/scripts/ds2-run.py --dry-run",
        ),
        # One `mkdir -p` away from the tail-shaped exemption that was NOT written.
        PolicyCase(
            "deny-python-script-in-a-lookalike-repo-dir",
            False,
            "python3 /tmp/ds2-mods-rs/scripts/patch.py",
        ),
        # A committed script does not launder the scratch script beside it.
        PolicyCase(
            "deny-committed-script-beside-a-scratch-script",
            False,
            f"python3 {REPO_ROOT}/scripts/ds2-run.py --dry-run && python3 /tmp/patch.py",
        ),
        # The shape the whole guard exists to stop, unchanged by the widening.
        PolicyCase(
            "deny-inline-python-file-write",
            False,
            "python3 -c \"open('notes.md','w').write('x')\"",
        ),
        # --- git_block_no_verify (builtin) -----------------------------------------------------
        PolicyCase("deny-commit-no-verify", False, 'git commit --no-verify -m "wip"'),
        # --- protected_paths (builtin) ---------------------------------------------------------
        # Read allowed, write blocked -- that asymmetry is the whole point of this builtin, so
        # both halves are asserted.
        PolicyCase("allow-read-protected-path", True, "cat /etc/hostname"),
        PolicyCase("deny-write-protected-path", False, "rm -f /etc/hostname"),
        # --- edit_no_tmp_scripts_guard ---------------------------------------------------------
        # Scripts belong in the repo where they are reviewable and survive the session; DATA in
        # /tmp is fine and intended. The .json case guards against the rule over-reaching into
        # artifacts, which is the failure mode that would make agents fight the guard.
        PolicyCase(
            "deny-write-python-script-to-tmp",
            False,
            tool_name="Write",
            tool_input={"file_path": "/tmp/helper.py", "content": "print(1)\n"},
        ),
        PolicyCase(
            "allow-write-json-artifact-to-tmp",
            True,
            tool_name="Write",
            tool_input={"file_path": "/tmp/measurements.json", "content": "{}\n"},
        ),
        PolicyCase(
            "allow-write-python-script-to-repo",
            True,
            tool_name="Write",
            tool_input={
                "file_path": str(REPO_ROOT / "scripts" / "example.py"),
                "content": "print(1)\n",
            },
        ),
        # --- docs_no_shouting --------------------------------------------------------------------
        # The PreToolUse arm of the 2026-09-23 directive on capitals. These three are here rather
        # than only in `opa test` because the policy's span stripping leans on `%`, which `opa fmt`
        # writes for the `rem` builtin -- and a builtin the WASM runtime cannot execute returns
        # undefined, so the rule would never fire and the engine would report a clean allow while
        # the interpreter suite stayed green. Only this layer runs the WASM build.
        PolicyCase(
            "deny-shouted-doc-comment",
            False,
            tool_name="Edit",
            tool_input={
                "file_path": str(REPO_ROOT / "crates" / "ds2-loader" / "src" / "lib.rs"),
                "old_string": "x",
                "new_string": "//! WHAT THIS DOES NOT CLAIM, and it claims nothing.",
            },
            # Matched on the reason rather than the rule_id: cupcake renders a lone decision as a
            # bare reason string with no [rule_id] prefix, so asserting the id would fail on a
            # working guard.
            expected_text="documentation that shouts",
        ),
        PolicyCase(
            "deny-shouted-markdown",
            False,
            tool_name="Write",
            tool_input={
                "file_path": str(REPO_ROOT / "docs" / "DS2-EXAMPLE.md"),
                "content": "NOTHING in ds2-save-file has been run.\n",
            },
            # Matched on the reason rather than the rule_id: cupcake renders a lone decision as a
            # bare reason string with no [rule_id] prefix, so asserting the id would fail on a
            # working guard.
            expected_text="documentation that shouts",
        ),
        # The allow-case is the load-bearing one: a sentence made of this repo's own vocabulary --
        # acronyms, a screaming-snake constant, a backticked Ghidra symbol, a save file and hex.
        PolicyCase(
            "allow-doc-comment-full-of-names",
            True,
            tool_name="Edit",
            tool_input={
                "file_path": str(REPO_ROOT / "crates" / "ds2-loader" / "src" / "lib.rs"),
                "old_string": "x",
                "new_string": (
                    "//! The DLL reads SAVE_DIR_BUILD through `FUN_1402e67f0` at 0xDEADBEEF, so an"
                    " MSVC build and a DS2 SOTFS save agree on the RVA. Config is TOML, CI is OK."
                ),
            },
        ),
    ]


SIGNAL_TIMEOUT_SECONDS = 25.0
OPA_TEST_TIMEOUT_SECONDS = 120.0


def _signal_event() -> str:
    """A PreToolUse event over an inert command -- nothing for any signal to report on."""
    return json.dumps(
        {
            "session_id": "cupcake-signal-contract",
            "transcript_path": "/tmp/cupcake-signal-contract.jsonl",
            "cwd": str(REPO_ROOT),
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo fmt --check", "timeout": DEFAULT_BASH_TIMEOUT_MS},
        }
    )


def run_signal_contract_checks() -> None:
    """Every signal must be executable, carry a shebang, and exit 0.

    Ported from er-mods-rs, where the two halves were separate functions and the exit-code half
    named that repo's own runtime-evidence scripts. Both halves are about how cupcake DELIVERS a
    signal rather than about any particular signal:

      * NO EXECUTABLE BIT. A script in `.cupcake/signals/` is auto-discovered and exec'd directly,
        not run through `bash`. Without the bit the kernel refuses and cupcake records exit 126.
      * NON-ZERO EXIT. Cupcake then replaces the signal's text with a failure record --
        `{"error", "exit_code", "output", "success"}` -- so every string comparison the policy
        makes against a word is undefined, the rule body fails, and the decision set comes back
        empty. Cupcake reports a clean allow and exits 0.

    Either way a guard is off in production while `opa test` stays green and nothing says so.
    Measured in er-mods-rs on 2026-09-16: a signal shipped without the bit, and the policy reading
    it allowed the exact edit it exists to refuse.

    The banal way to hit the second one is a `set -o pipefail` script whose last stage is a `grep`
    that finds nothing, or a helper that exits 1 to mean "no finding" -- both look fine by hand,
    because the finding IS the silence.
    """
    signals = sorted((REPO_ROOT / ".cupcake" / "signals").glob("*.sh"))
    if not signals:
        raise AssertionError(
            ".cupcake/signals/ holds no *.sh at all. Either the directory moved or this gate is "
            "watching the wrong place; an empty walk makes every signal innocent."
        )
    event = _signal_event()
    for script in signals:
        rel = script.relative_to(REPO_ROOT)
        if not os.access(script, os.X_OK):
            raise AssertionError(
                f"{rel} is not executable. Cupcake execs it directly, the kernel refuses with 126, "
                "cupcake replaces its output with a failure record, and every policy reading it "
                f"silently allows. Run `chmod +x {rel}`."
            )
        first = script.read_text(encoding="utf-8", errors="replace").splitlines()[:1]
        if not first or not first[0].startswith("#!"):
            raise AssertionError(
                f"{rel} has no shebang. Exec'd directly, the interpreter line is what decides what "
                "runs it; without one the exec fails the same way a missing bit does."
            )
        result = subprocess.run(
            ["bash", str(script)],
            cwd=REPO_ROOT,
            input=event,
            text=True,
            capture_output=True,
            check=False,
            timeout=SIGNAL_TIMEOUT_SECONDS,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"{rel} exited {result.returncode} on an event it has nothing to say about. A "
                "non-zero signal is replaced by a failure record and the guard reading it allows "
                f"everything, silently.\n{result.stdout}\n{result.stderr}"
            )


def run_rego_suites() -> None:
    """`opa test` over the WHOLE .cupcake tree, not a hand-listed set of suites.

    er-mods-rs keeps an `ORPHANED_REGO_SUITES` list here -- one entry per suite, naming the exact
    .rego files to load -- and the comment above that list records what the list costs: a suite
    added to `.cupcake/tests/` without an edit to it is born orphaned, "which is how four of them
    accumulated 89 never-executed assertions". Loading the tree removes the maintenance surface
    entirely, and it is not slower: one process, ~1000 assertions, under a second here.
    """
    if not shutil.which("opa"):
        print("skip: rego suites (no opa on PATH)")
        return
    result = subprocess.run(
        ["opa", "test", str(REPO_ROOT / ".cupcake")],
        cwd=REPO_ROOT,
        text=True,
        capture_output=True,
        check=False,
        timeout=OPA_TEST_TIMEOUT_SECONDS,
    )
    if result.returncode != 0:
        raise AssertionError(f"opa test failed for .cupcake/:\n{result.stdout}\n{result.stderr}")


def main() -> int:
    run_rego_suites()
    run_signal_contract_checks()

    cases_to_run = cases()

    max_workers = min(8, max(1, len(cases_to_run)))
    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        futures = {pool.submit(run_case, case): case for case in cases_to_run}
        for future in as_completed(futures):
            future.result()
    print(f"cupcake live-eval regression tests passed ({len(cases_to_run)} cases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
