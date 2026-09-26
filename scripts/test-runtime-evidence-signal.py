#!/usr/bin/env python3
"""End-to-end test of `.cupcake/signals/runtime_evidence_for_head.sh` against real git repositories.

The Rego suite pins what DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH does with a facts line. This pins
where the facts line comes from, which is where the 2026-09-25 miss was: the policy was right and
the signal handed it `game_code=0` for a push of `scripts/ds2-run.py`.

That push was one Bash call, `git commit -qam "..." && git push -u origin launcher-drop-flag`, on a
branch cut from `origin/main` with the launcher edited and uncommitted. The hook fires before the
command runs, so the commit did not exist yet, `HEAD` was `origin/main`, and the diff the signal
took was empty. The first case below is that command, verbatim, against the same tree shape.

Each case builds a throwaway repository with its own `origin`, a copy of the signal and of
`scripts/cupcake_push_scope.py`, and a fake game directory under a throwaway `HOME`, then pipes a
PreToolUse event to the signal exactly as cupcake does and reads the fields back. No game, no
network, and nothing outside the temporary directory is touched.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SIGNAL = REPO_ROOT / ".cupcake" / "signals" / "runtime_evidence_for_head.sh"
# Point this at an older copy of the signal to watch these cases fail against it.
SIGNAL = Path(os.environ.get("RUNTIME_EVIDENCE_SIGNAL_UNDER_TEST", SIGNAL))
SCOPE = REPO_ROOT / "scripts" / "cupcake_push_scope.py"

GAME_REL = ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
BUILT_REL = "target/x86_64-pc-windows-msvc/release/dinput8.dll"

# Verbatim from the transcript of the push that walked through.
CHAINED = (
    "git commit -qam \"$(printf 'chore(scripts): the launcher takes no switch to play online\\n\\n"
    "Online play is Seamless Co-op.\\n')\" && git push -u origin launcher-drop-flag 2>&1 | tail -3"
)


def git(repo: Path, *args: str, env: dict | None = None) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True, capture_output=True, text=True, env=env,
    ).stdout.strip()


def fields(line: str) -> dict[str, str]:
    out = {}
    for part in line.strip().split("|")[1:]:
        key, _, value = part.partition("=")
        out[key] = value
    return out


class World:
    """One repository with an origin, a checkout on a feature branch, and a fake game install."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.home = root / "home"
        self.repo = root / "repo"
        self.origin = root / "origin.git"
        self.game = self.home / GAME_REL
        self.env = dict(os.environ, HOME=str(self.home), GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t",
                        GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
        self.env.pop("CUPCAKE_RUNTIME_EVIDENCE_OVERRIDE", None)
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(self.origin)], check=True)
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.repo)], check=True)
        signals = self.repo / ".cupcake" / "signals"
        signals.mkdir(parents=True)
        shutil.copy2(SIGNAL, signals / SIGNAL.name)
        (self.repo / "scripts").mkdir()
        shutil.copy2(SCOPE, self.repo / "scripts" / SCOPE.name)
        (self.repo / "scripts" / "ds2-run.py").write_text("FLAGS = ['--no-offline']\n")
        (self.repo / "crates" / "ds2-x" / "src").mkdir(parents=True)
        (self.repo / "crates" / "ds2-x" / "src" / "lib.rs").write_text("// x\n")
        (self.repo / "docs").mkdir()
        (self.repo / "docs" / "a.md").write_text("a\n")
        (self.repo / ".gitignore").write_text("target/\n")
        git(self.repo, "add", "-A", env=self.env)
        self.commit("chore: base", offset=-1000)
        git(self.repo, "remote", "add", "origin", str(self.origin))
        git(self.repo, "push", "-q", "origin", "main", env=self.env)
        git(self.repo, "fetch", "-q", "origin", env=self.env)
        # A game install whose DLL is byte-identical to the built one and whose log attached.
        self.game.mkdir(parents=True)
        built = self.repo / BUILT_REL
        built.parent.mkdir(parents=True)
        built.write_bytes(b"MZ dll")
        shutil.copy2(built, self.game / "dinput8.dll")
        os.utime(self.game / "dinput8.dll", (time.time() - 900, time.time() - 900))

    def commit(self, message: str, offset: int = 0) -> None:
        when = f"@{int(time.time()) + offset} +0000"
        env = dict(self.env, GIT_COMMITTER_DATE=when, GIT_AUTHOR_DATE=when)
        git(self.repo, "commit", "-q", "-m", message, env=env)

    def write_log(self) -> None:
        """A fresh run: the DLL creates its log anew, so the file's birth is the run's start."""
        log = self.game / "ds2-loader.log"
        log.unlink(missing_ok=True)
        log.write_text("ds2-loader: attach awaiting-arxan-callback\n")

    def signal(self, command: str) -> dict[str, str]:
        event = {"hook_event_name": "PreToolUse", "tool_name": "Bash",
                 "tool_input": {"command": command}, "cwd": str(self.repo)}
        out = subprocess.run(
            ["bash", str(self.repo / ".cupcake" / "signals" / SIGNAL.name)],
            input=json.dumps(event), capture_output=True, text=True, cwd=self.repo, env=self.env,
        ).stdout
        return fields(out)


def birth_time_supported(directory: Path) -> bool:
    probe = directory / "birth-probe"
    probe.write_text("x")
    out = subprocess.run(["stat", "-c", "%W", str(probe)], capture_output=True, text=True).stdout.strip()
    return out.isdigit() and out != "0"


def main() -> int:
    bad = 0

    def check(name: str, got: dict, **want: str) -> None:
        nonlocal bad
        wrong = {k: (got.get(k), v) for k, v in want.items() if got.get(k) != v}
        bad += 1 if wrong else 0
        print(f"  {'FAIL' if wrong else 'ok  '} {name}" + (f": {wrong} in {got}" if wrong else ""))

    with tempfile.TemporaryDirectory() as tmp:
        # 1. The 2026-09-25 miss: launcher edited, uncommitted, committed and pushed in one call.
        w = World(Path(tmp) / "chained")
        git(w.repo, "checkout", "-q", "-b", "launcher-drop-flag", env=w.env)
        w.write_log()
        (w.repo / "scripts" / "ds2-run.py").write_text("FLAGS = []\n")
        check("a commit chained before the push carries the working tree", w.signal(CHAINED),
              game_code="1", pending="1")

        # 2. The same change committed first and pushed by a separate command: judged on the commit.
        git(w.repo, "add", "-A", env=w.env)
        w.commit("chore(scripts): drop the flag", offset=100)
        got = w.signal("git push -u origin launcher-drop-flag 2>&1 | tail -3")
        check("a separate push of a launcher commit whose run predates it", got,
              game_code="1", pending="0", fresh="0")

        # 3. A run launched after the commit clears it.
        # The commit is dated 100 s ahead; move it back behind a fresh log instead.
        git(w.repo, "commit", "-q", "--amend", "--no-edit",
            env=dict(w.env, GIT_COMMITTER_DATE=f"@{int(time.time()) - 50} +0000"))
        w.write_log()
        check("a run after the commit with the built DLL staged proves it",
              w.signal("git push -u origin launcher-drop-flag"),
              game_code="1", pending="0", fresh="1", dll_match="1", attached="1")

        # 4. A run that STARTED before the commit and is still logging after it. Its mtime is
        #    fresh; its birth is not, and the birth is when it loaded the code.
        if birth_time_supported(w.game):
            git(w.repo, "commit", "-q", "--amend", "--no-edit",
                env=dict(w.env, GIT_COMMITTER_DATE=f"@{int(time.time()) + 100} +0000"))
            log = w.game / "ds2-loader.log"
            later = time.time() + 200
            os.utime(log, (later, later))
            check("a session born before the commit is not fresh because it is still writing",
                  w.signal("git push"), game_code="1", fresh="0")
        else:
            print("  SKIPPED the birth-time case: this filesystem reports no birth time")

        # 5. `git push origin <branch>` from a checkout sitting on a docs branch is judged on <branch>.
        w2 = World(Path(tmp) / "refspec")
        git(w2.repo, "checkout", "-q", "-b", "game", env=w2.env)
        (w2.repo / "crates" / "ds2-x" / "src" / "lib.rs").write_text("// changed\n")
        git(w2.repo, "add", "-A", env=w2.env)
        w2.commit("feat(ds2-x): y")
        git(w2.repo, "checkout", "-q", "-b", "docs-only", "main", env=w2.env)
        (w2.repo / "docs" / "a.md").write_text("b\n")
        git(w2.repo, "add", "-A", env=w2.env)
        w2.commit("docs: b")
        check("a named refspec is diffed, not HEAD", w2.signal("git push origin game"), game_code="1")
        check("HEAD on a docs branch is out of jurisdiction", w2.signal("git push -u origin docs-only"),
              game_code="0", pending="0")
        check("a chained commit of docs only stays out of jurisdiction",
              w2.signal("git commit -am 'docs: c' && git push"), game_code="0", pending="1")
        check("--all is game code", w2.signal("git push --all origin"), game_code="1")

        # 5b. ds2-mods-rs-lgor: the push runs in a worktree the command cds into, while the hook is
        #     invoked from the main checkout. The worktree built the staged DLL and ran it after its
        #     commit; the main checkout's build is a different binary. Judged on the main checkout,
        #     this was `dll_match=0` -- the false refusal.
        w4 = World(Path(tmp) / "worktree")
        wt = w4.repo / ".claude" / "worktrees" / "agent-a"
        git(w4.repo, "worktree", "add", "-q", "-b", "build-url-dialog", str(wt), env=w4.env)
        (wt / "crates" / "ds2-x" / "src" / "lib.rs").write_text("// dialog\n")
        git(wt, "add", "-A", env=w4.env)
        when = f"@{int(time.time()) - 50} +0000"
        git(wt, "commit", "-q", "-m", "feat(ds2-x): dialog",
            env=dict(w4.env, GIT_COMMITTER_DATE=when, GIT_AUTHOR_DATE=when))
        (wt / BUILT_REL).parent.mkdir(parents=True)
        (wt / BUILT_REL).write_bytes(b"MZ worktree dll")
        shutil.copy2(wt / BUILT_REL, w4.game / "dinput8.dll")
        os.utime(w4.game / "dinput8.dll", (time.time() - 60, time.time() - 60))
        w4.write_log()
        verbatim = (
            "H=/home/banon/projects/ds2-mods-rs-wt-hk/scripts/ds2-harness.sh; timeout 8 $H 'block 30' "
            "'buttons 0x2000 4' >/dev/null 2>&1; sleep 0.5; timeout 8 $H 'block 30' 'buttons 0x2000 4' "
            f">/dev/null 2>&1; cd {wt} && git push -u origin build-url-dialog 2>&1 | tail -2"
        )
        proven = dict(game_code="1", attached="1", fresh="1", dll_match="1", pending="0", pushdir="1")
        check("cd into the worktree, then push: judged on the worktree", w4.signal(verbatim), **proven)
        check("a relative cd resolves against the event's cwd",
              w4.signal("cd .claude/worktrees/agent-a && git push -u origin build-url-dialog"), **proven)
        check("git -C <worktree> push: judged on the worktree",
              w4.signal(f"git -C {wt} push -u origin build-url-dialog"), **proven)
        check("a plain push from the main checkout is still judged on the main checkout",
              w4.signal("git push -u origin main"), dll_match="0", pushdir="1")
        check("a cd the signal cannot resolve is refused, not guessed",
              w4.signal('cd "$WT" && git push'), game_code="1", pushdir="0")
        check("a cd into a directory that is not a work tree is refused",
              w4.signal(f"cd {w4.root} && git push"), game_code="1", pushdir="0")

        # 6. The whole path through the real engine: `cupcake eval` over this checkout's .cupcake/,
        #    in a repository shaped like the one the miss happened in. This is what proves cupcake
        #    hands the pending event to the signal on stdin -- if it did not, `pending` would stay
        #    0, `game_code` would read the empty diff, and the verdict would be the allow of
        #    2026-09-25.
        if shutil.which("cupcake") and "RUNTIME_EVIDENCE_SIGNAL_UNDER_TEST" not in os.environ:
            w3 = World(Path(tmp) / "engine")
            shutil.rmtree(w3.repo / ".cupcake")
            shutil.copytree(REPO_ROOT / ".cupcake", w3.repo / ".cupcake")
            shutil.copytree(REPO_ROOT / "scripts", w3.repo / "scripts", dirs_exist_ok=True)
            (w3.repo / "scripts" / "ds2-run.py").write_text("FLAGS = ['--no-offline']\n")
            git(w3.repo, "add", "-A", env=w3.env)
            w3.commit("chore: tree", offset=-900)
            git(w3.repo, "push", "-q", "origin", "main", env=w3.env)
            git(w3.repo, "fetch", "-q", "origin", env=w3.env)
            git(w3.repo, "checkout", "-q", "-b", "launcher-drop-flag", env=w3.env)
            w3.write_log()
            (w3.repo / "scripts" / "ds2-run.py").write_text("FLAGS = []\n")
            event = {
                "hook_event_name": "PreToolUse", "session_id": "test-runtime-evidence-signal",
                "transcript_path": str(w3.root / "t.jsonl"), "cwd": str(w3.repo),
                "permission_mode": "default", "tool_name": "Bash",
                "tool_input": {"command": CHAINED, "description": "Commit and push"},
            }
            verdict = subprocess.run(
                ["cupcake", "eval", "--harness", "claude", "--strict", "--log-level", "error"],
                input=json.dumps(event), capture_output=True, text=True, cwd=w3.repo,
                env=dict(w3.env, CLAUDE_PROJECT_DIR=str(w3.repo)), timeout=60,
            ).stdout
            # The pending-commit reason specifically: a denial from some other guard would pass a
            # looser test and prove nothing about this signal.
            denied = "commits and pushes in one go" in verdict
            bad += 0 if denied else 1
            print(f"  {'ok  ' if denied else 'FAIL'} the real engine refuses the verbatim command"
                  + ("" if denied else f": {verdict[:400]!r}"))
        else:
            print("  SKIPPED the engine case: no cupcake on PATH, or an older signal is under test")

    if bad:
        print(f"runtime evidence signal: {bad} FAILED", file=sys.stderr)
        return 1
    print("runtime evidence signal: all cases passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
