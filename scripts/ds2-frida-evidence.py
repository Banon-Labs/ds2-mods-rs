#!/usr/bin/env python3
"""Record and judge Frida evidence, so a Rust edit has to be preceded by a measurement.

Ported from `../er-mods-rs/scripts/er-frida-evidence.py` on 2026-09-25, unchanged except for its
names: the log is `~/.local/state/ds2-mods-rs/frida-evidence.jsonl`, the override is
`DS2_FRIDA_EVIDENCE_LOG`. `scripts/ds2-frida-watch.py` was already calling a recorder beside itself
-- as `er-frida-evidence.py`, a file this repo never had -- so every watch here until now ended with
"could not record frida evidence" and recorded nothing. The history below is er-mods-rs's.

In this repo every DLL crate writes into the one `<Game>/ds2-loader.log`, so a telemetry record
names that log and the crate whose line it quotes:

    python3 scripts/ds2-frida-evidence.py --record-telemetry \
        --crate ds2-menu-row \
        --log "$GAME/ds2-loader.log" \
        --line "<a line ds2-menu-row wrote in this run, verbatim>"

# Why this exists

User directive 2026-09-16, after the agent tore the game down twice and ran two build/relaunch
cycles to land a change whose mechanism was never measured: "why would you build, if you haven't
proven you can do it with Frida yet?" AGENTS.md already said it -- "The order is Frida, then Frida,
then Frida, and only then a DLL ... Build a DLL when the mechanism is already known and the code is
the product, never to find something out" -- and the agent did it anyway. A note that is ignored is
not a rule, so this is the executable half.

`.cupcake/policies/claude/no_rust_edit_without_frida_proof.rego` refuses a Write/Edit under
`crates/**/*.rs` unless this says `PROVEN`, and the `--check` verdict is the only thing that opens
that door.

# What counts as evidence

A record is appended by the Frida entry points themselves when a session ends, carrying what that
session actually did: the agent file, the pid it attached to, how many `send()` messages came back,
and how long it ran. A session that attached and observed nothing reports `messages=0` and does not
count -- the point is going and looking, and a watcher that saw nothing did not look at anything.

The log lives under `XDG_STATE_HOME` (default `~/.local/state/ds2-mods-rs/`), not in the repo, so the
gate's evidence is not a file the gated Write tool can create.

# The second instrument, and the blind spot it closes

Frida reaches the game. It does not reach our own DLLs. A release `cdylib` in this workspace
exports `DllMain` and nothing else, so an unexported Rust static, a `pub(crate)` seam, or the
question "which of our functions calls which of our setters" has no address for `Interceptor` to
attach to and no name for `DebugSymbol` to resolve. For that class of change the gate used to
demand an instrument which physically cannot see the subject, and an agent facing it either stalls
or reaches for the forgery routes named below -- neither of which is the behaviour this was written
to get.

Measured 2026-09-19, which is the run this section exists for. `er-save-game-row` opened its
destination browser undressed because `gfx_swap::set_profile_05_010_edit_armed` has exactly one
caller, in `arm::arm_standalone`, and that shell hand-rolled its arm instead. (It merged into
`er-quit-menu` on 2026-09-20 and `arm_standalone` took over its installs; the example below names
the crate as it was measured.) The defect was already measured -- by our own code, at the branch,
in a live run:

    05_010 stats-panel edit not armed -- no browse row and no host that dresses a character row
    served 05_010_profileselect (the picker's own cache key) ... memory_replacement=false

That is not weaker than a Frida hook. At a branch it is stronger: a hook outside the module has to
infer which way the branch went from its effects, while the branch itself says so. So in-process
telemetry from a live run is admitted as evidence -- under three conditions that keep it a
measurement rather than an assertion:

  * the quoted line must be present verbatim in the named log, so it comes off a run rather than
    out of an argument;
  * the log must be newer than the last committed Rust change, the same staleness rule Frida
    evidence lives under;
  * it licenses one crate -- the shell whose telemetry it is -- and not the tree.

That last one makes it narrower than the Frida path, which opens every crate at once. (The er
example here named an er crate and its own log; the DS2 form is at the top of this docstring.)

# How far that goes, honestly

This heading used to read "and why it cannot be written by hand", which was false, and a false claim
here is worse than the hole it papers over -- the next agent believes it. Measured against the live
engine 2026-09-16, with no evidence present, every one of these was allowed:

    python3 scripts/ds2-frida-evidence.py --record --agent x --pid 1 --messages 12 --seconds 9
    echo '{"at": 9999999999, "messages": 12}' >> ~/.local/state/ds2-mods-rs/frida-evidence.jsonl
    an Edit to this file, to `.cupcake/signals/frida_evidence.sh`, or to the policy itself
    a Bash `cat > crates/<crate>/src/lib.rs <<EOF`, which is not an Edit and is not routed

The `XDG_STATE_HOME` placement closes exactly one route, the repo-relative Write, and nothing else.
Everything above is one command. So this is a gate against an agent that reaches for a build when it
should reach for a hook -- the failure it was written for -- and not against one that has decided to
get around it. Treat the two differently: the first is what the refusal text talks to, and the
second is a choice no amount of rego reaches.

# What makes a record stale

Evidence is spent by a commit. A record counts only if it is newer than `HEAD`'s commit time, so one
measurement licenses the edits of one change and the next change needs its own. Without that the
first Frida run of a session would license every edit after it forever, which is the same "I looked
once" excuse in a machine-readable costume.

    python3 scripts/ds2-frida-evidence.py --record --agent scripts/frida/x.js --pid 388 --messages 12
    python3 scripts/ds2-frida-evidence.py --check
    python3 scripts/ds2-frida-evidence.py --selftest
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import time

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent


def state_dir() -> pathlib.Path:
    """Where the evidence log lives: user state, deliberately outside the repo."""
    root = os.environ.get("XDG_STATE_HOME") or os.path.join(
        os.path.expanduser("~"), ".local", "state"
    )
    return pathlib.Path(root) / "ds2-mods-rs"


def log_path() -> pathlib.Path:
    override = os.environ.get("DS2_FRIDA_EVIDENCE_LOG")
    if override:
        return pathlib.Path(override)
    return state_dir() / "frida-evidence.jsonl"


def head_commit_time(repo: pathlib.Path) -> int | None:
    """When the newest committed Rust change landed, or `None` outside a repo.

    A record older than this is spent: the change it measured has been committed, and the next
    change needs its own measurement.

    "Rust change" and not "commit". The gate this feeds exists to stop a `.rs` file under
    `crates/` being written without somebody going and looking first, so the thing that consumes
    a measurement is a committed Rust change -- and only that. Keying on plain `HEAD` made every
    commit spend it, including ones that cannot possibly have used it: on 2026-09-17 a
    `scripts/`-only commit (`a8c11bb1`, the launch-gate cache) spent a live measurement taken
    minutes earlier, and the next Rust edit was refused with `spent-by-commit` for a reason that
    had nothing to do with Rust. That is a false refusal, not a strict one, and a gate that
    refuses for the wrong reason teaches the next agent to look for a way around it.

    The pathspec is the same shape the policy uses to decide what it guards: `*.rs` under
    `crates/`. A commit that touches both is still a Rust commit and still spends.
    """
    try:
        out = subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "log",
                "-1",
                "--format=%ct",
                "--",
                "crates/**/*.rs",
            ],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if out.returncode != 0:
        return None
    try:
        return int(out.stdout.strip())
    except ValueError:
        return None


def record(agent: str, pid: int, messages: int, seconds: float) -> int:
    path = log_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    row = {
        "at": int(time.time()),
        "agent": agent,
        "pid": pid,
        "messages": messages,
        "seconds": round(seconds, 2),
    }
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(row) + "\n")
    print(f"frida-evidence: recorded {row}")
    return 0


CRATE_NAME = re.compile(r"\A[A-Za-z0-9_-]+\Z")

# Short enough to quote comfortably, long enough that no single word passes. A one-word `--line`
# would match somewhere in almost any log, which turns the verbatim check into a formality.
MIN_TELEMETRY_LINE = 20


def record_telemetry(repo: pathlib.Path, crate: str, log: str, line: str) -> int:
    """Append an in-process-telemetry record, after proving it describes a real run.

    Every refusal below is the difference between a measurement and a claim, so each prints what
    it wanted rather than a bare failure.
    """
    if not CRATE_NAME.match(crate):
        print(f"refused: crate name {crate!r} is not a bare `[A-Za-z0-9_-]+` directory name")
        return 2
    crate_dir = repo / "crates" / crate
    if not crate_dir.is_dir():
        print(f"refused: {crate_dir} is not a crate in this workspace")
        return 2
    if len(line.strip()) < MIN_TELEMETRY_LINE:
        print(
            f"refused: the quoted line is {len(line.strip())} characters, "
            f"under the {MIN_TELEMETRY_LINE} a verbatim check needs to mean anything"
        )
        return 2

    log_file = pathlib.Path(log).expanduser()
    try:
        body = log_file.read_text(encoding="utf-8", errors="replace")
    except OSError as err:
        print(f"refused: cannot read {log_file}: {err}")
        return 2
    if line.strip() not in body:
        print(f"refused: {log_file} does not contain that line, so it is not what the run said")
        return 2

    # The log has to come from a run that happened after the last committed Rust change, or it
    # describes code that is already in. Same rule the Frida path lives under, read off the file
    # the game wrote rather than off a timestamp handed in on the command line.
    try:
        written = int(log_file.stat().st_mtime)
    except OSError as err:
        print(f"refused: cannot stat {log_file}: {err}")
        return 2
    head = head_commit_time(repo)
    if head is not None and written <= head:
        print(
            f"refused: {log_file} was last written before the newest committed Rust change, "
            f"so it measured code that is already committed"
        )
        return 2

    path = log_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    row = {
        "at": int(time.time()),
        "kind": "telemetry",
        "crate": crate,
        "log": str(log_file),
        # Flattened, because the verdict line this ends up in is parsed by the policy and a
        # newline in the middle of it would split the verdict in half.
        "line": " ".join(line.split()),
        "written": written,
    }
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(row) + "\n")
    print(f"frida-evidence: recorded {row}")
    return 0


def newest_record(path: pathlib.Path) -> dict | None:
    """The last well-formed record, or `None`.

    Read from the end rather than parsed whole: this runs in front of every edit and the log grows
    without bound. A malformed trailing line is skipped rather than fatal -- a broken log must not
    take the policy down with it.
    """
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    for line in reversed(lines):
        line = line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict) and "at" in row:
            return row
    return None


def check(repo: pathlib.Path) -> int:
    """Print one verdict line. `PROVEN` is the only one the policy opens on."""
    row = newest_record(log_path())
    if row is None:
        print("UNPROVEN no-frida-evidence nothing has attached to the game and reported back")
        return 1

    head = head_commit_time(repo)
    if row.get("kind") == "telemetry":
        if head is not None and int(row.get("at", 0)) <= head:
            print(
                "UNPROVEN spent-by-commit the last measurement predates HEAD, so it belongs to "
                "a change that is already committed"
            )
            return 1
        # `crate=` sits directly after the two fixed words because the policy anchors its match
        # there: everything to the right of it is free text that must not be able to impersonate
        # the field that decides which crate this opens.
        print(
            f"PROVEN telemetry crate={row.get('crate', '?')} "
            f"log={row.get('log', '?')} line={row.get('line', '?')!r}"
        )
        return 0

    messages = int(row.get("messages", 0) or 0)
    if messages <= 0:
        print(
            f"UNPROVEN silent-session the last watch on {row.get('agent', '?')} "
            f"reported {messages} messages, so it observed nothing"
        )
        return 1

    if head is not None and int(row.get("at", 0)) <= head:
        print(
            "UNPROVEN spent-by-commit the last measurement predates HEAD, so it belongs to "
            "a change that is already committed"
        )
        return 1

    print(
        f"PROVEN agent={row.get('agent', '?')} pid={row.get('pid', '?')} "
        f"messages={messages} seconds={row.get('seconds', '?')}"
    )
    return 0


def selftest() -> int:
    import contextlib
    import io
    import tempfile

    failures = 0

    def ok(label: str, condition: bool) -> None:
        nonlocal failures
        print(f"  {'ok  ' if condition else 'FAIL'}  {label}")
        failures += 0 if condition else 1

    with tempfile.TemporaryDirectory() as tmp:
        log = pathlib.Path(tmp) / "frida-evidence.jsonl"
        os.environ["DS2_FRIDA_EVIDENCE_LOG"] = str(log)
        empty = pathlib.Path(tmp) / "norepo"
        empty.mkdir()

        ok("an absent log is unproven", check(empty) == 1)

        record("scripts/frida/x.js", 388, 0, 3.0)
        ok("a silent session is unproven", check(empty) == 1)

        record("scripts/frida/x.js", 388, 12, 4.0)
        ok("a session that observed something is proven", check(empty) == 0)

        ok(
            "a malformed trailing line does not take the log down",
            (log.open("a", encoding="utf-8").write("{not json\n") or True)
            and check(empty) == 0,
        )

        # --- the second instrument -------------------------------------------------------
        #
        # A telemetry record has to describe a real run of a real crate, so every way of
        # handing it something else is a refusal rather than a weaker record.
        fake_repo = pathlib.Path(tmp) / "repo"
        (fake_repo / "crates" / "demo-crate").mkdir(parents=True)
        run_log = fake_repo / "run.log"
        quoted = "05_010 stats-panel edit not armed -- no browse row"
        run_log.write_text(f"demo: attached\ndemo: {quoted}\n", encoding="utf-8")

        ok(
            "a crate this workspace does not have is refused",
            record_telemetry(fake_repo, "not-a-crate", str(run_log), quoted) == 2,
        )
        ok(
            "a one-word quote is refused",
            record_telemetry(fake_repo, "demo-crate", str(run_log), "armed") == 2,
        )
        ok(
            "a line the log does not contain is refused",
            record_telemetry(fake_repo, "demo-crate", str(run_log), "a line nobody ever printed")
            == 2,
        )
        ok(
            "a verbatim line from a real log is recorded",
            record_telemetry(fake_repo, "demo-crate", str(run_log), quoted) == 0,
        )

        verdict = io.StringIO()
        with contextlib.redirect_stdout(verdict):
            code = check(empty)
        said = verdict.getvalue().strip()
        ok("telemetry evidence is proven", code == 0)
        # The policy anchors `^PROVEN telemetry crate=<name>` and reads nothing to the right of
        # it, so this prefix is a contract between the two files rather than a format detail.
        ok(
            "the verdict opens with the field the policy anchors on",
            said.startswith("PROVEN telemetry crate=demo-crate "),
        )

        ok("the log lives outside the repo by default", REPO_ROOT not in state_dir().parents)
        os.environ.pop("DS2_FRIDA_EVIDENCE_LOG", None)

    print("selftest: PASS" if not failures else f"selftest: {failures} check(s) failed")
    return 0 if not failures else 1


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="store_true", help="append a record and exit")
    parser.add_argument(
        "--record-telemetry",
        action="store_true",
        help="append an in-process-telemetry record, licensing one crate",
    )
    parser.add_argument("--crate", default="", help="the crate the telemetry record licenses")
    parser.add_argument("--log", default="", help="the live run's log file")
    parser.add_argument("--line", default="", help="a line that must be in that log verbatim")
    parser.add_argument("--agent", default="", help="the agent file that ran")
    parser.add_argument("--pid", type=int, default=0)
    parser.add_argument("--messages", type=int, default=0)
    parser.add_argument("--seconds", type=float, default=0.0)
    parser.add_argument("--check", action="store_true", help="print PROVEN or UNPROVEN")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args(argv)

    if args.selftest:
        return selftest()
    if args.record:
        return record(args.agent, args.pid, args.messages, args.seconds)
    if args.record_telemetry:
        return record_telemetry(REPO_ROOT, args.crate, args.log, args.line)
    if args.check:
        return check(REPO_ROOT)
    parser.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
