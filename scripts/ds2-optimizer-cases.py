#!/usr/bin/env python3
"""optimize_build's stats for a fixed set of SL 155 builds, with and without the floors: the
before/after evidence an optimizer change quotes in its commit message.

    python3 scripts/ds2-optimizer-cases.py            # the default five cases
    python3 scripts/ds2-optimizer-cases.py --sl 120
    python3 scripts/ds2-optimizer-cases.py --ref origin/main   # the same cases at another commit
    python3 scripts/ds2-optimizer-cases.py --corpus 20 --ref origin/main   # real builds beside both

`--ref` runs ds2-builds-recommend.py as it is at that git ref (read with `git show`, never written
to disk). `--corpus N` replaces the fixed cases with N random corpus builds (seed `--seed`) at SL
`--corpus-sl` with at most one attunement slot, each optimized as its own class with its first
right-hand weapon that is not a shield or catalyst (MugenMonkey's '?' infusion read as
No_Infusion), and prints the real build's ADP/AGL beside the optimizer's at the worktree and, with
`--ref`, at that ref.

Loads the corpus as main() does (Data, apply_regulation, load_corpus), so it takes ~20-60 s per
script loaded."""
import argparse
import importlib.util
import json
import random
import statistics
import subprocess
import sys
from pathlib import Path

CASES = [("Uchigatana", "Lightning", "bandit"), ("Heide_Lance", "No_Infusion", "bandit"),
         ("Black_Dragon_Greataxe", "Raw", "knight"), ("Moonlight_Greatsword", "Enchanted", "sorcerer"),
         ("Ivory_King_Ultra_Greatsword", "No_Infusion", "swordsman")]
SCRIPT = Path(__file__).with_name("ds2-builds-recommend.py")


def load(name: str, ref: str | None = None):
    """The recommender module (the worktree's, or the one at git `ref`) with its data and corpus."""
    if ref is None:
        spec = importlib.util.spec_from_file_location(name, SCRIPT)
        r = importlib.util.module_from_spec(spec)
        sys.modules[name] = r
        spec.loader.exec_module(r)
    else:
        rel = SCRIPT.relative_to(SCRIPT.parent.parent).as_posix()
        src = subprocess.run(["git", "-C", str(SCRIPT.parent), "show", f"{ref}:{rel}"], check=True,
                             capture_output=True, text=True).stdout
        r = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader=None))
        r.__file__ = str(SCRIPT)
        sys.modules[name] = r
        exec(compile(src, f"{ref}:{rel}", "exec"), r.__dict__)
    sp, mm = r.dump_site_tables(r.CACHE / "site-tables")
    data = r.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    r.apply_regulation(data)
    corpus, _ = r.load_corpus(data)
    return r, data, corpus


def agl_of(r, data, best) -> int:
    """Displayed AGL of an optimize_build result, at the stats its worn rings give."""
    e = r.gear_stats(data, best[3], best[4]) if best[4] else best[3]
    return r.agility(e["adaptability"], e["attunement"])


def cases(r, data, corpus, sl: int) -> None:
    for w, inf, cls in CASES:
        for uf in (True, False):
            best, _ = r.optimize_build(data, corpus, w, inf, sl, "damage", only_class=cls, use_floors=uf)
            head = f"{w} {inf} {cls} {'floors' if uf else 'no-floors'}:"
            if best is None:
                print(head, "no build")
                continue
            st = best[3]
            print(head, " ".join(f"{r.LABEL[s]} {st[s]}" for s in r.STATS), f"AGL {agl_of(r, data, best)}",
                  f"| dmg {best[0]:.1f}" + (f" rings {','.join(best[4])}" if best[4] else ""), flush=True)


def corpus_sample(mods: list, n: int, seed: int, lo: int, hi: int) -> None:
    r, data, corpus = mods[0][1]
    pool = []
    for b in corpus:
        sl = r.soul_level(data, b)
        if not lo <= sl <= hi or b.cls not in data.classes:
            continue
        if r.slots_of(data, r.effective(data, b), [x for x in b.rings if x in data.rings]) > 1:
            continue
        rh = [(w, i) for k, (w, i) in enumerate(b.hands) if k % 2 == 1 and w not in r.EMPTY
              and w in data.weapons and not data.weapons[w].get("isShield") and not r.CATALYST.search(w)]
        if not rh:
            continue
        w, inf = rh[0]
        if inf == "?":
            inf = "No_Infusion"
        if inf in data.weapons[w]["infusions"]:
            pool.append((b, sl, w, inf))
    print(f"{len(pool)} builds at SL {lo}-{hi} with <=1 slot; {n} drawn with seed {seed}")
    print("class | weapon | infusion | SL | real ADP/AGL | " + " | ".join(f"{t} ADP/AGL" for t, _ in mods))
    real, got = [], {t: [] for t, _ in mods}
    for b, sl, w, inf in random.Random(seed).sample(pool, min(n, len(pool))):
        adp = b.stats["adaptability"]
        real.append((adp, r.agility(adp, b.stats["attunement"])))
        cells = []
        for t, (m, d, c) in mods:
            best, _ = m.optimize_build(d, c, w, inf, sl, "damage", only_class=b.cls)
            if best is None:
                cells.append("no build")
                continue
            got[t].append((best[3]["adaptability"], agl_of(m, d, best)))
            cells.append(f"{got[t][-1][0]}/{got[t][-1][1]}")
        print(f"{b.cls} | {w} | {inf} | {sl} | {real[-1][0]}/{real[-1][1]} | " + " | ".join(cells), flush=True)
    med = lambda xs, i: statistics.median(x[i] for x in xs) if xs else None
    print(f"median real ADP {med(real, 0)} AGL {med(real, 1)}; "
          + "; ".join(f"{t} ADP {med(v, 0)} AGL {med(v, 1)}" for t, v in got.items()))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--sl", type=int, default=155)
    ap.add_argument("--ref", help="also (with --corpus) or instead run the script at this git ref")
    ap.add_argument("--corpus", type=int, metavar="N", help="N random corpus builds instead of the cases")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--corpus-sl", default="126-155", help="the corpus builds' SL range, lo-hi")
    a = ap.parse_args()
    if a.corpus is None:
        cases(*load("recommend", a.ref), a.sl)
        return 0
    mods = [("worktree", load("recommend"))]
    if a.ref:
        mods.append((a.ref, load("recommend_ref", a.ref)))
    lo, hi = (int(x) for x in a.corpus_sl.split("-"))
    corpus_sample(mods, a.corpus, a.seed, lo, hi)
    return 0


if __name__ == "__main__":
    sys.exit(main())
