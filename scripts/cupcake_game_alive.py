#!/usr/bin/env python3
"""Does the turn's closing prose claim DARK SOULS II is running, and did anything check?

WHY THIS EXISTS (user directive 2026-09-23, their words): "Wait. The game is up? Prove it. If you
can't prove it, rego policy for bad behavior."

THE INSTANCE, from the turn immediately before that directive. The agent had launched the game,
read the launcher's attach block, then over several turns kept telling the user the game was up and
to press a row in it:

    "The game is up on the new DLL with the rows armed -- press Load Character from File."

`scripts/ds2-teardown.py --status` answered `nothing running`. The process had crashed on its own
minutes earlier. Every one of those turns was the agent reporting a live process from memory of
having started one, which is a claim about the present tense made out of a past observation.

WHAT SEPARATES A CLAIM THAT EARNS ITS PLACE. The launcher's own success block is evidence that the
DLL attached -- at the moment it was printed. It is not evidence the process is alive now, and the
gap between the two is exactly where this failure lives: a game that attached, ran, and died leaves
the attach block on screen and nothing on the system. So the only thing that licenses the
present-tense claim is a liveness check made IN THE SAME TURN.

WHAT COUNTS AS A CHECK, and all three are commands the agent ran in that turn:
  * `ds2-teardown.py --status`, whose whole job is to classify the session's processes;
  * `ds2-run.py` itself, which launches and then gates on the DLL's own log line -- a turn that
    launched has observed the process it is talking about;
  * a `pgrep -x DarkSoulsII.exe`, the direct question, noting it answers "nothing" through a PID
    namespace and so is the weakest of the three.

The lexicon is deliberately about the PRESENT TENSE of a process. "I launched it" is a claim about
the past and is not caught; "it is running" is a claim about now and is.
"""

from __future__ import annotations

import re

#: Phrases that assert DARK SOULS II is alive RIGHT NOW. Present tense only: a past-tense report of
#: a launch ("launched it", "the launcher attached") is a claim about a moment that did happen, and
#: gagging it would cost the user the one honest thing the launcher produces.
ALIVE_PATTERNS = (
    r"\bthe game is (?:up|running|alive|live)\b",
    r"\b(?:it|game|ds2) is (?:still )?(?:up|running) (?:on|with)\b",
    r"\brunning (?:on|with) the (?:new|rebuilt|current) dll\b",
    r"\b(?:game|session) is up\b",
    r"\bthe game(?:'s| is) (?:still )?(?:up|alive|running)\b",
    r"\bis (?:up )?and (?:idle|waiting|running)\b",
    r"\bpress \*?\*?load character from file\*?\*?\b",
    r"\bgo (?:ahead and )?press the row\b",
)

#: Commands whose output answers "is that process alive". Matched against the turn's tool calls.
LIVENESS_COMMANDS = (
    "ds2-teardown.py --status",
    "ds2-teardown.py",
    "ds2-run.py",
    "pgrep -x DarkSoulsII.exe",
    "pgrep -x darksoulsii.exe",
)


def claims_alive(text: str) -> str | None:
    """The first phrase in `text` that asserts the game is running now, or None."""
    lowered = text.lower()
    for pattern in ALIVE_PATTERNS:
        found = re.search(pattern, lowered)
        if found:
            return found.group(0)
    return None


def checked_liveness(commands: list[str]) -> bool:
    """Whether any command in this turn could have observed whether the process is alive."""
    for command in commands:
        lowered = command.lower()
        for probe in LIVENESS_COMMANDS:
            if probe.lower() in lowered:
                return True
    return False


def first_offence(text: str, commands: list[str]) -> str | None:
    """The offending phrase when the turn claims a live game and checked nothing."""
    phrase = claims_alive(text)
    if phrase is None:
        return None
    if checked_liveness(commands):
        return None
    return phrase
