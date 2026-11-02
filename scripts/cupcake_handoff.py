#!/usr/bin/env python3
"""One definition of "did this closing message hand the turn to the user, or name a blocker".

Two Stop guards ask that question of the same closing message:

  * `.cupcake/signals/last_assistant_described_next_step.sh` (DS2-MODS-NO-DESCRIBED-NEXT-STEP): a
    named next step is excused when the turn hands off or names a blocker;
  * `scripts/cupcake_frida_attached.py` (DS2-MODS-NO-STOP-WHILE-FRIDA-ATTACHED): a turn end while the
    game is up with this session's Frida watcher attached is excused on the same two grounds.

The regexes were written, and measured on replays, inside the first signal's heredoc. They moved here
unchanged so the two guards cannot drift into disagreeing about whether one sentence asked the user
something. The measurement notes moved with them.

`scrub` and `prose_only` are the two views of a message the exemptions read: `scrub` drops code and
quotations, `prose_only` also drops markdown table rows.

Import contract (the same as `cupcake_turn_scan`): every consumer fails open, emitting nothing, if
this module cannot be imported.
"""
from __future__ import annotations

import re

from cupcake_turn_scan import blocked_on_user


def scrub(text):
    """Drop fenced code, backticked spans and double-quoted spans before prose matching.

    Quoting the ban is not committing it. Single quotes are left alone: the phrases carry
    apostrophes. This is also what lets a report about this guard describe its own triggers without
    halting the turn that describes them, provided the triggers are written as quotations.
    """
    text = re.sub(r"```.*?```", " ", text, flags=re.DOTALL)
    text = re.sub(r"`[^`]*`", " ", text)
    return re.sub(r'"[^"]*"', " ", text)


def prose_only(text):
    """`scrub`, and then drop markdown table rows.

    A table cell is a status column, not a sentence, and the pipes between cells make one row read
    as a run-on clause. Measured on the replay: the row `| the rebind | vanilla | just changed,
    builds clean, not yet run |` was quoted back as a next step described and abandoned, and the
    clause the agent would have been shown was three unrelated cells joined by slashes.
    `cupcake_turn_scan.prose_paragraphs` already draws this line for the wall-of-text rule; this is
    the same line drawn for one row at a time. Applied where a match is hunted, and to the two
    exemptions a table cell used to fake: a bare `?` (a row label for an open question is not a
    question to the user) and the bare words "blocker"/"blocked" (a row naming an unknown is not an
    external dependency). Everything else the exemptions read -- the named handoff phrases, and a
    blocker that names sudo, approval, a live game, a guard or a merge -- still reads the whole
    message, so a real blocker stated inside a table still exempts. Measured 2026-11-02; see
    `hands_off` and `names_blocker` below.
    """
    return "\n".join(
        line for line in scrub(text).splitlines() if not line.lstrip().startswith("|")
    )


# --- the ball was handed to the user ------------------------------------------------------------
# A question mark anywhere in the closing PROSE exempts the turn. That is broad on purpose: a genuine
# fork is exactly what the agent is allowed to stop for, and this rule must never gag one.
#
# A question mark inside a markdown table row does not count, and that is a narrowing measured on a
# real miss (2026-11-02). A status table whose first column is a list of open questions --
# "| Can the soapstone be used offline? | ... | **open** |" -- is a set of row labels for unknowns
# the agent resolves itself, not a question put to the user, and its `?` waved through "The next step
# is to find the code that reads ItemUsageParam" with nothing begun. The named handoff phrases below
# still read the whole message, table cells included: "your call" or "want me to" in a cell is
# addressed to the user wherever it sits.
QUESTION_RE = re.compile(r"\?")
HANDOFF_RE = re.compile(
    r"\bonce\s+(?:you|your|the\s+user)\b"
    r"|\bwhen\s+(?:you|your|the\s+user)\b"
    r"|\bafter\s+(?:you|your|the\s+user)\b"
    r"|\bif\s+(?:you|your|the\s+user)\b"
    r"|\byou'?(?:ll|d)\s+(?:need|have)\s+to\b|\byou\s+(?:need|have)\s+to\b"
    r"|\bneed\s+you\s+to\b|\bneed\s+the\s+user\s+to\b"
    r"|\blet\s+me\s+know\b|\btell\s+me\b|\bping\s+me\b"
    r"|\byour\s+call\b|\bsay\s+the\s+word\b|\bup\s+to\s+you\b"
    r"|\bwant\s+me\s+to\b|\bshall\s+i\b|\bshould\s+i\b"
    r"|\bwould\s+you\s+(?:like|prefer|rather)\b"
    r"|\bmy\s+recommendation\b|\bi\s+recommend\b"
    r"|\bnext\s+(?:session|turn)\b|\bre-?initiate\b|\bre-?ask\b"
    # A stated dependency on a user action plus a commitment to act on its result: "invade now and
    # I'll read the log". Lifted verbatim from `BLOCKED_RES` in
    # `.cupcake/signals/last_assistant_stall_on_friction.sh` so the two guards agree about the same
    # sentence rather than each inventing a rule for it.
    r"|\b(?:and|then|once|after|when)\b[^.\n]{0,60}\bi'?ll\b"
    r"|\bonly\s+you\s+can\b"
    # An outcome that can be settled only by the user looking. `AGENTS.md` names this as one of the
    # four legitimate ways to end a turn -- "an in-game observation you have no oracle for" -- and
    # the closing shape that says so is the passive one: "launched, but not yet confirmed by you or
    # by any oracle, because there is no telemetry proving the close fired". It was a halt on the
    # replay before this line existed. `by any oracle` is not enough on its own and is deliberately
    # absent: naming the missing instrument is the agent's cue to build one, and the exemption is
    # for the user's eyes, not for the absence.
    r"|\b(?:confirmed|verified|seen|observed|judged|decided)\s+(?:only\s+)?by\s+(?:you|the\s+user)\b"
    r"|\bby\s+(?:you|the\s+user)\s+or\s+by\b",
    re.IGNORECASE,
)

# --- a genuine blocker was stated ---------------------------------------------------------------
# Reporting a blocker is required behaviour, and a step waiting on approval or on something landing
# is not a step that could have been begun. Both exempt.
#
# The bare words -- "blocked", "blocker", "cannot proceed", "can't run" -- count only in prose. In a
# table cell they label a row, and the row they label is as often an unknown the agent can go and
# settle as a dependency outside it: "| Can the soapstone be used offline? | ... | **open**; this is
# the first blocker |" named the very question the closer then declined to start on, and it exempted
# that closer (2026-11-02). A blocker stated inside a table still exempts when it names what is
# outside the agent's reach -- sudo, approval, a live game, a guard, a merge, the user --
# because every other alternative of `BLOCKER_RE`, and `BLOCKED_ON_RE`, still reads the whole
# message.
BARE_BLOCKER_RE = re.compile(
    r"\bblocked\b|\bblocker\b|\bcannot\s+proceed\b"
    r"|\bcan(?:no|')?t\s+(?:proceed|run|test|verify|build|launch|start)\b",
    re.IGNORECASE,
)
BLOCKED_ON_RE = re.compile(
    r"\bblock(?:ed|er|ing)\b[^|\n]{0,60}?"
    r"\b(?:sudo|root|approval|credentials|network|you|your|the\s+user|a\s+live|the\s+game"
    r"|guard|policy|cupcake|opa|merge|merged|review|ci|checks?)\b",
    re.IGNORECASE,
)
BLOCKER_RE = re.compile(
    r"\brequires?\s+(?:sudo|root|approval|a\s+game|a\s+live|the\s+game|physical|credentials|network)\b"
    r"|\bneeds?\s+(?:sudo|root|approval|a\s+game|a\s+live|the\s+game\s+running)\b"
    r"|\bnot\s+(?:available|installed|possible|permitted|allowed|reachable)\b"
    r"|\b(?:guard|policy|cupcake|opa)\s+(?:denied|blocked|refused|refuses)\b"
    r"|\bdestructive\b|\birreversible\b"
    r"|\b(?:when|once|after)\s+(?:they|it|those|these|the\s+\w+)\s+"
    r"(?:land|lands|merge|merges|complete|completes|finish|finishes|is\s+merged|are\s+merged)\b",
    re.IGNORECASE,
)


def hands_off(text: str) -> bool:
    """The closing message asks the user something, or names something only the user can do."""
    message = scrub(text)
    return (
        bool(QUESTION_RE.search(prose_only(text)))
        or bool(HANDOFF_RE.search(message))
        or blocked_on_user(message)
    )


def names_blocker(text: str) -> bool:
    """The closing message names a genuine blocker: sudo, approval, a guard, a destructive step."""
    message = scrub(text)
    return (
        bool(BARE_BLOCKER_RE.search(prose_only(text)))
        or bool(BLOCKED_ON_RE.search(message))
        or bool(BLOCKER_RE.search(message))
    )
