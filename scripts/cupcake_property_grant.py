#!/usr/bin/env python3
"""Does the turn's closing prose hand the user permission over something that was already theirs?

WHY THIS EXISTS (user directive 2026-09-23). The instance, verbatim, from a turn that had just
finished building a DLL and had not relaunched the game:

    "the DLL is built but I have not relaunched, so your session is still yours to end when you
    want the next run."

The user's objection: the sentence hands the user a right they already hold. Nobody granted the
agent authority over when the user's own session ends, so there is nothing for the agent to hand
back -- the clause is ceremony wearing the costume of restraint. What belongs in that sentence is
the factual half only: the DLL is built, the agent has not relaunched. No clause about what is
theirs, because the agent was never in a position to say otherwise.

THE SHAPE THIS CATCHES, in general. A closing sentence that casts the agent as the one bestowing
control over the user's own property -- their session, their save, their decision, their time --
back to them. The tell is idiomatic: "yours to end", "up to you", "feel free to", "whenever you
want" are all stock phrases for granting permission, and every one of them presupposes the speaker
held the thing being granted. The agent never did.

WHAT SEPARATES THIS FROM ORDINARY POSSESSIVE PROSE. "Your save file" and "your own folder" name
whose property something is; they do not stage a handover. The lexicon below is built from fixed
idioms rather than a bare `your`/`yours` match for exactly that reason -- see GRANT_PATTERNS.

WHAT SEPARATES THIS FROM A QUESTION. "Do you want me to relaunch?" asks; it does not grant. None
of the patterns below match a bare "do/would you want" question, so the two shapes never collide.

The lexicon here is deliberately not exhaustive -- new stock phrasings for the same ceremony should
be added to GRANT_PATTERNS as they are found, the same way scripts/cupcake_game_alive.py expects its
own list to grow.
"""

from __future__ import annotations

import re

#: Stock phrases that stage the agent handing the user control over something that was already the
#: user's. Anchored on the idiom, not on a bare "your"/"yours", so ordinary possessive prose ("your
#: save file", "your own folder") never matches -- only the fixed handover phrasing does.
GRANT_PATTERNS = (
    r"\byours to \w+\b",  # "yours to end", "yours to keep", "yours to close", ...
    r"\b(?:is|are)\s+still\s+yours\b",  # "is still yours" / "are still yours"
    r"\bremains?\s+yours\b",  # "remains yours"
    r"\bthe choice is yours\b",
    r"\bit(?:'|’)?s your choice\b",
    r"\byour call\b",
    r"\byour decision to make\b",
    r"\byour prerogative\b",
    r"\bup to you\b",
    r"\bentirely up to you\b",
    r"\bwhenever you want\b",
    r"\bwhenever (?:works|suits) (?:for )?you\b",
    r"\bwhen you want the next\b",
    r"\bif you want me to\b",
    r"\bfeel free to\b",
    r"\bas you see fit\b",
    r"\bat your leisure\b",
    r"\btake your time\b",
    r"\b(?:i(?:'|’)?ll|i will) leave (?:that|it|this) to you\b",
    r"\bleaving (?:that|it|this) to you\b",
)


def _outside_spans(text: str, delimiter: str) -> str:
    """`text` with every delimited span removed, by parity rather than by rewriting.

    Split on the delimiter and keep the even-indexed pieces -- those are the ones outside it. An odd
    number of delimiters means the parity read says nothing, so the text comes back untouched and
    gets judged: half a quotation must not become a way to hide a claim.
    """
    parts = text.split(delimiter)
    if len(parts) % 2 == 0:
        return text
    return "\n".join(part for index, part in enumerate(parts) if index % 2 == 0)


def quotable(text: str) -> str:
    """`text` with backticked code and quoted spans taken out before it is judged.

    Quoting the offence is not committing it, and this carve-out exists for the same reason
    `scripts/cupcake_game_alive.py`'s does: explaining what this guard forbids requires typing the
    forbidden sentence somewhere, and a rule that cannot tell the mention from the use gags its own
    explanation. Copied from that module rather than rewritten, so the two guards cannot drift into
    disagreeing about what a quote looks like.
    """
    for delimiter in ("`", '"', "'"):
        text = _outside_spans(text, delimiter)
    return text


def first_offence(text: str) -> str | None:
    """The first phrase in `text` that hands the user permission over their own property, or None."""
    lowered = quotable(text).lower()
    for pattern in GRANT_PATTERNS:
        found = re.search(pattern, lowered)
        if found:
            return found.group(0)
    return None
