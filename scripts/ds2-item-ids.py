#!/usr/bin/env python3
"""Extract the DARK SOULS II item id -> name table out of a Cheat Engine table.

    scripts/ds2-item-ids.py "<table>.CT" --out crates/ds2-build-import-core/data/items.tsv

# Where the data comes from, and why it is not read from the game

The ids are `ItemParam` row ids -- the same `i32` the game's own item-grant function takes as
`ItemSpawn+0x04`. The NAMES are not in `ItemParam`; they live in the FMG message files, keyed by id.
Joining those two is the honest way to build this table and needs the encrypted regulation open
first.

Until then, the community Cheat Engine tables already carry the join, as a `<DropDownList>` of
`decimal_id:Display Name` lines, and all three tables in circulation carry byte-identical copies of
it. That is the source here. It is second-hand and this script says so rather than presenting the
result as read from the game.

# The format, read off the file

`<CheatEntry>` whose `<Description>` is `"ItemDropdown"`, containing a `<DropDownList>` whose text
body is one `id:name` per line. Entries the table's author never identified are spelled `UNKNOWN`
and are DROPPED here -- an id with no name is useless to a name lookup, and keeping it would let a
build that names nothing match something.

# What this refuses to do

It does not deduplicate by name. Several distinct ids share a display name in this game -- notably
the four armour pieces of a set differ only by their last digit while their names differ, but rings
carry `+1`/`+2`/`+3` tiers whose names DO differ, and a handful of genuine collisions remain. The
consumer decides what to do about a collision; this script reports the count so the consumer knows
there is something to decide.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

#: The entry whose dropdown carries the join. Same description in every table seen.
ENTRY = '<Description>"ItemDropdown"'

#: One `id:name` line. The id is decimal; the name runs to end of line.
LINE = re.compile(r"^\s*(\d+):(.*?)\s*$")

#: What the table's author writes for an id they never identified.
UNKNOWN = "UNKNOWN"


def dropdown(text: str) -> str:
    """The body of the ItemDropdown's `<DropDownList>`, or raise."""
    entry = text.find(ENTRY)
    if entry < 0:
        raise SystemExit(f"no {ENTRY!r} in this table -- is it a DS2 table?")
    start = text.find("<DropDownList", entry)
    end = text.find("</DropDownList>", start)
    if start < 0 or end < 0:
        raise SystemExit("the ItemDropdown entry has no <DropDownList> body")
    return text[text.index(">", start) + 1 : end]


def parse(body: str) -> list[tuple[int, str]]:
    """Every `(id, name)` with a real name, in file order."""
    out: list[tuple[int, str]] = []
    for line in body.splitlines():
        match = LINE.match(line)
        if not match:
            continue
        name = match.group(2)
        if not name or name == UNKNOWN:
            continue
        out.append((int(match.group(1)), name))
    if not out:
        raise SystemExit("the dropdown parsed to zero named rows -- the format changed")
    return out


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("table", type=Path, help="a Cheat Engine .CT carrying ItemDropdown")
    parser.add_argument("--out", type=Path, help="write TSV here instead of stdout")
    args = parser.parse_args(argv[1:])

    text = args.table.read_text(encoding="utf-8", errors="replace")
    rows = parse(dropdown(text))

    names: dict[str, int] = {}
    collisions = 0
    for item_id, name in rows:
        if name in names:
            collisions += 1
        names[name] = item_id

    lines = [
        "# DARK SOULS II item ids. id\\tname, tab-separated, in table order.",
        f"# Extracted by scripts/ds2-item-ids.py from {args.table.name}",
        "# SECOND-HAND: these are a community Cheat Engine table's id->name join, not a read of",
        "# the game's own ItemParam + FMG. Treat a surprising id as suspect before trusting it.",
        f"# {len(rows)} named rows, {collisions} duplicate display name(s).",
    ]
    lines += [f"{item_id}\t{name}" for item_id, name in rows]
    out = "\n".join(lines) + "\n"

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(out, encoding="utf-8")
        print(f"{len(rows)} rows -> {args.out} ({collisions} duplicate names)")
    else:
        sys.stdout.write(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
