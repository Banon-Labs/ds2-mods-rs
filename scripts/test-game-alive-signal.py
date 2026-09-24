#!/usr/bin/env python3
"""Pin scripts/cupcake_game_alive.py: which sentences claim a live game, and what excuses one.

`opa test` pins the policy's half -- a spoken signal halts. This pins the half that decides whether
the signal speaks at all, which is where every false positive and every miss of this guard lives.

Run: python3 scripts/test-game-alive-signal.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import cupcake_game_alive as alive  # noqa: E402

FAILURES: list[str] = []


def check(ok: bool, what: str) -> None:
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        FAILURES.append(what)


# --- the sentences that started this -------------------------------------------------------------
# Verbatim from the turns the user challenged with "Wait. The game is up? Prove it."
THE_INSTANCE = "The game is up on the new DLL with the rows armed -- press **Load Character from File**."

check(
    alive.claims_alive(THE_INSTANCE) is not None,
    "the sentence that prompted this guard is recognised as a live-game claim",
)
check(
    alive.first_offence(THE_INSTANCE, []) is not None,
    "and with no command in the turn it is an offence",
)
check(
    alive.first_offence(THE_INSTANCE, ["timeout 25 python3 scripts/ds2-teardown.py --status"]) is None,
    "a --status check in the same turn excuses it -- looking is the whole ask",
)
check(
    alive.first_offence(THE_INSTANCE, ["python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py --continue-slot 0"]) is None,
    "a launch in the same turn excuses it: the turn observed the process it is talking about",
)
check(
    alive.first_offence(THE_INSTANCE, ["pgrep -x DarkSoulsII.exe"]) is None,
    "and so does asking outright, weak as that answer is through a PID namespace",
)

# --- the tense split, which is the entire rule ---------------------------------------------------
# A claim about a moment that happened is provable and is not touched. A claim about now is not.
PAST_TENSE = (
    "Attached and autoloading slot 0, proven by the DLL's own log line.",
    "The launcher printed its success block after reading `ds2-loader: arxan`.",
    "I launched it with --continue-slot 0 and the block came back.",
    "That did not launch -- the launcher's block is a FAILED one.",
)
for sentence in PAST_TENSE:
    check(
        alive.claims_alive(sentence) is None,
        f"past tense is not a live-game claim: {sentence[:54]!r}",
    )

PRESENT_TENSE = (
    "The game is up.",
    "The game is running on the new DLL.",
    "Running with the rebuilt DLL -- press the row.",
    "The game's still up, press **Load Character from File** and pick the archive.",
)
for sentence in PRESENT_TENSE:
    check(
        alive.claims_alive(sentence) is not None,
        f"present tense is a live-game claim: {sentence[:54]!r}",
    )

# --- the instruction shape ------------------------------------------------------------------------
# "Press the row" is a live-game claim wearing an imperative: it only makes sense if a game is on
# screen, and it is the sentence that actually sends the user to look at nothing.
check(
    alive.claims_alive("Press **Load Character from File** and pick the archive.") is not None,
    "telling the user to press the row is itself a claim that a game is there to press it in",
)
check(
    alive.claims_alive("Go ahead and press the row when you're ready.") is not None,
    "and so is the softer spelling of the same instruction",
)

# --- false positives worth keeping out -------------------------------------------------------------
CLEAN = (
    "The row is registered and the caption is armed in the config.",
    "`ds2-teardown.py --status` answered `nothing running`.",
    "The build is running; I will report the exit code.",
    "The test suite is up to 161 checks.",
    "The picker opens in Z:\\home\\banon\\Downloads, measured in the log.",
)
for sentence in CLEAN:
    check(
        alive.claims_alive(sentence) is None,
        f"not a live-game claim: {sentence[:54]!r}",
    )

# --- the mention/use split ------------------------------------------------------------------------
# The guard's first live catch was the agent QUOTING the banned sentence while reporting the guard
# working. A rule that cannot tell naming the offence from committing it gags its own explanation.
QUOTED = (
    'You saw it halt me on "The game is up".',
    "The reason quotes the phrase: 'the game is running'.",
    "It fires on `the game is up` and on nothing else.",
    'The correction reads: "You told the user the game is running".',
)
for sentence in QUOTED:
    check(
        alive.claims_alive(sentence) is None,
        f"quoting the offence is not committing it: {sentence[:54]!r}",
    )

# And the bare claim still lands when it is not inside quotes -- the carve-out must not swallow it.
check(
    alive.claims_alive('The build is done. The game is up. Press the row.') is not None,
    "an unquoted claim beside quoted text is still a claim",
)

# An unbalanced quote must not become a hiding place: parity says nothing, so the text is judged.
check(
    alive.claims_alive('He said "the game is up and I believe him') is not None,
    "half a quotation does not exempt the sentence it opens",
)

# A turn that ran a liveness check and reported a DEAD game must never be gagged -- that sentence is
# the one this guard exists to produce.
check(
    alive.first_offence(
        "The game is not up -- `nothing running`.",
        ["timeout 25 python3 scripts/ds2-teardown.py --status"],
    )
    is None,
    "reporting a dead session after checking is always clean",
)

print()
if FAILURES:
    print(f"[test-game-alive-signal] FAILED ({len(FAILURES)})")
    for failure in FAILURES:
        print(f"  - {failure}")
    raise SystemExit(1)
print(f"[test-game-alive-signal] ok ({len(THE_INSTANCE) and 'the instance'} + tense split + false positives)")
