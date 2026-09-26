#!/usr/bin/env python3
"""Rate-limit a Monitor event stream to at most one notification per window.

Every stdout line a `Monitor` command emits becomes a conversation message. A
`tail -f | grep` over a live game log has no natural rate: on run
br-20260912-202348-248f one power-of-two-backed-off log line still produced a
notification every few seconds for minutes, and the user had to interrupt the
session three times to stop it. The harness's own suppression only kicks in
after the flood has already been delivered.

This filter sits at the end of such a pipeline and emits at most one line per
`--interval` seconds. Lines arriving inside a closed window are counted, not
dropped silently: the next emitted line carries `[+N suppressed in the last Ms]`
so nothing is lost about volume, and the last suppressed line is the one shown,
because for a progress stream the newest line is the informative one.

The window is wall-clock and starts closed-open: the first line of a stream is
always emitted immediately, so a monitor that fires once and exits is unaffected.

Usage, as the final stage of a Monitor command:

    tail -f some.log | grep -E --line-buffered "PATTERN" \
        | python3 scripts/monitor-throttle.py 10

`--selftest` runs the behaviour checks and needs no input.
"""

from __future__ import annotations

import argparse
import sys
import time

#: Below this, a stream is fast enough to flood the conversation, so the guard
#: in `.cupcake/policies/claude/monitor_rate_limit.rego` refuses it. Kept here
#: as well so a direct run reports the same number the policy enforces.
MIN_INTERVAL_SECONDS = 10.0


def throttle(lines, interval, emit, now=time.monotonic):
    """Emit at most one line per `interval` seconds from the iterable `lines`.

    Split out from `main` so `--selftest` can drive it with a fake clock and a
    list sink rather than real time and a real pipe.
    """
    window_start = None
    suppressed = 0
    pending = None
    for line in lines:
        stamp = now()
        if window_start is not None and stamp - window_start < interval:
            suppressed += 1
            pending = line
            continue
        window_start = stamp
        if suppressed:
            emit(f"{line}  [+{suppressed} suppressed in the last {interval:g}s]")
            suppressed = 0
            pending = None
        else:
            emit(line)
    # A stream that ends mid-window still owes its last line: dropping it would
    # lose the final state, which on a run log is the one that says how it ended.
    #
    # The count excludes `pending` itself. Mid-stream the suppressed lines are all
    # genuinely lost and a different line opens the new window, so the raw count is
    # right there; here the pending line is the one being shown, and counting it as
    # suppressed reports one loss that did not happen.
    if pending is not None:
        lost = suppressed - 1
        if lost > 0:
            emit(f"{pending}  [+{lost} suppressed in the last {interval:g}s]")
        else:
            emit(pending)


def selftest() -> int:
    failures = []

    def check(name, got, want):
        if got != want:
            failures.append(f"{name}: got {got!r} want {want!r}")

    # A clock the test drives by hand, one tick per line read.
    ticks = iter([0.0, 1.0, 2.0, 3.0, 30.0, 31.0])
    out = []
    throttle(["a", "b", "c", "d", "e", "f"], 10.0, out.append, now=lambda: next(ticks))
    check(
        "coalesces a burst and reports the count",
        out,
        [
            "a",
            "e  [+3 suppressed in the last 10s]",
            "f",
        ],
    )

    # First line is never delayed, and a single-line stream is untouched.
    out = []
    throttle(["only"], 10.0, out.append, now=lambda: 5.0)
    check("a single line passes straight through", out, ["only"])

    # A stream that ends inside a closed window still emits its last line.
    ticks = iter([0.0, 1.0, 2.0])
    out = []
    throttle(["x", "y", "z"], 10.0, out.append, now=lambda: next(ticks))
    check(
        "the final suppressed line is not lost",
        out,
        ["x", "z  [+1 suppressed in the last 10s]"],
    )

    # An empty stream emits nothing rather than an empty trailing line.
    out = []
    throttle([], 10.0, out.append, now=lambda: 0.0)
    check("an empty stream emits nothing", out, [])

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    print("monitor-throttle selftest: 4 checks passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "interval",
        nargs="?",
        type=float,
        default=MIN_INTERVAL_SECONDS,
        help=f"seconds between emitted lines (default and minimum {MIN_INTERVAL_SECONDS:g})",
    )
    parser.add_argument("--selftest", action="store_true", help="run the behaviour checks and exit")
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    if args.interval < MIN_INTERVAL_SECONDS:
        print(
            f"monitor-throttle: refusing interval {args.interval:g}s; the minimum is "
            f"{MIN_INTERVAL_SECONDS:g}s, which is what the Monitor guard enforces",
            file=sys.stderr,
        )
        return 2

    def emit(line: str) -> None:
        sys.stdout.write(line + "\n")
        sys.stdout.flush()

    throttle((line.rstrip("\n") for line in sys.stdin), args.interval, emit)
    return 0


if __name__ == "__main__":
    sys.exit(main())
