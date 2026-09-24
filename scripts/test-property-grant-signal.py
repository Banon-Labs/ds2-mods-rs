#!/usr/bin/env python3
"""Pin scripts/cupcake_property_grant.py: which sentences hand the user permission over their own
property, and what excuses one.

`opa test` pins the policy's half -- a spoken signal halts. This pins the half that decides whether
the signal speaks at all, which is where every false positive and every miss of this guard lives.

Run: python3 scripts/test-property-grant-signal.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import cupcake_property_grant as grant  # noqa: E402

FAILURES: list[str] = []


def check(ok: bool, what: str) -> None:
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        FAILURES.append(what)


# --- the sentence that started this ---------------------------------------------------------------
# Verbatim, 2026-09-23: a turn that had built a DLL and not relaunched the game.
THE_INSTANCE = (
    "the DLL is built but I have not relaunched, so your session is still yours to "
    "end when you want the next run."
)

check(
    grant.first_offence(THE_INSTANCE) is not None,
    "the sentence that prompted this guard is recognised as a property grant",
)

# --- the enumerated lexicon, one sentence per phrase ------------------------------------------------
FIRES = (
    "The session is yours to end whenever you like.",
    "The save file is yours to keep or delete.",
    "Your session is still yours.",
    "The rows are still yours, nothing has changed there.",
    "Ownership remains yours.",
    "The choice is yours.",
    "It's your choice whether to relaunch.",
    "That's your call.",
    "It's your decision to make, not mine.",
    "Relaunching is your prerogative.",
    "Whether to relaunch is up to you.",
    "The timing is entirely up to you.",
    "Relaunch whenever you want.",
    "Come back whenever suits you.",
    "I'll rebuild it when you want the next run.",
    "I can relaunch if you want me to.",
    "Feel free to close the session.",
    "Handle it as you see fit.",
    "Relaunch at your leisure.",
    "Take your time before the next run.",
    "I'll leave that to you.",
    "Leaving this to you for now.",
)
for sentence in FIRES:
    check(
        grant.first_offence(sentence) is not None,
        f"lexicon phrase is recognised as a property grant: {sentence[:54]!r}",
    )

# --- what must not fire -----------------------------------------------------------------------------
# A plain statement of agent action: no claim about what belongs to the user, nothing granted.
PLAIN_ACTION = (
    "I have not relaunched.",
    "The DLL is built and nothing is running.",
    "I built the DLL but did not relaunch the game.",
    "The build finished; the session was not restarted.",
)
for sentence in PLAIN_ACTION:
    check(
        grant.first_offence(sentence) is None,
        f"a plain statement of agent action is not a property grant: {sentence[:54]!r}",
    )

# A direct question. Asking is not granting -- the agent has not decided anything on the user's
# behalf, it is asking the user to decide, which is the opposite of the offence.
QUESTIONS = (
    "Do you want me to relaunch?",
    "Would you like me to relaunch now?",
    "Should I close the session or leave it running?",
)
for sentence in QUESTIONS:
    check(
        grant.first_offence(sentence) is None,
        f"a direct question is not a property grant: {sentence[:54]!r}",
    )

# Ordinary possessive prose. "your"/"yours" naming whose property something is, with no handover
# staged, must never fire -- the lexicon is anchored on the idiom for exactly this reason.
POSSESSIVE = (
    "Your container is still running.",
    "The bug is in your save file, not the loader.",
    "It writes into your own folder under Documents.",
    "Your build finished with no warnings.",
)
for sentence in POSSESSIVE:
    check(
        grant.first_offence(sentence) is None,
        f"ordinary possessive prose is not a property grant: {sentence[:54]!r}",
    )

# --- the mention/use split ---------------------------------------------------------------------------
# Quoting the offence to explain it must not commit it -- the guard's sibling was first caught on
# exactly this shape and the carve-out is copied from it verbatim for the same reason.
QUOTED = (
    'The line that started this was "so your session is still yours to end when you want the next run".',
    "The reason quotes the phrase: 'yours to end'.",
    "It fires on `feel free to` and on nothing else.",
    'The correction reads: "That sentence said the session was yours to end".',
)
for sentence in QUOTED:
    check(
        grant.first_offence(sentence) is None,
        f"quoting the offence is not committing it: {sentence[:54]!r}",
    )

# And the bare claim still lands when it is not inside quotes -- the carve-out must not swallow it.
check(
    grant.first_offence('The reason quotes "yours to end". Feel free to close it yourself.') is not None,
    "an unquoted grant beside quoted text is still a grant",
)

# An unbalanced quote must not become a hiding place: parity says nothing, so the text is judged.
check(
    grant.first_offence('He said "feel free to relaunch and I believe him') is not None,
    "half a quotation does not exempt the sentence it opens",
)

print()
if FAILURES:
    print(f"[test-property-grant-signal] FAILED ({len(FAILURES)})")
    for failure in FAILURES:
        print(f"  - {failure}")
    raise SystemExit(1)
print("[test-property-grant-signal] ok (the instance + lexicon + false positives)")
