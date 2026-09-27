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

Prints `UNSPENTEVIDENCE|branch=<branch>|sha=<sha>` or nothing.
Run `--selftest` to check the decision table.
"""
from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
from pathlib import Path

SCRIPT_REPO = Path(__file__).resolve().parent.parent


def _teardown_hook():
    """The teardown hook's own helpers, so the two agree on the log, the game dir and game code."""
    path = SCRIPT_REPO / "scripts" / "ds2-teardown-after-evidence-push.py"
    spec = importlib.util.spec_from_file_location("ds2_teardown_after_evidence_push", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def decide(*, alive: bool | None, attached: bool, build: tuple[str, bool] | None,
           in_worktree: bool | None, on_remote: bool | None, game_code: bool | None) -> bool:
    """True only when every fact is known and says the evidence is unspent. Pure, for the selftest."""
    if alive is not True or not attached or build is None or build[1]:
        return False
    return in_worktree is True and on_remote is False and game_code is True


def git(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, timeout=10)


def worktree_heads(porcelain: str) -> list[tuple[str, str]]:
    """(HEAD sha, branch) for each worktree in `git worktree list --porcelain`. A detached HEAD
    gets the branch `(detached)`."""
    heads, head, branch = [], None, None
    for line in porcelain.splitlines() + [""]:
        if line.startswith("HEAD "):
            head = line[5:].strip()
        elif line.startswith("branch "):
            branch = line[7:].strip().removeprefix("refs/heads/")
        elif not line.strip():
            if head:
                heads.append((head, branch or "(detached)"))
            head, branch = None, None
    return heads


def gather(repo: Path) -> str | None:
    """The line to print, or None. Every unknown answer returns None."""
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
    for head, branch in worktree_heads(listing.stdout):
        if git(repo, "merge-base", "--is-ancestor", sha, head).returncode != 0:
            continue
        holding.append(branch)
        diff = git(repo, "diff", "--name-only", f"origin/main...{head}")
        if diff.returncode == 0 and any(hook.is_game_code(p) for p in diff.stdout.splitlines()):
            game_code = True
            holding = [branch]
            break
    ok = decide(alive=True, attached=attached, build=build, in_worktree=bool(holding),
                on_remote=on_remote, game_code=game_code)
    return f"UNSPENTEVIDENCE|branch={holding[0]}|sha={sha}" if ok else None


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
    want_heads = [("aaa", "main"), ("bbb", "(detached)"), ("ccc", "estus-max-default-on")]
    got_heads = worktree_heads(porcelain)
    bad += got_heads != want_heads
    print(f"  {'ok  ' if got_heads == want_heads else 'FAIL'} worktree_heads parses porcelain")
    print("selftest: OK" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
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
