#!/usr/bin/env python3
"""Unit tests for scripts/cupcake_shouting.py -- the decision module behind both shouting guards.

The split of duty. `.cupcake/tests/no_shouting_at_turn_end_test.rego` pins the Stop rule (given a
score, does the turn halt) and `.cupcake/tests/docs_no_shouting_test.rego` pins the PreToolUse rule
(given an edit, is it refused). This file pins the CLASSIFICATION underneath both: which spans are
removed before anything is judged, which words count, and where a run starts and stops. That is
where every false positive will come from.

It also does something the two Rego suites cannot. The offence is defined twice -- once in Rego for
the edit that is about to hit disk, once in Python for the prose that is about to end a turn -- and
two definitions that drift apart are worse than one, because the disagreement is invisible until
somebody is refused by one arm and waved through by the other. So the patterns and the threshold
are compared across all three files here, byte for byte.

The false positive that matters. This repo is written in acronyms, register names, screaming-snake
constants, mangled Ghidra symbols and hex. A guard that read "capital letters" as the offence would
make it unwritable, and would do it in the one place an author cannot argue back -- after the text
is composed. So most of what follows proves that names stay silent.
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "scripts"))

import cupcake_shouting as shouting  # noqa: E402

DOCS_POLICY = REPO_ROOT / ".cupcake" / "policies" / "claude" / "docs_no_shouting.rego"
STOP_POLICY = REPO_ROOT / ".cupcake" / "policies" / "claude" / "no_shouting_at_turn_end.rego"


@dataclass(frozen=True)
class Case:
    name: str
    text: str
    shouts: bool
    why: str


CASES = [
    # --- capitalised identifiers are names (measured 2026-09-25) ---------------------------------
    Case(
        "rule_ids_in_prose",
        "The port kept DS2-MODS-NO-FIX-CLAIM-WITHOUT-RUNTIME-EVIDENCE and DS2-MODS-NO-GREP-FOR-BUILD-ERRORS.",
        False,
        "a rule id is a name; a hyphen is a word boundary, so without the identifier drop its FOR "
        "and WITHOUT read as shouted function words",
    ),
    Case(
        "shout_beside_a_rule_id",
        "BUILTIN-GIT-BLOCK-NO-VERIFY is NOT optional.",
        True,
        "the identifier drop takes only the identifier; the NOT beside it is still a shout",
    ),
    # --- the four shapes the user named, all of which must be refused ---------------------------
    Case(
        "one_word_absolute",
        "NOTHING in ds2-save-file has been run.",
        True,
        "a single capitalised word, which the run rule cannot see at all. The user named this one "
        "explicitly, and it is the whole reason the function-word list exists beside the run rule",
    ),
    Case(
        "shouted_clause_mid_sentence",
        "THE GAME IS INVISIBLE TO pgrep, because it runs under Wine.",
        True,
        "the commonest form of the habit here: a clause shouted inside an ordinary sentence",
    ),
    Case(
        "shouted_heading",
        "# WHAT THIS DOES NOT CLAIM",
        True,
        "a heading shouted instead of being marked up as a heading",
    ),
    Case(
        "hyphenated_one_word_shout",
        "STARTUP-ONLY, both of them.",
        True,
        "the hyphen ends the run, so this is caught by `ONLY` alone -- which is why the word list "
        "is checked per word rather than per run",
    ),
    Case(
        "content_words_only",
        "ONE ROW PITCH PER ADDED ROW.",
        True,
        "four or more capitalised words with no function word among them, which the word list "
        "cannot see by construction. No line of this shape survives in the tracked tree today, so "
        "the case is synthetic on purpose: it is what the run rule is for, and without it nothing "
        "would notice if that rule stopped working",
    ),
    # --- identifiers, which are one token to a word boundary --------------------------------------
    Case(
        "screaming_snake_constants",
        "Writes under SAVE_DIR_BUILD, sized by FE_INGAME_MENU_ITEM_VECTOR_CAPACITY.",
        False,
        "an underscore is a word character, so a screaming-snake constant never matches a "
        "capitalised WORD and cannot start a run. This is why there is no allow-list of identifiers",
    ),
    Case(
        "ghidra_symbol_and_save_file",
        "Reads DS2SOFS0000.sl2 through FUN_1402e67f0, entered from DllMain.",
        False,
        "a digit ends the word and a lowercase letter disqualifies it, so a save file name, a "
        "Ghidra placeholder and the Windows entry point are all invisible to both rules",
    ),
    Case(
        "hex_and_numbers",
        "The flag sits at 0xDEADBEEF and the stride is 0x58, measured over 12 rows.",
        False,
        "hex constants and numbers are not capitalised words",
    ),
    # --- acronyms and names in ordinary sentences -------------------------------------------------
    Case(
        "acronym_vocabulary",
        "The DLL is built for MSVC and its RVA table is checked against the PE headers.",
        False,
        "the vocabulary this repo is made of. A name in capitals was never the offence",
    ),
    Case(
        "crypto_acronyms",
        "The save is AES in CBC mode, keyed per PID, with an MD5 over the BND4 container.",
        False,
        "same from the other side, and MD5/BND4 fail the word test on their digits as well",
    ),
    Case(
        "tooling_acronyms",
        "Config is TOML, the manifest is JSON, the text is UTF-8, CI runs on every PR, and that is "
        "OK on any CPU the API supports.",
        False,
        "eight acronyms in one sentence, none adjacent, so no run forms and none is on the word list",
    ),
    Case(
        "comma_separated_acronyms",
        "Sections: PE, COFF, IAT, TLS, PEB.",
        False,
        "Punctuation ends a run, and this is the case that requires it. Five acronyms in a row "
        "would otherwise read as one shouted phrase; the commas are what say they are a list",
    ),
    Case(
        "four_adjacent_acronyms_is_the_known_residue",
        "The DLRF DLUT DLKR DLIO tags open the container.",
        True,
        "the residue of the run rule, recorded rather than hidden: four acronyms with nothing "
        "between them do look like a run. Every occurrence of this in the repo is already inside "
        "backticks or a fenced block, which is where a list of section tags belongs",
    ),
    Case(
        "sentence_initial_capital",
        "Only the loader touches the game. Nothing else does.",
        False,
        "one capitalised letter at the start of a sentence is how English works",
    ),
    Case(
        "roman_numerals_and_a_proper_noun",
        "The build is staged for DARK SOULS II and nothing else.",
        False,
        "three capitalised words, which is what a proper noun written out looks like. The "
        "threshold sits one above this on purpose: dropping to three would add 86 runs to the "
        "sweep that tuned the guard, and 80 of them are the game's name, a title-screen prompt "
        "and two file markers",
    ),
    # --- verbatim spans ---------------------------------------------------------------------------
    Case(
        "inside_backticks",
        "The row reads `PRESS ANY BUTTON` until a save exists.",
        False,
        "a menu string in backticks is a quoted string, not the author raising their voice",
    ),
    Case(
        "inside_a_fenced_block",
        "The log looks like this:\n\n```\nTHE GAME IS INVISIBLE TO pgrep\nNOTHING WAS ATTACHED\n```\n\n"
        "That is the whole record.",
        False,
        "a fence spans lines, so it is removed from the whole text before it is cut into lines",
    ),
    Case(
        "inside_quotation_marks",
        'The scene animates the "DARK SOULS II: SCHOLAR OF THE FIRST SIN" text in.',
        False,
        "a quotation is somebody else's words. This is what keeps the game's full title legal -- it "
        "is quoted both times it appears here -- and what stops the guard punishing an agent for "
        "quoting the user's own directive back at them",
    ),
    Case(
        "removed_span_does_not_weld_two_runs",
        "LOAD GAME `FeSubStateTitleLoadDataList` NEW GAME.",
        False,
        "Removing a span must not create a run. Two capitalised words either side of a code span "
        "are two runs of two, not one run of four; the pieces are rejoined with a newline rather "
        "than a space for exactly this reason",
    ),
    Case(
        "unbalanced_backtick_is_judged",
        "THE GAME IS INVISIBLE TO pgrep ` and the tick never closes",
        True,
        "an odd number of delimiters makes the parity read meaningless, so the text is judged "
        "rather than trusted. Half a code span must not be a way to hide a shouted sentence",
    ),
    # --- ordinary prose, which is the overwhelmingly common case ----------------------------------
    Case(
        "plain_report",
        "The picker reads each slot out of the container and renders one row per character. Soul "
        "level is deliberately absent; the formula is not settled.",
        False,
        "the shape of almost every turn and almost every doc comment. This must never cost anybody "
        "anything",
    ),
    Case(
        "empty",
        "",
        False,
        "an empty edit is not an offence",
    ),
]


@dataclass(frozen=True)
class ScopeCase:
    name: str
    path: str
    text: str
    judged: bool
    why: str


SCOPE_CASES = [
    ScopeCase(
        "markdown_whole_file",
        "docs/DS2-NOTES.md",
        "WHAT THIS DOES NOT CLAIM",
        True,
        "Markdown is documentation end to end",
    ),
    ScopeCase(
        "rust_inner_doc_comment",
        "crates/ds2-loader/src/lib.rs",
        "//! WHAT THIS DOES NOT CLAIM",
        True,
        "`//!` is documentation",
    ),
    ScopeCase(
        "rust_outer_doc_comment",
        "crates/ds2-loader/src/lib.rs",
        "/// WHAT THIS DOES NOT CLAIM",
        True,
        "`///` is documentation",
    ),
    ScopeCase(
        "rust_ordinary_comment",
        "crates/ds2-loader/src/lib.rs",
        "// WHAT THIS DOES NOT CLAIM",
        False,
        "an ordinary `//` comment is a note beside the code, not prose anybody skims. Same cut "
        "docs_no_size_metrics makes, and keeping the two policies on one scope is deliberate",
    ),
    ScopeCase(
        "rust_string_literal",
        "crates/ds2-game-base/src/log.rs",
        '    log::warn!("NOTHING WAS ATTACHED AND THE PROBE NEVER RAN");',
        False,
        "a log line the code emits is a measurement being reported, not documentation",
    ),
    ScopeCase(
        "shell_script",
        "scripts/ds2-stage.sh",
        "echo 'THE GAME IS INVISIBLE TO pgrep'",
        False,
        "a file that is neither Markdown nor Rust carries no documentation scope at all",
    ),
]


def check_cases() -> list[str]:
    failures = []
    for case in CASES:
        got = shouting.shouts(case.text)
        if got != case.shouts:
            score, sample = shouting.shouting_score(case.text)
            failures.append(
                f"{case.name}: shouts={got} (score {score}, sample {sample!r}), expected "
                f"{case.shouts} -- {case.why}"
            )
            continue
        if case.shouts:
            _, sample = shouting.shouting_score(case.text)
            if not sample:
                failures.append(
                    f"{case.name}: refused but produced no sample line, so the correction would "
                    f"quote nothing"
                )
    return failures


def check_scope() -> list[str]:
    failures = []
    for case in SCOPE_CASES:
        got = shouting.shouts(shouting.doc_text(case.path, case.text))
        if got != case.judged:
            failures.append(
                f"{case.name}: judged={got}, expected {case.judged} -- {case.why}"
            )
    return failures


def check_two_rules_are_both_load_bearing() -> list[str]:
    """Neither half may be removable, or the guard is carrying dead weight it will be tuned by."""
    failures = []
    run_only = "ONE ROW PITCH PER ADDED ROW"
    if not re.search(shouting.RUN_PATTERN, run_only):
        failures.append(f"the run pattern no longer matches {run_only!r}")
    if re.search(shouting.EMPHASIS_PATTERN, run_only):
        failures.append(
            f"{run_only!r} now contains a word-list hit, so the run rule is no longer the only "
            "thing catching it and this case has stopped proving anything"
        )
    word_only = "NOTHING in ds2-save-file has been run"
    if re.search(shouting.RUN_PATTERN, word_only):
        failures.append(f"the run pattern unexpectedly matches the one-word case {word_only!r}")
    if not re.search(shouting.EMPHASIS_PATTERN, word_only):
        failures.append(f"the word list no longer matches {word_only!r}")
    return failures


def check_score_matches_the_two_patterns() -> list[str]:
    """The score and the two regexes have to be the same test said two ways.

    The Stop arm re-applies a single number and the PreToolUse arm applies two patterns. If those
    ever disagree, one arm refuses what the other allows -- so the equivalence is asserted over
    every case above rather than assumed from the arithmetic.
    """
    failures = []
    for case in CASES:
        judged = shouting.strip_verbatim(case.text)
        by_pattern = bool(re.search(shouting.RUN_PATTERN, judged)) or bool(
            re.search(shouting.EMPHASIS_PATTERN, judged)
        )
        by_score = shouting.shouts(case.text)
        if by_pattern != by_score:
            failures.append(
                f"{case.name}: the patterns say {by_pattern} and the score says {by_score}. The "
                f"Stop arm and the PreToolUse arm would disagree about this text"
            )
    return failures


def check_policies_carry_the_same_definition() -> list[str]:
    """The two Rego policies and this module must hold one definition between them.

    Compared literally rather than semantically: the patterns are string constants in the policy,
    so a byte comparison is the strongest check available and the cheapest to keep true.
    """
    failures = []
    docs = DOCS_POLICY.read_text(encoding="utf-8")
    stop = STOP_POLICY.read_text(encoding="utf-8")

    for label, pattern in (
        ("run_pattern", shouting.RUN_PATTERN),
        ("emphasis_pattern", shouting.EMPHASIS_PATTERN),
        ("capitalised_identifier_pattern", shouting.IDENTIFIER_PATTERN),
    ):
        expected = f"{label} := `{pattern}`"
        if expected not in docs:
            failures.append(
                f"{DOCS_POLICY.name} does not carry this module's {label} verbatim. The two arms "
                f"of the guard now forbid different things. Expected the line:\n    {expected}"
            )

    threshold = f"min_score := {shouting.MIN_RUN}"
    if threshold not in stop:
        failures.append(
            f"{STOP_POLICY.name} does not carry `{threshold}`, so the Stop arm's threshold and "
            f"MIN_RUN ({shouting.MIN_RUN}) have drifted apart"
        )

    # The run pattern encodes the threshold too, as the repetition count, and nothing else checks
    # that the two spellings of the same number agree.
    if f"{{{shouting.MIN_RUN - 1},}}" not in shouting.RUN_PATTERN:
        failures.append(
            f"the run pattern does not require {shouting.MIN_RUN} words: {shouting.RUN_PATTERN!r}"
        )
    return failures


def check_routing() -> list[str]:
    """A policy the dispatcher does not name exports into nothing and fails open with no sign."""
    failures = []
    evaluate = (REPO_ROOT / ".cupcake" / "system" / "evaluate.rego").read_text(encoding="utf-8")
    for verb, package in (
        ("deny", "docs_no_shouting"),
        ("halt", "no_shouting_at_turn_end"),
    ):
        needle = f"data.cupcake.policies.claude.{package}.{verb}"
        if needle not in evaluate:
            failures.append(
                f"evaluate.rego does not route {package}.{verb}. The policy compiles, loads and "
                f"contributes nothing, and the transcript says nothing about it"
            )
    signal = REPO_ROOT / ".cupcake" / "signals" / "last_assistant_shouting.sh"
    if not signal.is_file():
        failures.append(f"missing signal script {signal}")
    elif not signal.stat().st_mode & 0o100:
        failures.append(
            f"{signal} is not executable, so cupcake cannot run it, the policy sees an empty "
            f"signal, and the Stop hook returns a clean allow. This exact failure shipped here on "
            f"2026-09-23 with the whole suite green"
        )
    return failures


def main() -> int:
    failures = (
        check_cases()
        + check_scope()
        + check_two_rules_are_both_load_bearing()
        + check_score_matches_the_two_patterns()
        + check_policies_carry_the_same_definition()
        + check_routing()
    )
    if failures:
        for failure in failures:
            print(f"[test-shouting-signal] FAIL: {failure}", file=sys.stderr)
        return 1
    print(
        f"[test-shouting-signal] ok ({len(CASES)} text shapes, {len(SCOPE_CASES)} scopes, "
        f"MIN_RUN={shouting.MIN_RUN}, both patterns matched against both policies)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
