#!/usr/bin/env python3
"""The optimizer's ADP against real builds': optimize_build at SL 155 for a random sample of the
corpus's SL 126-155 builds with at most one attunement slot, each at its own class and first
right-hand weapon that is not a shield or catalyst (MugenMonkey's unknown infusion '?' read as
No_Infusion), beside the ADP the sample's own builds have.

    python3 scripts/ds2-agility-cases.py                  # 20 builds, seed 0
    python3 scripts/ds2-agility-cases.py --n 20 --seed 1  # a second, disjoint-by-seed sample
    python3 scripts/ds2-agility-cases.py --floors none    # the optimizer without the floors only
    python3 scripts/ds2-agility-cases.py --corpus-only    # the corpus's ADP/ATT/AGL distribution
    python3 scripts/ds2-agility-cases.py --module scripts/<copy of the recommender>.py  # before

Loads the corpus as main() does (Data, apply_regulation, load_corpus), so it takes ~20-60 s, and
each optimize_build a few seconds more."""
import argparse
import importlib.util
import json
import random
import statistics
import sys
from collections import Counter
from pathlib import Path


def load(module: Path):
    spec = importlib.util.spec_from_file_location("recommend", module)
    r = importlib.util.module_from_spec(spec)
    sys.modules["recommend"] = r
    spec.loader.exec_module(r)
    sp, mm = r.dump_site_tables(r.CACHE / "site-tables")
    data = r.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    r.apply_regulation(data)
    corpus, _ = r.load_corpus(data)
    return r, data, corpus


def weapon_of(r, data, b):
    for (w, inf), slot in zip(b.hands, r.HAND_SLOTS):
        if slot.startswith("rh") and w not in r.EMPTY and w in data.weapons \
                and not data.weapons[w].get("isShield") and not r.CATALYST.search(w):
            return w, "No_Infusion" if inf == "?" else inf
    return None


def summary(xs: list[int]) -> str:
    return (f"n {len(xs)} median {statistics.median(xs):g} mean {statistics.mean(xs):.1f} "
            f"min {min(xs)} max {max(xs)}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--n", type=int, default=20)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--sl", type=int, default=155)
    ap.add_argument("--floors", choices=["both", "none", "floors"], default="both")
    ap.add_argument("--module", type=Path, default=Path(__file__).with_name("ds2-builds-recommend.py"))
    ap.add_argument("--corpus-only", action="store_true")
    a = ap.parse_args()
    r, data, corpus = load(a.module)
    lo, hi = r.SL_BRACKETS[r.sl_bracket(a.sl)]
    pool = [b for b in corpus if lo <= r.soul_level(data, b) <= hi
            and r.slots_of(data, r.effective(data, b)) <= 1 and weapon_of(r, data, b)]
    idx = lambda st: (3 * st["adaptability"] + st["attunement"]) // 4
    print(f"corpus SL {lo}-{hi}, <=1 slot, a right-hand weapon: {len(pool)} builds")
    print("  ADP", summary([b.stats["adaptability"] for b in pool]))
    print("  ATT", summary([b.stats["attunement"] for b in pool]))
    print("  VIG", summary([b.stats["vigor"] for b in pool]))
    print("  AGL", summary([r.agility(b.stats["adaptability"], b.stats["attunement"]) for b in pool]))
    top = Counter(r.agility(b.stats["adaptability"], b.stats["attunement"]) for b in pool).most_common(8)
    print("  most common AGL:", ", ".join(f"{g} x{c}" for g, c in top))
    print("  agility index", summary([idx(b.stats) for b in pool]))
    if a.corpus_only:
        return 0
    sample = random.Random(a.seed).sample(pool, min(a.n, len(pool)))
    modes = {"both": (False, True), "none": (False,), "floors": (True,)}[a.floors]
    got = {uf: [] for uf in modes}
    for b in sample:
        w, inf = weapon_of(r, data, b)
        line = [f"{b.cls:10s} {w} {inf} | own ADP {b.stats['adaptability']:2d} ATT {b.stats['attunement']:2d}"]
        for uf in modes:
            best, _ = r.optimize_build(data, corpus, w, inf, a.sl, "damage", only_class=b.cls, use_floors=uf)
            if best is None:
                line.append(f"{'floors' if uf else 'none'}: no build")
                continue
            st = best[3]
            got[uf].append(st["adaptability"])
            line.append(f"{'floors' if uf else 'none'}: VIG {st['vigor']} ADP {st['adaptability']} "
                        f"ATT {st['attunement']} AGL {r.agility(st['adaptability'], st['attunement'])}")
        print(" | ".join(line), flush=True)
    print("sample's own ADP", summary([b.stats["adaptability"] for b in sample]))
    for uf in modes:
        if got[uf]:
            print(f"optimizer ADP, {'floors' if uf else 'no floors'}:", summary(got[uf]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
