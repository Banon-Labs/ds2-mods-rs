#!/usr/bin/env python3
"""Run `frida/route-endpoints.js` once against the live game and print what it found.

WHY A SCRIPT AND NOT `frida -l`: the agent sends one structured message and stops. This waits for
exactly that message, prints it as a table, and detaches -- so nothing stays injected, and there is
no timer to wedge (see the `frida-js-thread-wedges-in-ds2` memory).

Needs the Wine-side server already up:  python3 scripts/ds2-frida-up.py
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import frida

REPO_ROOT = Path(__file__).resolve().parent.parent
AGENT = REPO_ROOT / "scripts" / "frida" / "route-endpoints.js"
ADDRESS = "127.0.0.1:27042"
TARGET = "DarkSoulsII.exe"
#: The agent does one pass at load; anything past this is the JS thread not running at all.
DEADLINE_SECONDS = 20.0


def main() -> int:
    device = frida.get_device_manager().add_remote_device(ADDRESS)
    pid = None
    for process in device.enumerate_processes():
        if process.name == TARGET:
            pid = process.pid
            break
    if pid is None:
        print(f"{TARGET} is not running under the prefix server at {ADDRESS}")
        return 1

    received: list[dict] = []

    def on_message(message, _data):
        if message.get("type") == "send":
            received.append(message["payload"])
        else:
            print("agent error:", message)

    session = device.attach(pid)
    script = session.create_script(AGENT.read_text())
    script.on("message", on_message)
    script.load()

    deadline = time.monotonic() + DEADLINE_SECONDS
    while not received and time.monotonic() < deadline:
        time.sleep(0.1)
    session.detach()

    if not received:
        print(f"no answer in {DEADLINE_SECONDS:.0f}s -- the JS thread is not running")
        return 1

    found = received[0]
    if "error" in found:
        print("refused:", found["error"])
        return 1

    print(f"module base {found['base']}   local player {found['local']} at {found['localPosition']}")
    print(f"PlayerCtrl vtable {found['playerVtable']}")
    print()
    header = f"{'ctrl':<20}{'class':<22}{'m':>8}  {'name':<22}verdict"
    print(header)
    print("-" * len(header))
    entries = sorted(
        found["roster"],
        key=lambda row: (row["meters"] is None, row["meters"] or 0.0),
    )
    for row in entries:
        meters = "-" if row["meters"] is None else f"{row['meters']:.1f}"
        name = row.get("name") or "-"
        print(
            f"{row['ctrl']:<20}{row['klass']:<22}{meters:>8}  {name:<22}{row['verdict']}"
        )

    players = [r for r in found["roster"] if r["klass"] == "PlayerCtrl" and not r["isLocal"]]
    print()
    print(f"{len(found['roster'])} character(s); {len(players)} remote PlayerCtrl object(s)")

    print()
    print("where each PlayerCtrl's name sits, and what the same slots hold on the others:")
    for row in found["roster"]:
        if row["klass"] != "PlayerCtrl":
            continue
        where = "not found" if row.get("nameAt") is None else f"+0x{row['nameAt']:x}"
        print(f"  {row['ctrl']}  name {where}  {row.get('name') or '-'}")
        for slot in row.get("probe") or []:
            text = slot["text"] or "-"
            print(f"      +0x{slot['at']:<4x} {slot['value'] or 'null':<20} {text}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
