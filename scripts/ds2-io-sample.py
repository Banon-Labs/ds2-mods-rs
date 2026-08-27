#!/usr/bin/env python3
"""Sample a running DARK SOULS II process's I/O and CPU counters, from outside it.

    python3 scripts/ds2-io-sample.py --wait 120 --seconds 12

Answers one question the in-process timeline cannot: is a stretch of startup **disk-bound or
CPU-bound**? `docs/DS2-BOOT-WORK.md` localises 3.06s of boot to the window between input
initialisation and the title flow, and nothing in the loader DLL can say what that window is doing
without hooking file APIs -- which means a naked thunk around a ten-argument import, inside a
process that carries 48 Arxan integrity stubs.

This needs none of that. Linux already counts the bytes.

WHAT IT READS, per sample, straight out of procfs:

* `/proc/<pid>/io` -- `rchar` (bytes returned by read syscalls, including page-cache hits) and
  `read_bytes` (bytes actually fetched from the block device). The GAP between them is the
  interesting part: `rchar` climbing while `read_bytes` stays flat is a warm cache, which is a
  completely different problem from a cold one.
* `/proc/<pid>/stat` -- `utime` and `stime`, in clock ticks. CPU burnt in the same interval.

So a window where `rchar` climbs is reading; one where `utime` climbs is computing; one where
neither climbs is WAITING, and waiting is the signature the two one-second floors already showed.

WHAT IT DOES NOT DO. It does not attach, trace, stop or signal the process -- every read is a
`read()` on a procfs file, so a run under this sampler is the same run it would have been without
it. That is the point: the game is under Proton with Arxan live, and an observer that can perturb
the thing it observes is worth less than no observer.

CAVEAT ON THE PID. Several `wine:` entries share the name `DarkSoulsII.exe`; the real one is the
`wine64-preloader` child. This picks the pid with the largest RSS, which is that one by a wide
margin, and prints what it chose so a wrong pick is visible rather than silent.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path

GAME_COMM = "DarkSoulsII.exe"
#: `utime`/`stime` are in clock ticks; every Linux this repo targets uses 100.
CLOCK_TICKS_PER_SECOND = 100


def candidate_pids() -> list[int]:
    result = subprocess.run(["pgrep", "-x", GAME_COMM], capture_output=True, text=True)
    return [int(line) for line in result.stdout.split() if line.isdigit()]


def rss_kb(pid: int) -> int:
    try:
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1])
    except OSError:
        pass
    return 0


def pick_pid(pids: list[int]) -> int | None:
    """The real game process, by RSS. Wine's placeholder entries are tiny; the game is hundreds of
    megabytes, so the margin is not a close call -- but the chosen pid is printed regardless."""
    return max(pids, key=rss_kb) if pids else None


def counters(pid: int) -> dict[str, int] | None:
    try:
        io = {}
        for line in Path(f"/proc/{pid}/io").read_text().splitlines():
            key, _, value = line.partition(": ")
            io[key] = int(value)
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()
        # After the "(comm) " split, field 0 is `state`, so utime is index 11 and stime 12.
        return {
            "rchar": io.get("rchar", 0),
            "read_bytes": io.get("read_bytes", 0),
            "utime": int(fields[11]),
            "stime": int(fields[12]),
        }
    except (OSError, IndexError, ValueError):
        return None


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wait", type=float, default=120.0, help="seconds to wait for the process")
    parser.add_argument("--seconds", type=float, default=12.0, help="seconds to sample once found")
    parser.add_argument("--interval", type=float, default=0.02, help="seconds between samples")
    args = parser.parse_args(argv[1:])

    deadline = time.monotonic() + args.wait
    pid = None
    while time.monotonic() < deadline:
        pid = pick_pid(candidate_pids())
        if pid and counters(pid):
            break
        pid = None
        time.sleep(0.05)
    if pid is None:
        print(f"no {GAME_COMM} within {args.wait}s", file=sys.stderr)
        return 2

    started = time.monotonic()
    print(f"# pid {pid}  rss {rss_kb(pid)} kB  interval {args.interval}s")
    print("# t_ms\td_rchar_kb\td_read_kb\td_cpu_ms")
    previous = counters(pid)
    last = started
    while time.monotonic() - started < args.seconds:
        time.sleep(args.interval)
        current = counters(pid)
        if current is None:
            print(f"# process gone at t={1000 * (time.monotonic() - started):.0f}ms")
            break
        now = time.monotonic()
        cpu_ticks = (current["utime"] + current["stime"]) - (previous["utime"] + previous["stime"])
        print(
            f"{1000 * (now - started):.0f}\t"
            f"{(current['rchar'] - previous['rchar']) / 1024:.0f}\t"
            f"{(current['read_bytes'] - previous['read_bytes']) / 1024:.0f}\t"
            f"{1000 * cpu_ticks / CLOCK_TICKS_PER_SECOND:.0f}"
        )
        previous, last = current, now
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
