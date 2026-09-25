#!/usr/bin/env python3
"""Print the `Run-Stamp:` line a PR body footer needs, for the commit HEAD points at.

    python3 scripts/pr-run-stamp.py --from-last-check          # from scripts/check.sh's own record
    python3 scripts/pr-run-stamp.py --gate runtime --result pass [--at 2026-09-25T10:00:00Z] [k=v ...]
    python3 scripts/pr-run-stamp.py --check-body <file> [--need-pass]   # would the guard accept it?

Format and rules: scripts/cupcake_run_stamp.py. The guard that reads it:
.cupcake/policies/claude/pr_requires_run_stamp.rego.

`--from-last-check` refuses a record for a different commit (you committed after the run) or one
taken with uncommitted changes in the tree (the run did not test the commit), because in both cases
the stamp would describe a run of something else. Re-run scripts/check.sh.
"""
from __future__ import annotations

import argparse
import datetime as dt
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cupcake_run_stamp as rs  # noqa: E402


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout.strip()


def die(msg: str) -> None:
    print(f"pr-run-stamp: {msg}", file=sys.stderr)
    raise SystemExit(1)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--rev", default="HEAD", help="commit to stamp (default HEAD)")
    ap.add_argument("--from-last-check", action="store_true", help="use scripts/check.sh's recorded run")
    ap.add_argument("--gate", help="what ran, one word: runtime, check.sh, ds2-run.py ...")
    ap.add_argument("--result", choices=["pass", "fail"])
    ap.add_argument("--at", help="when the run finished, YYYY-MM-DDTHH:MM:SSZ (default: now)")
    ap.add_argument("--check-body", metavar="FILE", help="judge a body file against --rev instead of printing")
    ap.add_argument("--need-pass", action="store_true", help="with --check-body: apply the `gh pr ready` rule")
    ap.add_argument("extra", nargs="*", help="extra key=value fields, e.g. log=br-20260925-1")
    a = ap.parse_args()

    sha = git("rev-parse", "--verify", a.rev + "^{commit}")
    commit_epoch = int(git("log", "-1", "--format=%ct", sha))

    if a.check_body:
        with open(a.check_body, encoding="utf-8") as f:
            ok, why = rs.judge(f.read(), sha, commit_epoch, rs._now(), need_pass=a.need_pass)
        print(f"{'ok' if ok else 'REFUSED'}: {why} (sha={sha})")
        return 0 if ok else 1

    extra = {}
    for kv in a.extra:
        k, sep, v = kv.partition("=")
        if not sep:
            die(f"extra field {kv!r} is not key=value")
        extra[k] = v

    if a.from_last_check:
        if a.gate or a.result or a.at:
            die("--from-last-check takes gate, result and time from the record; do not pass them")
        path = git("rev-parse", "--path-format=absolute", "--git-path", "ds2-last-check")
        try:
            fields = dict(p.split("=", 1) for p in open(path, encoding="utf-8").read().split())
        except (OSError, ValueError):
            die(f"no readable check.sh record at {path}; run scripts/check.sh first")
        if fields.get("sha") != sha:
            die(f"the last check.sh run tested {fields.get('sha')}, not {sha}; run scripts/check.sh again")
        if fields.get("tree") != "clean":
            die("the last check.sh run had uncommitted changes in the tree, so it did not test the commit; "
                "commit, then run scripts/check.sh again")
        at = rs.parse_stamps(f"Run-Stamp: sha={sha} at={fields.get('at')} gate=x result=pass")
        if not at:
            die(f"the record's time {fields.get('at')!r} is not YYYY-MM-DDTHH:MM:SSZ")
        print(rs.format_stamp(sha, at[0]["at"], fields["gate"], fields["result"], extra))
        return 0

    if not a.gate or not a.result:
        die("give --from-last-check, or both --gate and --result")
    if a.at:
        try:
            at_epoch = int(dt.datetime.strptime(a.at, "%Y-%m-%dT%H:%M:%SZ")
                           .replace(tzinfo=dt.timezone.utc).timestamp())
        except ValueError:
            die(f"--at {a.at!r} is not YYYY-MM-DDTHH:MM:SSZ")
    else:
        at_epoch = rs._now()
    if at_epoch < commit_epoch:
        die("--at is before the commit's own time, so that run cannot have run this commit")
    try:
        print(rs.format_stamp(sha, at_epoch, a.gate, a.result, extra))
    except ValueError as e:
        die(str(e))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
