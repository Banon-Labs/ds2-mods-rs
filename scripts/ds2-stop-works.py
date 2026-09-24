#!/usr/bin/env python3
"""Hold `frida/stop-works.js` on the live game and report whether the stop call stops anything.

Stays attached for `--seconds` (default 45) so a teardown has time to happen, then drains whatever
the agent collected and prints the verdict. Needs the Wine-side server already up:

    python3 scripts/ds2-frida-up.py
    uv run --with frida python3 scripts/ds2-stop-works.py --seconds 60
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

import frida

REPO_ROOT = Path(__file__).resolve().parent.parent
AGENT = REPO_ROOT / "scripts" / "frida" / "stop-works.js"
ADDRESS = "127.0.0.1:27042"
TARGET = "DarkSoulsII.exe"


def verdict(samples: list[dict]) -> str:
    """What the before/after pairs say, counted rather than eyeballed."""
    stopped = 0
    still_alive = 0
    unlinked = 0
    nothing = 0
    for sample in samples:
        for before, after in zip(sample["before"], sample["after"]):
            if before["alive"] is None:
                nothing += 1
            elif after["node"] is None:
                unlinked += 1
            elif before["alive"] and not after["alive"]:
                stopped += 1
            elif before["alive"] and after["alive"]:
                still_alive += 1
            else:
                nothing += 1
    total = stopped + still_alive + unlinked
    lines = [
        f"  alive before -> dead after   {stopped:>4}   the stop works",
        f"  alive before -> ALIVE after  {still_alive:>4}   the stop did nothing",
        f"  node pointer went null       {unlinked:>4}   unlinked rather than cleared",
        f"  no node either side          {nothing:>4}   an empty half, says nothing",
    ]
    if total == 0:
        lines.append("\nNO LIVE NODE WAS EVER SEEN. Every block passed to the stop already had "
                     "null or dead nodes, so this run cannot say whether the stop works.")
    elif still_alive and not stopped:
        lines.append("\nTHE STOP IS A NO-OP on every effect it was handed. Stones the teardown "
                     "'put out' are still playing, and each teardown orphans its whole trail.")
    elif stopped and not still_alive:
        lines.append("\nTHE STOP WORKS on every effect it was handed. The pile on screen is not "
                     "orphaned stones; it is the trail being re-laid on the same ground.")
    else:
        lines.append("\nMIXED: some stopped and some did not, so it is not a property of the call "
                     "alone. Compare the blocks -- the difference is in the effects, not the stop.")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=45.0)
    args = parser.parse_args()

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
        elif message.get("type") == "error":
            print("agent error:", message.get("description"))

    session = device.attach(pid)
    script = session.create_script(AGENT.read_text())
    script.on("message", on_message)
    script.load()
    print(f"attached to pid {pid}; watching stop calls for {args.seconds:.0f}s")

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

    samples = found["samples"]
    if found.get("partial"):
        print(f"(drained early: {found.get('seen', 0)} stop call(s) seen)")
    print(f"{len(samples)} stop call(s) sampled\n")
    for sample in samples[:8]:
        for index, (before, after) in enumerate(zip(sample["before"], sample["after"])):
            if before["node"] is None and after["node"] is None:
                continue
            print(
                f"  call {sample['call']:>3} half {index}  node {before['node']}"
                f" -> {after['node']}"
                f"  flags {before.get('flags')} -> {after.get('flags')}"
                f"   alive {before['alive']} -> {after['alive']}"
            )
    print()
    print(verdict(samples))

    # DID THE NODE ITSELF CHANGE? The block being cleared says the controller let go; only the
    # node's own bytes say whether the effect was stopped or merely detached and left playing.
    print()
    untouched = 0
    changed = 0
    unreadable = 0
    first_diff = None
    for sample in samples:
        for before, after in zip(sample.get("nodeBefore") or [], sample.get("nodeAfter") or []):
            if before is None or after is None:
                continue
            if before["bytes"] is None or after["bytes"] is None:
                unreadable += 1
                continue
            if before["bytes"] == after["bytes"]:
                untouched += 1
                continue
            changed += 1
            if first_diff is None:
                offsets = [
                    i // 2
                    for i in range(0, len(before["bytes"]), 2)
                    if before["bytes"][i : i + 2] != after["bytes"][i : i + 2]
                ]
                first_diff = (before["at"], offsets)
    print("the effect node itself, across the same call:")
    print(f"  byte-for-byte unchanged      {untouched:>4}   detached only -- still playing")
    print(f"  something in the node changed{changed:>4}   the engine acted on the effect")
    print(f"  could not be read after      {unreadable:>4}   freed, which is also a stop")
    if first_diff is not None:
        where = ", ".join(f"+0x{off:x}" for off in first_diff[1][:12])
        print(f"  first node that changed: {first_diff[0]} at {where}")

    # THE ALIVE WORD, SPELLED OUT. `ds2_rva::KATANA_SFX_NODE_ALIVE_OFFSET` is +0x58 and the bit is
    # 0x40000000, so byte +0x5b is the one that carries it. Printing the whole u32 either side
    # turns "a byte changed near the right place" into "the bit the crate reads was cleared".
    def word_at(entry: dict, offset: int) -> int | None:
        raw = entry.get("bytes")
        if not raw:
            return None
        chunk = raw[offset * 2 : offset * 2 + 8]
        if len(chunk) < 8:
            return None
        return int.from_bytes(bytes.fromhex(chunk), "little")

    print()
    print("  the alive word at +0x58, and bit 0x40000000 in it:")
    shown = 0
    for sample in samples:
        for before, after in zip(sample.get("nodeBefore") or [], sample.get("nodeAfter") or []):
            if before is None or after is None or shown >= 6:
                continue
            was = word_at(before, 0x58)
            now = word_at(after, 0x58)
            if was is None or now is None:
                continue
            shown += 1
            print(
                f"    {before['at']}  0x{was:08x} -> 0x{now:08x}"
                f"   alive {bool(was & 0x40000000)} -> {bool(now & 0x40000000)}"
            )
    if untouched and not changed and not unreadable:
        print(
            "\nTHE STOP DETACHES AND DOES NOT STOP. The control block is cleared, the effect node"
            "\nis untouched, so every stone the teardown 'put out' is still burning in the world."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
