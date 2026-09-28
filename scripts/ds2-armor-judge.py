#!/usr/bin/env python3
"""Judge an armor set against the build corpus: how often each piece is worn, at this soul level,
and whether the set's strongest defenses are the ones builds at this level usually lead with.

    scripts/ds2-armor-judge.py --sl 35 --head "Manikin Mask" --chest "Alonne Captain Armor" \\
        --hands "Steel Gauntlets" --legs "Blood-Stained Skirt" [--band 10]

The corpus and item tables are ds2-builds-recommend.py's own (`load_corpus`, the SoulsPlanner
tables), so a piece is judged against exactly the builds the Build Recommender learns from.
Defense numbers are the planner's per-piece values; they rank sets against each other and are not
the in-game total, which adds the stat-derived part.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import statistics
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("rec", HERE / "ds2-builds-recommend.py")
rec = importlib.util.module_from_spec(spec)
sys.modules["rec"] = rec
spec.loader.exec_module(rec)

DEFENSES = ["physicalDEF", "strikeDEF", "slashDEF", "thrustDEF", "magicDEF", "fireDEF", "lightningDEF", "darkDEF"]
ELEMENTS = ["magicDEF", "fireDEF", "lightningDEF", "darkDEF"]
BARE = {"", "naked", "none"}


def piece_key(data, slot: str, name: str) -> str:
    for key, row in data.armor[slot].items():
        if row.get("name") == name or key == name:
            return key
    raise SystemExit(f"no {slot} named {name!r} in the planner's table")


def set_defense(data, keys: dict[str, str]) -> dict[str, float]:
    out = {d: 0.0 for d in DEFENSES}
    for slot, key in keys.items():
        row = data.armor[slot].get(key)
        if not row:
            continue
        for d in DEFENSES:
            out[d] += float(row.get(d) or 0)
    return out


def percentile(value: float, values: list[float]) -> float:
    return 100.0 * sum(v <= value for v in values) / len(values) if values else 0.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sl", type=int, required=True)
    ap.add_argument("--band", type=int, default=10, help="builds within +/- this many soul levels (default 10)")
    for slot in rec.ARMOR_SLOTS:
        ap.add_argument(f"--{slot}", required=True)
    a = ap.parse_args()

    tables = rec.CACHE / "site-tables"
    sp_json, mm_json = rec.dump_site_tables(tables)
    data = rec.Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    corpus, _ = rec.load_corpus(data)
    mine = {slot: piece_key(data, slot, getattr(a, slot)) for slot in rec.ARMOR_SLOTS}
    band = [b for b in corpus if abs(rec.soul_level(data, b) - a.sl) <= a.band]
    print(f"corpus {len(corpus)} builds; SL {a.sl - a.band}-{a.sl + a.band}: {len(band)} builds\n")

    print("piece usage              all builds        this SL band      rank in slot (band)")
    for i, slot in enumerate(rec.ARMOR_SLOTS):
        worn_all = Counter(b.armor[i] for b in corpus if len(b.armor) > i)
        worn_band = Counter(b.armor[i] for b in band if len(b.armor) > i)
        key = mine[slot]
        ranked = [k for k, _ in worn_band.most_common() if k.lower() not in BARE]
        rank = ranked.index(key) + 1 if key in ranked else None
        top = ", ".join(f"{data.armor[slot].get(k, {}).get('name', k)} {n / len(band):.0%}"
                        for k, n in worn_band.most_common(3))
        print(f"  {slot:5} {data.armor[slot][key]['name']:24} "
              f"{worn_all[key] / len(corpus):6.1%}           {worn_band[key] / max(len(band), 1):6.1%}"
              f"            {rank if rank else 'unworn'} of {len(ranked)}   (top: {top})")

    ours = set_defense(data, mine)
    theirs = [set_defense(data, dict(zip(rec.ARMOR_SLOTS, b.armor))) for b in band if len(b.armor) == 4]
    print("\ndefense      yours   band median   your percentile in band")
    for d in DEFENSES:
        vals = [t[d] for t in theirs]
        print(f"  {d[:-3]:11} {ours[d]:6.0f}   {statistics.median(vals):8.0f}      {percentile(ours[d], vals):5.0f}%")

    def lead(t):
        return max(ELEMENTS, key=lambda d: t[d])[:-3]

    leads = Counter(lead(t) for t in theirs)
    print(f"\nyour highest elemental defense: {lead(ours)}; "
          f"builds in the band lead with: "
          + ", ".join(f"{k} {n / len(theirs):.0%}" for k, n in leads.most_common()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
