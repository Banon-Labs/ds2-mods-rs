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

# The reload quiet window

`scripts/ds2-frida-watch.py` hot-reloads its agent on every edit and writes a
line tagged `"agent.reloaded"` to its log before each load. An agent re-runs its
load-time one-shots on every reload, so any of them that matches the monitor's
grep notified once per edit: measured 2026-11-02, when the soapstone `give` loop
in `scripts/frida/summon-sign-trace.js` sent a line with an `error` key on every
load. The interval does not help there -- edits are minutes apart.

So a line containing `"agent.reloaded"` opens a quiet window of
`--reload-quiet` seconds (default 15, 0 disables). Every line inside it is
dropped, the marker included; a second marker restarts the window. When the
window closes it emits ONE line, `[reload: N lines suppressed, K matched a
failure word; last: <line>]`, and only if at least one suppressed line matched
a failure word (error, exception, detached, Traceback). A reload that produced
only its usual chatter emits nothing. The window closes on the clock, not on the
next line, so a failed reload is reported even when the log then goes quiet.

The window only opens if the marker reaches this filter. A grep in front of it
must keep `agent\\.reloaded` in its alternation; the Monitor guard in
`.cupcake/policies/claude/monitor_rate_limit.rego` refuses a Frida log monitor
whose grep would drop it.

Usage, as the final stage of a Monitor command:

    tail -n0 -F "$S/summon-sign.jsonl" \\
        | grep -E --line-buffered 'agent\\.reloaded|PATTERN|error|detached' \\
        | python3 -u scripts/monitor-throttle.py 10

`--selftest` runs the behaviour checks and needs no input.
"""

from __future__ import annotations

import argparse
import os
import re
import select
import sys
import time

#: Below this, a stream is fast enough to flood the conversation, so the guard
#: in `.cupcake/policies/claude/monitor_rate_limit.rego` refuses it. Kept here
#: as well so a direct run reports the same number the policy enforces.
MIN_INTERVAL_SECONDS = 10.0

#: Long enough to cover an agent's load-time one-shots, including the ones it
#: schedules a few seconds out; short enough that a real event after the edit is
#: not hidden for long.
DEFAULT_RELOAD_QUIET_SECONDS = 15.0

#: The substring `scripts/ds2-frida-watch.py` writes before every agent load. The
#: watcher's selftest checks that its marker record contains exactly this.
RELOAD_MARKER = '"agent.reloaded"'

#: What makes a suppressed line worth one summary.
FAILURE_WORDS = re.compile(r"error|Error|ERROR|exception|Exception|detached|Traceback")

#: How much of the last failing line the summary quotes.
SUMMARY_LINE_CHARS = 300


class Throttle:
    """The interval gate plus the reload quiet window, driven by `feed` and `tick`.

    Split out from `main` so `--selftest` can drive it with a fake clock and a
    list sink rather than real time and a real pipe.
    """

    def __init__(self, interval, emit, now=time.monotonic, reload_quiet=DEFAULT_RELOAD_QUIET_SECONDS):
        self.interval = interval
        self.emit = emit
        self.now = now
        self.reload_quiet = reload_quiet
        self.window_start = None
        self.suppressed = 0
        self.pending = None
        self.quiet_until = None
        self.quiet_dropped = 0
        self.quiet_failures = 0
        self.quiet_last_failure = None

    # --- the interval gate, unchanged from the original filter ---------------

    def _gate(self, line, stamp):
        if self.window_start is not None and stamp - self.window_start < self.interval:
            self.suppressed += 1
            self.pending = line
            return
        self.window_start = stamp
        if self.suppressed:
            self.emit(f"{line}  [+{self.suppressed} suppressed in the last {self.interval:g}s]")
            self.suppressed = 0
            self.pending = None
        else:
            self.emit(line)

    # --- the reload quiet window ---------------------------------------------

    def deadline(self):
        """When `tick` next has work to do, or `None` when nothing is waiting on the clock."""
        return self.quiet_until

    def _close_quiet(self, stamp):
        dropped, failures, last = self.quiet_dropped, self.quiet_failures, self.quiet_last_failure
        self.quiet_until = None
        self.quiet_dropped = 0
        self.quiet_failures = 0
        self.quiet_last_failure = None
        if dropped and failures:
            quoted = last[:SUMMARY_LINE_CHARS]
            self._gate(
                f"[reload: {dropped} lines suppressed, {failures} matched a failure word; last: {quoted}]",
                stamp,
            )

    def tick(self):
        stamp = self.now()
        if self.quiet_until is not None and stamp >= self.quiet_until:
            self._close_quiet(stamp)

    def feed(self, line):
        stamp = self.now()
        if self.quiet_until is not None and stamp >= self.quiet_until:
            self._close_quiet(stamp)
        if self.reload_quiet > 0 and RELOAD_MARKER in line:
            # The marker opens or restarts the window and is never shown itself:
            # one notification per edit is the noise this exists to remove.
            self.quiet_until = stamp + self.reload_quiet
            return
        if self.quiet_until is not None:
            self.quiet_dropped += 1
            if FAILURE_WORDS.search(line):
                self.quiet_failures += 1
                self.quiet_last_failure = line
            return
        self._gate(line, stamp)

    def close(self):
        # A stream that ends inside a quiet window still owes its summary.
        if self.quiet_until is not None:
            self._close_quiet(self.now())
        # A stream that ends mid-window still owes its last line: dropping it would
        # lose the final state, which on a run log is the one that says how it ended.
        #
        # The count excludes `pending` itself. Mid-stream the suppressed lines are all
        # genuinely lost and a different line opens the new window, so the raw count is
        # right there; here the pending line is the one being shown, and counting it as
        # suppressed reports one loss that did not happen.
        if self.pending is not None:
            lost = self.suppressed - 1
            if lost > 0:
                self.emit(f"{self.pending}  [+{lost} suppressed in the last {self.interval:g}s]")
            else:
                self.emit(self.pending)


def throttle(lines, interval, emit, now=time.monotonic, reload_quiet=DEFAULT_RELOAD_QUIET_SECONDS):
    """Run `lines` through a `Throttle`. A `None` item is a clock tick with no line."""
    gate = Throttle(interval, emit, now=now, reload_quiet=reload_quiet)
    for line in lines:
        if line is None:
            gate.tick()
        else:
            gate.feed(line)
    gate.close()


def read_fd(fd, gate):
    """Feed `gate` the lines on `fd`, waking at its deadline when no line arrives.

    `select` on the raw descriptor rather than iterating `sys.stdin`: a buffered
    reader can hold lines `select` cannot see, and a plain `for line in stdin`
    blocks past the end of a quiet window, so its summary would wait for the next
    line -- which after a failed reload may never come.
    """
    buffered = b""
    while True:
        deadline = gate.deadline()
        timeout = None if deadline is None else max(0.0, deadline - gate.now())
        ready, _, _ = select.select([fd], [], [], timeout)
        if not ready:
            gate.tick()
            continue
        chunk = os.read(fd, 65536)
        if not chunk:
            break
        buffered += chunk
        *complete, buffered = buffered.split(b"\n")
        for raw in complete:
            gate.feed(raw.decode("utf-8", errors="replace").rstrip("\r"))
    if buffered:
        gate.feed(buffered.decode("utf-8", errors="replace").rstrip("\r"))
    gate.close()


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

    marker = '{"family": "frida-watch", "tag": "agent.reloaded", "agent": "x.js", "load": 1}'
    give_error = '{"message": {"payload": {"tag": "give", "error": "no slot"}}}'

    # The 2026-11-02 shape: a reload whose one-shots send an `error` line. The
    # marker and the burst are all dropped, and ONE summary arrives when the
    # window closes on the clock (the None item is a tick with no line).
    clock = iter([0.0, 100.0, 101.0, 102.0, 116.0])
    out = []
    throttle(["start", marker, give_error, '{"tag": "hooked"}', None], 10.0, out.append, now=lambda: next(clock))
    check(
        "a reload with a failure line collapses to one summary at the window's end",
        out,
        ["start", f"[reload: 2 lines suppressed, 1 matched a failure word; last: {give_error}]"],
    )

    # A reload whose one-shots are only the usual chatter emits nothing at all.
    clock = iter([0.0, 1.0, 2.0, 16.0])
    out = []
    throttle([marker, '{"tag": "hooked"}', '{"tag": "mode"}', None], 10.0, out.append, now=lambda: next(clock))
    check("a reload with no failure line emits nothing", out, [])

    # A bare marker with nothing behind it emits nothing either.
    clock = iter([0.0, 16.0])
    out = []
    throttle([marker, None], 10.0, out.append, now=lambda: next(clock))
    check("a reload that produced no lines emits nothing", out, [])

    # After the window, lines flow through the interval gate again.
    clock = iter([0.0, 1.0, 20.0, 21.0])
    out = []
    throttle([marker, '{"tag": "hooked"}', "after", "after-2"], 10.0, out.append, now=lambda: next(clock))
    check(
        "the window ends and the next line is emitted",
        out,
        ["after", "after-2"],
    )

    # A second marker restarts the window, so rapid edits are one quiet stretch.
    clock = iter([0.0, 10.0, 18.0, 22.0, 40.0])
    out = []
    throttle([marker, marker, give_error, "Traceback (most recent call last)", None], 10.0, out.append,
             now=lambda: next(clock))
    check(
        "a marker inside the window restarts it and the counts accumulate",
        out,
        ["[reload: 2 lines suppressed, 2 matched a failure word; last: Traceback (most recent call last)]"],
    )

    # A stream that ends inside the window still reports the failure.
    clock = iter([0.0, 1.0, 2.0])
    out = []
    throttle([marker, "session detached: process-terminated"], 10.0, out.append, now=lambda: next(clock))
    check(
        "a stream ending inside the window still emits its summary",
        out,
        ["[reload: 1 lines suppressed, 1 matched a failure word; last: session detached: process-terminated]"],
    )

    # 0 disables the window: the marker is an ordinary line.
    clock = iter([0.0, 20.0])
    out = []
    throttle([marker, give_error], 10.0, out.append, now=lambda: next(clock), reload_quiet=0)
    check("--reload-quiet 0 passes the marker and its burst through", out, [marker, give_error])

    # A closed window ends on the clock even when no line follows, through the
    # real descriptor reader: the summary is written while the pipe is still open.
    import threading

    read_end, write_end = os.pipe()
    os.write(write_end, (marker + "\n" + give_error + "\n").encode())
    out = []
    emitted = threading.Event()

    def record(line):
        out.append(line)
        emitted.set()

    gate = Throttle(10.0, record, reload_quiet=0.2)
    reader = threading.Thread(target=read_fd, args=(read_end, gate), daemon=True)
    reader.start()
    # Waits on the emit itself, bounded so a broken reader fails the check
    # instead of hanging the gate. The write end is still open, so only the
    # deadline can have produced the line.
    emitted.wait(5.0)
    seen_open = list(out)
    os.close(write_end)
    reader.join(5.0)
    os.close(read_end)
    check(
        "the descriptor reader emits the summary at the deadline, before more input or EOF",
        seen_open,
        [f"[reload: 1 lines suppressed, 1 matched a failure word; last: {give_error}]"],
    )
    check("the descriptor reader returns at EOF", reader.is_alive(), False)

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    print("monitor-throttle selftest: 13 checks passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "interval",
        nargs="?",
        type=float,
        default=MIN_INTERVAL_SECONDS,
        help=f"seconds between emitted lines (default and minimum {MIN_INTERVAL_SECONDS:g})",
    )
    parser.add_argument(
        "--reload-quiet",
        type=float,
        default=DEFAULT_RELOAD_QUIET_SECONDS,
        metavar="SECS",
        help=(
            f"after a line containing {RELOAD_MARKER}, drop every line for SECS seconds and then emit "
            f"one summary only if a dropped line matched a failure word "
            f"(default {DEFAULT_RELOAD_QUIET_SECONDS:g}; 0 disables)"
        ),
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
    if args.reload_quiet < 0:
        print("monitor-throttle: --reload-quiet must be 0 or more", file=sys.stderr)
        return 2

    def emit(line: str) -> None:
        sys.stdout.write(line + "\n")
        sys.stdout.flush()

    read_fd(sys.stdin.fileno(), Throttle(args.interval, emit, reload_quiet=args.reload_quiet))
    return 0


if __name__ == "__main__":
    sys.exit(main())
