#!/usr/bin/env python3
"""optimize_build's stats for a fixed set of SL 155 builds, with and without the floors: the
before/after evidence an optimizer change quotes in its commit message.

    python3 scripts/ds2-optimizer-cases.py            # the default five cases
    python3 scripts/ds2-optimizer-cases.py --sl 120

Loads the corpus as main() does (Data, apply_regulation, load_corpus), so it takes ~20-60 s."""
import argparse
import importlib.util
import json
import sys
from pathlib import Path

CASES = [("Uchigatana", "Lightning", "bandit"), ("Heide_Lance", "No_Infusion", "bandit"),
         ("Black_Dragon_Greataxe", "Raw", "knight"), ("Moonlight_Greatsword", "Enchanted", "sorcerer"),
         ("Ivory_King_Ultra_Greatsword", "No_Infusion", "swordsman")]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--sl", type=int, default=155)
    a = ap.parse_args()
    spec = importlib.util.spec_from_file_location("recommend", Path(__file__).with_name("ds2-builds-recommend.py"))
    r = importlib.util.module_from_spec(spec)
    sys.modules["recommend"] = r
    spec.loader.exec_module(r)
    sp, mm = r.dump_site_tables(r.CACHE / "site-tables")
    data = r.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    r.apply_regulation(data)
    corpus, _ = r.load_corpus(data)
    for w, inf, cls in CASES:
        for uf in (True, False):
            best, _ = r.optimize_build(data, corpus, w, inf, a.sl, "damage", only_class=cls, use_floors=uf)
            head = f"{w} {inf} {cls} {'floors' if uf else 'no-floors'}:"
            if best is None:
                print(head, "no build")
                continue
            st = best[3]
            print(head, " ".join(f"{r.LABEL[s]} {st[s]}" for s in r.STATS),
                  f"| dmg {best[0]:.1f}" + (f" rings {','.join(best[4])}" if best[4] else ""), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
