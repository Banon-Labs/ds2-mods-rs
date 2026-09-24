#!/usr/bin/env python3
"""Count how many times each spot on the ground is asked for a Prism Stone.

A trail laid once asks for each site once. A site asked for repeatedly is the clump the player
sees: many effects stacked on one patch of ground, which reads as a fountain rather than a marker.

Needs the Wine-side server up:  python3 scripts/ds2-frida-up.py
"""

from __future__ import annotations

import argparse
import math
import sys
import time
from pathlib import Path

import frida

REPO_ROOT = Path(__file__).resolve().parent.parent
AGENT = REPO_ROOT / "scripts" / "frida" / "stone-sites.js"
ADDRESS = "127.0.0.1:27042"
TARGET = "DarkSoulsII.exe"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=30.0)
    args = parser.parse_args()

    device = frida.get_device_manager().add_remote_device(ADDRESS)
    pid = next((p.pid for p in device.enumerate_processes() if p.name == TARGET), None)
    if pid is None:
        print(f"{TARGET} is not running under the prefix server at {ADDRESS}")
        return 1

    received: list[dict] = []
    session = device.attach(pid)
    script = session.create_script(AGENT.read_text())
    script.on(
        "message",
        lambda m, _d: received.append(m["payload"])
        if m.get("type") == "send"
        else print("agent:", m.get("description")),
    )
    script.load()
    print(f"attached to pid {pid}; counting Prism Stone spawns for {args.seconds:.0f}s")

    deadline = time.monotonic() + args.seconds
    while not received and time.monotonic() < deadline:
        time.sleep(0.25)
    if not received:
        script.post({"type": "drain"})
        time.sleep(1.5)
    session.detach()

    if not received:
        print("the agent never answered -- the JS thread is not running")
        return 1
    found = received[-1]
    if "error" in found:
        print("refused:", found["error"])
        return 1

    sites = list(found["sites"].values())
    total = found["spawns"]
    print(f"\n{total} Prism Stone spawn(s) across {len(sites)} distinct site(s)")
    print(f"({found.get('others', 0)} spawn(s) of other effects, the game's own, ignored)\n")
    if not sites:
        return 0

    sites.sort(key=lambda s: s["count"], reverse=True)
    print("the most-repeated ground, worst first:")
    print(f"  {'count':>6}  {'id':>4}  position")
    for site in sites[:12]:
        at = site["at"]
        print(
            f"  {site['count']:>6}  {site['id']:>4}  "
            f"[{at[0]:8.2f} {at[1]:7.2f} {at[2]:8.2f}]"
        )

    repeated = [s for s in sites if s["count"] > 1]
    stacked = sum(s["count"] - 1 for s in repeated)
    print(
        f"\n{len(repeated)} site(s) asked for more than once; {stacked} spawn(s) landed on ground "
        "that already had a stone"
    )
    worst = sites[0]
    if worst["count"] > 3:
        at = worst["at"]
        print(
            f"\nTHE CLUMP IS REAL AND IT IS AT [{at[0]:.2f} {at[1]:.2f} {at[2]:.2f}]: that one "
            f"patch was asked for {worst['count']} times.\nThe trail's own duplicate rule refuses "
            "a candidate within half a spacing of a stone it BELIEVES is down,\nso this many "
            "repeats means the trail forgot them -- a teardown, a lane reopening, or a spawn that "
            "returned no handle."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
