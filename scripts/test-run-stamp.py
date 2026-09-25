#!/usr/bin/env python3
"""Tests for the Run-Stamp half that decides: scripts/cupcake_run_stamp.py and scripts/pr-run-stamp.py.

`opa test` pins verdict -> denial; scripts/test-cupcake-policies.py drives the live engine. This pins
which body earns which verdict, through the same `verdict()` the signal calls, with git and gh pinned
by the CUPCAKE_PR_STAMP_* overrides so the result never depends on this checkout's HEAD or the network.
"""
from __future__ import annotations

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
