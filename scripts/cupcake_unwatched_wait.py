#!/usr/bin/env python3
"""Decide whether the last turn ended by saying it is waiting while nothing is watching.

User, 2026-11-02: "Write a rego policy for when you say "I'm waiting" and you don't have a monitor".
The instance: the agent ended turns on "Blocked on #368's checks" and similar, and twice its own wait
was an `until ... do sleep` loop with no deadline. A wait is only real if something will wake the
agent when the thing waited on happens: a backgrounded command (its exit re-invokes the agent), an
async subagent (its completion notifies), or a Monitor. Without one, "I'm waiting for CI" ends the
turn and nothing ever comes back -- the user has to notice and ask.

WHAT SPEAKS: the turn's CLOSING message (its last run of prose, after its last tool call) says it is
waiting or blocked ("I'm waiting", "waiting for/on/until", "blocked on", "once it lands", "until it
finishes", "I'll check back"), AND `cupcake_turn_scan.live_background_work` finds nothing running.
Prints `UNWATCHED:<phrase>`; .cupcake/policies/claude/no_unwatched_wait.rego halts the Stop on it.

WHAT DOES NOT:
  * a wait on the user ("waiting on you to review", "blocked on your answer"): the user's reply is
    what wakes the agent, so no watcher is owed (`cupcake_turn_scan.blocked_on_user`);
  * any live backgrounded command, async subagent, Monitor or SendMessage;
  * prose earlier in the turn: a mid-turn "waiting on the build" followed by a tool call that then
    reads the build is narration of work that happened, not where the turn stopped;
  * quoted text and code spans, so quoting the rule (this file, a review) is not saying it.

Run `--selftest` to check the decision table. Fails open (no output) on any error.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cupcake_turn_scan as scan  # noqa: E402

WAIT_RE = re.compile(
    r"\b(?:i'?m|i\s+am)\s+(?:still\s+)?(?:waiting|blocked)\b"
    r"|\bwaiting\s+(?:for|on|until)\b"
    r"|\bblocked\s+on\b"
    r"|\b(?:once|when|until)\s+(?:it|that|they|the\s+\w+(?:'s)?(?:\s+\w+)?)\s+"
    r"(?:lands|land|finishes|finish|completes|complete|returns|return|is\s+done|are\s+done|reports|report)\b"
    r"|\bi'?ll\s+(?:check|look)\s+(?:back|again)\b"
    r"|\bstill\s+(?:running|pending|in\s+progress)\b",
    re.IGNORECASE,
)
QUOTED_RE = re.compile(r'"[^"\n]*"|“[^”\n]*”|`[^`\n]*`')


def closing_text(turn: scan.Turn) -> str:
    """The prose after the turn's last tool call: what the turn actually ended on."""
    out: list[str] = []
    for kind, value in reversed(turn.blocks):
        if kind == "tool":
            break
        out.append(value)
    return "\n".join(reversed(out))


def wait_phrase(text: str) -> str | None:
    """The wait the text announces, unless it is a wait on the user."""
    plain = QUOTED_RE.sub(" ", re.sub(r"```.*?```", " ", text, flags=re.DOTALL))
    if scan.blocked_on_user(plain):
        return None
    m = WAIT_RE.search(plain)
    return m.group(0) if m else None


def decide(closing: str, live: bool) -> str | None:
    """`UNWATCHED:<phrase>` when the closing prose waits and nothing is watching, else None."""
    if live:
        return None
    phrase = wait_phrase(closing)
    return f"UNWATCHED:{phrase}" if phrase else None


def gather() -> str | None:
    path = scan.latest_transcript()
    if not path:
        return None
    events = scan.load_events(path)
    turn = scan.last_text_turn(scan.split_turns(events))
    if turn is None:
        return None
    return decide(closing_text(turn), bool(scan.live_background_work(events)))


def selftest() -> int:
    bad = 0

    def check(what: str, got, want) -> None:
        nonlocal bad
        ok = got == want
        bad += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {what}" + ("" if ok else f": got {got!r}, want {want!r}"))

    speaks = [
        "I'm waiting for CI on #368.",
        "Blocked on #368's checks finishing.",
        "Still running: the fixture export. I'll check back.",
        "Waiting until the build finishes.",
        "I'll report once the run lands.",
        "CI is still pending on host-tests.",
    ]
    for text in speaks:
        check(f"speaks with nothing live: {text!r}", decide(text, live=False) is not None, True)
        check(f"silent with work live: {text!r}", decide(text, live=True), None)
    silent = [
        "I'm waiting on you to review and merge #368.",
        "Blocked on your answer: which branch?",
        "Waiting for you to run the sudo command in the new tab.",
        "The fix is in #368; CI passed and it is ready to merge.",
        'The rule fires on "I\'m waiting" when nothing is live.',
        "The guard matches `waiting for` in prose.",
    ]
    for text in silent:
        check(f"silent: {text!r}", decide(text, live=False), None)

    turn = scan.Turn()
    turn.blocks = [("text", "Waiting on the build."), ("tool", {"name": "Bash"}), ("text", "Done: it built.")]
    check("only the prose after the last tool call counts", closing_text(turn), "Done: it built.")
    turn.blocks = [("tool", {"name": "Bash"}), ("text", "Blocked on CI."), ("text", "Nothing else.")]
    check("the closing run joins its text blocks", closing_text(turn), "Blocked on CI.\nNothing else.")
    print("selftest: OK" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    try:
        line = gather()
    except Exception:
        return 0
    if line:
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
