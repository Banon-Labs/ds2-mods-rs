#!/usr/bin/env python3
"""PostToolUse(Bash) hook: take the game down once its run has been used as push evidence.

User directive 2026-09-26: "I just want the game to go down when you're done collecting evidence."
A runtime run exists to satisfy DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH, and the push is where that
evidence is spent. So after a `git push` the game is torn down when all of these hold:

  * the push landed: the pushed checkout's HEAD is now on a remote-tracking branch;
  * the game is alive (`ds2-teardown.py --status` lists DarkSoulsII.exe);
  * the running build IS that checkout's build: the staged `dinput8.dll` has the same sha256 as the
    checkout's built one, and `ds2-loader.log` has the `ds2-loader: attach` line;
  * when the log names its build commit (`build git=<sha>`), that commit is clean and is HEAD or an
    ancestor of it.

A policy cannot do this. Rego can refuse a command or add context, but it cannot run the teardown,
and an obligation the agent has to remember is the failure this replaces. This hook runs it.

The pushed checkout is resolved by `scripts/cupcake_push_target_repo.py`, the same resolver the push
guard uses, so `cd <worktree> && git push` is judged in the worktree. `UNKNOWN` does nothing.
Any error does nothing: a hook that fails must not kill a session.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SCRIPT_REPO = Path(__file__).resolve().parent.parent
GAME_DIR = Path(os.environ.get(
    "DS2_TEARDOWN_AFTER_PUSH_GAME_DIR",
    str(Path.home() / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"),
))
# Mirrors STAGED_DLL_NAME / BUILT_DLL in scripts/ds2-run.py and the push guard's signal.
BUILT_REL = Path("target/x86_64-pc-windows-msvc/release/dinput8.dll")
PUSH_RE = re.compile(r"\bgit\b(?:\s+-C\s+\S+)?[^;&|]*?\bpush\b")
BUILD_RE = re.compile(r"build git=([0-9a-f]{7,40})(-dirty)?")


def sha256(path: Path) -> str | None:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return None


def git(repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, timeout=10)


def pushed_repo(event: dict) -> Path | None:
    cwd = event.get("cwd") or os.getcwd()
    out = subprocess.run(
        [sys.executable, str(SCRIPT_REPO / "scripts/cupcake_push_target_repo.py"), "--cwd", cwd],
        input=json.dumps(event), capture_output=True, text=True, timeout=10,
    ).stdout.strip()
    if out == "SELF":
        return Path(cwd)
    if out.startswith("REPO "):
        return Path(out[5:])
    return None


def game_alive() -> bool:
    out = subprocess.run(
        [sys.executable, str(SCRIPT_REPO / "scripts/ds2-teardown.py"), "--status"],
        capture_output=True, text=True, timeout=20,
    ).stdout
    return any("alive" in line and "DarkSoulsII.exe" in line for line in out.splitlines())


def log_build(log_text: str) -> tuple[str, bool] | None:
    found = BUILD_RE.findall(log_text)
    if not found:
        return None
    sha, dirty = found[-1]
    return sha, bool(dirty)


def is_game_code(path: str) -> bool:
    """The paths DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH demands a run for."""
    return path.startswith("crates/") or path == "scripts/ds2-run.py"


def branch_game_code(repo: Path) -> bool:
    """Whether HEAD changes game code against origin/main. Unknown counts as no: a hook that
    cannot tell must leave the game alone."""
    out = git(repo, "diff", "--name-only", "origin/main...HEAD")
    return out.returncode == 0 and any(is_game_code(p) for p in out.stdout.splitlines())


def decide(*, landed: bool, game_code: bool, alive: bool, staged: str | None, built: str | None,
           attached: bool, build: tuple[str, bool] | None, build_in_head: bool) -> tuple[bool, str]:
    """Whether to tear down, and why not when not. Pure, so the selftest covers every branch."""
    if not landed:
        return False, "the push did not land"
    # 2026-09-26: a push of scripts/frida/ only tore down a session the user was playing. A push
    # that carries no game code needed no run, so no run was being spent as its evidence.
    if not game_code:
        return False, "the pushed branch changes no game code"
    if not alive:
        return False, "no game is running"
    if not attached:
        return False, "the log has no attach line"
    if staged is None or built is None or staged != built:
        return False, "the running DLL is not this checkout's build"
    if build is not None and (build[1] or not build_in_head):
        return False, "the log's build commit is dirty or not in the pushed HEAD"
    return True, "the running build is the one just pushed"


def run(event: dict) -> str | None:
    if event.get("tool_name") != "Bash":
        return None
    command = (event.get("tool_input") or {}).get("command", "")
    if not PUSH_RE.search(command):
        return None
    repo = pushed_repo(event)
    if repo is None:
        return None
    head = git(repo, "rev-parse", "HEAD").stdout.strip()
    landed = bool(head) and bool(git(repo, "branch", "-r", "--contains", head).stdout.strip())
    log = GAME_DIR / "ds2-loader.log"
    try:
        log_text = log.read_text(errors="replace")
    except OSError:
        log_text = ""
    build = log_build(log_text)
    build_in_head = build is not None and git(repo, "merge-base", "--is-ancestor", build[0], head).returncode == 0
    ok, why = decide(
        landed=landed, game_code=branch_game_code(repo), alive=game_alive(), staged=sha256(GAME_DIR / "dinput8.dll"),
        built=sha256(repo / BUILT_REL), attached="ds2-loader: attach" in log_text,
        build=build, build_in_head=build_in_head,
    )
    if not ok:
        return None
    done = subprocess.run([sys.executable, str(SCRIPT_REPO / "scripts/ds2-teardown.py")],
                          capture_output=True, text=True, timeout=60)
    tail = (done.stdout.strip().splitlines() or ["(no output)"])[-1]
    return f"DS2 torn down after the push ({why}, HEAD {head[:9]}): {tail}"


def selftest() -> int:
    base = dict(landed=True, game_code=True, alive=True, staged="a", built="a", attached=True,
                build=None, build_in_head=False)
    cases = [
        ("everything matches", {}, True),
        ("push did not land", {"landed": False}, False),
        ("branch changes no game code (frida-only push, 2026-09-26)", {"game_code": False}, False),
        ("no game", {"alive": False}, False),
        ("no attach line", {"attached": False}, False),
        ("other checkout's DLL", {"built": "b"}, False),
        ("no built DLL", {"built": None}, False),
        ("named clean build in HEAD", {"build": ("abc1234", False), "build_in_head": True}, True),
        ("named dirty build", {"build": ("abc1234", True), "build_in_head": True}, False),
        ("named build not in HEAD", {"build": ("abc1234", False), "build_in_head": False}, False),
    ]
    bad = 0
    for name, override, want in cases:
        got, _ = decide(**{**base, **override})
        bad += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} {name}")
    for text, want in (
        ("ds2-loader 0.1.0 build git=9c028a19d50bd2f19b727eaadc0b5c9229047abf module=x", ("9c028a19d50bd2f19b727eaadc0b5c9229047abf", False)),
        ("build git=abc1234-dirty module", ("abc1234", True)),
        ("ds2-loader: attach", None),
    ):
        got = log_build(text)
        bad += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} log_build {text[:40]!r}")
    for path, want in (
        ("crates/ds2-loader/src/lib.rs", True), ("scripts/ds2-run.py", True),
        ("scripts/frida/estus-sync.js", False), ("docs/a.md", False), ("scripts/ds2-run.pyc", False),
    ):
        got = is_game_code(path)
        bad += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} game code {path!r}")
    for command, want in (
        ("git push -u origin x", True), ("cd /w && git push", True), ("git -C /w push", True),
        ("git status", False), ("gh pr create", False), ("echo git; ls push", False),
    ):
        got = bool(PUSH_RE.search(command))
        bad += got != want
        print(f"  {'ok  ' if got == want else 'FAIL'} push match {command!r}")
    print("selftest: OK" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    try:
        message = run(json.load(sys.stdin))
    except Exception:  # noqa: BLE001 -- a broken hook must never take a session down
        return 0
    if message:
        print(json.dumps({
            "systemMessage": message,
            "hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": message},
        }))
    return 0


if __name__ == "__main__":
    sys.exit(main())
