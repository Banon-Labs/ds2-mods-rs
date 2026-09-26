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


# --- the third instrument: the build ---------------------------------------------------------
#
# Added 2026-09-26 (PR #199, host-tests). A hand-written `extern "system"` block in
# `ds2-invasion-path` carried no `#[link(name = "user32")]`, and the MSVC test link failed:
#
#     lld-link: error: undefined symbol: GetAsyncKeyState
#     >>> referenced by .../crates/ds2-invasion-path/src/lib.rs:378
#
# The defect is in the linker's input. Frida reaches the game and telemetry reaches a running DLL;
# a DLL that does not link never runs, so neither instrument can see it, and the gate demanded one
# of them anyway. For that class the instrument is the build, and its measurement is the error it
# printed. The conditions that keep it a measurement:
#
#   * the quoted line is a WHOLE line of the log, not a fragment of one, and it is a compiler or
#     linker error header (`error[E..]: ...`, `error: ...`, `lld-link: error: ...`);
#   * the log is a failed cargo build: it carries cargo's own `error: could not compile` footer;
#   * the error's own location lines (`--> path` from rustc, `>>> referenced by path` from lld),
#     between it and the next diagnostic, name a `.rs` file under `crates/<crate>/` -- so the crate
#     it opens is the one the compiler blamed, not the one the agent names;
#   * the log is newer than the last committed Rust change, as for the other two instruments;
#   * it opens that one crate and nothing else.
#
# What it cannot tell: that the fix the agent then writes is the right one. Neither can the other
# two. The build that follows the edit is what answers that.

# A diagnostic header, after its indentation and any `= note: ` wrapper are taken off. rustc wraps
# the linker's stderr in `= note:`, which is where an lld error line actually sits in a cargo log.
BUILD_ERROR_HEADER = re.compile(
    r"\A(?:error(?:\[E\d{4}\])?: \S|(?:lld-link|rust-lld|ld\.lld|ld64\.lld|ld): error: \S)"
)
# Any diagnostic header, error or not: where one error's block ends.
ANY_DIAGNOSTIC_HEADER = re.compile(
    r"\A(?:(?:error|warning)(?:\[[A-Z]\d{4}\])?:|(?:lld-link|rust-lld|ld\.lld|ld64\.lld|ld): (?:error|warning):)"
)
# The location lines rustc and lld print under an error, and the path they carry.
BUILD_LOCATION = re.compile(r"\A(?:-->|>>> referenced by|>>> defined at)\s+(\S+?\.rs)(?::\d+)*\b")
CARGO_FAILED = "error: could not compile"
# How far below its header an error's location lines may sit. lld lists a few `>>>` lines; rustc
# puts `-->` on the next line. Past this it is some other diagnostic's business.
BUILD_BLOCK_LINES = 12


def _diagnostic_text(raw: str) -> str:
    text = raw.strip()
    if text.startswith("= note:"):
        text = text[len("= note:") :].strip()
    return text


def _blamed_crates(lines: list[str], header: int) -> set[str]:
    """The crates whose `.rs` files the error at `lines[header]` names in its location lines."""
    crates: set[str] = set()
    for raw in lines[header + 1 : header + 1 + BUILD_BLOCK_LINES]:
        text = _diagnostic_text(raw)
        if ANY_DIAGNOSTIC_HEADER.match(text):
            break
        found = BUILD_LOCATION.match(text)
        if not found:
            continue
        parts = pathlib.PurePosixPath(found.group(1).replace("\\", "/")).parts
        for index, part in enumerate(parts[:-1]):
            if part == "crates" and index + 1 < len(parts) - 1:
                crates.add(parts[index + 1])
                break
    return crates


def record_build(repo: pathlib.Path, crate: str, log: str, line: str) -> int:
    """Append a build-error record, after proving the compiler blamed a file in `crate`."""
    if not CRATE_NAME.match(crate):
        print(f"refused: crate name {crate!r} is not a bare `[A-Za-z0-9_-]+` directory name")
        return 2
    crate_dir = repo / "crates" / crate
    if not crate_dir.is_dir():
        print(f"refused: {crate_dir} is not a crate in this workspace")
        return 2
    if "\n" in line.strip() or "\r" in line:
        print("refused: quote one line of the log, not several")
        return 2
    quoted = _diagnostic_text(line)
    if len(quoted) < MIN_TELEMETRY_LINE:
        print(
            f"refused: the quoted line is {len(quoted)} characters, "
            f"under the {MIN_TELEMETRY_LINE} a verbatim check needs to mean anything"
        )
        return 2
    if not BUILD_ERROR_HEADER.match(quoted):
        print(
            "refused: that is not a compiler or linker error line -- quote the `error: ...`, "
            "`error[E....]: ...` or `lld-link: error: ...` header itself"
        )
        return 2

    log_file = pathlib.Path(log).expanduser()
    try:
        body = log_file.read_text(encoding="utf-8", errors="replace")
    except OSError as err:
        print(f"refused: cannot read {log_file}: {err}")
        return 2
    if CARGO_FAILED not in body:
        print(f"refused: {log_file} has no `{CARGO_FAILED}` line, so it is not a failed cargo build")
        return 2
    lines = body.splitlines()
    headers = [i for i, raw in enumerate(lines) if _diagnostic_text(raw) == quoted]
    if not headers:
        print(
            f"refused: no line of {log_file} is that line, whole -- quote the error line as the "
            f"build printed it, not a fragment of it"
        )
        return 2
    blamed: set[str] = set()
    for header in headers:
        blamed |= _blamed_crates(lines, header)
    if crate not in blamed:
        named = ", ".join(sorted(blamed)) or "no file under crates/"
        print(
            f"refused: that error's location lines (`-->` / `>>> referenced by`) name {named}, "
            f"not crates/{crate}/ -- the build decides which crate it blamed"
        )
        return 2

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
        "kind": "build",
        "crate": crate,
        "log": str(log_file),
        "line": " ".join(quoted.split()),
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
    kind = row.get("kind")
    if kind in ("telemetry", "build"):
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
            f"PROVEN {kind} crate={row.get('crate', '?')} "
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

        # --- the third instrument: a failed build ----------------------------------------
        #
        # Shaped on the PR #199 host-tests log: rustc wraps lld's stderr in `= note:`, and the
        # file lld blames sits on the `>>> referenced by` line under the error, not in it.
        (fake_repo / "crates" / "demo-path").mkdir(parents=True)
        (fake_repo / "crates" / "demo-effects").mkdir(parents=True)
        lld_error = "lld-link: error: undefined symbol: GetAsyncKeyState"
        build_log = fake_repo / "build.log"
        build_log.write_text(
            "   Compiling demo-effects v0.1.0 (/w/crates/demo-effects)\n"
            "error: linking with `lld-link` failed: exit status: 1\n"
            "  |\n"
            f"  = note: {lld_error}\n"
            "          >>> referenced by /w/.claude/worktrees/x/crates/demo-path/src/lib.rs:378\n"
            "          >>>               demo_effects.rcgu.o:(demo_path::poll)\n"
            "\n"
            "error: could not compile `demo-effects` (lib test) due to 1 previous error\n",
            encoding="utf-8",
        )

        def build(crate: str, line: str, log_file: pathlib.Path = build_log) -> int:
            with contextlib.redirect_stdout(io.StringIO()):
                return record_build(fake_repo, crate, str(log_file), line)

        ok(
            "a build error opens the crate its `>>> referenced by` line names",
            build("demo-path", lld_error) == 0,
        )
        verdict = io.StringIO()
        with contextlib.redirect_stdout(verdict):
            code = check(empty)
        ok(
            "the build verdict opens with the field the policy anchors on",
            code == 0 and verdict.getvalue().startswith("PROVEN build crate=demo-path "),
        )
        ok(
            "the crate that was compiling, but not blamed, is refused",
            build("demo-effects", lld_error) == 2,
        )
        ok(
            "the `= note:` spelling of the same line is the same line",
            build("demo-path", f"= note: {lld_error}") == 0,
        )
        ok(
            "a fragment of the error line is refused",
            build("demo-path", "error: undefined symbol: GetAsyncKeyState") == 2,
        )
        ok(
            "a location line is not an error line",
            build(
                "demo-path",
                ">>> referenced by /w/.claude/worktrees/x/crates/demo-path/src/lib.rs:378",
            )
            == 2,
        )
        ok(
            "cargo's footer names no file, so it blames no crate",
            build("demo-effects", "error: could not compile `demo-effects` (lib test) due to 1 previous error") == 2,
        )
        ok(
            "a crate this workspace does not have is refused",
            build("not-a-crate", lld_error) == 2,
        )
        not_a_build = fake_repo / "notes.log"
        not_a_build.write_text(
            f"{lld_error}\n>>> referenced by crates/demo-path/src/lib.rs:1\n", encoding="utf-8"
        )
        ok(
            "a log without cargo's `could not compile` footer is not a failed build",
            build("demo-path", lld_error, not_a_build) == 2,
        )
        rustc_log = fake_repo / "rustc.log"
        rustc_log.write_text(
            "error[E0425]: cannot find value `x` in this scope\n"
            " --> crates/demo-path/src/lib.rs:4:5\n"
            "  |\n"
            "error: could not compile `demo-path` (lib) due to 1 previous error\n",
            encoding="utf-8",
        )
        ok(
            "a rustc error opens the crate its `-->` line names",
            build("demo-path", "error[E0425]: cannot find value `x` in this scope", rustc_log)
            == 0,
        )
        ok(
            "and not a crate it does not name",
            build("demo-effects", "error[E0425]: cannot find value `x` in this scope", rustc_log)
            == 2,
        )

        # Staleness, against a real git history: a Rust commit dated after the log was written
        # means the log measured code that is already committed.
        git_repo = pathlib.Path(tmp) / "gitrepo"
        (git_repo / "crates" / "demo-path" / "src").mkdir(parents=True)
        (git_repo / "crates" / "demo-path" / "src" / "lib.rs").write_text("", encoding="utf-8")
        future = str(int(time.time()) + 86400)
        git_env = dict(
            os.environ,
            GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t",
            GIT_COMMITTER_EMAIL="t@t", GIT_AUTHOR_DATE=f"@{future} +0000",
            GIT_COMMITTER_DATE=f"@{future} +0000",
        )
        for cmd in (["init", "-q"], ["add", "-A"], ["commit", "-q", "--no-verify", "-m", "x"]):
            subprocess.run(["git", "-C", str(git_repo), *cmd], env=git_env, check=True,
                           capture_output=True)
        stale_log = git_repo / "build.log"
        stale_log.write_text(build_log.read_text(encoding="utf-8"), encoding="utf-8")
        with contextlib.redirect_stdout(io.StringIO()):
            stale = record_build(git_repo, "demo-path", str(stale_log), lld_error)
        ok("a build log older than the last committed Rust change is refused", stale == 2)

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
    parser.add_argument(
        "--record-build",
        action="store_true",
        help="append a compiler/linker-error record, licensing the one crate the error blamed",
    )
    parser.add_argument("--crate", default="", help="the crate the telemetry/build record licenses")
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
    if args.record_build:
        return record_build(REPO_ROOT, args.crate, args.log, args.line)
    if args.check:
        return check(REPO_ROOT)
    parser.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
