#!/usr/bin/env python3
"""Re-rank --best-weapons --json rows by damage x EHP instead of damage alone.

    python3 scripts/ds2-rerank-ehp.py --level 20 --skip Raw DIR [DIR ...]

The --best-weapons score is the moveset rate; --spend ehp only decides how each row's free points
are spent. This ranks every row of every <infusion>.json in each DIR by score x max HP / mean hit
taken, the same product ehp_value spends by (hit_points, hit_taken against own_defense at the
stats the rings give).
"""

from __future__ import annotations

import argparse
import glob
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("dirs", nargs="+")
    ap.add_argument("--level", default="20")
    ap.add_argument("--skip", default="", help="infusions to leave out, comma-separated")
    ap.add_argument("--top", type=int, default=12)
    a = ap.parse_args()
    rec = _load("rec", "ds2-builds-recommend.py")
    bb = _load("bb", "ds2-bracket-builds.py")
    data = bb.load_data(rec)
    corpus, _ = rec.load_corpus(data)
    threat = rec.threat_hit(data, corpus)
    skip = {"builds", *filter(None, a.skip.split(","))}
    rows = []
    for d in a.dirs:
        for f in sorted(glob.glob(f"{d}/*.json")):
            inf = Path(f).stem
            if inf in skip:
                continue
            for r in json.loads(Path(f).read_text()).get(a.level, []):
                eff = rec.gear_stats(data, r["stats"], r["rings"])
                hp = rec.hit_points(data, eff)
                taken = rec.hit_taken(threat, rec.own_defense(data, eff, r["rings"]))
                rows.append({"ehp_score": r["score"] * hp / taken, "hp": hp, "hit_taken": taken,
                             "dir": d, "infusion": inf, **r})
    rows.sort(key=lambda x: -x["ehp_score"])
    for x in rows[:a.top]:
        print(f"{x['ehp_score']:9.0f}  dmg {x['score']:7.1f}  hp {x['hp']:4.0f}  hit {x['hit_taken']:5.1f}  "
              f"{x['label']:10} {x['infusion']:11} {x['weapon']:24} {x['class']:9} {Path(x['dir']).name}")
    print(json.dumps({k: rows[0][k] for k in ("weapon", "infusion", "class", "stats", "rings")}))


if __name__ == "__main__":
    main()
