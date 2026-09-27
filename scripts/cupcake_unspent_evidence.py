#!/usr/bin/env python3
"""Whether the running DARK SOULS II build is run evidence nobody has spent yet.

User directive 2026-09-26: "as soon as you collect your commit sha evidence for runtime testing
that's required for a PR, you're obligated to tear down". scripts/ds2-teardown-after-evidence-push.py
does the teardown, but it keys on the `git push`, so an agent that collects the evidence and does not
push leaves the game up forever. That happened the same day: commit 2f2d0a5 was launched, its
`build git=2f2d0a5...` line read out of ds2-loader.log, and the turn ended with "I haven't pushed yet
because the push hook would close this game".

The Stop policy `no_unspent_run_evidence` halts that turn end. This module decides when, and it
speaks only when every one of these is known to hold:

  1. DarkSoulsII.exe is alive (`ds2-teardown.py --status`);
  2. ds2-loader.log has the `ds2-loader: attach` line and a `build git=<sha>` line that is not dirty;
  3. that commit is in the HEAD of some local worktree of this repo, and no remote-tracking branch
     contains it -- the evidence exists and has not been spent;
  4. a worktree HEAD holding it changes game code against origin/main (the teardown hook's
     `is_game_code`), so its push needs this run and the teardown hook will close the game after it.

It does not speak for a pushed or merged build (the user's own play sessions), a dirty build, no
game, or a branch with no game code. Anything unknown, and any error, is silence: a signal that
fails must not wedge a session.

Nor does it speak while the user is hand-testing that worktree's builds (ds2-mods-rs-wdnr). On
2026-09-27 it halted turn ends for half an hour while the user drove an in-game save picker on
save-picker-panel: each fix was relaunched at the user's request, each relaunch was a new unpushed
commit, and the push the halt demanded fires the teardown hook, which closed the game under the
user's hands. The 2026-09-26 turn this guard exists for looks the same from the machine -- a
launch, a build line, a turn end that hands the game to the user -- so the difference is taken from
the user, not from the agent's prose: the user has written to the session after a launch from the
worktree holding the running commit, and the turn ends within HAND_TEST_WINDOW_S of that message.
In a hand-test loop the user answers each relaunch before the next, so that holds for every
iteration after the first. The first launch from a worktree in a session still halts, which is the
2026-09-26 turn exactly. So does a turn ending HAND_TEST_WINDOW_S after the user last wrote: the
test is over and the build is unspent evidence again.

Player-input lines in ds2-loader.log were considered and not used. The log has no timestamps, main
has no line that only player input writes (`ds2-input-harness` lines are the agent's injected
input), and the picker's `picker open` / `picker chose` exist on one unmerged branch.

Prints `UNSPENTEVIDENCE|branch=<branch>|sha=<sha>` or nothing.
Run `--selftest` to check the decision table, and `--replay <transcript.jsonl> <worktree>` to see
what the hand-test exemption says at each turn end of a real session.
"""
from __future__ import annotations

import importlib.util
import os
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path

SCRIPT_REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPT_REPO / "scripts"))

# How long after the user's last message a turn end still counts as part of their hand test. A
# hand-test reply takes minutes; in the save-picker loop every relaunch was answered within six.
HAND_TEST_WINDOW_S = 30 * 60

# Test-only: `<sha>|<branch>|<worktree path>` stands in for the machine facts (game alive, clean
# build line, unpushed, game code) so the Stop guard tests drive the transcript half for real.
MACHINE_OVERRIDE_ENV = "CUPCAKE_UNSPENT_EVIDENCE_MACHINE_OVERRIDE"


def _teardown_hook():
    """The teardown hook's own helpers, so the two agree on the log, the game dir and game code."""
    path = SCRIPT_REPO / "scripts" / "ds2-teardown-after-evidence-push.py"
    spec = importlib.util.spec_from_file_location("ds2_teardown_after_evidence_push", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def decide(*, alive: bool | None, attached: bool, build: tuple[str, bool] | None,
           in_worktree: bool | None, on_remote: bool | None, game_code: bool | None,
           hand_test: bool = False) -> bool:
    """True only when every fact is known and says the evidence is unspent, and the user is not
    hand-testing it. Pure, for the selftest."""
    if alive is not True or not attached or build is None or build[1] or hand_test:
        return False
    return in_worktree is True and on_remote is False and game_code is True


# --- is the user hand-testing this worktree? ------------------------------------------------------

_CD_RE = re.compile(r"(?:^|[\s(])cd\s+(\"[^\"]+\"|'[^']+'|[^\s;&|)]+)")
_RUN_PATH_RE = re.compile(r"(\"[^\"]*ds2-run\.py\"|'[^']*ds2-run\.py'|[^\s;&|()'\"]*ds2-run\.py)")


def _expand(path: str) -> str:
    return os.path.expanduser(os.path.expandvars(path.strip("'\"")))


def launch_roots(command: str, cwd: str) -> list[str]:
    """The worktree each real `ds2-run.py` launch in one Bash command runs from: the directory above
    `scripts/ds2-run.py`, resolved against the last `cd` before it, or else the call's cwd."""
    from cupcake_launch_deferral import is_launch_command

    roots, base = [], cwd
    for segment in re.split(r"&&|\|\||;|\n", command or ""):
        for cd in _CD_RE.findall(segment):
            base = os.path.join(base, _expand(cd))
        if not is_launch_command(segment):
            continue
        found = _RUN_PATH_RE.search(segment)
        if found:
            script = os.path.normpath(os.path.join(base, _expand(found.group(1))))
            roots.append(os.path.dirname(os.path.dirname(script)))
    return roots


def _when(ev: dict) -> datetime | None:
    try:
        return datetime.fromisoformat(str(ev["timestamp"]).replace("Z", "+00:00"))
    except (KeyError, TypeError, ValueError):
        return None


# Prompt-shaped events the user did not write: harness notices, other agents, local commands.
_NOT_USER_WORDS = re.compile(
    r"\s*(?:<task-notification>|<command-name>|<command-message>|<local-command|<bash-input>"
    r"|<bash-stdout>|<bash-stderr>|<system-reminder>|Stop hook feedback:|\[Request interrupted"
    r"|Another Claude session sent a message|Caveat:)"
)


def user_words(ev: dict) -> bool:
    """An event in which the user wrote to the session."""
    from cupcake_turn_scan import is_real_user_prompt

    if not is_real_user_prompt(ev) or ev.get("isMeta") or ev.get("isCompactSummary"):
        return False
    content = ev.get("message", {}).get("content")
    if isinstance(content, list):
        content = "\n".join(b.get("text", "") for b in content
                            if isinstance(b, dict) and b.get("type") == "text")
    return isinstance(content, str) and content.strip() != "" and not _NOT_USER_WORDS.match(content)


def hand_test(events: list[dict], worktree: str) -> bool:
    """Whether the user is hand-testing builds of `worktree`: their last message came after a launch
    from that worktree, and the transcript's last event is within HAND_TEST_WINDOW_S of it."""
    want = os.path.realpath(worktree)
    first_launch = last_words = end = None
    for ev in events:
        when = _when(ev)
        if when is None:
            continue
        end = when if end is None or when > end else end
        if user_words(ev):
            last_words = when
        if first_launch is not None or ev.get("type") != "assistant":
            continue
        for block in ev.get("message", {}).get("content", []) or []:
            if not (isinstance(block, dict) and block.get("type") == "tool_use"
                    and block.get("name") == "Bash"):
                continue
            roots = launch_roots((block.get("input") or {}).get("command", ""), ev.get("cwd") or "/")
            if any(os.path.realpath(r) == want for r in roots):
                first_launch = when
    if first_launch is None or last_words is None or end is None:
        return False
    return last_words > first_launch and (end - last_words).total_seconds() <= HAND_TEST_WINDOW_S


def transcript_hand_test(worktree: str) -> bool:
    """`hand_test` on the session transcript. No transcript is no hand test: the exemption rests on
    the user's own messages, so without them the guard keeps its 2026-09-26 behaviour."""
    from cupcake_turn_scan import latest_transcript, load_events

    path = latest_transcript()
    return bool(path) and hand_test(load_events(path), worktree)


def git(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, timeout=10)


def worktree_heads(porcelain: str) -> list[tuple[str, str, str]]:
    """(HEAD sha, branch, path) for each worktree in `git worktree list --porcelain`. A detached
    HEAD gets the branch `(detached)`."""
    heads, head, branch, path = [], None, None, ""
    for line in porcelain.splitlines() + [""]:
        if line.startswith("worktree "):
            path = line[9:].strip()
        elif line.startswith("HEAD "):
            head = line[5:].strip()
        elif line.startswith("branch "):
            branch = line[7:].strip().removeprefix("refs/heads/")
        elif not line.strip():
            if head:
                heads.append((head, branch or "(detached)", path))
            head, branch, path = None, None, ""
    return heads


def gather(repo: Path) -> str | None:
    """The line to print, or None. Every unknown answer returns None."""
    override = os.environ.get(MACHINE_OVERRIDE_ENV)
    if override:
        sha, branch, worktree = override.split("|", 2)
        if transcript_hand_test(worktree):
            return None
        return f"UNSPENTEVIDENCE|branch={branch}|sha={sha}"
    hook = _teardown_hook()
    if not hook.game_alive():
        return None
    try:
        log_text = (hook.GAME_DIR / "ds2-loader.log").read_text(errors="replace")
    except OSError:
        return None
    attached = "ds2-loader: attach" in log_text
    build = hook.log_build(log_text)
    if not decide(alive=True, attached=attached, build=build, in_worktree=True, on_remote=False, game_code=True):
        return None
    sha = git(repo, "rev-parse", "--verify", "--quiet", f"{build[0]}^{{commit}}").stdout.strip()
    if not sha:
        return None
    remote = git(repo, "branch", "-r", "--contains", sha)
    if remote.returncode != 0:
        return None
    on_remote = bool(remote.stdout.strip())
    listing = git(repo, "worktree", "list", "--porcelain")
    if listing.returncode != 0:
        return None
    holding, game_code = [], False
    for head, branch, path in worktree_heads(listing.stdout):
        if git(repo, "merge-base", "--is-ancestor", sha, head).returncode != 0:
            continue
        holding.append((branch, path))
        diff = git(repo, "diff", "--name-only", f"origin/main...{head}")
        if diff.returncode == 0 and any(hook.is_game_code(p) for p in diff.stdout.splitlines()):
            game_code = True
            holding = [(branch, path)]
            break
    ok = decide(alive=True, attached=attached, build=build, in_worktree=bool(holding),
                on_remote=on_remote, game_code=game_code,
                hand_test=bool(holding) and transcript_hand_test(holding[0][1]))
    return f"UNSPENTEVIDENCE|branch={holding[0][0]}|sha={sha}" if ok else None


def selftest() -> int:
    base = dict(alive=True, attached=True, build=("2f2d0a5", False), in_worktree=True,
                on_remote=False, game_code=True)
    cases = [
        ("unpushed game-code build running with its evidence in the log (2026-09-26)", {}, True),
        ("build already on a remote branch (a play session of a pushed or merged build)", {"on_remote": True}, False),
        ("remote containment unknown", {"on_remote": None}, False),
        ("dirty build", {"build": ("2f2d0a5", True)}, False),
        ("log names no build", {"build": None}, False),
        ("log has no attach line", {"attached": False}, False),
        ("game not running", {"alive": False}, False),
        ("game liveness unknown", {"alive": None}, False),
        ("commit in no local worktree", {"in_worktree": False}, False),
        ("branch changes no game code", {"game_code": False}, False),
        ("game code unknown", {"game_code": None}, False),
        ("the user is hand-testing the worktree's builds (2026-09-27)", {"hand_test": True}, False),
    ]
    bad = 0
    for name, override, want in cases:
        got = decide(**{**base, **override})
        bad += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} {name}")
    porcelain = (
        "worktree /r\nHEAD aaa\nbranch refs/heads/main\n\n"
        "worktree /r/w\nHEAD bbb\ndetached\n\n"
        "worktree /r/x\nHEAD ccc\nbranch refs/heads/estus-max-default-on\n"
    )
    want_heads = [("aaa", "main", "/r"), ("bbb", "(detached)", "/r/w"),
                  ("ccc", "estus-max-default-on", "/r/x")]
    got_heads = worktree_heads(porcelain)
    bad += got_heads != want_heads
    print(f"  {'ok  ' if got_heads == want_heads else 'FAIL'} worktree_heads parses porcelain")

    wt = "/p/ds2-mods-rs-wt-picker"
    roots = [
        ("absolute path", f"cd {wt} && python3 {wt}/scripts/ds2-run.py > /tmp/run.log 2>&1", "/", [wt]),
        ("relative path after cd", f"cd {wt} && python3 scripts/ds2-run.py --seamless", "/", [wt]),
        ("relative path from the call's cwd", "python3 scripts/ds2-run.py", wt, [wt]),
        ("$HOME spelling", "python3 $HOME/x/scripts/ds2-run.py",
         "/", [os.path.expandvars("$HOME/x")]),
        ("--dry-run is not a launch", f"python3 {wt}/scripts/ds2-run.py --dry-run", "/", []),
        ("--help is not a launch", f"python3 {wt}/scripts/ds2-run.py --help | grep x", "/", []),
        ("a grep naming it is not a launch", f"grep -n BUILT {wt}/scripts/ds2-run.py", "/", []),
    ]
    for name, command, cwd, want in roots:
        got = launch_roots(command, cwd)
        bad += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} launch_roots: {name}")

    def at(minute: int) -> str:
        return f"2026-09-27T19:{minute:02d}:00.000Z"

    def user(minute: int, text, **extra) -> dict:
        return {"type": "user", "timestamp": at(minute), "message": {"content": text}, **extra}

    def launch(minute: int, root: str = wt) -> dict:
        block = {"type": "tool_use", "name": "Bash",
                 "input": {"command": f"python3 {root}/scripts/ds2-run.py > /tmp/r.log 2>&1"}}
        return {"type": "assistant", "timestamp": at(minute), "cwd": "/",
                "message": {"content": [block]}}

    def said(minute: int) -> dict:
        return {"type": "assistant", "timestamp": at(minute),
                "message": {"content": [{"type": "text", "text": "The game is up."}]}}

    loop = [user(0, "test the picker"), launch(1), said(2), user(4, "the rows are offset"),
            launch(6), said(7)]
    tests = [
        ("first launch of the worktree in the session: 2026-09-26's turn, still halts",
         [user(0, "make it default on"), launch(1), said(2)], False),
        ("the user wrote after a launch and the agent relaunched (2026-09-27 picker loop)", loop, True),
        ("the user's message is over HAND_TEST_WINDOW_S before the turn end",
         loop + [said(40)], False),
        ("the user wrote after a launch from a different worktree",
         [user(0, "go"), launch(1, "/p/other"), user(3, "now estus"), launch(5), said(6)], False),
        ("a task notification is not the user",
         [user(0, "go"), launch(1), user(3, "<task-notification> <task-id>x</task-id>"), said(4)],
         False),
        ("stop hook feedback is not the user",
         [user(0, "go"), launch(1), user(3, "Stop hook feedback: push", isMeta=True), said(4)], False),
        ("an interrupt is not the user",
         [user(0, "go"), launch(1),
          user(3, [{"type": "text", "text": "[Request interrupted by user]"}]), said(4)], False),
        ("another agent's message is not the user",
         [user(0, "go"), launch(1), user(3, "Another Claude session sent a message: done"), said(4)],
         False),
        ("a tool result is not the user",
         [user(0, "go"), launch(1),
          user(3, [{"type": "tool_result", "tool_use_id": "t", "content": "ok"}]), said(4)], False),
        ("list-shaped user text counts",
         [user(0, "go"), launch(1), user(3, [{"type": "text", "text": "clicks are offset"}]),
          said(4)], True),
    ]
    for name, events, want in tests:
        got = hand_test(events, wt)
        bad += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} hand_test: {name}")
    print("selftest: OK" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def replay(transcript: str, worktree: str) -> int:
    """Audit: `hand_test` at every turn end a Stop hook judged in a real transcript, i.e. every
    `stop_hook_summary` event, measured on the events before it."""
    from cupcake_turn_scan import load_events

    events = load_events(transcript)
    for i, ev in enumerate(events):
        if ev.get("type") == "system" and ev.get("subtype") == "stop_hook_summary":
            verdict = "silent (hand test)" if hand_test(events[:i], worktree) else "may halt"
            print(f"{ev.get('timestamp', '?')} {verdict}")
    return 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    if len(sys.argv) == 4 and sys.argv[1] == "--replay":
        return replay(sys.argv[2], sys.argv[3])
    try:
        repo = Path(os.environ.get("CLAUDE_PROJECT_DIR") or SCRIPT_REPO)
        line = gather(repo)
    except Exception:  # noqa: BLE001 -- a broken signal must never take a session down
        return 0
    if line:
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
