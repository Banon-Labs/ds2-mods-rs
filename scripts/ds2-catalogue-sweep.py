#!/usr/bin/env python3
"""Find the names in real soulsplanner builds that `items.tsv` cannot resolve.

    scripts/ds2-catalogue-sweep.py --range 1 60
    scripts/ds2-catalogue-sweep.py 253 1 42

# Why this exists

`crates/ds2-build-import-core/data/items.tsv` is SECOND-HAND -- a community Cheat Engine table's
id->name join rather than a read of the game's own `ItemParam` plus its FMG names. So the honest
question is not "is it correct" (nothing here can answer that) but "what does it MISS", and the
cheapest way to ask is to run real builds through it.

The alternative was finding out the way we found out about `Estus_Flask` and the six slots named
`""`: a player pressed the row in a live game and read the log. That costs a press, and in this mod
a press is not free -- it raises soul memory, which the engine has no path to lower. This costs an
HTTP GET.

# What it reports, and the three are different problems

* **UNKNOWN** -- no catalogue row carries this name. Either the catalogue is missing an item or the
  planner spells it differently. Each one is an item a player silently does not receive.
* **AMBIGUOUS** -- several ids carry the name. `ds2-build-import` grants the lowest and logs every
  candidate; these are listed so the choice is reviewable rather than merely logged at runtime.
* **EMPTY** -- a placeholder for an unfilled slot. Counted, never reported as a problem. They are
  the majority of every build and the reason a raw "unresolved" count means nothing.

# It is polite to the planner

One request per build, a second apart, and every response is cached under the scratchpad so a rerun
costs nothing. This reads public pages the same way a browser does; there is no reason to be
expensive about it.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
import urllib.error
from pathlib import Path

# The fetcher and parser already exist in `ds2-soulsplanner.py`; this only adds the catalogue join
# over the top. That filename has a hyphen in it, which is not a legal module name, so it is
# executed rather than imported -- the alternative is duplicating the page parsing, and two copies
# of it would disagree the first time the planner changed shape.
_PLANNER = Path(__file__).resolve().parent / "ds2-soulsplanner.py"
_ns: dict = {}
exec(compile(_PLANNER.read_text(), str(_PLANNER), "exec"), _ns)  # noqa: S102
fetch, body_script, parse_build = _ns["fetch"], _ns["body_script"], _ns["parse_build"]

REPO = Path(__file__).resolve().parents[1]
CATALOGUE = REPO / "crates/ds2-build-import-core/data/items.tsv"
CACHE = Path(
    "/tmp/claude-1000/-home-banon-projects-ds2-mods-rs"
    "/96ac580b-a25b-45c5-be18-4a82d12760c9/scratchpad/soulsplanner-cache"
)

#: Mirrors `ds2_build_import_core::items::EMPTY_SLOTS`, compared after normalisation.
EMPTY = {"", "nospell", "noitem", "noring", "noinfusion", "barefists", "none"}

#: Mirrors `ds2_build_import_core::items::normalise`: letters and digits only, lowercased.
NON_ALNUM = re.compile(r"[^A-Za-z0-9]")


def normalise(name: str) -> str:
    return NON_ALNUM.sub("", name).lower()


def catalogue() -> dict[str, list[int]]:
    out: dict[str, list[int]] = {}
    for line in CATALOGUE.read_text(encoding="utf-8").splitlines():
        if line.startswith("#") or not line.strip():
            continue
        head, _, name = line.partition("\t")
        try:
            out.setdefault(normalise(name), []).append(int(head))
        except ValueError:
            continue
    return out


def build_names(build: dict) -> list[tuple[str, str]]:
    """Every `(slot_kind, name)` a build names, with the weapon/infusion pairing undone.

    WEAPONS ARE INTERLEAVED `name, infusion, name, infusion, ...` across the six slots. Reading that
    list flat puts `Dark` and `Bleed` through the item lookup, where they resolve to nothing and
    read as a missing item rather than as an infusion -- the exact false positive this tool exists
    to avoid producing.
    """
    out: list[tuple[str, str]] = []
    for kind in ("armor", "rings", "spells", "items"):
        out += [(kind, name) for name in build.get(kind, [])]
    weapons = build.get("weapons", [])
    out += [("weapons", name) for name in weapons[0::2]]
    out += [("infusions", name) for name in weapons[1::2]]
    return out


def load(build_id: int, delay: float) -> dict | None:
    CACHE.mkdir(parents=True, exist_ok=True)
    cached = CACHE / f"{build_id}.json"
    if cached.exists():
        return json.loads(cached.read_text())
    try:
        build = parse_build(body_script(fetch(build_id)))
    except ValueError as error:
        # MOST OF THESE ARE NOT FAILURES. A build id nobody has used returns a page with no
        # `<body><script>` at all, which is indistinguishable here from a page whose shape changed
        # -- and reporting a nonexistent build as a parse error made 31 of the first 40 ids look
        # like a broken tool. Sweeping a range means asking about ids that do not exist.
        print(f"  build {build_id}: no build here ({error})", file=sys.stderr)
        return None
    except (urllib.error.URLError, OSError) as error:
        print(f"  build {build_id}: REQUEST FAILED: {error}", file=sys.stderr)
        return None
    finally:
        time.sleep(delay)
    cached.write_text(json.dumps(build))
    return build


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("ids", type=int, nargs="*", help="build ids to sweep")
    parser.add_argument("--range", type=int, nargs=2, metavar=("FIRST", "LAST"))
    parser.add_argument("--delay", type=float, default=1.0, help="seconds between requests")
    args = parser.parse_args(argv[1:])

    ids = list(args.ids)
    if args.range:
        ids += list(range(args.range[0], args.range[1] + 1))
    if not ids:
        parser.error("give some build ids, or --range FIRST LAST")

    names = catalogue()
    unknown: dict[str, set[str]] = {}
    ambiguous: dict[str, list[int]] = {}
    seen = resolved = empty = builds = 0

    for build_id in ids:
        build = load(build_id, args.delay)
        if build is None:
            continue
        builds += 1
        for kind, raw in build_names(build):
            # Infusions are not items and are never looked up.
            if kind == "infusions":
                continue
            seen += 1
            key = normalise(raw)
            if key in EMPTY:
                empty += 1
                continue
            ids_for = names.get(key)
            if not ids_for:
                unknown.setdefault(raw, set()).add(f"{kind}#{build_id}")
            elif len(ids_for) > 1:
                ambiguous[raw] = ids_for
                resolved += 1
            else:
                resolved += 1

    print(f"\n{builds} builds, {seen} named slots: {resolved} resolved, {empty} empty, "
          f"{len(unknown)} distinct names unresolved")

    if ambiguous:
        print(f"\nAMBIGUOUS ({len(ambiguous)}) -- the lowest id is granted, every candidate logged:")
        for name, candidates in sorted(ambiguous.items()):
            print(f"  {name:40s} {candidates}")

    if unknown:
        print(f"\nUNKNOWN ({len(unknown)}) -- a player silently does not receive these:")
        for name, where in sorted(unknown.items()):
            sample = ", ".join(sorted(where)[:4])
            print(f"  {name:40s} {sample}")
        return 1
    print("\nEvery named item in every build resolved.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
