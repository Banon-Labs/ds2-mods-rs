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

    def write_log(self, build: str | None = None) -> None:
        """A fresh run: the DLL creates its log anew, so the file's birth is the run's start.

        `build` is the `build git=` value ds2-loader's first line carries, or None for a DLL that
        predates it.
        """
        log = self.game / "ds2-loader.log"
        log.unlink(missing_ok=True)
        first = f"ds2-loader 0.1.0 build git={build} module=S:\\x\\DINPUT8.dll\n" if build else ""
        log.write_text(first + "ds2-loader: attach awaiting-arxan-callback\n")

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

        # 5b. The 2026-09-26 refusal: game code committed and run, then a docs-only commit on top.
        #     The run still covers every game-code commit on the branch, so it stays fresh.
        w4 = World(Path(tmp) / "docs-on-top")
        git(w4.repo, "checkout", "-q", "-b", "game-then-docs", env=w4.env)
        (w4.repo / "crates" / "ds2-x" / "src" / "lib.rs").write_text("// changed\n")
        git(w4.repo, "add", "-A", env=w4.env)
        w4.commit("feat(ds2-x): y", offset=-50)
        w4.write_log()
        (w4.repo / "docs" / "a.md").write_text("after the run\n")
        git(w4.repo, "add", "-A", env=w4.env)
        w4.commit("docs: after the run", offset=100)
        check("a docs commit after the run does not make the run stale",
              w4.signal("git push -u origin game-then-docs"), game_code="1", fresh="1")
        (w4.repo / "crates" / "ds2-x" / "src" / "lib.rs").write_text("// changed again\n")
        git(w4.repo, "add", "-A", env=w4.env)
        w4.commit("fix(ds2-x): z", offset=200)
        check("a game-code commit after the run still makes it stale",
              w4.signal("git push -u origin game-then-docs"), game_code="1", fresh="0")

        # 5c. A log naming its build commit is judged by commits, not times.
        w5 = World(Path(tmp) / "build-sha")
        git(w5.repo, "checkout", "-q", "-b", "sha", env=w5.env)
        (w5.repo / "crates" / "ds2-x" / "src" / "lib.rs").write_text("// v1\n")
        git(w5.repo, "add", "-A", env=w5.env)
        w5.commit("feat(ds2-x): v1", offset=300)  # dated after the log: times alone would refuse
        built = git(w5.repo, "rev-parse", "HEAD").strip()
        w5.write_log(build=built)
        check("a run naming the pushed commit is fresh whatever the clock says",
              w5.signal("git push -u origin sha"), game_code="1", fresh="1")
        w5.write_log(build=built + "-dirty")
        check("a dirty build covers nothing", w5.signal("git push -u origin sha"), fresh="0")
        w5.write_log(build=built)
        (w5.repo / "docs" / "a.md").write_text("later\n")
        git(w5.repo, "add", "-A", env=w5.env)
        w5.commit("docs: later", offset=400)
        check("a docs commit after the named build stays covered",
              w5.signal("git push -u origin sha"), fresh="1")
        (w5.repo / "crates" / "ds2-x" / "src" / "lib.rs").write_text("// v2\n")
        git(w5.repo, "add", "-A", env=w5.env)
        w5.commit("fix(ds2-x): v2", offset=-900)  # dated before the log: times alone would allow
        check("a game-code commit after the named build is not covered",
              w5.signal("git push -u origin sha"), fresh="0")

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
