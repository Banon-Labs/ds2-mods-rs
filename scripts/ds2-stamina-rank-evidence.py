#!/usr/bin/env python3
"""How each stamina-aware score of a --best-weapons row ranks against what real builds carry.

    python3 scripts/ds2-builds-recommend.py --best-weapons standard --sweep 40,100,150,200 \\
        --window 1.5 --json --top 1000 --jobs 12 > DIR/after.json
    python3 scripts/ds2-stamina-rank-evidence.py DIR/before.json DIR/after.json

Per file and SL, Spearman's rho against the bracket's melee-weapon shares (adoption, as
--best-weapons prints it) for: `value`, the optimizer's own one-hit objective (what a window of 0
ranks by -- with r1_sustain, one hit times the share of a stamina window the build keeps its R1
paying for); `score`, the R1 hits in the window; and the stamina metrics --rank offers,
`damage_per_stamina` (per-stamina) and `bar_damage` (bar). Read-only: it ranks rows already
written. Published builds are not match logs, so a rho here is a weak check, not a fit.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCORES = ["value", "score", "damage_per_stamina", "bar_damage"]


def _load_rec():
    spec = importlib.util.spec_from_file_location("ds2_builds_recommend", HERE / "ds2-builds-recommend.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ds2_builds_recommend"] = mod
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("files", nargs="+", type=Path, help="--best-weapons --json outputs")
    ap.add_argument("--weapons", default="Dagger,Fume_Ultra_Greatsword",
                    help="weapon keys whose rank, value and END to print per file and SL")
    a = ap.parse_args()
    rec = _load_rec()
    sp, mm = rec.dump_site_tables(rec.CACHE / "site-tables")
    data = rec.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    rec.apply_regulation(data)
    corpus, _ = rec.load_corpus(data)
    adopted = rec.adoption(data, corpus)
    print(f"{'file':>12} {'SL':>4} {'rows':>5} " + " ".join(f"{s:>18}" for s in SCORES))
    for f in a.files:
        for sl, rows in json.loads(f.read_text()).items():
            n, _, by = adopted.get(rec.sl_bracket(int(sl)), [0, None, {}])
            share = [by[r["key"]] / n if n else 0.0 for r in rows]
            got = []
            for s in SCORES:
                xs = [r.get(s, (r.get("metrics") or {}).get(s)) for r in rows]
                keep = [(x, y) for x, y in zip(xs, share) if x is not None]
                rho = rec.spearman([x for x, _ in keep], [y for _, y in keep])
                got.append("n/a" if rho is None else f"{rho:+.3f}")
            print(f"{f.stem:>12} {sl:>4} {len(rows):>5} " + " ".join(f"{g:>18}" for g in got))
            by_value = sorted(rows, key=lambda r: -r["value"])
            for key in a.weapons.split(","):
                at = next((i for i, r in enumerate(by_value) if r["key"] == key), None)
                if at is None:
                    continue
                r = by_value[at]
                print(f"{'':>12} {'':>4} {key}: #{at + 1} of {len(rows)} by value {r['value']:.1f}, "
                      f"END {r['stats']['endurance']}, window {r['score']:.1f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
