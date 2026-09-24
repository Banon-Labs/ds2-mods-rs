#!/usr/bin/env python3
"""Does `ds2-input-harness` still answer, with whatever is or is not plugged in?

    python3 scripts/ds2-harness-liveness.py [--timeout SECONDS]

WHY THIS EXISTS. The harness's state machine advanced once per DirectInput KEYBOARD poll, so
everything it does -- holds, countdowns, the closed camera loop, even answering `status` -- ran
only while DARK SOULS II happened to be reading a device. On 2026-09-22 it went completely deaf
mid-session: three `status` commands at fresh sequence numbers produced not one log line, while
the process was alive and burning 135% CPU. Unplug the controller and you get the same silence
for a different reason.

That is the failure this checks for, and it is worth a script rather than an eyeball because the
symptom is ABSENCE. Nothing appears in the log, nothing crashes, and a run that measured nothing
reads exactly like a run that measured zero. The user has said they will disconnect the Xbox
controller periodically to provoke it, which makes this the acceptance test for the harness
being device-independent: it must pass with the pad connected, with it unplugged, and across it
being unplugged while the game runs.

HOW. Writes a `status` command with a sequence number no previous command used, then watches the
game's own log for the reply the harness writes when it executes one. Exit 0 if the reply
arrives, 1 if it does not, 2 if the game or the log is not there to ask. Nothing here inspects
the harness's internals: the only evidence accepted is the harness speaking.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

GAME_DIR = Path(
    "/home/banon/.local/share/Steam/steamapps/common/"
    "Dark Souls II Scholar of the First Sin/Game"
)
LOG = GAME_DIR / "ds2-loader.log"
COMMANDS = GAME_DIR / "ds2-input-harness-cmd.txt"

#: The harness writes this when it runs a `status`. Its presence is the whole of the evidence.
REPLY = "ds2-input-harness: status:"

#: `/proc/bus/input/devices` names every attached device; this is how the report says whether a
#: pad was plugged in for THIS measurement, which is the variable the user is changing.
PAD_NAMES = re.compile(r"pad|joy|xbox|dual|controller", re.IGNORECASE)


def pad_attached() -> bool:
    """Is any gamepad-looking device attached right now?"""
    try:
        devices = Path("/proc/bus/input/devices").read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False
    return any(PAD_NAMES.search(line) for line in devices.splitlines() if line.startswith("N: "))


def game_running() -> bool:
    """`-x` rather than `-f`: a full-command-line match would match this script."""
    return subprocess.run(["pgrep", "-x", "DarkSoulsII.exe"], capture_output=True).returncode == 0


def next_sequence(text: str) -> int:
    """A sequence number no command in the log has used.

    The harness runs a command when the NUMBER changes, so re-using one is inert -- and an inert
    command is indistinguishable from a dead harness, which is the one confusion this script
    exists to prevent.
    """
    used = [int(found) for found in re.findall(r"command #(\d+)", text)]
    return max(used, default=0) + 1


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--timeout",
        type=float,
        default=10.0,
        help="seconds to wait for the harness to answer. DEFAULT: 10, which is hundreds of "
        "frames -- a harness ticking on anything at all replies inside one.",
    )
    arguments = parser.parse_args(argv[1:])

    if not game_running():
        print("NOT MEASURED: DARK SOULS II is not running, so there is nothing to ask.")
        return 2
    if not LOG.is_file():
        print(f"NOT MEASURED: no log at {LOG}")
        return 2

    before = LOG.read_text(encoding="utf-8", errors="replace")
    sequence = next_sequence(before)
    pad = pad_attached()
    print(f"asking (sequence {sequence}, gamepad {'attached' if pad else 'NOT attached'})")
    try:
        COMMANDS.write_text(f"{sequence}\nstatus\n", encoding="utf-8")
    except OSError as error:
        print(f"NOT MEASURED: could not write {COMMANDS}: {error}")
        return 2

    deadline = time.monotonic() + arguments.timeout
    while time.monotonic() < deadline:
        grown = LOG.read_text(encoding="utf-8", errors="replace")[len(before) :]
        for line in grown.splitlines():
            if REPLY in line:
                print(f"ALIVE: {line.strip()}")
                return 0
        time.sleep(0.25)

    print(
        f"DEAF: no reply in {arguments.timeout:.0f}s with the gamepad "
        f"{'attached' if pad else 'NOT attached'}. The harness is alive only while something "
        f"it depends on is being polled -- which is the bug, not the weather."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
