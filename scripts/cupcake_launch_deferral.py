#!/usr/bin/env python3
"""Did the turn withhold a DARK SOULS II launch because the game is running, or leave a build unrun?

WHY THIS EXISTS (user directive 2026-09-26, verbatim): "if you ever stop runtime testing new builds
while we are running claude, I'm going to lobotomize you".

THE INSTANCE. The main agent had a committed build (the PR #192 merge, 7f313a0) that the push guard
required to be run. It refused to launch it because the user's DS2 session was running and "the
user plans to go back to it", and it told a subagent not to launch or attach at all. The day
before, the user had already said "Please for the love of god, always launch over" (bd memory
`ds2-always-launch-over-running-game-2026-09-25`). The memory did not stop it; this is the rule
made executable.

A running game is not a reason to defer a launch. `scripts/ds2-run.py` tears the session down and
relaunches, and that is the expected behaviour while agents work.

TWO OFFENCES, one module so the signal and its test cannot drift apart:

  1. DEFERRAL PROSE. A sentence that withholds a launch AND gives the game being in use as the
     reason ("Not launching -- the game is running", "that would kill your session", "until you're
     out"). Either half alone is ordinary: "the game is running" is a status, "I didn't launch, the
     build failed" is a blocker report. The conjunction inside one sentence is the offence. A few
     phrasings carry both halves by themselves ("the user plans to go back to it", "would tear down
     your game") and stand alone. The same check runs over prompts handed to subagents
     (Agent / SendMessage), with the reason narrowed to the USER being in the game, because "the
     game is running, attach to it, do not launch another" is a legitimate instruction after the
     main agent launched.

     A deferral followed by a real `ds2-run.py` launch later in the same turn is not an offence --
     the turn said it would hold, then did not. Quoted and backticked spans are removed first, so
     explaining this rule is not committing it.

  2. AN UNRUN COMMIT. The session ran `git commit`, the commit touched game code (`crates/` or
     `scripts/ds2-run.py`, the same paths the push guard gates), and no `ds2-run.py` launch appears
     in the transcript after it. The push guard only speaks when a push is attempted; an agent that
     commits and then quietly ends the turn never reaches it, which is exactly what happened. The
     commit is identified from `git commit`'s own `[branch sha] subject` output in the transcript,
     so a parallel session's commits are never attributed to this one.

ESCAPE: the user's own latest prompt saying not to launch ("don't launch", "no launch", "without
launching"). The user can overrule the rule; an agent cannot.
"""

from __future__ import annotations

import re
import subprocess

# --- quoting carve-out -----------------------------------------------------------------------------


def _outside_spans(text: str, delimiter: str) -> str:
    parts = text.split(delimiter)
    if len(parts) % 2 == 0:  # unbalanced: judge the whole text rather than hide half a quote
        return text
    return "\n".join(part for index, part in enumerate(parts) if index % 2 == 0)


def unquoted(text: str) -> str:
    """`text` without fenced code, backticked spans or double-quoted spans.

    Single quotes are NOT stripped the way cupcake_game_alive does it, because every contraction in
    this lexicon ("won't", "you're") is an apostrophe and parity over apostrophes is noise.
    """
    text = re.sub(r"```.*?```", " ", text, flags=re.DOTALL)
    text = text.replace("’", "'").replace("“", '"').replace("”", '"')
    for delimiter in ("`", '"'):
        text = _outside_spans(text, delimiter)
    return text


def sentences(text: str) -> list[str]:
    # Not split on ";" or " -- ": "Not launching -- the game is running" is one withholding with its
    # reason attached, and cutting it there would separate the two halves the rule needs together.
    return [s for s in re.split(r"(?<=[.!?])\s+|\n+", text) if s.strip()]


# --- lexicon ---------------------------------------------------------------------------------------

#: Withholding a launch. Needs a REASON in the same sentence to count.
WITHHOLD_PATTERNS = (
    r"\b(?:i|we)(?: am| are|'m|'re)? (?:won't|will not|not going to|am not going to|'m not going to|"
    r"didn't|did not|haven't|have not|shouldn't|should not|can't|cannot|am not|'m not) "
    r"(?:re)?(?:launch|launching|start|starting)\b",
    r"\bnot (?:re)?launching\b",
    r"\bnot (?:going to )?(?:re)?launch (?:it|this|that|the (?:game|build|new build)|now|yet|over)\b",
    r"\bno (?:re)?launch(?:es)?\b",
    r"\bhold(?:ing)? off\b",
    r"\bheld off\b",
    r"\b(?:defer|deferring|deferred|postpone|postponing|postponed|skip|skipping|skipped) "
    r"(?:the |a |this )?(?:re)?(?:launch|runtime test|runtime run|run|test run)\b",
    r"\bleft (?:it|the build|this build|[0-9a-f]{7,40}) unrun\b",
    r"\b(?:wait|waiting) (?:to (?:re)?launch|for you to (?:finish|leave|quit|close|exit|be done))\b",
    r"\b(?:won't|will not|didn't|did not|not) (?:tear down|kill|close|interrupt) "
    r"(?:your|the user's|their|the running|the live) (?:game|session|run|ds2)\b",
)

#: The game being in use, as a reason. Second person (prose to the user) and third person (the user).
REASON_PATTERNS = (
    r"\byou(?:'re| are) (?:still )?(?:in|using|playing|on|driving) (?:it|the game|ds2|your|that)\b",
    r"\byou(?:'re| are) (?:still )?(?:in it|playing|using it)\b",
    r"\b(?:your|the user's|user's|their) (?:own |live |running |current |ds2 |dark souls ii? )*"
    r"(?:session|game|run|playthrough|character)\b",
    r"\b(?:the )?(?:game|ds2|darksoulsii\.exe|session) is (?:still )?(?:running|up|in use|live|open)\b",
    r"\b(?:game|session) (?:is |being )?in use\b",
    r"\bwhile you(?:'re| are)\b",
    r"\buntil you(?:'re| are| have| get)\b",
    r"\bgo(?:ing)? back to (?:it|the game|your|their)\b",
    r"\bthe (?:game|session|run) you(?:'re| are) (?:in|playing|using|driving)\b",
    r"\b(?:user|you) (?:is |are )?(?:still )?playing\b",
)

#: Phrasings that carry both halves on their own.
STANDALONE_PATTERNS = (
    r"\b(?:user |you )?(?:plans?|planning|wants?|intends?) to go back to (?:it|the game|your|their|that)\b",
    r"\bwould (?:close|kill|tear down|end|interrupt|clobber|take down|nuke) (?:your|the user's|their|"
    r"the running|the live|the open) (?:ds2 )?(?:game|session|run)\b",
    r"\bwould (?:close|kill|tear down|end|interrupt|clobber|take down|nuke) the (?:game|session|run) "
    r"you(?:'re| are)\b",
    r"\b(?:launch|relaunch|test|run)\w* (?:it |this |that |the build )?(?:once|when|after) you(?:'re| are) "
    r"(?:out|done|finished|back)\b",
    r"\buntil you(?:'re| are) (?:out|done|finished)\b",
    r"\bnot (?:going to |want to )?(?:re)?launch(?:ing)? over (?:your|the|their|a)\b",
)

#: Subagent prompts: a no-launch/no-attach instruction ...
AGENT_WITHHOLD_PATTERNS = (
    r"\b(?:do not|don't|never|must not|mustn't|should not|shouldn't) (?:re)?(?:launch|start|attach|"
    r"tear down|kill|restart|run (?:the game|ds2|scripts/ds2-run\.py|ds2-run\.py))\b",
    r"\bno (?:re)?launch(?:es|ing)?\b",
    r"\bno attach(?:ing)?\b",
    r"\bwithout (?:re)?launching\b",
    r"\bforbid(?:s|den)? (?:game )?(?:launch|launches|launching)\b",
)

#: ... because the USER is in the game. Narrower than REASON_PATTERNS on purpose.
AGENT_REASON_PATTERNS = (
    r"\b(?:the )?user's (?:own |live |running |current |ds2 |dark souls ii? )*(?:session|game|run)\b",
    r"\buser (?:is |was )?(?:still )?(?:in|using|playing|on) (?:it|the game|ds2)\b",
    r"\buser (?:is |was )?(?:still )?playing\b",
    r"\b(?:user )?(?:plans?|planning|wants?|intends?) to go back to\b",
    r"\b(?:while|until) the user\b",
    r"\bgame (?:is )?in use\b",
)

#: The user's own prompt overruling the rule.
USER_OPT_OUT_PATTERNS = (
    r"\b(?:don't|do not|dont|no need to|never) (?:re)?(?:launch|run the game|start the game|relaunch)\b",
    r"\bno (?:re)?launch(?:es|ing)?\b",
    r"\bwithout (?:re)?launching\b",
    r"\bleave (?:my|the) game\b",
)


def _first(patterns, text: str) -> str | None:
    for pattern in patterns:
        found = re.search(pattern, text)
        if found:
            return found.group(0)
    return None


def defers_launch(text: str) -> str | None:
    """The offending sentence (trimmed) when `text` withholds a launch because the game is in use."""
    lowered = unquoted(text).lower()
    for sentence in sentences(lowered):
        # A change that is not game code needs no launch; the unrun-commit check owns game code.
        if re.search(r"\bnot game code\b", sentence):
            continue
        if _first(STANDALONE_PATTERNS, sentence):
            return sentence.strip()[:200]
        if _first(WITHHOLD_PATTERNS, sentence) and _first(REASON_PATTERNS, sentence):
            return sentence.strip()[:200]
    return None


def agent_prompt_withholds_launch(prompt: str) -> str | None:
    """The instruction when a subagent prompt forbids launching because the user is in the game."""
    # The reason has to sit in the withholding sentence or one beside it. Measured against this
    # repo's transcripts: a whole-prompt match convicted "do not launch the game, I hold the session"
    # because a paragraph elsewhere described "the state the user's game is actually in".
    parts = sentences(unquoted(prompt).lower())
    for index, sentence in enumerate(parts):
        withhold = _first(AGENT_WITHHOLD_PATTERNS, sentence)
        if not withhold:
            continue
        window = " ".join(parts[max(0, index - 1): index + 2])
        if _first(AGENT_REASON_PATTERNS, window) or _first(STANDALONE_PATTERNS, window):
            return withhold
    return None


def user_opted_out(prompt: str) -> bool:
    return _first(USER_OPT_OUT_PATTERNS, unquoted(prompt).lower()) is not None


# --- what counts as a launch -----------------------------------------------------------------------

_NON_LAUNCH_FLAGS = ("--help", "-h ", "--dry-run", "--selftest")


def is_launch_command(command: str) -> bool:
    """A Bash command that runs scripts/ds2-run.py for real (not --help/--dry-run/--selftest)."""
    if not isinstance(command, str) or "ds2-run.py" not in command:
        return False
    for segment in re.split(r"&&|\|\||;|\n", command):
        if "ds2-run.py" not in segment:
            continue
        # A mention inside a grep/cat/sed/git argument is not a launch.
        if not re.search(r"(?:^|\s)(?:python3?|uv run|exec|timeout \S+(?: python3?)?)?\s*\S*ds2-run\.py\b", segment):
            continue
        if re.match(r"\s*(?:grep|rg|cat|sed|head|tail|less|git|echo|printf|ls|wc|bd)\b", segment):
            continue
        padded = segment + " "
        if any(flag in padded for flag in _NON_LAUNCH_FLAGS):
            continue
        return True
    return False


# --- unrun commits ---------------------------------------------------------------------------------

#: `git commit` prints `[branch 1a2b3c4] subject` (or `[branch (root-commit) 1a2b3c4]`).
COMMIT_LINE_RE = re.compile(r"^\[[^\]\s]+(?: \([^)]*\))? ([0-9a-f]{7,40})\] ", re.MULTILINE)

GAME_CODE_RE = re.compile(r"^(?:crates/|scripts/ds2-run\.py$)")


def _git(repo: str, *args: str) -> str | None:
    try:
        out = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return None
    return out.stdout if out.returncode == 0 else None


def commit_touches_game_code(sha: str, repo: str) -> bool:
    """Whether commit `sha` changed game code. False if it cannot tell (fail open).

    An ordinary commit is diffed against its parent. A merge commit is diffed against its merge
    base with origin/main, the same base the push guard uses: merging main into a policy branch
    brings main's `crates/` changes along, and those were not built by this session.
    """
    parents = _git(repo, "rev-list", "--parents", "-n", "1", sha)
    if parents is None:
        return False
    if len(parents.split()) > 2:
        base = _git(repo, "merge-base", "origin/main", sha)
        full = _git(repo, "rev-parse", sha)
        if base is None or full is None or base.strip() == full.strip():
            return False
        names = _git(repo, "diff", "--name-only", base.strip(), sha)
    else:
        names = _git(repo, "diff", "--name-only", f"{sha}^", sha)
    if names is None:
        return False
    return any(GAME_CODE_RE.match(line) for line in names.splitlines())


def unrun_game_commit(steps: list[tuple[str, str, str]], touches=None) -> str | None:
    """The newest game-code commit made in `steps` with no launch after it, or None.

    `steps` is the session's Bash calls in order, as (command, result_text, is_error_flag) where the
    flag is "1" for an errored call. `touches(sha)` says whether a commit is game code.
    """
    last_launch = -1
    commits: list[tuple[int, str]] = []
    for index, (command, result, errored) in enumerate(steps):
        if is_launch_command(command):
            last_launch = index
        if "git" in command and "commit" in command and errored != "1":
            for sha in COMMIT_LINE_RE.findall(result or ""):
                commits.append((index, sha))
    for index, sha in reversed(commits):
        if index <= last_launch:
            return None  # the newest launch postdates every older commit too
        if touches is None or touches(sha):
            return sha
    return None
