#!/usr/bin/env python3
"""Tests for the Run-Stamp half that decides: scripts/cupcake_run_stamp.py and scripts/pr-run-stamp.py.

`opa test` pins verdict -> denial; scripts/test-cupcake-policies.py drives the live engine. This pins
which body earns which verdict, through the same `verdict()` the signal calls, with git and gh pinned
by the CUPCAKE_PR_STAMP_* overrides so the result never depends on this checkout's HEAD or the network.
"""
from __future__ import annotations

import datetime
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import cupcake_run_stamp as rs  # noqa: E402

SHA = "0123456789abcdef0123456789abcdef01234567"
OTHER = "fedcba9876543210fedcba9876543210fedcba98"
COMMIT = 1790300000  # 2026-09-25T01:33:20Z
NOW = COMMIT + 3600

failures: list[str] = []


def check(name: str, got, want) -> None:
    if got != want:
        failures.append(f"{name}: got {got!r}, want {want!r}")


def stamp(sha=SHA, at=COMMIT + 60, gate="check.sh", result="pass", extra=None) -> str:
    return rs.format_stamp(sha, at, gate, result, extra)


def body(*lines: str) -> str:
    return "## What changed\n\nx\n\n## Why\n\ny\n\n## Evidence\n\nz\n\n---\n" + "\n".join(lines) + "\n"


def with_env(**kw):
    class Ctx:
        def __enter__(self):
            self.old = {k: os.environ.get(k) for k in kw}
            os.environ.update({k: str(v) for k, v in kw.items()})

        def __exit__(self, *a):
            for k, v in self.old.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
    return Ctx()


def create_verdict(cmd: str, cwd: str = "/nonexistent") -> str:
    with with_env(CUPCAKE_PR_STAMP_HEAD_OVERRIDE=f"{SHA} {COMMIT}", CUPCAKE_PR_STAMP_NOW_OVERRIDE=NOW):
        return rs.verdict({"tool_input": {"command": cmd}, "cwd": cwd})


def ready_verdict(pr_body: str, head: str = SHA, cmd: str = "gh pr ready 69") -> str:
    view = {"body": pr_body, "headRefOid": head,
            "commits": [{"oid": OTHER, "committedDate": "2026-01-01T00:00:00Z"},
                        {"oid": head, "committedDate": "2026-09-25T01:33:20Z"}]}
    with with_env(CUPCAKE_PR_STAMP_VIEW_OVERRIDE=json.dumps(view), CUPCAKE_PR_STAMP_NOW_OVERRIDE=NOW):
        return rs.verdict({"tool_input": {"command": cmd}, "cwd": "/nonexistent"})


# --- format -------------------------------------------------------------------------------------
line = stamp(extra={"log": "br-1"})
check("format", line, f"Run-Stamp: sha={SHA} at=2026-09-25T01:34:20Z gate=check.sh result=pass log=br-1")
check("roundtrip", rs.parse_stamps(line), [{"sha": SHA, "at": COMMIT + 60, "gate": "check.sh", "result": "pass"}])
for bad in [dict(sha="abc"), dict(gate="two words"), dict(result="ok")]:
    try:
        stamp(**bad)
        failures.append(f"format accepted {bad}")
    except ValueError:
        pass
check("short sha not a stamp", rs.parse_stamps("Run-Stamp: sha=0123456 at=2026-09-25T04:54:20Z gate=x result=pass"), [])
check("local time not a stamp", rs.parse_stamps(f"Run-Stamp: sha={SHA} at=2026-09-25T04:54:20 gate=x result=pass"), [])
# Newlines erased or welded by the engine: the stamp is still found mid-line.
check("mid-line", len(rs.parse_stamps("footer text " + line + " more")), 1)

# --- gh pr create -------------------------------------------------------------------------------
with tempfile.TemporaryDirectory() as d:
    good = Path(d, "good.md")
    good.write_text(body(line, "Written by x, authorized by @y"))
    none = Path(d, "none.md")
    none.write_text(body("Written by x, authorized by @y"))
    wrong = Path(d, "wrong.md")
    wrong.write_text(body(stamp(sha=OTHER)))
    stale = Path(d, "stale.md")
    stale.write_text(body(stamp(at=COMMIT - 60)))
    future = Path(d, "future.md")
    future.write_text(body(stamp(at=NOW + 3600)))
    failed = Path(d, "failed.md")
    failed.write_text(body(stamp(result="fail")))
    both = Path(d, "both.md")
    both.write_text(body(stamp(sha=OTHER), line))

    base = "gh pr create --draft --title 'feat(x): y' --body-file "
    check("create good", create_verdict(base + str(good)), f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
    check("create missing", create_verdict(base + str(none)), f"RUNSTAMP|verb=create|ok=0|why=missing|sha={SHA}")
    check("create wrong sha", create_verdict(base + str(wrong)), f"RUNSTAMP|verb=create|ok=0|why=wrong-sha|sha={SHA}")
    check("create stale", create_verdict(base + str(stale)), f"RUNSTAMP|verb=create|ok=0|why=stale|sha={SHA}")
    check("create future", create_verdict(base + str(future)), f"RUNSTAMP|verb=create|ok=0|why=future|sha={SHA}")
    # A draft may carry a failing run; it may not carry none.
    check("create failed run ok", create_verdict(base + str(failed)), f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
    check("create one of two stamps matches", create_verdict(base + str(both)), f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
    check("create relative body file", create_verdict(base + "good.md", cwd=d), f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
    check("create unreadable body file", create_verdict(base + str(Path(d, "gone.md"))), f"RUNSTAMP|verb=create|ok=0|why=missing|sha={SHA}")
    check("create after cd &&", create_verdict(f"cd {d} && " + base + str(good)), f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
check("create inline body", create_verdict(f"gh pr create -d -t x -b '{line}'"), f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
check("create stdin heredoc", create_verdict(f"gh pr create -d -t x --body-file - <<'EOF'\n## What changed\n{line}\nEOF"),
      f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
check("create substituted heredoc", create_verdict(f"gh pr create -d -t x --body \"$(cat <<'EOF'\n{line}\nEOF\n)\""),
      f"RUNSTAMP|verb=create|ok=1|why=ok|sha={SHA}")
check("create no body", create_verdict("gh pr create --draft --fill"), f"RUNSTAMP|verb=create|ok=0|why=missing|sha={SHA}")
# The heredoc's text is not a command: a body mentioning `gh pr create` in a file write is not one.
check("heredoc mention", create_verdict("cat > /tmp/b.md <<'EOF'\ngh pr create --draft\nEOF"), "RUNSTAMP|verb=none")
check("quoted mention", create_verdict("git commit -m 'gh pr create needs a stamp'"), "RUNSTAMP|verb=none")

# --- gh pr create for ANOTHER repository (bd ds2-mods-rs-3sb) -------------------------------------
# Real throwaway checkouts, no HEAD override: `session` stands in for this repository (on main, and
# carrying a same-named `ds2-paramdefs` branch as a trap), `other` for the fromsoftware-rs worktree.
GIT_ID = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]


def git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(repo), *GIT_ID, *args], check=True, capture_output=True,
                          text=True).stdout.strip()


def new_repo(path: Path, branch: str, remote: str, url: str) -> str:
    subprocess.run(["git", "init", "-q", "-b", branch, str(path)], check=True)
    git(path, "remote", "add", remote, url)
    git(path, "commit", "-q", "--allow-empty", "-m", f"{path.name} {branch}")
    return git(path, "rev-parse", "HEAD")


def branch_api(sha: str, epoch: int) -> dict:
    """The part of `gh api repos/<o>/<r>/branches/<b>` the gate reads."""
    date = datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return {"commit": {"sha": sha, "commit": {"committer": {"date": date}}}}


def real_verdict(cmd: str, cwd: str, api: dict | None = None) -> str:
    env = {"CUPCAKE_PR_STAMP_BRANCH_API_OVERRIDE": json.dumps(api)} if api is not None else {}
    saved = {k: os.environ.pop(k, None) for k in ("CUPCAKE_PR_STAMP_HEAD_OVERRIDE", "CUPCAKE_PR_STAMP_NOW_OVERRIDE")}
    try:
        with with_env(**env):
            return rs.verdict({"tool_input": {"command": cmd}, "cwd": cwd})
    finally:
        for k, v in saved.items():
            if v is not None:
                os.environ[k] = v


with tempfile.TemporaryDirectory() as d:
    session, other = Path(d, "session"), Path(d, "other")
    session_head = new_repo(session, "main", "origin", "https://github.com/Banon-Labs/ds2-mods-rs.git")
    git(session, "checkout", "-q", "-b", "ds2-paramdefs")
    git(session, "commit", "-q", "--allow-empty", "-m", "trap")
    trap = git(session, "rev-parse", "HEAD")
    git(session, "checkout", "-q", "main")
    other_head = new_repo(other, "ds2-paramdefs", "fork", "https://github.com/chozandrias76/fromsoftware-rs.git")
    other_epoch = int(git(other, "log", "-1", "--format=%ct"))
    session_epoch = int(git(session, "log", "-1", "--format=%ct", "main"))
    Path(other, "body.txt").write_text(body(stamp(sha=other_head, at=other_epoch + 1)))
    other_body = str(Path(other, "body.txt"))
    session_body = Path(d, "session-body.txt")
    session_body.write_text(body(stamp(sha=session_head, at=session_epoch + 1)))
    api = branch_api(other_head, other_epoch)
    create = "gh pr create --draft --repo chozandrias76/fromsoftware-rs --base main --head ds2-paramdefs --title T "

    # The issue's own reproduction: a leading cd into the other checkout, relative body file there.
    check("cross-repo after cd", real_verdict(f"cd {other} && {create}--body-file body.txt", str(session)),
          f"RUNSTAMP|verb=create|ok=1|why=ok|sha={other_head}")
    # Invoked from the other checkout itself: its `fork` remote is the --repo, resolved locally.
    check("cross-repo from its checkout", real_verdict(create + "--body-file body.txt", str(other)),
          f"RUNSTAMP|verb=create|ok=1|why=ok|sha={other_head}")
    # From this checkout with no cd: it has no remote for --repo, so GitHub answers -- and the
    # same-named local branch (`trap`) is not mistaken for the other repository's.
    check("cross-repo via api", real_verdict(create + f"--body-file {other_body}", str(session), api),
          f"RUNSTAMP|verb=create|ok=1|why=ok|sha={other_head}")
    check("cross-repo branch not on github", real_verdict(create + f"--body-file {other_body}", str(session), {}),
          "RUNSTAMP|verb=create|ok=0|why=no-head")
    check("cross-repo stamp for the trap branch", real_verdict(create + f"--body-file {other_body}", str(session),
          branch_api(trap, other_epoch)),
          f"RUNSTAMP|verb=create|ok=0|why=wrong-sha|sha={trap}")
    # This repository, unchanged: HEAD of the invoking checkout, and --head resolved locally.
    check("this repo HEAD", real_verdict(f"gh pr create --draft --title T --body-file {session_body}", str(session)),
          f"RUNSTAMP|verb=create|ok=1|why=ok|sha={session_head}")
    check("this repo --head", real_verdict(f"gh pr create --draft --head main --title T --body-file {session_body}",
                                           str(session)), f"RUNSTAMP|verb=create|ok=1|why=ok|sha={session_head}")
    # A cd inside a subshell moves nothing; the local ds2-paramdefs is this checkout's trap branch.
    check("subshell cd does not move", real_verdict(
        f"(cd {other}) && gh pr create --draft --head ds2-paramdefs --title T --body-file {other_body}", str(session)),
        f"RUNSTAMP|verb=create|ok=0|why=wrong-sha|sha={trap}")
    # A cd the text cannot resolve resolves nothing.
    check("unresolvable cd", real_verdict(f'cd "$W" && {create}--body-file {other_body}', str(session)),
          "RUNSTAMP|verb=create|ok=0|why=no-head")

# --- gh pr ready --------------------------------------------------------------------------------
check("ready good", ready_verdict(body(line)), f"RUNSTAMP|verb=ready|ok=1|why=ok|sha={SHA}")
check("ready runtime stamp", ready_verdict(body(stamp(gate="runtime", extra={"log": "ds2-loader.log"}))),
      f"RUNSTAMP|verb=ready|ok=1|why=ok|sha={SHA}")
# Pushed after drafting: the draft's stamp names the old head.
check("ready stale sha", ready_verdict(body(line), head=OTHER), f"RUNSTAMP|verb=ready|ok=0|why=wrong-sha|sha={OTHER}")
check("ready missing", ready_verdict(body()), f"RUNSTAMP|verb=ready|ok=0|why=missing|sha={SHA}")
check("ready stale time", ready_verdict(body(stamp(at=COMMIT - 1))), f"RUNSTAMP|verb=ready|ok=0|why=stale|sha={SHA}")
check("ready latest failed", ready_verdict(body(stamp(at=COMMIT + 10), stamp(at=COMMIT + 20, result="fail"))),
      f"RUNSTAMP|verb=ready|ok=0|why=not-pass|sha={SHA}")
check("ready fail then pass", ready_verdict(body(stamp(at=COMMIT + 10, result="fail"), stamp(at=COMMIT + 20))),
      f"RUNSTAMP|verb=ready|ok=1|why=ok|sha={SHA}")
check("ready undo", ready_verdict(body(), cmd="gh pr ready 69 --undo"), "RUNSTAMP|verb=none")
with with_env(CUPCAKE_PR_STAMP_VIEW_OVERRIDE="{}"):
    check("ready no pr", rs.verdict({"tool_input": {"command": "gh pr ready"}}), "RUNSTAMP|verb=ready|ok=0|why=no-pr")
check("ready selector", rs._positional(["gh", "pr", "ready", "-R", "o/r", "69"]), "69")

# --- the signal wrapper exits 0 and prints a verdict --------------------------------------------
sig = HERE.parent / ".cupcake" / "signals" / "pr_run_stamp.sh"
r = subprocess.run([str(sig)], input=json.dumps({"tool_input": {"command": "ls"}}), capture_output=True, text=True)
check("signal non-gh", (r.returncode, r.stdout.strip()), (0, "RUNSTAMP|verb=none"))
r = subprocess.run([str(sig)], input="not json gh", capture_output=True, text=True)
check("signal garbage exits 0", r.returncode, 0)

# --- the helper ---------------------------------------------------------------------------------
helper = [sys.executable, str(HERE / "pr-run-stamp.py")]
r = subprocess.run(helper + ["--gate", "runtime", "--result", "pass", "log=x"], capture_output=True, text=True, cwd=HERE)
head = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=HERE).stdout.strip()
parsed = rs.parse_stamps(r.stdout)
check("helper prints a stamp for HEAD", (r.returncode, [p["sha"] for p in parsed]), (0, [head]))
check("helper keeps extras", r.stdout.strip().endswith(" log=x"), True)
r = subprocess.run(helper + ["--gate", "two words", "--result", "pass"], capture_output=True, text=True, cwd=HERE)
check("helper refuses a bad gate", r.returncode, 1)
r = subprocess.run(helper + ["--gate", "x", "--result", "pass", "--at", "2000-01-01T00:00:00Z"], capture_output=True, text=True, cwd=HERE)
check("helper refuses a run older than the commit", r.returncode, 1)

if failures:
    print("\n".join("FAIL " + f for f in failures))
    raise SystemExit(1)
print("run-stamp tests passed")
