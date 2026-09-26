"""Stack-merge guard: `gh pr merge --delete-branch` while another open PR is based on the head branch.

Incident 2026-09-26: the base PRs of stacks were merged with `--delete-branch`. Deleting a branch that
other open pull requests use as their base makes GitHub CLOSE those pull requests instead of
retargeting them, and a PR merged in the middle of that window lands in the dependent's branch rather
than main. #107, #108, #115 and #129 were closed that way, #131 merged into #129's branch, and all of
them had to be recreated as #139-#143.

Read by `.cupcake/signals/stack_merge_dependents.sh` and mapped to a refusal by
`.cupcake/policies/claude/no_delete_branch_under_stack.rego`. All deciding is here, because the WASM
runtime cupcake evaluates policies in drops composed regexes (scripts/check-cupcake-wasm-builtins.py).

Emits one line:

    STACKMERGE|verb=none                                         no `gh pr merge` in the command
    STACKMERGE|verb=merge|delete=0                               merges, keeps the branch
    STACKMERGE|verb=merge|delete=1|ok=1|pr=<n>|head=<b>          deletes a branch nobody is based on
    STACKMERGE|verb=merge|delete=1|ok=0|why=dependents|pr=<n>|head=<b>|base=<b>|deps=<n>,<n>
    STACKMERGE|verb=merge|delete=1|ok=0|why=unchecked            GitHub could not be asked

`unchecked` is refused, like `no-pr` in cupcake_run_stamp.py: an unanswered question is not a "no".
Only the delete is refused, never the merge, so a merge without `--delete-branch` is always open.

CUPCAKE_STACK_MERGE_OVERRIDE pins GitHub's answer for the tests: a JSON object
`{"number": n, "headRefName": b, "baseRefName": b, "isCrossRepository": false, "dependents": [n...]}`,
or the string `unreachable`. An agent setting it to pass the gate is writing the evidence itself.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys

try:
    from cupcake_run_stamp import gh_invocations, _flag_value
except ImportError:  # run as a file from elsewhere
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from cupcake_run_stamp import gh_invocations, _flag_value

# `gh pr merge --help`, verified 2026-09-25 against gh on this machine.
VALUE_LONG = {"--author-email", "--body", "--body-file", "--match-head-commit", "--subject", "--repo"}
VALUE_SHORT = set("AbFtR")
GH_TIMEOUT = 10


def deletes_branch(argv: list[str]) -> bool:
    """True when the flags ask gh to delete the head branch. The last spelling wins, as in pflag."""
    delete = False
    i = 3
    while i < len(argv):
        t = argv[i]
        if t == "--":
            break
        if t.startswith("--"):
            name, eq, val = t.partition("=")
            if name == "--delete-branch":
                delete = (val.lower() not in ("false", "0", "f")) if eq else True
            elif name in VALUE_LONG and not eq:
                i += 1
        elif t.startswith("-") and len(t) > 1:
            j = 1
            while j < len(t):
                c = t[j]
                if c in VALUE_SHORT:
                    if j == len(t) - 1:
                        i += 1
                    break
                if c == "d":
                    if t[j + 1:j + 2] == "=":
                        delete = t[j + 2:].lower() not in ("false", "0", "f")
                        break
                    delete = True
                j += 1
        i += 1
    return delete


def selector(argv: list[str]) -> str | None:
    """The `<number> | <url> | <branch>` operand, or None for the current branch's PR."""
    i = 3
    while i < len(argv):
        t = argv[i]
        if t == "--":
            return argv[i + 1] if i + 1 < len(argv) else None
        if t.startswith("--"):
            if "=" not in t and t in VALUE_LONG:
                i += 1
        elif t.startswith("-") and len(t) > 1:
            for j, c in enumerate(t[1:], start=1):
                if c in VALUE_SHORT:
                    if j == len(t) - 1:
                        i += 1
                    break
                if c == "=":
                    break
        else:
            return t
        i += 1
    return None


def _gh(args: list[str], cwd: str) -> object:
    r = subprocess.run(["gh", *args], cwd=cwd or None, capture_output=True, text=True, timeout=GH_TIMEOUT)
    if r.returncode != 0:
        raise OSError(r.stderr.strip() or f"gh exited {r.returncode}")
    return json.loads(r.stdout)


def ask_github(argv: list[str], cwd: str) -> dict:
    """PR <N>'s head/base and the open PRs based on its head. Raises when GitHub cannot be asked."""
    override = os.environ.get("CUPCAKE_STACK_MERGE_OVERRIDE")
    if override:
        if override.strip() == "unreachable":
            raise OSError("override: unreachable")
        return json.loads(override)
    repo = _flag_value(argv, ("-R", "--repo"))
    repo_args = ["--repo", repo] if repo else []
    view = ["pr", "view"]
    sel = selector(argv)
    if sel:
        view.append(sel)
    pr = _gh(view + repo_args + ["--json", "number,headRefName,baseRefName,isCrossRepository"], cwd)
    head = pr.get("headRefName") or ""
    if not head:
        raise OSError("gh pr view returned no headRefName")
    deps = _gh(["pr", "list", "--state", "open", "--base", head, "--limit", "100",
                "--json", "number"] + repo_args, cwd)
    pr["dependents"] = [d.get("number") for d in deps if d.get("number") != pr.get("number")]
    return pr


def verdict(event: dict) -> str:
    ti = event.get("tool_input") or {}
    cmd = ti.get("command") if isinstance(ti, dict) else None
    if not isinstance(cmd, str):
        return "STACKMERGE|verb=none"
    for argv, here in gh_invocations(cmd, event.get("cwd") or ""):
        low = [a.lower() for a in argv]
        if not (len(low) >= 3 and low[1] == "pr" and low[2] == "merge"):
            continue
        if not deletes_branch(argv):
            continue
        if here is None:
            return "STACKMERGE|verb=merge|delete=1|ok=0|why=unchecked"
        try:
            pr = ask_github(argv, here)
        except (OSError, ValueError, AttributeError, TypeError, subprocess.SubprocessError):
            return "STACKMERGE|verb=merge|delete=1|ok=0|why=unchecked"
        head = pr.get("headRefName") or ""
        tag = f"pr={pr.get('number')}|head={head}"
        deps = [str(n) for n in pr.get("dependents") or [] if n is not None]
        if pr.get("isCrossRepository") or not deps:
            return f"STACKMERGE|verb=merge|delete=1|ok=1|{tag}"
        return (f"STACKMERGE|verb=merge|delete=1|ok=0|why=dependents|{tag}"
                f"|base={pr.get('baseRefName') or 'main'}|deps={','.join(deps)}")
    for argv, _ in gh_invocations(cmd, event.get("cwd") or ""):
        low = [a.lower() for a in argv]
        if len(low) >= 3 and low[1] == "pr" and low[2] == "merge":
            return "STACKMERGE|verb=merge|delete=0"
    return "STACKMERGE|verb=none"


def _selftest() -> int:
    failures: list[str] = []

    def check(name, got, want):
        if got != want:
            failures.append(f"{name}: got {got!r}, want {want!r}")

    def run(cmd, override):
        old = os.environ.get("CUPCAKE_STACK_MERGE_OVERRIDE")
        os.environ["CUPCAKE_STACK_MERGE_OVERRIDE"] = override
        try:
            return verdict({"tool_input": {"command": cmd}, "cwd": "/nonexistent"})
        finally:
            if old is None:
                os.environ.pop("CUPCAKE_STACK_MERGE_OVERRIDE", None)
            else:
                os.environ["CUPCAKE_STACK_MERGE_OVERRIDE"] = old

    stacked = json.dumps({"number": 129, "headRefName": "boot-timeline", "baseRefName": "main",
                          "isCrossRepository": False, "dependents": [131]})
    alone = json.dumps({"number": 129, "headRefName": "boot-timeline", "baseRefName": "main",
                        "isCrossRepository": False, "dependents": []})
    denied = "STACKMERGE|verb=merge|delete=1|ok=0|why=dependents|pr=129|head=boot-timeline|base=main|deps=131"
    allowed = "STACKMERGE|verb=merge|delete=1|ok=1|pr=129|head=boot-timeline"

    check("long flag, dependent", run("gh pr merge 129 --squash --delete-branch", stacked), denied)
    check("short flag, dependent", run("gh pr merge 129 -s -d", stacked), denied)
    check("clustered short flag", run("gh pr merge 129 -sd", stacked), denied)
    check("after cd", run("cd /x && gh pr merge 129 --delete-branch", stacked), denied)
    check("no dependents", run("gh pr merge 129 --squash --delete-branch", alone), allowed)
    check("no delete", run("gh pr merge 129 --squash", stacked), "STACKMERGE|verb=merge|delete=0")
    check("delete=false", run("gh pr merge 129 --delete-branch=false", stacked), "STACKMERGE|verb=merge|delete=0")
    check("-b value is not -d", run("gh pr merge 129 -s -b d", stacked), "STACKMERGE|verb=merge|delete=0")
    check("-bd is a body", run("gh pr merge 129 -s -bd", stacked), "STACKMERGE|verb=merge|delete=0")
    check("unreachable", run("gh pr merge 129 -d", "unreachable"), "STACKMERGE|verb=merge|delete=1|ok=0|why=unchecked")
    check("unresolvable cd", run('cd "$d" && gh pr merge 129 -d', stacked), "STACKMERGE|verb=merge|delete=1|ok=0|why=unchecked")
    check("other gh", run("gh pr view 129 --json headRefName", stacked), "STACKMERGE|verb=none")
    check("quoted mention", run("git commit -m 'gh pr merge 129 -d'", stacked), "STACKMERGE|verb=none")

    check("selector number", selector(["gh", "pr", "merge", "-R", "o/r", "129", "-d"]), "129")
    check("selector after -t value", selector(["gh", "pr", "merge", "-t", "subj", "br"]), "br")
    check("selector none", selector(["gh", "pr", "merge", "--squash", "-d"]), None)

    for f in failures:
        print("FAIL", f)
    print(f"cupcake_stack_merge selftest: {'FAILED' if failures else 'OK'}")
    return 1 if failures else 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        raise SystemExit(_selftest())
    try:
        ev = json.load(sys.stdin)
    except ValueError:
        raise SystemExit(0)
    print(verdict(ev))
