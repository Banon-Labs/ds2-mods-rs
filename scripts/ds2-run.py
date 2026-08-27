#!/usr/bin/env python3
"""Stage `dinput8.dll` into DARK SOULS II and launch it, gated on the DLL's own log line.

WHY A TOOL AND NOT AN AGENT'S SENTENCE
--------------------------------------
"I launched it and it worked" is a claim a tool can make and a person can check. This script
prints its success block ONLY after reading the loader DLL's `ds2-loader: arxan ...` line out of
the log file the DLL itself wrote during THIS run. Nothing else -- not the process existing, not
Steam returning zero, not a window appearing -- is accepted as evidence, because a copy-pasted
block is a promise to whoever reads it. On timeout it says so plainly and exits non-zero.

Patterned on `../er-mods-rs/scripts/er-run-branch.py`, whose testimony discipline (and whose two
hard-won log-tailing bugs -- rotation and partial lines, both commented below) this reproduces.

THE PIPELINE
------------
  1. PREFLIGHT   -- the built DLL exists; the game directory exists and is writable; `steam` is
                    on PATH. Refuse rather than guess.
  2. STAGE       -- copy the built DLL over `<Game>/dinput8.dll`.
  3. FINGERPRINT -- print the SHA-256 of the STAGED file, read back off disk. A build that
                    failed, or that "succeeded" without recompiling, leaves the previous DLL in
                    place; a run against that produces evidence for code which is not the code
                    under test. The hash is what makes that visible instead of invisible.
  4. CONFIGURE   -- write `<Game>/ds2-mods.toml` for the requested arm, and print it verbatim.
                    The DLL reads this itself in `DllMain`; see below.
  5. LAUNCH      -- `steam -applaunch 335300` with WINEDLLOVERRIDES="dinput8=n,b". Without that
                    override Wine's builtin dinput8 wins the load and our DLL never runs.
  6. TESTIMONY   -- poll `<Game>/ds2-loader.log` for the DLL's own line. Success block only if
                    it appears; a FAILED block, and a non-zero exit, otherwise.

WHY THE ARM IS IN A FILE AND NOT IN THE ENVIRONMENT
---------------------------------------------------
It was `DS2_ARXAN_PROBE=1` / `DS2_ARXAN_PROBE_SKIP_NEUTER=1`, set here, and it did not work. A
real run produced, from the DLL's own attach line:

    ds2-loader: attach awaiting-arxan-callback probe=off arm=neuter-arxan
                DS2_ARXAN_PROBE=<unset> DS2_ARXAN_PROBE_SKIP_NEUTER=<unset>

`steam -applaunch` hands the request to an ALREADY-RUNNING Steam client over IPC, and that client
starts the game from ITS environment. `WINEDLLOVERRIDES` survives only because it is in the
per-app Steam launch options -- a different channel, and the one setting that cannot move into the
config file, because Wine reads it to decide whether to map our DLL at all.

The fixes available for the environment were "quit Steam before every run" and "edit the launch
options between the two arms". Both are manual steps BETWEEN THE TWO HALVES OF ONE EXPERIMENT, and
a manual step there is a step that eventually gets skipped. A file beside the DLL travels through
no IPC, and both arms now run back to back with nothing to do in between.

THE M1 EXPERIMENT
-----------------
`--probe` runs the Arxan-survival experiment in `crates/ds2-loader/src/arxan_probe.rs`: one
MinHook detour, watched byte-for-byte to see whether Arxan reverts it. It has TWO ARMS and both
have to be run, because "the detour survived" means nothing on its own -- it is equally
consistent with "dearxan saved us" and with "Arxan never cared about this page":

    --probe neuter       neuter_arxan runs first.  Answers: does hooking work WITH dearxan?
    --probe skip-neuter  Arxan's 48 stubs left live. Answers: was Arxan ever a threat here?

The verdict block is assembled from the DLL's own `ds2-probe:` lines and from nothing else. In
particular the arm is READ BACK out of the log and compared against the one that was requested.
That guard was written for environment variables that could silently vanish, and moving to a file
did not retire it: a file can fail to be written, be written to the wrong directory, or be left
over from a previous run -- and a run against a stale file would report a perfectly well-formed
verdict for an experiment nobody asked for.

Usage:
    python3 scripts/ds2-run.py --dry-run   # stage nothing, launch nothing, report what it would do
    python3 scripts/ds2-run.py --dry-run --probe skip-neuter   # ... for that arm
    python3 scripts/ds2-run.py --selftest  # exercise the log tailer and the verdict logic
    python3 scripts/ds2-run.py             # the real run, probe off
    python3 scripts/ds2-run.py --probe neuter        # arm A
    python3 scripts/ds2-run.py --probe skip-neuter   # arm B
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

GAME_DIR = (
    Path.home()
    / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
)
APPID = "335300"

#: cargo emits this because `crates/ds2-loader/Cargo.toml` sets `[lib] name = "dinput8"`.
BUILT_DLL = REPO_ROOT / "target/x86_64-pc-windows-msvc/release/dinput8.dll"
STAGED_DLL_NAME = "dinput8.dll"

#: Mirrors `LOG_FILE_NAME` in `crates/ds2-loader/src/lib.rs`. Both halves of this contract have
#: to change together: rename it on one side only and every run reports a false "did not load".
LOG_NAME = "ds2-loader.log"

#: The strong witness. Written from dearxan's callback, so it proves the DLL loaded AND that
#: `neuter_arxan` reached its callback -- which is the whole point of this build.
ARXAN_LINE_PREFIX = "ds2-loader: arxan"

#: The weak witness. Written at `DLL_PROCESS_ATTACH`. It proves only that the DLL was loaded.
#: Kept separate from the strong one on purpose: "loaded, and dearxan never reported" is a
#: completely different failure from "never loaded", and conflating them sends you hunting a
#: WINEDLLOVERRIDES problem that does not exist.
ATTACH_LINE_PREFIX = "ds2-loader: attach"

#: Mirrors `PROBE_LINE_PREFIX` in `crates/ds2-loader/src/arxan_probe.rs`. A different prefix from
#: the loader's on purpose: one of them is testimony about LOADING and the other is testimony
#: about an EXPERIMENT, and a run that only did the first must not be able to read as the second.
PROBE_LINE_PREFIX = "ds2-probe:"

#: Mirrors `CONFIG_LINE_PREFIX` in that module. The DLL echoes the config it read back into the
#: log under this prefix -- the path, whether the file was there at all, and every key verbatim --
#: before it acts on any of it. `ds2-loader:` rather than `ds2-probe:` because it is written even
#: when the probe is off, which is exactly the run where you most need to know what it read.
CONFIG_LINE_PREFIX = "ds2-loader: config"

#: Mirrors `CONFIG_FILE_NAME` in `crates/ds2-loader/src/arxan_probe.rs`. The DLL reads this out of
#: its own game directory -- the directory of the running executable -- so this script writes it
#: to exactly the directory it stages the DLL into. `--selftest` checks the spelling against the
#: Rust source, because a rename on one side alone turns every run into a silent "probe off".
CONFIG_NAME = "ds2-mods.toml"

#: Mirrors `CONFIG_SECTION` and the four `KEY_*` constants in that module.
CONFIG_SECTION = "arxan_probe"
KEY_ENABLED = "enabled"
KEY_SKIP_NEUTER = "skip_neuter"
KEY_POLL_INTERVAL_MS = "poll_interval_ms"
KEY_HEARTBEAT_INTERVAL_MS = "heartbeat_interval_ms"

#: Defaults for the two LIVE keys, written into the generated file as commented-out lines so the
#: file documents them without changing behaviour. Mirrors `DEFAULT_*` in the same module.
DEFAULT_POLL_INTERVAL_MS = 1000
DEFAULT_HEARTBEAT_INTERVAL_MS = 10000

#: CLI arm -> (`[arxan_probe]` settings to write, the `arm=` token the DLL must report back).
#:
#: THE SECOND HALF OF EACH ENTRY IS A GUARD, not decoration, and it did not stop being one when
#: these moved out of the environment. A config file has its own ways to be wrong: it can fail to
#: be written, be written to the wrong directory (a game dir that moved, a second install), or be
#: left over from a previous run against a DLL that no longer reads it. A run that lost the file
#: entirely installs no probe and is caught by the missing install line -- but a run against a
#: STALE file would quietly execute whichever arm that file names and produce a perfectly
#: well-formed verdict for an experiment nobody asked for. That is the failure this comparison
#: exists to make impossible, and it is why the arm is read back out of the log.
PROBE_ARMS: dict[str, tuple[dict[str, bool], str | None]] = {
    "off": ({KEY_ENABLED: False, KEY_SKIP_NEUTER: False}, None),
    "neuter": ({KEY_ENABLED: True, KEY_SKIP_NEUTER: False}, "neuter-arxan"),
    "skip-neuter": ({KEY_ENABLED: True, KEY_SKIP_NEUTER: True}, "skip-neuter-arxan"),
}

#: How long to keep reading the log after the probe says it is installed. The probe heartbeats
#: every 10s, so the default is 18 of them. It is a WINDOW and not a proof of anything past its
#: end, which is why the verdict block prints the number rather than rounding it to "it survived".
OBSERVE_SECONDS = 180.0

#: Native first, then builtin. Wine prefers its own `dinput8` without this and the proxy is
#: simply never mapped.
DLL_OVERRIDE = "dinput8=n,b"

#: How long to wait for the DLL to speak. DS2 boots through Proton and dearxan analyses 48 Arxan
#: stubs single-threaded before its callback runs; neither has been timed on this machine, so
#: this is a deliberately loose bound, not a measurement.
TESTIMONY_BUDGET_SECONDS = 240.0
POLL_SECONDS = 0.5

#: /proc/<pid>/comm is truncated to 15 characters and "DarkSoulsII.exe" is exactly 15, so the
#: exact-comm match works. `pgrep -f 'DarkSoulsII.exe'` does NOT: it matches this script's own
#: command line, which contains the string, and reports the game running before it has started.
GAME_COMM = "DarkSoulsII.exe"

EXIT_OK = 0
EXIT_NO_TESTIMONY = 2
EXIT_ERROR = 3
#: The probe was asked for and did not produce a verdict: it never installed, it refused the site,
#: it never heartbeat, or the arm in the log is not the arm that was requested. Distinct from
#: EXIT_NO_TESTIMONY because the DLL may have loaded and reported perfectly well -- the thing that
#: did not happen is the EXPERIMENT, and those two send you to different places to look.
EXIT_NO_PROBE_VERDICT = 4


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pgrep_exact(comm: str) -> list[int]:
    """PIDs whose exact comm is `comm`. Empty on no match; `pgrep` exits 1 for that."""
    proc = subprocess.run(
        ["pgrep", "-x", comm], capture_output=True, text=True, timeout=10
    )
    return [int(line) for line in proc.stdout.split() if line.isdigit()]


def steam_running() -> bool:
    """Is the Steam client up? Native Linux only -- this repo's game runs through Proton."""
    return bool(pgrep_exact("steam") or pgrep_exact("steamwebhelper"))


class LogTail:
    """Read only what THIS run appended, surviving the DLL's startup log rotation.

    `ds2_game_base::log` renames `<log>` to `<log>.prev` and truncates on the DLL's first write,
    so a plain offset would either miss the new file or replay the old one. Remembering the
    inode as well makes "everything since launch" exact. Both rotation shapes are handled, and
    missing the second one cost `../er-mods-rs` a live run:

      * REPLACED           -- new inode, so read the whole new file.
      * TRUNCATED IN PLACE -- same inode, size dropped below our offset. Seeking to the old
                              offset lands past EOF and reads NOTHING, so the DLL's startup
                              lines are invisible and a perfectly healthy run is reported silent.
    """

    def __init__(self, path: Path) -> None:
        self.path = path
        try:
            stat = path.stat()
            self.inode: int | None = stat.st_ino
            self.offset = stat.st_size
        except OSError:
            self.inode, self.offset = None, 0

    def new_text(self) -> str:
        try:
            stat = self.path.stat()
        except OSError:
            return ""
        rotated = stat.st_ino != self.inode or stat.st_size < self.offset
        start = 0 if rotated else self.offset
        try:
            with self.path.open("rb") as handle:
                handle.seek(min(start, stat.st_size))
                data = handle.read()
        except OSError:
            return ""
        # A LINE IS EVIDENCE ONLY ONCE IT IS TERMINATED. A read landing mid-write returns a
        # PREFIX, and a prefix of "ds2-loader: arxan status=ok detected=true ..." is
        # indistinguishable from a DLL that reported less than it did. Hand back whole lines
        # only; the remainder arrives complete on the next poll, milliseconds later.
        end = data.rfind(b"\n")
        if end < 0:
            return ""
        consumed = (0 if rotated else start) + end + 1
        self.inode, self.offset = stat.st_ino, consumed
        return data[: end + 1].decode("utf-8", errors="replace")


def await_testimony(tail: LogTail) -> dict:
    """Block until the DLL reports its Arxan result, or the budget runs out.

    Verdicts:
      confirmed        -- the `ds2-loader: arxan` line is on disk. This run is what it says.
      attached-silent  -- the DLL loaded and said so, but dearxan's callback never reported.
                          The proxy and the override WORKED; the Arxan step is what did not.
      silent           -- no line at all. Most likely the DLL was never mapped.
    """
    deadline = time.monotonic() + TESTIMONY_BUDGET_SECONDS
    attach_line: str | None = None
    game_seen = False
    while True:
        for line in tail.new_text().splitlines():
            stripped = line.strip()
            if stripped.startswith(ARXAN_LINE_PREFIX):
                return {
                    "status": "confirmed",
                    "line": stripped,
                    "attach_line": attach_line,
                    "waited": TESTIMONY_BUDGET_SECONDS - (deadline - time.monotonic()),
                }
            if stripped.startswith(ATTACH_LINE_PREFIX):
                attach_line = stripped

        alive = bool(pgrep_exact(GAME_COMM))
        game_seen = game_seen or alive
        if game_seen and not alive:
            # The game came up and went away. Waiting longer cannot produce a line, because
            # there is no longer a process to write one. Re-read once first: the exit path and
            # the last write race, and losing that race would report silence over real evidence.
            for line in tail.new_text().splitlines():
                stripped = line.strip()
                if stripped.startswith(ARXAN_LINE_PREFIX):
                    return {
                        "status": "confirmed",
                        "line": stripped,
                        "attach_line": attach_line,
                        "waited": TESTIMONY_BUDGET_SECONDS - (deadline - time.monotonic()),
                    }
                if stripped.startswith(ATTACH_LINE_PREFIX):
                    attach_line = stripped
            return {
                "status": "attached-silent" if attach_line else "silent",
                "attach_line": attach_line,
                "game_exited": True,
                "game_seen": True,
            }

        if time.monotonic() >= deadline:
            return {
                "status": "attached-silent" if attach_line else "silent",
                "attach_line": attach_line,
                "game_exited": False,
                "game_seen": game_seen,
            }
        time.sleep(POLL_SECONDS)


# ================================================================================================
# THE M1 PROBE: reading a verdict out of the DLL's own lines.
#
# Nothing below infers anything. It parses `ds2-probe:` lines and reports what they say. The one
# judgement it makes -- "survived" vs "reverted" -- is a restatement of two fields on the last
# heartbeat, and the block prints those fields next to it so the restatement can be checked.
# ================================================================================================


def fields(line: str) -> dict[str, str]:
    """The `key=value` tokens of a log line.

    Byte windows are logged as `expected=[48 89 5c ...]`, which splits into an `expected=[48`
    token followed by bare hex tokens. The bare ones have no `=` and are dropped; the truncated
    `expected` is never read by anything here. That is deliberate rather than sloppy -- the hex is
    for a human comparing two windows by eye, and the scalars are what this code reasons about.
    """
    parsed: dict[str, str] = {}
    for token in line.split():
        key, sep, value = token.partition("=")
        if sep and key not in parsed:
            parsed[key] = value
    return parsed


def absorb_probe_line(line: str, state: dict) -> None:
    """Sort one log line into `state`. Non-probe lines are ignored."""
    if not line.startswith(PROBE_LINE_PREFIX):
        return
    body = line[len(PROBE_LINE_PREFIX) :].strip()
    kind = body.split(" ", 1)[0] if body else ""
    parsed = fields(line)

    if kind == "install-failed":
        state["install_failed"] = state["install_failed"] or line
    elif kind == "VOID":
        state["void"] = state["void"] or line
    elif kind == "install":
        # Three lines share this word. `rva=` is on the first (the one that names the arm and the
        # address), `minhook=` on the one that reports what was written. Matching on a field
        # rather than on word position means a reordered line does not become a parse failure.
        if "rva" in parsed:
            state["install"] = state["install"] or line
            state["arm"] = state["arm"] or parsed.get("arm")
        elif "minhook" in parsed:
            state["patched"] = state["patched"] or line
        elif "prologue-match" in parsed:
            state["prologue"] = state["prologue"] or line
    elif kind == "watching":
        state["watching"] = state["watching"] or line
        state["arm"] = state["arm"] or parsed.get("arm")
    elif kind == "heartbeat":
        state["heartbeats"].append(line)
    elif kind in ("SITE", "TRAMP"):
        state["events"].append(line)
    elif kind == "config":
        # The probe re-read its config mid-run and something changed, or someone edited a
        # startup-only key and was told it does not apply. Either way the run was touched while
        # it was being measured, and the verdict block must say so rather than quietly average
        # over it.
        state["config_events"].append(line)
    elif kind == "detach":
        state["detach"] = line


def new_probe_state() -> dict:
    return {
        "install": None,
        "prologue": None,
        "patched": None,
        "watching": None,
        "install_failed": None,
        "void": None,
        "arm": None,
        "heartbeats": [],
        "events": [],
        "config_events": [],
        "detach": None,
        "game_exited": False,
        "observed": 0.0,
    }


def watch_probe(tail: LogTail, seconds: float) -> dict:
    """Read probe lines until the observation window closes or the game goes away."""
    state = new_probe_state()
    started = time.monotonic()
    deadline = started + seconds
    game_seen = False
    while True:
        for line in tail.new_text().splitlines():
            absorb_probe_line(line.strip(), state)

        alive = bool(pgrep_exact(GAME_COMM))
        game_seen = game_seen or alive
        if game_seen and not alive:
            # Same race as `await_testimony`: the detach line and the process disappearing happen
            # at the same moment, and losing that race would drop the one line that says the exit
            # was orderly. Drain once more before concluding.
            time.sleep(POLL_SECONDS)
            for line in tail.new_text().splitlines():
                absorb_probe_line(line.strip(), state)
            state["game_exited"] = True
            break

        if time.monotonic() >= deadline:
            break
        time.sleep(POLL_SECONDS)
    state["observed"] = time.monotonic() - started
    return state


def probe_block(requested: str, state: dict) -> tuple[str, int]:
    """Render the verdict for one arm, and the exit code that goes with it.

    Returns `EXIT_OK` whenever the experiment RAN, whether or not the detour survived: a detour
    that Arxan reverted is a successful experiment with an unwelcome answer, and exiting non-zero
    on it would train whoever reads this into treating the real finding as a tooling failure.
    Non-zero means no verdict was produced at all.
    """
    _, expected_arm = PROBE_ARMS[requested]
    head = ["```", "================== M1: ARXAN vs A MINHOOK DETOUR =================="]
    tail_rule = ["==================================================================", "```"]

    def refused(reason: str, detail: list[str]) -> tuple[str, int]:
        lines = head + [f"  NO VERDICT     {reason}"] + [f"  {d}" for d in detail] + tail_rule
        return "\n".join(lines), EXIT_NO_PROBE_VERDICT

    if state["void"]:
        return refused(
            "the probe refused the hook site and patched nothing",
            [
                state["void"],
                "",
                "The five bytes at the hook site were not the prologue recorded in",
                "`ds2-rva::ARXAN_PROBE_HOOK_SITE_PROLOGUE`, so something reached that",
                "function before the probe did. Anything measured after that would have",
                "been a measurement of THAT, not of Arxan. Check the game build against",
                "`ds2_rva::BUILD_ID`, and check for another mod in the game directory.",
            ],
        )

    if not state["install"]:
        detail = [state["install_failed"]] if state["install_failed"] else []
        return refused(
            "the probe never installed",
            detail
            + [
                "",
                f"No `{PROBE_LINE_PREFIX} install ... rva=` line appeared.",
                "",
                f"Check the loader's `{CONFIG_LINE_PREFIX}` lines, which say what it read and",
                "from where:",
                f"  status=MISSING  -- the DLL looked in a different directory than this script",
                f"                     wrote to. Compare the path on that line against",
                f"                     {GAME_DIR / CONFIG_NAME}.",
                f"  status=found and {KEY_ENABLED}=\"false\"",
                "                  -- the file that was read is not the one this run wrote.",
                f"  a REJECTED line -- the file was read and that key could not be used.",
                "",
                "If instead there are no config lines at all, the DLL that loaded is an older",
                "build than the one staged: check the sha256 above.",
            ],
        )

    if state["arm"] != expected_arm:
        return refused(
            f"WRONG ARM -- asked for {expected_arm!r}, the DLL ran {state['arm']!r}",
            [
                state["install"],
                "",
                "This is the dangerous case, which is why it is a hard failure rather than a",
                "note: the run is well-formed and its numbers are real, but they answer a",
                "different question than the one that was asked.",
                "",
                "The config file this run wrote is quoted in the block above. If the DLL read a",
                "different one, its own `" + CONFIG_LINE_PREFIX + "` line names the path it",
                "read -- most likely a second game install, or a stale file the DLL reached",
                "before this script rewrote it.",
            ],
        )

    if not state["heartbeats"]:
        return refused(
            "the probe installed but never reported a heartbeat",
            [
                state["install"],
                "",
                f"Observed {state['observed']:.1f}s. The probe heartbeats every 10s, so either",
                "the window was too short, or the poller thread did not run. Every event line",
                "seen, if any, follows.",
            ]
            + state["events"],
        )

    last = fields(state["heartbeats"][-1])
    hits = last.get("hits", "?")
    site = last.get("site", "?")
    tramp = last.get("tramp", "?")
    site_diverged = last.get("site-diverged", "?")
    tramp_diverged = last.get("tramp-diverged", "?")
    site_ok = site == "intact" and site_diverged == "0"
    tramp_ok = tramp == "intact" and tramp_diverged == "0"
    fired = hits not in ("0", "?")

    lines = head + [
        f"  arm            {state['arm']}   (requested --probe {requested})",
        f"  observed       {state['observed']:.1f}s, {len(state['heartbeats'])} heartbeats,"
        f" game exited: {'yes' if state['game_exited'] else 'no'}",
        "",
        f"  {state['install']}",
    ]
    if state["prologue"]:
        lines.append(f"  {state['prologue']}")
    if state["patched"]:
        lines.append(f"  {state['patched']}")
    lines += [
        "",
        "  THE FOUR MEASUREMENTS, from the last heartbeat:",
        f"    {state['heartbeats'][-1]}",
        "",
        f"    1. detour fired    {'YES' if fired else 'NO'}   hits={hits}",
        f"    2. hook site       {site}   ({site_diverged} divergence(s))",
        f"    3. trampoline      {tramp}   ({tramp_diverged} divergence(s))",
        f"    4. arm             {state['arm']}",
    ]

    if state["events"]:
        lines += ["", "  STATE CHANGES (each with the observed bytes):"]
        lines += [f"    {event}" for event in state["events"]]

    if state["config_events"]:
        # NOT a footnote. The config file was edited while the measurement was running, so the
        # window above is not homogeneous, and a reader comparing two arms needs to know that
        # before comparing anything.
        lines += [
            "",
            "  THE CONFIG FILE WAS TOUCHED DURING THIS WINDOW:",
        ]
        lines += [f"    {event}" for event in state["config_events"]]
        lines += [
            "    The arm cannot change mid-run and the DLL says so when asked to; a poll or",
            "    heartbeat interval CAN, and if one did, the cadence above is not uniform.",
        ]

    lines += [
        "",
        "  exit           "
        + (
            state["detach"]
            if state["detach"]
            else "no `ds2-probe: detach` line -- the process did not wind down through"
        ),
    ]
    if not state["detach"]:
        lines.append(
            "                 ExitProcess. It crashed, was killed, or is still running; the"
        )
        lines.append("                 last heartbeat above is the time of death if it died.")

    lines += ["", "  READING"]
    if site_ok and tramp_ok and fired:
        lines.append(
            f"    The detour SURVIVED and FIRED for the whole {state['observed']:.0f}s window."
        )
    elif site_ok and tramp_ok and not fired:
        lines += [
            "    The patch was never touched -- AND the function was never called.",
            "    The hook site held, but a detour that never runs is not much of a test",
            "    of whether a running one survives. Play further into the game, or move to",
            "    `ds2_rva::ARXAN_PROBE_HOOK_SITE_BACKUP`.",
        ]
    else:
        if not site_ok:
            lines.append("    THE HOOK SITE WAS REVERTED. Read the SITE lines above for when and")
            lines.append("    to what. This is the finding the experiment was built to catch.")
        if not tramp_ok:
            lines += [
                "    THE TRAMPOLINE WAS CORRUPTED. Note that this is exactly the failure a hit",
                "    counter alone would have misread: the counter goes quiet while the hook",
                "    site still looks pristine, and the obvious conclusion -- 'the function",
                "    stopped being called' -- would have been wrong.",
            ]

    if requested == "neuter":
        lines += [
            "",
            "    THIS IS HALF THE EXPERIMENT. dearxan neutered Arxan's 48 stubs before the",
            "    probe installed, so this says nothing about whether Arxan was ever a threat.",
            "    Run `--probe skip-neuter` for the other half.",
        ]
    else:
        lines += [
            "",
            "    THIS IS THE ARM WITHOUT dearxan: Arxan's 48 stubs were live for the whole",
            "    window. Compare against a `--probe neuter` run before concluding whether",
            "    dearxan is load-bearing for hooking.",
        ]

    lines += [
        "",
        "  NOT claimed    anything past the window above; that every Arxan check ran during",
        "                 it; or anything about any hook site other than the one named above.",
    ]
    return "\n".join(lines + tail_rule), EXIT_OK


def running_block(context: dict) -> str:
    lines = [
        "```",
        "============= DARK SOULS II IS RUNNING, WITH OUR DLL IN IT =============",
        f"  started        {context['started']}",
        f"  appid          {APPID}",
        f"  override       WINEDLLOVERRIDES={DLL_OVERRIDE}",
        "",
        f"  staged         {context['staged']}",
        f"  sha256         {context['sha256']}",
        f"  game pids      {context['game_pids'] or '<none right now>'}",
        "",
        # THE CONFIGURATION UNDER TEST, VERBATIM, in the block that gets copy-pasted. The arm is
        # the variable the whole experiment turns on and it no longer travels in a command line
        # anyone can see, so the transcript has to carry the file itself or it carries nothing.
        f"  config         {context['config_path']}",
        quoted_config(context["config"], indent="    | "),
        "",
        "  PROVEN BY      the DLL's own log line, not by the process existing:",
        f"    {context['testimony']}",
    ]
    if context.get("attach_line"):
        lines.append(f"    {context['attach_line']}")
    lines += [
        f"  log            {context['log']}",
        f"  waited         {context['waited']:.1f}s",
        "",
        "  NOT claimed    window visible / menu reached / input working / Arxan actually",
        "                 defeated. This block says the proxy loaded and dearxan reported;",
        "                 read the line above for WHAT it reported.",
        "========================================================================",
        "```",
    ]
    return "\n".join(lines)


def failed_block(reason: str, detail: list[str]) -> str:
    lines = [
        "```",
        "================ DARK SOULS II: NO DLL TESTIMONY ================",
        f"  reason  {reason}",
    ]
    lines.extend(f"  {line}" for line in detail)
    lines += ["=================================================================", "```"]
    return "\n".join(lines)


def preflight(dry_run: bool) -> list[str]:
    """Return the problems that make a real run impossible. Empty list means go."""
    problems: list[str] = []
    if not BUILT_DLL.is_file():
        problems.append(
            f"built DLL not found: {BUILT_DLL}\n"
            "    build it: cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader"
        )
    if not GAME_DIR.is_dir():
        problems.append(f"game directory not found: {GAME_DIR}")
    elif not os.access(GAME_DIR, os.W_OK):
        problems.append(f"game directory is not writable: {GAME_DIR}")
    if not dry_run and shutil.which("steam") is None:
        problems.append("`steam` is not on PATH")
    return problems


def launch_env(probe: str) -> dict[str, str]:
    """The environment a run needs, over this process's own.

    ONE VARIABLE, and it does not depend on the arm. `WINEDLLOVERRIDES` is the only setting that
    genuinely has to travel through Steam, because Wine reads it to decide whether to map our DLL
    at all -- before there is a DLL running to read anything. Everything the DLL itself decides
    now comes out of the config file; see `config_text`.
    """
    del probe  # the arm is in the config file now, not in the environment
    return {"WINEDLLOVERRIDES": DLL_OVERRIDE}


def config_text(probe: str) -> str:
    """The exact bytes of `<Game>/ds2-mods.toml` for this arm.

    Deterministic: the same arm produces the same file every time, with no timestamp and no
    hostname, so two runs of the same arm are trivially comparable and `--selftest` can assert on
    the content rather than around it.
    """
    settings, _ = PROBE_ARMS[probe]
    return f"""\
# DARK SOULS II mod settings. Read by `dinput8.dll` out of this directory -- the directory of the
# running executable -- in `DllMain`, before the game's entry point.
#
# WRITTEN BY scripts/ds2-run.py ON EVERY LAUNCH. Edits to the two startup-only keys below are
# overwritten by the next run, deliberately: the arm under test has to be the arm that was asked
# for, and the launcher reads the arm back out of the log to prove it was.
#
# THIS FILE REPLACED TWO ENVIRONMENT VARIABLES, and the reason is measured rather than stylistic.
# `DS2_ARXAN_PROBE` and `DS2_ARXAN_PROBE_SKIP_NEUTER` were set in the launcher's environment and
# arrived at the game UNSET: `steam -applaunch` hands the request to an already-running Steam
# client over IPC, and the game inherits THAT client's environment. A file beside the DLL travels
# through no IPC at all, and both arms can now be run back to back with nothing to do in between.

[{CONFIG_SECTION}]
# STARTUP-ONLY. Both are consumed in DllMain, before the game's entry point, because that is the
# only moment the choice can be made: `skip_neuter` decides whether Arxan's 48 stubs are patched
# before the Arxan entry stub runs, and there is no un-neutering a live process. Editing either
# one while the game is running changes nothing and says so in the log.
{KEY_ENABLED} = {str(settings[KEY_ENABLED]).lower()}
{KEY_SKIP_NEUTER} = {str(settings[KEY_SKIP_NEUTER]).lower()}

# LIVE. Re-read by the probe's poller thread through `ds2_hotkey_config::reload::HotFile`, which
# compares the file's TEXT rather than its mtime -- a Proton prefix sits on filesystems that stamp
# mtime to a whole second, so two edits inside one second would be invisible to an mtime watcher.
# An edit here takes effect within one poll interval, without restarting the game. Neither changes
# WHAT is measured: the byte windows and their baselines are fixed when the hook goes in.
#
# Defaults shown. Uncomment to change.
# {KEY_POLL_INTERVAL_MS} = {DEFAULT_POLL_INTERVAL_MS}
# {KEY_HEARTBEAT_INTERVAL_MS} = {DEFAULT_HEARTBEAT_INTERVAL_MS}
"""


def write_config(directory: Path, probe: str) -> tuple[Path, str]:
    """Write the config for `probe` into `directory`; return the path and what was written."""
    path = directory / CONFIG_NAME
    text = config_text(probe)
    path.write_text(text, encoding="utf-8")
    return path, text


def quoted_config(text: str, indent: str = "    ") -> str:
    """The config file's own lines, indented, for a transcript.

    PRINTED IN FULL rather than summarised. The configuration under test is the variable this
    whole change exists to make visible, and a block that says "wrote the config" is exactly the
    claim that turned out to be false last time.
    """
    return "\n".join(f"{indent}{line}" if line else indent.rstrip() for line in text.splitlines())


def report_environment(probe: str) -> None:
    """Print the environment facts that decide whether the run reaches the game intact."""
    print(f"[env] game dir     {GAME_DIR}")
    print(f"[env] built DLL    {BUILT_DLL}")
    variables = launch_env(probe)
    print("[env] launch with  " + " ".join(f"{k}={v}" for k, v in variables.items()))
    print(f"[env] config       {GAME_DIR / CONFIG_NAME}")
    if probe != "off":
        _, expected_arm = PROBE_ARMS[probe]
        print(f"[env] probe arm    {probe} -- the DLL must report back arm={expected_arm}")
    if steam_running():
        # ONLY the override rides this channel now. The probe settings used to be named here too,
        # and were the reason this warning existed at all; they are in the config file precisely
        # so that an already-running Steam client cannot lose them.
        print(
            "[env] steam        ALREADY RUNNING -- `steam -applaunch` hands the request to the\n"
            "                   running client over IPC, and the game then inherits THAT\n"
            "                   client's environment. WINEDLLOVERRIDES is therefore at risk, and\n"
            "                   it is the one setting that cannot move into the config file:\n"
            "                   Wine reads it to decide whether to map our DLL at all. If the run\n"
            "                   comes back with no testimony, that is the first thing to rule\n"
            "                   out -- quit Steam and re-run so this invocation starts the\n"
            "                   client, or set the per-app launch options to\n"
            "                     "
            + " ".join(f'{k}="{v}"' for k, v in variables.items())
            + " %command%\n"
            "                   The probe settings are NOT at risk: they are in the config file,\n"
            "                   which the DLL reads off disk itself."
        )
    else:
        print(
            "[env] steam        not running -- this invocation starts the client, so the\n"
            "                   variables above are inherited by everything it launches."
        )


def stage() -> tuple[Path, str]:
    """Copy the built DLL into the game directory; return the staged path and ITS hash."""
    staged = GAME_DIR / STAGED_DLL_NAME
    shutil.copyfile(BUILT_DLL, staged)
    # Hash the STAGED file, read back off disk, not the source. The point of printing a hash is
    # to describe the bytes that will actually be loaded.
    return staged, sha256(staged)


def dry_run(probe: str, observe: float) -> int:
    print("[dry-run] staging nothing, launching nothing.")
    report_environment(probe)
    problems = preflight(dry_run=True)
    for problem in problems:
        print(f"[dry-run] WOULD REFUSE: {problem}")

    staged = GAME_DIR / STAGED_DLL_NAME
    if BUILT_DLL.is_file():
        print(f"[dry-run] built    sha256 {sha256(BUILT_DLL)}  {BUILT_DLL}")
    if staged.is_file():
        current = sha256(staged)
        print(f"[dry-run] staged   sha256 {current}  {staged}")
        if BUILT_DLL.is_file() and current != sha256(BUILT_DLL):
            print("[dry-run] staged DLL DIFFERS from the built one; a real run would replace it.")
    else:
        print(f"[dry-run] staged   <absent>  {staged}")

    config_path = GAME_DIR / CONFIG_NAME
    if config_path.is_file():
        current = config_path.read_text(encoding="utf-8")
        if current == config_text(probe):
            print(f"[dry-run] config   present and ALREADY MATCHES this arm  {config_path}")
        else:
            print("[dry-run] config   present and DIFFERS; a real run would replace it")
            print(f"[dry-run]          {config_path}")
    else:
        print(f"[dry-run] config   <absent>  {config_path}")

    log_path = GAME_DIR / LOG_NAME
    environment = " ".join(f"{k}={v}" for k, v in launch_env(probe).items())
    print(f"[dry-run] would copy   {BUILT_DLL}")
    print(f"[dry-run]         to   {staged}")
    # THE CONFIGURATION UNDER TEST, VERBATIM. It is the whole variable this run turns on, so a
    # dry-run that did not show it would be hiding the one thing it exists to preview.
    print(f"[dry-run] would write  {config_path}")
    print(quoted_config(config_text(probe), indent="[dry-run]   | "))
    print(f"[dry-run] would launch env {environment} steam -applaunch {APPID}")
    print(f"[dry-run] would poll   {log_path}")
    # The DLL echoes the config back BEFORE it decides anything, so these are the first lines a
    # real run puts in the log. Previewing them in that order is what lets someone reading a real
    # log compare it line for line.
    settings, expected_arm = PROBE_ARMS[probe]
    enabled = str(settings[KEY_ENABLED]).lower()
    skip = str(settings[KEY_SKIP_NEUTER]).lower()
    print(f"[dry-run]     expect  {CONFIG_LINE_PREFIX} file=\"{config_path}\" status=found bytes=..")
    print(f"[dry-run]             {CONFIG_LINE_PREFIX} [{CONFIG_SECTION}] {KEY_ENABLED}=\"{enabled}\" {KEY_SKIP_NEUTER}=\"{skip}\" {KEY_POLL_INTERVAL_MS}=<absent> {KEY_HEARTBEAT_INTERVAL_MS}=<absent>")
    print(f"[dry-run]             {CONFIG_LINE_PREFIX} resolved probe={'on' if probe != 'off' else 'off'} arm={expected_arm or 'neuter-arxan'} poll={DEFAULT_POLL_INTERVAL_MS}ms heartbeat={DEFAULT_HEARTBEAT_INTERVAL_MS}ms ...")
    print(f"[dry-run]         for  a line starting {ARXAN_LINE_PREFIX!r}")
    print(f"[dry-run]         upto {TESTIMONY_BUDGET_SECONDS:.0f}s, then FAIL with exit {EXIT_NO_TESTIMONY}")

    if probe == "off":
        print("[dry-run] probe     off -- no probe lines expected, no verdict block")
    else:
        print(f"[dry-run] probe     {probe}")
        print(f"[dry-run]         then observe {observe:.0f}s for {PROBE_LINE_PREFIX!r} lines:")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} install ... arm={expected_arm} rva=... va=...")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} install original=[..] expected=[..] prologue-match=true")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} install minhook=ok trampoline=0x.. patched=[..] site-jmp=true")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} watching arm={expected_arm} poll={DEFAULT_POLL_INTERVAL_MS}ms heartbeat={DEFAULT_HEARTBEAT_INTERVAL_MS}ms site-window=.. trampoline-window=..")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} heartbeat uptime=..s arm={expected_arm} hits=.. site=intact tramp=intact site-diverged=0 tramp-diverged=0")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} SITE|TRAMP  ... state=DIVERGED ... expected=[..] observed=[..]   (on any change)")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} config ... RELOADED|STARTUP-ONLY-IGNORED  (only if the file is edited mid-run)")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} detach ...                                          (orderly exit only)")
        print(f"[dry-run]         and REFUSE a verdict (exit {EXIT_NO_PROBE_VERDICT}) if the log's arm is not {expected_arm!r}")

    print(f"[dry-run] log present now: {log_path.is_file()}")
    print(f"[dry-run] {GAME_COMM} running now: {bool(pgrep_exact(GAME_COMM))}")
    return EXIT_ERROR if problems else EXIT_OK


def launch(probe: str, observe: float) -> int:
    report_environment(probe)
    problems = preflight(dry_run=False)
    if problems:
        for problem in problems:
            print(f"REFUSING TO LAUNCH: {problem}", file=sys.stderr)
        return EXIT_ERROR

    staged, digest = stage()
    print(f"[stage] {staged}")
    print(f"[stage] sha256 {digest}")

    # BEFORE LAUNCHING, and after staging: the DLL reads this in `DllMain`, so it has to be on
    # disk before the game starts, and it is rewritten every run so a file left over from the
    # other arm cannot decide this one.
    config_path, config = write_config(GAME_DIR, probe)
    print(f"[config] {config_path}")

    log_path = GAME_DIR / LOG_NAME
    # Take the tail's mark BEFORE launching. Everything it hands back afterwards is this run's.
    # Deliberately NOT deleting the log: the DLL rotates it to `.prev` itself on its first write,
    # and deleting here would destroy the previous run's evidence for no gain.
    tail = LogTail(log_path)

    environment = launch_env(probe)
    started = datetime.now(timezone.utc).isoformat(timespec="seconds")
    subprocess.Popen(
        ["steam", "-applaunch", APPID],
        env={**os.environ, **environment},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        start_new_session=True,  # survives this shell, and this agent turn
    )
    print(
        f"[launch] steam -applaunch {APPID}  ("
        + " ".join(f"{k}={v}" for k, v in environment.items())
        + ")"
    )
    print(f"[launch] waiting up to {TESTIMONY_BUDGET_SECONDS:.0f}s for {log_path}")

    verdict = await_testimony(tail)
    if verdict["status"] != "confirmed":
        if verdict["status"] == "attached-silent":
            reason = "the DLL LOADED but dearxan never reported"
            detail = [
                f"saw       {verdict['attach_line']}",
                f"missing   a line starting {ARXAN_LINE_PREFIX!r}",
                "",
                "The proxy and the WINEDLLOVERRIDES both worked -- this is not a loading",
                "failure. neuter_arxan's callback did not reach the log, so look at dearxan,",
                "not at the override.",
            ]
        else:
            reason = "no DLL testimony at all"
            detail = [
                f"waited    {TESTIMONY_BUDGET_SECONDS:.0f}s for any line in {log_path}",
                f"{GAME_COMM} was "
                + (
                    "seen and has since exited"
                    if verdict.get("game_exited")
                    else ("seen and is still running" if verdict.get("game_seen") else "never seen")
                ),
                "",
                "Most likely the DLL was never mapped: check WINEDLLOVERRIDES actually reached",
                "the game (see the [env] steam note above), and that the staged DLL is the one",
                "whose sha256 is printed above.",
            ]
        print(failed_block(reason, detail))
        return EXIT_NO_TESTIMONY

    print(
        running_block(
            {
                "started": started,
                "staged": staged,
                "sha256": digest,
                "log": log_path,
                "testimony": verdict["line"],
                "attach_line": verdict.get("attach_line"),
                "waited": verdict["waited"],
                "game_pids": pgrep_exact(GAME_COMM),
                "config_path": config_path,
                "config": config,
            }
        )
    )
    if probe == "off":
        return EXIT_OK

    # The loader has spoken; now wait for the experiment. Note that the SAME tail continues --
    # the probe's install lines may already be in the buffer this loop is about to read, because
    # they are written from the same Arxan callback that produced the line above.
    print(f"[probe] observing {observe:.0f}s for {PROBE_LINE_PREFIX!r} lines")
    probe_state = watch_probe(tail, observe)
    block, code = probe_block(probe, probe_state)
    print(block)
    return code


def selftest() -> int:
    """Exercise the log tailer against the rotations it exists to survive.

    This is the only part of the script that can silently turn a good run into a reported
    failure, so it is the only part with a test.
    """
    import tempfile

    ok = True

    def check(condition: bool, label: str) -> None:
        nonlocal ok
        if not condition:
            ok = False
        print(("  ok   " if condition else "  FAIL ") + label)

    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / LOG_NAME

        path.write_text("STALE FROM AN EARLIER RUN\n", encoding="utf-8")
        tail = LogTail(path)
        check(tail.new_text() == "", "a pre-existing log contributes nothing to this run")

        # REPLACED: the DLL rotated the old file aside and made a new one.
        path.rename(path.with_suffix(path.suffix + ".prev"))
        path.write_text(f"{ATTACH_LINE_PREFIX} x\n", encoding="utf-8")
        check(
            ATTACH_LINE_PREFIX in tail.new_text(),
            "a replaced (new-inode) log is read from the top",
        )

        # TRUNCATED IN PLACE: same inode, size drops below our offset.
        with path.open("r+b") as handle:
            handle.truncate(0)
        with path.open("ab") as handle:
            handle.write(b"short\n")
        check("short" in tail.new_text(), "a truncated-in-place log is re-read from the top")

        # PARTIAL LINE: a read landing mid-write must yield nothing, then the whole line.
        with path.open("ab") as handle:
            handle.write(f"{ARXAN_LINE_PREFIX} status=ok detected=".encode())
        check(tail.new_text() == "", "an unterminated line is withheld until it is complete")
        with path.open("ab") as handle:
            handle.write(b"true blocking_entrypoint=true\n")
        text = tail.new_text()
        check(
            text.strip() == f"{ARXAN_LINE_PREFIX} status=ok detected=true blocking_entrypoint=true",
            "the completed line arrives whole on the next poll",
        )
        check(tail.new_text() == "", "a line already handed back is not handed back twice")

    check(
        f'name = "{BUILT_DLL.stem}"' in (REPO_ROOT / "crates/ds2-loader/Cargo.toml").read_text(),
        f"the crate really is named to emit {BUILT_DLL.name}",
    )
    loader_src = (REPO_ROOT / "crates/ds2-loader/src/lib.rs").read_text()
    check(f'"{LOG_NAME}"' in loader_src, f"the DLL writes the log this polls for ({LOG_NAME})")
    check(f'"{ARXAN_LINE_PREFIX}"' in loader_src, "the DLL writes the line this gates on")
    check(f'"{ATTACH_LINE_PREFIX}"' in loader_src, "the DLL writes the attach line")

    # THE PROBE CONTRACT. Every string below is spelled in two files that cannot check each other
    # at compile time; a rename on one side turns every run into a false "the probe never
    # installed" or, worse, a false "wrong arm". This is the only place that failure is cheap.
    probe_src = (REPO_ROOT / "crates/ds2-loader/src/arxan_probe.rs").read_text()
    check(f'"{PROBE_LINE_PREFIX}"' in probe_src, f"the DLL writes the probe prefix ({PROBE_LINE_PREFIX})")
    check(f'"{CONFIG_LINE_PREFIX}"' in probe_src, f"the DLL echoes what it read ({CONFIG_LINE_PREFIX})")
    check(f'"{CONFIG_NAME}"' in probe_src, f"the DLL reads the config file this writes ({CONFIG_NAME})")
    check(f'"{CONFIG_SECTION}"' in probe_src, f"the DLL reads the section this writes ([{CONFIG_SECTION}])")
    for key in (KEY_ENABLED, KEY_SKIP_NEUTER, KEY_POLL_INTERVAL_MS, KEY_HEARTBEAT_INTERVAL_MS):
        check(f'"{key}"' in probe_src, f"the DLL reads {CONFIG_SECTION}.{key}")
    for arm, (_, expected_arm) in PROBE_ARMS.items():
        if expected_arm is not None:
            check(f'"{expected_arm}"' in probe_src, f"the DLL can report arm={expected_arm} (--probe {arm})")
    rva_src = (REPO_ROOT / "crates/ds2-rva/src/lib.rs").read_text()
    check("ARXAN_PROBE_HOOK_SITE" in rva_src, "the hook site RVA is recorded in ds2-rva")

    # THE ENVIRONMENT IS NO LONGER A CHANNEL, and this is the check that keeps it that way. The
    # variables did not merely stop working -- they were measured arriving unset, because
    # `steam -applaunch` starts the game from an already-running client's environment. Anything
    # that reads configuration back out of the environment reintroduces that failure silently.
    check(
        "std::env::var" not in probe_src and "std::env::var" not in loader_src,
        "the DLL reads NO configuration from the environment",
    )
    check(
        set(launch_env("neuter")) == {"WINEDLLOVERRIDES"},
        "the launch environment carries only WINEDLLOVERRIDES",
    )

    # THE CONFIG FILE ITSELF. It is written by this script and parsed by the DLL, and the two
    # halves are checked here against a parser that mirrors the DLL's rules: `[section]` headers,
    # `key = value`, `#` comments, strict `true`/`false`.
    def parse_config(text: str) -> tuple[dict[tuple[str, str], str], list[str]]:
        """A mirror of `ds2_hotkey_config::kv::KeyValues`, to the extent this file uses it."""
        values: dict[tuple[str, str], str] = {}
        unusable: list[str] = []
        section = ""
        for raw in text.splitlines():
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("["):
                if not line.endswith("]"):
                    unusable.append(line)
                    continue
                section = line[1:-1].strip()
                continue
            key, sep, value = line.partition("=")
            if not sep or not key.strip():
                unusable.append(line)
                continue
            value = value.strip()
            if not value.startswith('"') and " #" in value:
                value = value.split(" #", 1)[0].strip()
            values[(section, key.strip())] = value.strip('"')
        return values, unusable

    for arm, (settings, expected_arm) in PROBE_ARMS.items():
        text = config_text(arm)
        values, unusable = parse_config(text)
        check(not unusable, f"--probe {arm} writes a file with no unusable lines")
        for key, wanted in settings.items():
            check(
                values.get((CONFIG_SECTION, key)) == str(wanted).lower(),
                f"--probe {arm} writes [{CONFIG_SECTION}] {key} = {str(wanted).lower()}",
            )
        # The DLL accepts `true` and `false` and nothing else. A generator that emitted Python's
        # `True` would read as a rejected value and the probe would silently stay off.
        check(
            all(
                values.get((CONFIG_SECTION, key)) in ("true", "false")
                for key in (KEY_ENABLED, KEY_SKIP_NEUTER)
            ),
            f"--probe {arm} writes booleans the DLL's strict parser accepts",
        )
        # The two live keys stay commented out, so the DLL's own defaults are what run. Writing
        # them would silently pin the cadence to whatever this script happened to believe.
        check(
            (CONFIG_SECTION, KEY_POLL_INTERVAL_MS) not in values
            and (CONFIG_SECTION, KEY_HEARTBEAT_INTERVAL_MS) not in values,
            f"--probe {arm} leaves the live keys at the DLL's defaults",
        )
        check(
            f"# {KEY_POLL_INTERVAL_MS} = {DEFAULT_POLL_INTERVAL_MS}" in text
            and f"# {KEY_HEARTBEAT_INTERVAL_MS} = {DEFAULT_HEARTBEAT_INTERVAL_MS}" in text,
            f"--probe {arm} documents the live keys and their defaults in the file",
        )

    # THE ARMS MUST DIFFER, and in exactly one key. If two arms ever generated the same file the
    # A/B comparison would be two runs of the same experiment, and the arm-readback guard would
    # not catch it because the DLL would be reporting truthfully.
    neuter_values, _ = parse_config(config_text("neuter"))
    skip_values, _ = parse_config(config_text("skip-neuter"))
    differing = {key for key in neuter_values | skip_values if neuter_values.get(key) != skip_values.get(key)}
    check(
        differing == {(CONFIG_SECTION, KEY_SKIP_NEUTER)},
        f"the two arms differ in exactly one key ({KEY_SKIP_NEUTER})",
    )
    check(
        config_text("neuter") == config_text("neuter"),
        "the generated config is deterministic -- same arm, same bytes",
    )

    # WRITING IT, for real, to a temp directory. `write_config` is the one function here that
    # touches the game directory before a launch, and a run whose config never landed looks
    # exactly like a run whose probe never installed.
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        path, written = write_config(directory, "skip-neuter")
        check(path == directory / CONFIG_NAME, f"the config is written as {CONFIG_NAME}")
        check(path.read_text(encoding="utf-8") == written, "what was written is what was returned")
        check(
            parse_config(path.read_text(encoding="utf-8"))[0][(CONFIG_SECTION, KEY_SKIP_NEUTER)]
            == "true",
            "the file on disk parses back to the arm that was requested",
        )
        # REWRITING OVER THE OTHER ARM. Both arms run back to back with no user action between
        # them, so the second run must fully replace the first run's file rather than merge with
        # it or append to it.
        _, rewritten = write_config(directory, "neuter")
        check(
            path.read_text(encoding="utf-8") == rewritten == config_text("neuter"),
            "a second arm's write REPLACES the first arm's file",
        )
        check(
            parse_config(path.read_text(encoding="utf-8"))[0][(CONFIG_SECTION, KEY_SKIP_NEUTER)]
            == "false",
            "and the replaced file parses back to the second arm",
        )

    # The transcript has to carry the file, or the arm under test is invisible to whoever reads
    # the block later.
    quoted = quoted_config(config_text("skip-neuter"), indent="    | ")
    check(
        f"{KEY_SKIP_NEUTER} = true" in quoted and quoted.startswith("    | "),
        "the config is quoted into the transcript verbatim and indented",
    )

    # THE VERDICT LOGIC. It is the only code here that turns lines into a conclusion, so it is the
    # only code here that can turn a real finding into the wrong headline. Every branch that a run
    # can actually reach gets a synthetic log.
    def verdict_for(lines: list[str], requested: str = "neuter", **overrides) -> tuple[str, int]:
        state = new_probe_state()
        for line in lines:
            absorb_probe_line(line, state)
        state.update(overrides)
        return probe_block(requested, state)

    installed = [
        f"{PROBE_LINE_PREFIX} install arm=neuter-arxan base=0x0000000140000000 rva=0x00832e70 va=0x0000000140832e70",
        f"{PROBE_LINE_PREFIX} install original=[48 89 5c 24 08 57 48 83 ec 20 48 8b d9 48 8b 0d] expected=[48 89 5c 24 08] prologue-match=true",
        f"{PROBE_LINE_PREFIX} install minhook=ok trampoline=0x0000000012340000 patched=[e9 8b 1a 3c ff 57 48 83 ec 20 48 8b d9 48 8b 0d] site-jmp=true",
        f"{PROBE_LINE_PREFIX} watching arm=neuter-arxan poll=1.0s heartbeat=10.0s site-window=16 trampoline-window=64",
    ]
    healthy = f"{PROBE_LINE_PREFIX} heartbeat uptime=180.0s arm=neuter-arxan hits=48213991 site=intact tramp=intact site-diverged=0 tramp-diverged=0"

    block, code = verdict_for(installed + [healthy])
    check(code == EXIT_OK and "SURVIVED and FIRED" in block, "a clean run reads as survived")
    check("hits=48213991" in block, "the hit count is quoted, not summarised")

    silent = f"{PROBE_LINE_PREFIX} heartbeat uptime=180.0s arm=neuter-arxan hits=0 site=intact tramp=intact site-diverged=0 tramp-diverged=0"
    block, code = verdict_for(installed + [silent])
    check(
        code == EXIT_OK and "never called" in block and "SURVIVED" not in block,
        "an intact patch that never fired is NOT reported as a surviving detour",
    )

    reverted = [
        f"{PROBE_LINE_PREFIX} SITE uptime=41.0s arm=neuter-arxan state=DIVERGED prev=intact hits=98765 va=0x0000000140832e70 expected=[e9 8b 1a 3c ff 57 48 83 ec 20 48 8b d9 48 8b 0d] observed=[48 89 5c 24 08 57 48 83 ec 20 48 8b d9 48 8b 0d]",
        f"{PROBE_LINE_PREFIX} heartbeat uptime=50.0s arm=neuter-arxan hits=98765 site=DIVERGED tramp=intact site-diverged=1 tramp-diverged=0",
    ]
    block, code = verdict_for(installed + reverted)
    check(code == EXIT_OK, "a reverted hook still EXITS ZERO -- the experiment ran")
    check("HOOK SITE WAS REVERTED" in block, "a reverted hook says so")
    check("observed=[48 89 5c 24 08" in block, "the observed bytes are reproduced verbatim")

    corrupt = f"{PROBE_LINE_PREFIX} heartbeat uptime=50.0s arm=neuter-arxan hits=0 site=intact tramp=DIVERGED site-diverged=0 tramp-diverged=1"
    block, _ = verdict_for(installed + [corrupt])
    check(
        "TRAMPOLINE WAS CORRUPTED" in block,
        "a corrupt trampoline is reported even though the site looks intact",
    )

    block, code = verdict_for(installed + [healthy], requested="skip-neuter")
    check(
        code == EXIT_NO_PROBE_VERDICT and "WRONG ARM" in block,
        "a log whose arm is not the requested one REFUSES to produce a verdict",
    )

    block, code = verdict_for([], requested="neuter")
    check(code == EXIT_NO_PROBE_VERDICT and "never installed" in block, "no install line, no verdict")

    void = f"{PROBE_LINE_PREFIX} VOID prologue-mismatch va=0x0000000140832e70 -- already patched"
    block, code = verdict_for([void])
    check(code == EXIT_NO_PROBE_VERDICT and "refused the hook site" in block, "a VOID run is not a result")

    block, code = verdict_for(installed)
    check(code == EXIT_NO_PROBE_VERDICT and "never reported a heartbeat" in block, "installed but silent is not a result")

    # A config edited mid-window is not a footnote: the measurement cadence may have changed
    # underneath the numbers above, and a reader comparing two arms has to be told.
    touched = [
        f"{PROBE_LINE_PREFIX} config uptime=30.0s RELOADED poll=1000ms heartbeat=10000ms -> poll=1000ms heartbeat=60000ms",
    ]
    block, code = verdict_for(installed + touched + [healthy])
    check(code == EXIT_OK, "a mid-run config reload is not itself a failure")
    check("CONFIG FILE WAS TOUCHED" in block, "a mid-run config reload is reported, not swallowed")
    check("heartbeat=60000ms" in block, "the reload is quoted verbatim")

    ignored = [
        f"{PROBE_LINE_PREFIX} config uptime=30.0s STARTUP-ONLY-IGNORED enabled=\"true\" skip_neuter=\"true\" -- this run is still arm=neuter-arxan",
    ]
    block, code = verdict_for(installed + ignored + [healthy])
    check(
        code == EXIT_OK and "STARTUP-ONLY-IGNORED" in block,
        "an attempt to switch arms mid-run is reported and does NOT change the arm",
    )
    check(
        "arm            neuter-arxan" in block,
        "and the verdict still reports the arm that actually ran",
    )

    block, _ = verdict_for(installed + [healthy])
    check("CONFIG FILE WAS TOUCHED" not in block, "an untouched config says nothing at all")

    block, _ = verdict_for(installed + [healthy], detach=f"{PROBE_LINE_PREFIX} detach uptime=612.4s arm=neuter-arxan hits=99 site=intact tramp=intact site-diverged=0 tramp-diverged=0")
    check("detach uptime=612.4s" in block, "an orderly exit is reported as one")
    block, _ = verdict_for(installed + [healthy])
    check("did not wind down through" in block, "a missing detach line is reported as a possible crash")

    print("selftest: " + ("OK" if ok else "FAILED"))
    return EXIT_OK if ok else EXIT_ERROR


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="report what a run would do; stage nothing, launch nothing",
    )
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="test the log tailer, the DLL/script contract and the verdict logic",
    )
    parser.add_argument(
        "--probe",
        choices=sorted(PROBE_ARMS),
        default="off",
        help=(
            "run the M1 Arxan-survival experiment. `neuter` runs it with dearxan having "
            "neutered Arxan first; `skip-neuter` leaves Arxan's 48 stubs live. BOTH ARMS ARE "
            "NEEDED -- neither one alone distinguishes 'dearxan saved the hook' from 'Arxan "
            "never touched it'. Default: off, which is the plain loader run."
        ),
    )
    parser.add_argument(
        "--observe",
        type=float,
        default=OBSERVE_SECONDS,
        metavar="SECONDS",
        help=(
            f"how long to watch the probe after it installs (default {OBSERVE_SECONDS:.0f}). "
            "Ignored when --probe is off. The verdict is explicitly scoped to this window."
        ),
    )
    args = parser.parse_args()

    if args.selftest:
        return selftest()
    if args.dry_run:
        return dry_run(args.probe, args.observe)
    return launch(args.probe, args.observe)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
