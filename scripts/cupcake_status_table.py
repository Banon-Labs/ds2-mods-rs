"""Deciding whether a markdown table is a PROGRESS REPORT rather than data.

Owned by one module so the signal, its regression test and any false-positive audit cannot drift
into three different answers -- the same reason `cupcake_turn_scan.prose_paragraphs` is shared.

THE DISTINCTION THIS MODULE EXISTS TO DRAW. A table is the right shape for data the reader has to
scan: an offset and its value, a file and its hash, a flag and its effect. The repo's one-paragraph
rule actively pushes content into tables for exactly that reason. What it is NOT the right shape for
is the agent's own progress -- `done` / `next` / `missing` / `after that` -- because a grid of those
is a status report that reads as a deliverable while containing no work.

So nothing here looks at whether a table is present. It looks at what is in the CELLS.
"""

from __future__ import annotations

import re

#: Progress words. Each is matched as a whole word inside a table CELL, so "the next section"
#: in ordinary prose above the table is never seen and a column literally headed `next` is.
#:
#: `done` and `state` are in the list and are the two that could plausibly appear in a real data
#: table, which is what MIN_HITS is for: one of them alone proves nothing.
PROGRESS_WORDS = (
    "done",
    "next",
    "todo",
    "to do",
    "missing",
    "pending",
    "not yet",
    "planned",
    "remaining",
    "after that",
    "afterwards",
    "upcoming",
    "in progress",
    "wip",
    "still to",
    "not built",
    "not started",
    "unbuilt",
    "state",
    "status",
    "stage",
    "step",
)

PROGRESS_RE = re.compile(
    r"(?<![\w-])(?:%s)(?![\w-])" % "|".join(re.escape(w) for w in PROGRESS_WORDS),
    re.IGNORECASE,
)

#: TWO hits, not one. A measurement table that happens to carry a `state` column, or one cell
#: reading `done`, is not a status report; two progress words in one table is no longer a
#: coincidence. Measured against the turn that prompted this guard: its table scored four.
MIN_HITS = 2

#: The separator row markdown requires under a table's header: `|---|---|`, with optional colons
#: for alignment. Its presence is what distinguishes a table from a line that merely contains a
#: pipe -- a shell pipeline inside prose, or `a | b` in a type signature.
SEPARATOR_RE = re.compile(r"^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$")

#: A request for a table, in the user's own words. Deliberately narrow: "summarise" is not a table
#: request, and treating it as one would hand back the exemption this guard exists to remove.
ASKED_RE = re.compile(
    r"(?<![\w-])(tables?|tabular|tabulate|matrix|grid|side[- ]by[- ]side|compare|comparison|"
    r"checklist|spreadsheet)(?![\w-])",
    re.IGNORECASE,
)


def table_rows(text: str) -> list[str]:
    """Every row of every markdown table in `text`, separator rows excluded.

    A "table" needs a separator row; a run of pipe-bearing lines without one is prose. Rows are
    collected from the whole block the separator belongs to, above it as well as below, so a
    header cell reading `state` counts like any other cell.
    """
    lines = text.split("\n")
    marked = [False] * len(lines)
    for index, line in enumerate(lines):
        if not SEPARATOR_RE.match(line) or "-" not in line:
            continue
        # Walk out in both directions over contiguous pipe-bearing lines.
        for step in (-1, 1):
            at = index + step
            while 0 <= at < len(lines) and "|" in lines[at] and lines[at].strip():
                marked[at] = True
                at += step
    return [line for line, keep in zip(lines, marked) if keep]


def cells(row: str) -> list[str]:
    """The row's cells, without the pipes and without surrounding whitespace."""
    return [cell.strip() for cell in row.strip().strip("|").split("|")]


def progress_table_hits(text: str) -> tuple[int, str]:
    """(how many progress words appear in table cells, the first offending row).

    Counting per CELL rather than per row: a single cell reading "done, host-testable" is one
    signal, not two, and a row of four status cells is four.
    """
    hits = 0
    sample = ""
    for row in table_rows(text):
        for cell in cells(row):
            if not PROGRESS_RE.search(cell):
                continue
            hits += 1
            if not sample:
                sample = " ".join(row.split())[:120]
    return hits, sample


def user_asked_for_a_table(prompt: str) -> bool:
    """Whether the user asked for structure. An asked-for table is never this guard's business."""
    return bool(ASKED_RE.search(prompt or ""))


def last_user_prompt(events: list[dict]) -> str:
    """The text of the most recent real user prompt.

    `cupcake_turn_scan._user_content_string` handles only the string-content shape; a prompt whose
    content is a block list would come back empty and silently exempt nothing, so both shapes are
    read here.
    """
    try:
        from cupcake_turn_scan import is_real_user_prompt
    except Exception:
        return ""
    for event in reversed(events):
        if not is_real_user_prompt(event):
            continue
        content = event.get("message", {}).get("content")
        if isinstance(content, str):
            return content
        if isinstance(content, list):
            out = []
            for block in content:
                if isinstance(block, dict) and block.get("type") == "text":
                    out.append(block.get("text", ""))
            return "\n".join(out)
        return ""
    return ""
