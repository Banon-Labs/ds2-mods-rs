#!/usr/bin/env python3
"""Print the live route polyline `ds2-invasion-path` is laying stones along, and where it stops.

Needs the Wine-side server up:  python3 scripts/ds2-frida-up.py
"""

from __future__ import annotations

import math
import sys
import time
from pathlib import Path

import frida

REPO_ROOT = Path(__file__).resolve().parent.parent
AGENT = REPO_ROOT / "scripts" / "frida" / "route-points.js"
ADDRESS = "127.0.0.1:27042"
TARGET = "DarkSoulsII.exe"
DEADLINE_SECONDS = 20.0

#: A stone is spaced `marker_spacing_meters` apart (2.7 by default), so a hop several times that
#: with nothing between it is the "jumps over terrain" the player is looking at.
JUMP_METERS = 8.0
#: Two points closer than this are the same place, and lay a stone on a stone.
SAME_PLACE_METERS = 0.05


def dist(a, b):
    if a is None or b is None:
        return None
    return math.dist(a, b)


def main() -> int:
    device = frida.get_device_manager().add_remote_device(ADDRESS)
    pid = next((p.pid for p in device.enumerate_processes() if p.name == TARGET), None)
    if pid is None:
        print(f"{TARGET} is not running under the prefix server at {ADDRESS}")
        return 1

    received: list[dict] = []
    session = device.attach(pid)
    script = session.create_script(AGENT.read_text())
    script.on("message", lambda m, _d: received.append(m["payload"]) if m.get("type") == "send" else print("agent:", m))
    script.load()
    deadline = time.monotonic() + DEADLINE_SECONDS
    while not received and time.monotonic() < deadline:
        time.sleep(0.1)
    session.detach()
    if not received:
        print("no answer -- the JS thread is not running")
        return 1
    found = received[0]
    if "error" in found:
        print("refused:", found["error"])
        return 1

    local = found["local"]
    print(f"local player at {local}")
    for target in found["targets"]:
        print(f"  target {target['ctrl']}  {target['name'] or '-'}  at {target['at']}"
              f"  {dist(local, target['at']):.1f} m away")
    print(f"\n{len(found['routes'])} decodable route(s) on the navigation system's list\n")

    for route in found["routes"]:
        points = route["points"]
        print(f"planner {route['planner']}  --  {len(points)} point(s)")
        jumps = 0
        repeats = 0
        previous = None
        for index, entry in enumerate(points):
            at = entry["at"]
            step = dist(previous, at)
            flag = ""
            if step is not None and step >= JUMP_METERS:
                flag = f"   <-- JUMP of {step:.1f} m"
                jumps += 1
            elif step is not None and step <= SAME_PLACE_METERS:
                flag = "   <-- SAME PLACE as the point before it"
                repeats += 1
            shown = f"[{at[0]:8.2f} {at[1]:7.2f} {at[2]:8.2f}]"
            stepped = "   -" if step is None else f"{step:7.2f}"
            print(f"  {index:>3} seg {entry['segment']:>2} slot {entry['slot']:>3}  {shown} {stepped}{flag}")
            previous = at
        finish = points[-1]["at"]
        print(f"  start is {dist(local, points[0]['at']):.1f} m from the player")
        nearest = None
        for target in found["targets"]:
            gap = dist(finish, target["at"])
            if gap is not None and (nearest is None or gap < nearest[0]):
                nearest = (gap, target)
        if nearest is not None:
            gap, target = nearest
            print(f"  END is {gap:.1f} m from {target['name'] or target['ctrl']}"
                  f" -- the trail stops that far short of them")
        print(f"  {jumps} jump(s) over {JUMP_METERS:.0f} m, {repeats} repeated point(s)\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
