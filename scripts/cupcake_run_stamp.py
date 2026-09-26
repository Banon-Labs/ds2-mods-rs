"""Run-Stamp: the footer line that ties a pull request to the last run of the commit it carries.

User directive 2026-09-25, their words: "a PR shouldn't be able to be drafted unless it has a
timestamped/sha stamped or other metadata stamped line(s) related to the commit that is paired with
both the drafting of the PR, and then the marking as ready".

THE FORMAT, one line, fields in this order, space separated, extra `key=value` fields allowed after
the four required ones:

    Run-Stamp: sha=<40 hex> at=<YYYY-MM-DDTHH:MM:SSZ> gate=<name> result=<pass|fail> [key=value ...]

  sha     the commit the run tested -- the PR's head commit, full length.
  at      when the run finished, UTC. Must be at or after that commit's committer time: a run older
          than the commit cannot have run the commit.
  gate    what ran: `check.sh`, `runtime`, a script name. No spaces.
  result  pass or fail.

Written by `scripts/pr-run-stamp.py`, never by hand.

TWO GATES read it, via `.cupcake/signals/pr_run_stamp.sh` and the policy
`.cupcake/policies/claude/pr_requires_run_stamp.rego`:

  * `gh pr create` is refused unless the body carries a stamp whose sha is the commit being proposed
    (`--head <branch>` if given, else HEAD of the invoking checkout) and whose time is fresh. The
    invoking checkout is the event's cwd moved by any `cd` in front of the call; when `--repo` names
    a repository that checkout has no remote for, the branch is read from GitHub instead.
  * `gh pr ready` is refused unless the LIVE body (`gh pr view --json body,headRefOid,commits`) has a
    stamp whose sha is the PR's current headRefOid, fresh against that commit's time, and the latest
    such stamp says `result=pass`. A push after drafting therefore needs a new run and a new stamp.

The global draft guard (`~/.config/cupcake/policies/claude/github_pr_draft_guard.rego`) refuses
`gh pr ready` outright today. The ready arm here is the gate that is left standing if that ever
changes, and it is the condition a human marking the PR ready in the web UI is expected to check.

All decisions are made here, in Python, and reported to Rego as `RUNSTAMP|verb=..|ok=..|why=..`,
because the WASM runtime cupcake evaluates policies in silently drops composed regexes (see
scripts/check-cupcake-wasm-builtins.py). The Rego side only maps a verdict to a denial.
"""
from __future__ import annotations

import datetime as _dt
import json
import os
import re
import shlex
import subprocess

STAMP_PREFIX = "Run-Stamp:"

# Unanchored on purpose. The hook engine rewrites unquoted newlines and welds heredoc bodies onto
# their reader before a signal sees the command, so a stamp typed on its own line may arrive in the
# middle of one. The field grammar is strict enough that position is not needed to find it.
STAMP_RE = re.compile(
    r"Run-Stamp:[ \t]*sha=(?P<sha>[0-9a-fA-F]{40})[ \t]+at=(?P<at>\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)"
    r"[ \t]+gate=(?P<gate>[^\s=]+)[ \t]+result=(?P<result>pass|fail)(?=\s|$)"
)

# A stamp from the future is a hand-typed one. Five minutes absorbs clock skew between this machine
# and nothing else -- both times it is compared with come from here or from git.
FUTURE_SKEW_SECONDS = 300


def format_stamp(sha: str, at_epoch: int, gate: str, result: str, extra: dict[str, str] | None = None) -> str:
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError(f"sha must be 40 lowercase hex characters, got {sha!r}")
    if not gate or re.search(r"[\s=]", gate):
        raise ValueError(f"gate must be one word with no '=', got {gate!r}")
    if result not in ("pass", "fail"):
        raise ValueError(f"result must be pass or fail, got {result!r}")
    at = _dt.datetime.fromtimestamp(at_epoch, _dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    line = f"{STAMP_PREFIX} sha={sha} at={at} gate={gate} result={result}"
    for k, v in (extra or {}).items():
        if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", k) or not v or re.search(r"\s", v):
            raise ValueError(f"extra field must be key=value with no spaces, got {k!r}={v!r}")
        line += f" {k}={v}"
    return line


def parse_stamps(text: str) -> list[dict]:
    out = []
    for m in STAMP_RE.finditer(text or ""):
        at = _dt.datetime.strptime(m.group("at"), "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=_dt.timezone.utc)
        out.append({
            "sha": m.group("sha").lower(),
            "at": int(at.timestamp()),
            "gate": m.group("gate"),
            "result": m.group("result"),
        })
    return out


def judge(body: str, sha: str, commit_epoch: int, now_epoch: int, need_pass: bool) -> tuple[bool, str]:
    """(ok, why). `why` is a stable code the policy turns into a sentence."""
    stamps = parse_stamps(body)
    if not stamps:
        return False, "missing"
    mine = [s for s in stamps if s["sha"] == sha.lower()]
    if not mine:
        return False, "wrong-sha"
    live = [s for s in mine if commit_epoch <= s["at"] <= now_epoch + FUTURE_SKEW_SECONDS]
    if not live:
        if any(s["at"] > now_epoch + FUTURE_SKEW_SECONDS for s in mine):
            return False, "future"
        return False, "stale"
    if need_pass:
        latest = max(live, key=lambda s: s["at"])
        if latest["result"] != "pass":
            return False, "not-pass"
    return True, "ok"


# --------------------------------------------------------------------------------------------------
# The command: which gh call, and what body it carries
# --------------------------------------------------------------------------------------------------

WRAPPERS = {"sudo", "doas", "env", "command", "nohup", "time", "builtin", "exec", "xargs", "nice",
            "stdbuf", "setsid", "then", "do", "else", "elif", "!"}


def _strip_heredocs(cmd: str) -> str:
    out, until = [], None
    for line in cmd.split("\n"):
        if until is not None:
            if line.strip() == until:
                until = None
            continue
        out.append(line)
        m = re.search(r"<<-?\s*([\"']?)([A-Za-z_][A-Za-z0-9_]*)\1", line)
        if m:
            until = m.group(2)
    return "\n".join(out)


def _cd_target(argv: list[str], here: str | None) -> str | None:
    """The directory `cd <argv>` lands in, or None when it cannot be known from the text."""
    if here is None:
        return None
    operands = [a for a in argv[1:] if a not in ("-L", "-P", "--")]
    if not operands:
        return os.path.expanduser("~")
    target = operands[0]
    if target == "-" or "$" in target or "`" in target:
        return None
    target = os.path.expanduser(target)
    if not os.path.isabs(target):
        if not here:
            return None
        target = os.path.join(here, target)
    return os.path.normpath(target)


def gh_invocations(cmd: str, cwd: str = "") -> list[tuple[list[str], str | None]]:
    """Every `gh` argv in the command, with the directory it runs in.

    The directory is the event's cwd moved by each `cd` in front of the call (bd ds2-mods-rs-3sb):
    `cd <other checkout> && gh pr create ...` proposes a commit of THAT checkout, and resolving it in
    the session checkout either fails or names the wrong commit. A `cd` inside `( ... )` only moves
    the subshell. None means a `cd` the text cannot resolve (`cd -`, `cd "$dir"`), and the caller
    then resolves nothing rather than guessing.
    """
    text = _strip_heredocs(cmd)
    try:
        lex = shlex.shlex(text, posix=True, punctuation_chars="();<>|&")
        lex.whitespace_split = True
        toks = list(lex)
    except ValueError:
        toks = text.split()
    out: list[tuple[list[str], str | None]] = []
    here: str | None = cwd
    stack: list[str | None] = []
    seg: list[str] = []

    def flush() -> None:
        nonlocal here
        i = 0
        while i < len(seg) and (re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", seg[i]) or seg[i] in WRAPPERS):
            i += 1
        argv = seg[i:]
        if not argv:
            return
        name = os.path.basename(argv[0])
        if name == "gh":
            out.append((argv, here))
        elif name == "cd":
            here = _cd_target(argv, here)
        elif name in ("pushd", "popd"):
            here = None

    for t in toks:
        if t and all(c in "();<>|&" for c in t):
            flush()
            seg = []
            for c in t:
                if c == "(":
                    stack.append(here)
                elif c == ")" and stack:
                    here = stack.pop()
        else:
            seg.append(t)
    flush()
    return out


def _flag_value(argv: list[str], names: tuple[str, ...]) -> str | None:
    val = None
    for i, t in enumerate(argv):
        for n in names:
            if t == n and i + 1 < len(argv):
                val = argv[i + 1]
            elif n.startswith("--") and t.startswith(n + "="):
                val = t[len(n) + 1:]
    return val


def body_of_create(argv: list[str], raw_cmd: str, cwd: str) -> str:
    inline = _flag_value(argv, ("--body", "-b"))
    path = _flag_value(argv, ("--body-file", "-F"))
    if path is not None:
        if path == "-":
            return raw_cmd  # the heredoc feeding stdin is still in the raw command
        p = os.path.expanduser(os.path.expandvars(path))
        if not os.path.isabs(p):
            p = os.path.join(cwd or "", p)
        try:
            with open(p, encoding="utf-8", errors="replace") as f:
                return f.read()
        except OSError:
            return ""
    if inline is not None:
        if "$(" in inline or "`" in inline:
            return raw_cmd  # an unexpanded substitution: its heredoc text is in the raw command
        return inline
    return ""


def _positional(argv: list[str]) -> str | None:
    """First non-flag operand after `gh pr ready`, skipping values of the flags that take one."""
    takes_value = {"-R", "--repo"}
    i = 3
    while i < len(argv):
        t = argv[i]
        if t in takes_value:
            i += 2
            continue
        if t.startswith("-"):
            i += 1
            continue
        return t
    return None


def _git(cwd: str, *args: str) -> str:
    r = subprocess.run(["git", "-C", cwd or ".", *args], capture_output=True, text=True, timeout=5)
    return r.stdout.strip() if r.returncode == 0 else ""


def _repo_slug(spec: str) -> str:
    """`owner/name`, lowercased, from `owner/name`, `host/owner/name` or a remote URL."""
    s = spec.strip().rstrip("/")
    if s.endswith(".git"):
        s = s[:-4]
    parts = [p for p in re.split(r"[/:]", s) if p]
    return "/".join(parts[-2:]).lower() if len(parts) >= 2 else ""


def _remote_for(cwd: str, repo: str) -> str | None:
    """The name of the remote in `cwd` that points at `repo`, or None when none does."""
    want = _repo_slug(repo)
    for line in _git(cwd, "remote", "-v").splitlines():
        fields = line.split()
        if len(fields) >= 2 and _repo_slug(fields[1]) == want:
            return fields[0]
    return None


def _branch_via_api(repo: str, head_branch: str | None, cwd: str) -> tuple[str, int]:
    """(sha, committer epoch) of a branch as GitHub has it, for a repo with no local checkout here."""
    slug = _repo_slug(repo)
    if not slug:
        return "", 0
    owner, name = slug.split("/", 1)
    branch = head_branch or _git(cwd, "branch", "--show-current")
    if not branch:
        return "", 0
    if ":" in branch:
        owner, branch = branch.split(":", 1)
    override = os.environ.get("CUPCAKE_PR_STAMP_BRANCH_API_OVERRIDE")
    try:
        if override:
            data = json.loads(override)
        else:
            r = subprocess.run(["gh", "api", f"repos/{owner}/{name}/branches/{branch}"],
                               cwd=cwd or None, capture_output=True, text=True, timeout=15)
            if r.returncode != 0:
                return "", 0
            data = json.loads(r.stdout)
        sha = (data.get("commit") or {}).get("sha") or ""
        date = (((data.get("commit") or {}).get("commit") or {}).get("committer") or {}).get("date") or ""
    except (OSError, ValueError, AttributeError, subprocess.SubprocessError):
        return "", 0
    if not re.fullmatch(r"[0-9a-fA-F]{40}", sha) or not date:
        return "", 0
    return sha.lower(), _iso_epoch(date)


def head_of(cwd: str | None, head_branch: str | None, repo: str | None = None) -> tuple[str, int]:
    """(sha, committer epoch) of the commit a `gh pr create` proposes.

    Resolved in `cwd`, the directory the call runs in after any leading `cd`. When `--repo` names a
    repository that checkout has no remote for, the checkout cannot say what that repository's
    branch holds (bd ds2-mods-rs-3sb: a PR for another repo, created from this session's checkout,
    was refused as `no-head`), so the branch is read from GitHub instead.
    """
    override = os.environ.get("CUPCAKE_PR_STAMP_HEAD_OVERRIDE")
    if override:
        sha, epoch = override.split()
        return sha, int(epoch)
    if cwd is None:
        return "", 0
    remote = "origin"
    if repo:
        found = _remote_for(cwd, repo)
        if found is None:
            return _branch_via_api(repo, head_branch, cwd)
        remote = found
    rev = "HEAD"
    if head_branch:
        rev = head_branch.split(":", 1)[-1]
        if not _git(cwd, "rev-parse", "--verify", "--quiet", rev + "^{commit}"):
            rev = f"{remote}/{rev}"
    sha = _git(cwd, "rev-parse", "--verify", "--quiet", rev + "^{commit}")
    epoch = _git(cwd, "log", "-1", "--format=%ct", sha) if sha else ""
    return sha, int(epoch or 0)


def pr_view(cwd: str, argv: list[str]) -> dict:
    override = os.environ.get("CUPCAKE_PR_STAMP_VIEW_OVERRIDE")
    if override:
        return json.loads(override)
    cmd = ["gh", "pr", "view"]
    sel = _positional(argv)
    if sel:
        cmd.append(sel)
    repo = _flag_value(argv, ("-R", "--repo"))
    if repo:
        cmd += ["--repo", repo]
    cmd += ["--json", "body,headRefOid,commits"]
    r = subprocess.run(cmd, cwd=cwd or None, capture_output=True, text=True, timeout=20)
    if r.returncode != 0:
        return {}
    return json.loads(r.stdout)


def _iso_epoch(s: str) -> int:
    return int(_dt.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp())


def _now() -> int:
    o = os.environ.get("CUPCAKE_PR_STAMP_NOW_OVERRIDE")
    return int(o) if o else int(_dt.datetime.now(_dt.timezone.utc).timestamp())


def verdict(event: dict) -> str:
    ti = event.get("tool_input") or {}
    cmd = ti.get("command") if isinstance(ti, dict) else None
    if not isinstance(cmd, str):
        return "RUNSTAMP|verb=none"
    cwd = event.get("cwd") or ""
    for argv, here in gh_invocations(cmd, cwd):
        low = [a.lower() for a in argv]
        if len(low) >= 3 and low[1] == "pr" and low[2] == "create":
            body = body_of_create(argv, cmd, here or "")
            sha, epoch = head_of(here, _flag_value(argv, ("--head", "-H")), _flag_value(argv, ("--repo", "-R")))
            if not sha:
                return "RUNSTAMP|verb=create|ok=0|why=no-head"
            ok, why = judge(body, sha, epoch, _now(), need_pass=False)
            return f"RUNSTAMP|verb=create|ok={int(ok)}|why={why}|sha={sha}"
        if len(low) >= 3 and low[1] == "pr" and low[2] == "ready" and "--undo" not in low:
            try:
                pr = pr_view(here or "", argv)
            except (OSError, ValueError, subprocess.SubprocessError):
                pr = {}
            head = (pr.get("headRefOid") or "").lower()
            if not head:
                return "RUNSTAMP|verb=ready|ok=0|why=no-pr"
            epoch = 0
            for c in pr.get("commits") or []:
                if (c.get("oid") or "").lower() == head and c.get("committedDate"):
                    epoch = _iso_epoch(c["committedDate"])
            if not epoch:
                return f"RUNSTAMP|verb=ready|ok=0|why=no-commit-time|sha={head}"
            ok, why = judge(pr.get("body") or "", head, epoch, _now(), need_pass=True)
            return f"RUNSTAMP|verb=ready|ok={int(ok)}|why={why}|sha={head}"
    return "RUNSTAMP|verb=none"
