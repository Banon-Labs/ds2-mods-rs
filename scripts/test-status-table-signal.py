#!/usr/bin/env python3
"""Unit tests for scripts/cupcake_status_table.py -- the decision module behind the Stop guard
DS2-MODS-NO-STATUS-TABLE-AT-TURN-END.

THE SPLIT OF DUTY. `.cupcake/tests/no_status_table_at_turn_end_test.rego` pins the RULE: given
hits/asked/blocked, does the turn halt. This file pins the CLASSIFICATION that produces those facts,
which is where every false positive will come from: what counts as a table at all, which cells are
progress words, and which prompts count as asking for one. `scripts/test-cupcake-stop-guards.py`
drives the third layer -- the signal script, the WASM runtime and the real hook -- against a
transcript fixture.

THE FALSE POSITIVE THAT MATTERS, and the reason most of this file is the negative side. This repo's
one-paragraph rule (DS2-MODS-WALL-OF-TEXT) tells the agent, on every single prompt, to "put what
remains in a table, a list or a code block, which are scanned rather than read". A guard that fired
on data tables would therefore fight a rule enforced on every turn, and would do it in the one place
the agent is least able to argue back -- after the text is already on screen. So the tests below
spend more effort proving tables of DATA stay silent than proving progress tables halt.
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cupcake_status_table as table  # noqa: E402


@dataclass(frozen=True)
class HitCase:
    name: str
    text: str
    hits: int
    why: str


# The turn that prompted the guard (user directive 2026-09-23), verbatim apart from column padding.
VERBATIM_TURN = """The picker's data layer is in.

| piece | state |
|---|---|
| name/state/stats per slot, in Rust | done, host-testable |
| soul level | deliberately absent |
| picker rows over those slots | next |
| load one chosen character instead of the whole container | after that |
"""

HIT_CASES = [
    HitCase(
        "verbatim_progress_table",
        VERBATIM_TURN,
        5,
        "the 2026-09-23 failure: `state` in the header, `state` again inside "
        "'name/state/stats per slot, in Rust', then `done`, `next` and `after that`. Two of its "
        "four rows are work that was not done, in the format that makes not-doing-it look like a "
        "deliverable",
    ),
    HitCase(
        "two_column_state_table",
        "| item | state |\n|---|---|\n| parser | done |\n| writer | pending |\n",
        3,
        "the minimal shape of the same defect -- header plus both cells -- which is what this looks "
        "like when there is less to report",
    ),
    HitCase(
        "header_cells_count",
        "| piece | status |\n|-----|-----|\n| ds2-sl2-core | shipped |\n",
        1,
        "THE HEADER IS A ROW. `table_rows` walks OUT from the separator in both directions, so a "
        "column literally headed `status` is counted like any other cell. Miss that and a two-row "
        "progress table scores one instead of three and walks straight past MIN_HITS",
    ),
    HitCase(
        "measurement_table_one_stray_done",
        "| offset | value |\n|---|---|\n| 0x00a3c0 | 0x4141 |\n| 0x00a3c8 | done |\n",
        1,
        "ONE progress word in a data table is a coincidence, not a status report. This is what "
        "MIN_HITS=2 exists for, and it is the false positive that would cost the most: an "
        "offset/value grid is the shape the one-paragraph rule asks for",
    ),
    HitCase(
        "shell_pipeline_in_prose",
        "I ran `grep -c done build.log | wc -l` and then `cat rows | sort | uniq -c`, which "
        "reported the next batch as pending.\n",
        0,
        "A SEPARATOR ROW IS REQUIRED. Pipes alone are not a table: a shell pipeline quoted in prose "
        "carries `|` and, in this repo, routinely carries `done`/`next`/`pending` beside it. "
        "Without the separator requirement this guard would halt turns for quoting a command",
    ),
    HitCase(
        "type_signature_in_prose",
        "The field is `Option<u32> | None` and the state machine moves next on a zero.\n",
        0,
        "the same requirement from the other direction -- `a | b` in a type or a grammar is not a "
        "table however many progress words share the line",
    ),
    HitCase(
        "prose_above_the_table_is_not_scanned",
        "Next up is the value map, and the remaining work is listed after it.\n\n"
        "| offset | value |\n|---|---|\n| 0x10 | 0x4141 |\n",
        0,
        "matching is per CELL, so ordinary prose around a data table -- which is where words like "
        "`next` and `remaining` genuinely belong -- never arms the guard",
    ),
    HitCase(
        "words_are_whole_words",
        "| field | meaning |\n|---|---|\n| stepper | motor index |\n| donee | grant target |\n",
        0,
        "`done` inside `donee` and `step` inside `stepper` are not progress words; the lexicon is "
        "matched on whole-word boundaries so substrings in real data cannot accumulate hits",
    ),
    HitCase(
        "one_cell_counts_once",
        "| piece | state |\n|---|---|\n| picker | done, next up after that |\n",
        2,
        "counting is per cell, not per word: the crowded cell is ONE signal, and the `state` header "
        "is the second -- which is exactly the 2-hit floor, so the sample stays quotable",
    ),
]


@dataclass(frozen=True)
class AskCase:
    name: str
    prompt: str
    asked: bool
    why: str


ASK_CASES = [
    AskCase(
        "summarise_is_not_a_table_request",
        "summarise where the save picker got to",
        False,
        "THE EXEMPTION THIS MUST NOT GRANT. 'summarise' is the word the agent would reach for to "
        "excuse the table it wanted to write anyway -- reading it as a table request hands back the "
        "exemption the whole guard exists to remove. A summary is prose, and the one-paragraph rule "
        "already says how long it gets",
    ),
    AskCase(
        "summary_noun_is_not_either",
        "give me a summary of the sl2 work",
        False,
        "same word in its noun form, for the same reason",
    ),
    AskCase("asked_table", "give me a table of the offsets", True, "the plain request"),
    AskCase("asked_tables_plural", "two tables please", True, "plural spelling"),
    AskCase(
        "asked_compare",
        "compare the two layouts",
        True,
        "a comparison is structure by nature, and the agent should be free to lay it out in a grid",
    ),
    AskCase("asked_side_by_side", "put them side by side", True, "the hyphen-free spelling"),
    AskCase("asked_checklist", "give me a checklist for the port", True, "an asked-for checklist"),
    AskCase(
        "asked_matrix", "a matrix of flag against effect", True, "the word used for a data grid"
    ),
    AskCase(
        "tablet_is_not_table",
        "read the tablet inscription",
        False,
        "word-boundary check: `table` inside `tablet` is not a request",
    ),
    AskCase(
        "keep_going_is_not_a_request",
        "keep going",
        False,
        "the ordinary prompt the failing turn actually answered",
    ),
]


def check_hits() -> list[str]:
    failures = []
    for case in HIT_CASES:
        hits, sample = table.progress_table_hits(case.text)
        if hits != case.hits:
            failures.append(f"{case.name}: hits={hits}, expected {case.hits} -- {case.why}")
            continue
        # A halt quotes the sample back so the agent knows which table to delete; a firing case
        # with no sample would produce a correction pointing at nothing.
        if hits >= table.MIN_HITS and not sample:
            failures.append(f"{case.name}: {hits} hits but no sample row to quote in the halt")
        if hits < table.MIN_HITS and sample and hits == 0:
            failures.append(f"{case.name}: no hits but a sample was produced: {sample!r}")
    return failures


def check_min_hits() -> list[str]:
    """MIN_HITS is the guard's whole false-positive budget, so it is asserted directly."""
    failures = []
    if table.MIN_HITS != 2:
        failures.append(
            f"MIN_HITS is {table.MIN_HITS}, expected 2. The policy hardcodes 2 as well "
            "(no_status_table_at_turn_end.rego, `min_hits`); change both or neither."
        )
    stray = "| offset | value |\n|---|---|\n| 0x10 | done |\n"
    hits, _ = table.progress_table_hits(stray)
    if hits >= table.MIN_HITS:
        failures.append(
            f"a single stray `done` in a measurement table scored {hits} >= MIN_HITS "
            f"({table.MIN_HITS}) -- the guard would halt turns for reporting measurements"
        )
    return failures


def check_separator_requirement() -> list[str]:
    """`table_rows` is the gate everything else sits behind: no separator row, no table."""
    failures = []
    no_separator = "| done | next |\n| still to | pending |\n"
    if table.table_rows(no_separator):
        failures.append(
            "pipe-bearing lines with no separator row were read as a table; a shell pipeline or an "
            "ASCII-art line would then be enough to halt a turn"
        )
    with_separator = "| a | b |\n|---|---|\n| c | d |\n"
    rows = table.table_rows(with_separator)
    if len(rows) != 2:
        failures.append(
            f"a two-row table yielded {len(rows)} rows, expected 2 (header + body, separator "
            f"excluded): {rows!r}"
        )
    aligned = "| a | b |\n|:---|---:|\n| c | d |\n"
    if len(table.table_rows(aligned)) != 2:
        failures.append("an alignment-colon separator (`|:---|---:|`) was not recognised")
    if table.cells("| a | b, c |") != ["a", "b, c"]:
        failures.append(f"cell splitting is wrong: {table.cells('| a | b, c |')!r}")
    return failures


def check_asks() -> list[str]:
    failures = []
    for case in ASK_CASES:
        got = table.user_asked_for_a_table(case.prompt)
        if got != case.asked:
            failures.append(
                f"{case.name}: user_asked_for_a_table({case.prompt!r}) = {got}, "
                f"expected {case.asked} -- {case.why}"
            )
    if table.user_asked_for_a_table(""):
        failures.append("an empty prompt was read as asking for a table")
    if table.user_asked_for_a_table(None):  # type: ignore[arg-type]
        failures.append("a missing prompt was read as asking for a table")
    return failures


def check_last_user_prompt() -> list[str]:
    """Both content shapes, because a prompt read as empty silently exempts nothing -- it makes
    `asked` 0, which is the fail-closed direction, but it also loses every real exemption."""
    failures = []
    events = [
        {"type": "user", "message": {"role": "user", "content": "give me a table of the offsets"}},
        {"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "ok"}]}},
    ]
    if table.last_user_prompt(events) != "give me a table of the offsets":
        failures.append("string-shaped user content was not read")
    events_blocks = [
        {
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": "side by side please"}]},
        }
    ]
    if table.last_user_prompt(events_blocks) != "side by side please":
        failures.append(
            "block-list user content was not read -- this is the shape cupcake_turn_scan's own "
            "helper drops, which is why this module reads both"
        )
    # A tool result is a `user` event too, and reading one as the prompt would make the exemption
    # depend on whatever a command happened to print.
    events_tool = [
        {"type": "user", "message": {"role": "user", "content": "keep going"}},
        {
            "type": "user",
            "message": {
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "table of 4 rows"}],
            },
        },
    ]
    if table.last_user_prompt(events_tool) != "keep going":
        failures.append("a tool_result event was mistaken for the user's prompt")
    if table.last_user_prompt([]) != "":
        failures.append("an empty transcript did not yield an empty prompt")
    return failures


def main() -> int:
    failures = (
        check_hits()
        + check_min_hits()
        + check_separator_requirement()
        + check_asks()
        + check_last_user_prompt()
    )
    if failures:
        for failure in failures:
            print(f"[test-status-table-signal] FAIL: {failure}", file=sys.stderr)
        return 1
    print(
        f"[test-status-table-signal] ok ({len(HIT_CASES)} table shapes, {len(ASK_CASES)} prompts, "
        f"MIN_HITS={table.MIN_HITS}, separator and prompt-shape contracts)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
