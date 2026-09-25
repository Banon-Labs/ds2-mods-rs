#!/usr/bin/env python3
"""Classify a closing message that hands the user back a rule the user wrote.

# The failure

2026-09-23. A turn ended: "It stays draft -- undrafting is yours." The user had authored that rule
themselves, in a memory and in the global policy that denies `gh pr ready`. Their reply is the rule
this file makes executable:

    "If undrafting is mine, because I told you it was mine, does it need to be stated? Is there a
     way to guard against you telling me details that I provided to you through rego policies and
     agent instructions that I already knew?"

The sentence carried nothing. The user knew who undrafts, because they are the one who decided it;
reading it back spends their attention confirming their own instruction and crowds out the one
paragraph they actually read.

# What is flagged, and what deliberately is not

Only ownership restatement: a clause that assigns an action to the user, or denies it to the agent,
where the assignment comes from a standing rule the user authored. "Undrafting is yours." "I don't
push." "Leaving it draft for you."

Three things are deliberately left alone, because each carries something the user does not have:

  * A report that a guard fired. "The body guard forced two rewrites" is news -- it happened during
    this turn and they were not watching.
  * The exact command. `AGENTS.md` positively requires handing the user the pasteable command for
    work the agent may not do, spelled absolutely; a guard that made that unspeakable would delete
    one instruction to satisfy another.
  * Naming what is unpushed and on which branch. Same paragraph of `AGENTS.md`: that is a fact
    about the tree, not a recital of the rule.

So the line this draws is between the fact and the rule. `PR #481: <url>` is a fact. "It stays
draft because undrafting is yours" is a rule the user is being read their own copy of.

Quoted spans are stripped before matching, so a report about this guard -- this docstring, the
policy, a message explaining the halt -- cannot trip it.
"""

from __future__ import annotations

import re

# Actions whose ownership the user has already settled in their own instructions or policies. Each
# is enforced somewhere executable, which is what makes restating it redundant rather than useful:
# the global `gh pr ready` denial (GLOBAL-GITHUB-PR-DRAFT-GUARD) and `git_block_main_commit.rego`.
#
# `push` is left out, and that is the one change from the er-mods-rs original (ported 2026-09-25).
# There, `git_block_any_push.rego` makes pushing the user's, so "I don't push" restates their rule.
# Here CLAUDE.md makes pushing the agent's job, so the same sentence is not a restatement at all --
# it is false, and a halt that told the agent "the user already owns that rule" would be teaching
# the wrong rule.
OWNED_ACTIONS = r"(?:undraft\w*|merg\w*|publish\w*|releas\w*|ship\w*)"

# "<action> is yours" and its variants. The possessive is what makes it an ownership claim rather
# than a description of state.
_OWNERSHIP_RE = re.compile(
    r"\b" + OWNED_ACTIONS + r"\b[^.;!?\n]{0,40}?\bis\b\s+"
    r"(?:yours|your\s+(?:call|decision|choice|job|move)|up\s+to\s+you|for\s+you\s+to\s+\w+)",
    re.IGNORECASE,
)

# The same claim from the other side: the agent declining an action the user already forbade it.
_SELF_DENIAL_RE = re.compile(
    r"\b(?:i|we|the\s+agent|this\s+agent)\b\s+"
    r"(?:do(?:es)?\s+not|don't|doesn't|won't|will\s+not|can(?:'t|not)|may\s+not|never|"
    r"am\s+not\s+to)\s+"
    r"(?:\w+\s+){0,2}?" + OWNED_ACTIONS,
    re.IGNORECASE,
)

# "leaving it draft for you" / "it stays draft" / "so you'll need to undraft".
_HANDBACK_RE = re.compile(
    r"\b(?:stays?|remains?|left|leaving|keep(?:ing|s)?)\b[^.;!?\n]{0,30}\bdraft\b"
    r"|\bso\s+you(?:'ll|\s+will|\s+can|\s+need\s+to|\s+have\s+to)\b[^.;!?\n]{0,30}\b"
    + OWNED_ACTIONS,
    re.IGNORECASE,
)

RULES = (
    ("ownership", _OWNERSHIP_RE),
    ("self-denial", _SELF_DENIAL_RE),
    ("handback", _HANDBACK_RE),
)

# A pasteable command. `AGENTS.md` requires handing one over for work the agent may not do, so a
# closing message carrying one is obeying an instruction rather than reciting it.
_COMMAND_RE = re.compile(
    r"(?:^|\n)\s*(?:```|\$\s|git\s+push|gh\s+pr\s+ready)"
    r"|`[^`\n]*\b(?:git\s+push|gh\s+pr\s+ready)\b[^`\n]*`",
    re.IGNORECASE,
)

# A guard that fired during this turn is an event the user was not watching for, not a standing
# rule they already hold.
_GUARD_EVENT_RE = re.compile(
    r"\b(?:guard|policy|hook|cupcake|pre-commit|gate)\b[^.;!?\n]{0,60}?"
    r"\b(?:block(?:ed)?|refus(?:ed)?|den(?:ied)?|reject(?:ed)?|forced|caught|fired)\b",
    re.IGNORECASE,
)


def strip_quoted(text: str) -> str:
    """Remove fenced blocks, backtick spans and double-quoted spans.

    Same reasoning as the neighbouring classifiers: a message that quotes one of these sentences --
    to report the halt, to explain the guard, to cite the user -- must not be charged for it.
    """
    text = re.sub(r"```.*?```", " ", text, flags=re.DOTALL)
    text = re.sub(r"`[^`\n]*`", " ", text)
    text = re.sub(r'"[^"\n]*"', " ", text)
    return text


def restated_user_rule(text: str) -> tuple[str, str] | None:
    """Return `(clause, rule_id)` when the text hands back a rule the user authored."""
    scrubbed = strip_quoted(text)
    for rule_id, pattern in RULES:
        match = pattern.search(scrubbed)
        if match:
            return match.group(0).strip(), rule_id
    return None


def hands_over_command(text: str) -> bool:
    """The message carries the pasteable command `AGENTS.md` asks for."""
    return bool(_COMMAND_RE.search(text))


def reports_guard_event(text: str) -> bool:
    """The message reports a guard that fired, which the user was not watching for."""
    return bool(_GUARD_EVENT_RE.search(strip_quoted(text)))


def selftest() -> int:
    cases = 0

    hit = restated_user_rule(
        "Draft PR #481: https://example/pull/481\n\nIt stays draft -- undrafting is yours."
    )
    assert hit and hit[1] in {"ownership", "handback"}, hit
    cases += 1

    for text in (
        "Merging is your call.",
        "The merge is up to you.",
        "I don't merge, in any form.",
        "The agent does not undraft.",
        "Leaving it draft for you.",
        "So you'll need to undraft it.",
    ):
        assert restated_user_rule(text), text
        cases += 1

    # Facts, not rules: none of these may halt a turn. The last two are the ds2 divergence: pushing
    # is the agent's job here, so talking about it is never handing the user back their own rule.
    for text in (
        "I pushed it to origin/port-er-guard-policies.",
        "Pushing is done; the branch is up to date with origin.",
        "Draft PR #481: https://github.com/Banon-Labs/er-mods-rs/pull/481",
        "Committed as 91b39cdf on branch fix/quickbar-selection-and-system-crash, unpushed.",
        "CI is still running: stage / lint is in_progress.",
        "selectedQuickSlot went from -1 to 0 across the import.",
    ):
        assert restated_user_rule(text) is None, text
        cases += 1

    # A quoted instance is a report about the rule, not an instance of it.
    assert restated_user_rule('The guard halts on "undrafting is yours".') is None
    cases += 1
    assert restated_user_rule("The guard halts on `pushing is your call`.") is None
    cases += 1

    # The two exemptions.
    assert hands_over_command("Unpushed. Run `git push -u origin fix/quickbar`.")
    cases += 1
    assert not hands_over_command("It stays draft -- undrafting is yours.")
    cases += 1
    assert reports_guard_event("The body guard blocked two drafts over the 2500-character cap.")
    cases += 1
    assert not reports_guard_event("Undrafting is yours.")
    cases += 1

    print(f"selftest: {cases} cases passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(selftest())
