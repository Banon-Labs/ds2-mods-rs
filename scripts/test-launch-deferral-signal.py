#!/usr/bin/env python3
"""Pin scripts/cupcake_launch_deferral.py: which turns withhold a launch, and which pass.

`opa test` pins the policy half (a spoken signal halts). This pins the half that decides whether the
signal speaks, which is where every false positive and every miss of the guard lives.

Run: python3 scripts/test-launch-deferral-signal.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import cupcake_launch_deferral as ld  # noqa: E402

FAILURES: list[str] = []


def check(ok: bool, what: str) -> None:
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        FAILURES.append(what)


# --- prose that withholds a launch because the game is in use -------------------------------------
DEFERS = (
    # the shape of the 2026-09-26 instance
    "I'm not launching 7f313a0 -- your DS2 session is running and you plan to go back to it.",
    "The user plans to go back to it, so the build stays unrun for now.",
    "Not launching yet: the game is running.",
    "I won't launch while you're in the game.",
    "Holding off on the launch until you're out.",
    "I'll launch it once you're done.",
    "Relaunching would kill your session, so I left it.",
    "That would tear down the game you're in.",
    "I didn't relaunch because you're still playing.",
    "Skipping the runtime test while your game is up.",
    "Not going to launch over your running game.",
    "I’m not launching — you’re in it.",
)
for sentence in DEFERS:
    check(ld.defers_launch(sentence) is not None, f"withholds a launch: {sentence[:60]!r}")

# --- must pass: past-tense launch reports, blockers, statuses, quotations -------------------------
CLEAN = (
    "Launched over the running game; ds2-run.py tore down your session first.",
    "ds2-run.py tore down the game you were in and relaunched on the new DLL.",
    "The launcher printed its success block after reading the DLL's log line.",
    "That did not launch -- ds2-run.py exited 1 because the DLL is missing.",
    "I didn't launch because the build failed: cargo exited 101.",
    "The game is running on run br-20260926-1200.",
    "Your session was torn down by the relaunch, as expected.",
    'The guard fires on "not launching, the game is running" and nothing else.',
    "It fires on `until you're out` and on `would kill your session`.",
    "The test suite is running in the background.",
    "No teardown was needed; nothing was running.",
    "Hold off on merging until CI is green.",
    # measured: a docs change needs no launch, and saying so is not a deferral
    "The change is not game code, so no relaunch was required and I did not make one -- the game is still up.",
)
for sentence in CLEAN:
    check(ld.defers_launch(sentence) is None, f"not a deferral: {sentence[:60]!r}")

# --- subagent prompts ------------------------------------------------------------------------------
AGENT_DEFERS = (
    "Static work only. Do not launch or attach: the user's DS2 session is running and they plan to go back to it.",
    "No launch -- the user is still playing.",
    "Don't attach to the game while the user is in it.",
)
for prompt in AGENT_DEFERS:
    check(ld.agent_prompt_withholds_launch(prompt) is not None, f"agent withholding: {prompt[:60]!r}")

AGENT_CLEAN = (
    # the main agent launched; telling a watcher to attach instead of launching another is fine
    "The game is running (I launched it at 12:00). Do not launch another; attach frida to it.",
    "Decompile FUN_1401234 in Ghidra and report the struct layout.",
    "Launch with scripts/ds2-run.py --item-warn and report the log line.",
    "Do not push; the user's review comes first.",
    # measured false positive: the reason was a paragraph away, describing game state
    "Constraints unchanged: do not launch the game, I hold the session. Exit 0, build by exit code.\n\n"
    "A solo player standing in Majula, which is the state the user's game is actually in.",
)
for prompt in AGENT_CLEAN:
    check(ld.agent_prompt_withholds_launch(prompt) is None, f"agent prompt clean: {prompt[:60]!r}")

# --- the user's own opt-out -----------------------------------------------------------------------
check(ld.user_opted_out("don't launch, I'm in the middle of a boss"), "the user saying don't launch opts out")
check(ld.user_opted_out("commit it without launching"), "without launching opts out")
check(not ld.user_opted_out("fix the dialog and launch it"), "an ordinary prompt does not opt out")

# --- what counts as a launch ----------------------------------------------------------------------
LAUNCHES = (
    "python3 scripts/ds2-run.py --item-warn",
    "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py --continue-slot 0",
    "cargo build --release && python3 scripts/ds2-run.py",
)
for command in LAUNCHES:
    check(ld.is_launch_command(command), f"a launch: {command[:60]!r}")
NOT_LAUNCHES = (
    "python3 scripts/ds2-run.py --help",
    "python3 scripts/ds2-run.py --dry-run",
    "python3 scripts/ds2-run.py --selftest",
    "grep -n STAGED scripts/ds2-run.py",
    "git diff scripts/ds2-run.py",
    "python3 scripts/ds2-teardown.py --status",
)
for command in NOT_LAUNCHES:
    check(not ld.is_launch_command(command), f"not a launch: {command[:60]!r}")

# --- unrun commits ---------------------------------------------------------------------------------
game = {"1111111", "3333333"}


def touches(sha: str) -> bool:
    return sha in game


commit = ("git commit -m x", "[feat 1111111] feat(x): y\n 1 file changed", "0")
policy_commit = ("git commit -m p", "[feat 2222222] fix(cupcake): p", "0")
launch = ("python3 scripts/ds2-run.py", "success", "0")

check(ld.unrun_game_commit([commit], touches) == "1111111", "a game-code commit with no launch is unrun")
check(ld.unrun_game_commit([commit, launch], touches) is None, "a launch after the commit clears it")
check(ld.unrun_game_commit([launch, commit], touches) == "1111111", "a launch before the commit does not")
check(ld.unrun_game_commit([policy_commit], touches) is None, "a commit touching no game code is not gated")
check(
    ld.unrun_game_commit([commit, launch, policy_commit], touches) is None,
    "a policy commit on top of a launched build needs no second launch",
)
check(
    ld.unrun_game_commit([commit, ("git commit -m z", "[feat (root-commit) 3333333] z", "0")], touches) == "3333333",
    "the root-commit spelling of the commit line is read",
)
check(
    ld.unrun_game_commit([("git commit -m x", "[feat 1111111] x", "1")], touches) is None,
    "an errored commit call is ignored",
)
check(
    ld.unrun_game_commit([("git log --oneline", "[feat 1111111] x", "0")], touches) is None,
    "commit-shaped output from a non-commit command is ignored",
)
check(
    ld.unrun_game_commit([("python3 scripts/ds2-run.py --dry-run", "", "0"), commit], touches) == "1111111",
    "a dry run is not a launch",
)

# Real git objects: a known crates commit and a known policy-only commit from this repo's history.
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
check(ld.commit_touches_game_code("970c7d9", REPO), "970c7d9 (a crates/ commit) is game code")
check(not ld.commit_touches_game_code("87e830c", REPO), "87e830c (a cupcake-only commit) is not")
check(not ld.commit_touches_game_code("deadbeefdead", REPO), "an unknown sha fails open")

print()
if FAILURES:
    print(f"[test-launch-deferral-signal] FAILED ({len(FAILURES)})")
    for failure in FAILURES:
        print(f"  - {failure}")
    raise SystemExit(1)
print("[test-launch-deferral-signal] ok")
