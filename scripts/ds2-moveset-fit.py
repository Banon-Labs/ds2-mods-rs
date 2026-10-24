#!/usr/bin/env python3
"""Fit the moveset rank's weights to what real builds carry, and check them on brackets left out.

    for s in 40,70 100,125 155 200 250 838; do
      python3 scripts/ds2-builds-recommend.py --best-weapons No_Infusion --sweep $s --rank moveset \\
          --top 1000 --json --jobs 16 > DIR/s_$s.json
    done
    python3 scripts/ds2-moveset-fit.py DIR/s_*.json              # print the fit and its held-out check
    python3 scripts/ds2-moveset-fit.py DIR/s_*.json -o WEIGHTS   # also write the weights

WHAT IS FITTED. Every --best-weapons row ranked by moveset carries `families` (moveset_metrics):
per family its best engagement's rate and the five factors that multiply it. The features are the
log of each family's raw rate (rate over its factors) and the log of each R1 factor; a family a
weapon lacks gets the bracket's lowest raw rate. The target is the weapon's share of the bracket's
real builds (adoption), as a percentile within its bracket, so brackets of 14 and 10,038 builds
weigh alike. Ridge regression, standardized features. A log-linear score is a product of the
features raised to the fitted weights: the hand-set factors of moveset_metrics are the special
case where every weight is 1.

THE CHECK. Leave one bracket out: fit on the others, rank the left-out bracket's weapons, and take
Spearman's rho against its shares -- printed beside the same rho for the R1 window score
(`damage_with_status`) and the hand-set moveset score, the two ranks this one replaces. Weights
that do not beat the window score held out are not worth writing.

Published builds are not match logs: a weapon's share also carries looks, availability and
fashion, so even a right model leaves most of the rank unexplained (er-mods-rs's best moveset rho
was 0.195).
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import sys
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
FAMILIES = ["r1", "r2", "move", "jump"]
FACTORS = ["f_reach", "f_stagger", "f_trade", "f_punish", "f_stamina"]
FEATURES = [f"log_{f}" for f in FAMILIES] + [f"log_r1_{f}" for f in FACTORS]
RIDGE = 1.0


def _load_rec():
    spec = importlib.util.spec_from_file_location("ds2_builds_recommend", HERE / "ds2-builds-recommend.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ds2_builds_recommend"] = mod
    spec.loader.exec_module(mod)
    return mod


def raw_rate(f: dict) -> float:
    """A family's (damage + status) / commit before its factors: `raw` when the row has it, else
    its rate over the factors (rows ranked before the exponents existed, every exponent 1)."""
    return f["raw"] if "raw" in f else f["rate"] / math.prod(f[x] or 1.0 for x in FACTORS)


def features(rows: list[dict]) -> np.ndarray:
    raws = {fam: [raw_rate(r["metrics"]["families"][fam]) for r in rows if fam in r["metrics"]["families"]]
            for fam in FAMILIES}
    floor = {fam: min(v) if v else 1.0 for fam, v in raws.items()}
    out = []
    for r in rows:
        fams = r["metrics"]["families"]
        x = [math.log(max(1e-6, raw_rate(fams[fam]) if fam in fams else floor[fam])) for fam in FAMILIES]
        r1 = fams.get("r1") or {}
        x += [math.log(r1.get(f) or 1.0) for f in FACTORS]
        out.append(x)
    return np.array(out)


def percentile(v: list[float]) -> np.ndarray:
    v = np.array(v)
    return np.array([(np.sum(v < x) + 0.5 * (np.sum(v == x) - 1)) / max(1, len(v) - 1) for x in v])


def ridge(X: np.ndarray, y: np.ndarray, lam: float = RIDGE) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    mu, sd = X.mean(0), X.std(0)
    sd[sd == 0] = 1.0
    Z = (X - mu) / sd
    w = np.linalg.solve(Z.T @ Z + lam * np.eye(Z.shape[1]), Z.T @ (y - y.mean()))
    return w, mu, sd


#: Families whose weight the fit chooses (0..1, FAMILY_GRID); r1 stays at 1, the unit the rate is in.
WEIGHTED_FAMILIES = ["r2", "move", "jump"]
FAMILY_GRID = [0.0, 0.25, 0.5, 0.75, 1.0]


def anchored(rows: list[dict], exps: list[float]) -> list[float]:
    """moveset_metrics' score with each factor raised to its exponent in `exps` (FACTORS order,
    then one weight per WEIGHTED_FAMILIES): per family s = weight x raw rate x prod(factor ^
    exponent), the weapon sum(s^2) / sum(s). Exponents and weights of 1 are the hand-set score; the
    raw rate keeps exponent 1, so the score stays HP per second."""
    weight = dict(zip(WEIGHTED_FAMILIES, exps[len(FACTORS):]))
    out = []
    for r in rows:
        s = [weight.get(fam, 1.0) * raw_rate(f) * math.prod((f[x] or 1.0) ** e for x, e in zip(FACTORS, exps))
             for fam, f in r["metrics"]["families"].items()]
        s = [x for x in s if x > 0]
        out.append(sum(x * x for x in s) / sum(s) if s else 0.0)
    return out


#: Exponents tried per factor. None below 0: each factor is an advantage by its mechanics (more
#: reach, more staggers, trades won, punishes landed, stamina left), so a fit may find one
#: irrelevant but not invert it -- unconstrained, adoption pushes stagger to -2, which is players
#: preferring light weapons, not staggering being bad.
ANCHOR_GRID = [round(0.25 * i, 2) for i in range(25)]


def fit_anchored(rec, train: list[tuple[list[dict], list[float]]]) -> list[float]:
    """The FACTORS exponents (ANCHOR_GRID, coordinate ascent from all 1s) that maximise the mean
    Spearman rho of `anchored` against adoption share over the `train` brackets."""
    exps = [1.0] * (len(FACTORS) + len(WEIGHTED_FAMILIES))

    def mean_rho(e):
        return float(np.mean([rec.spearman(anchored(rows, e), share) or 0.0 for rows, share in train]))

    best = mean_rho(exps)
    for _ in range(4):
        moved = False
        for i in range(len(exps)):
            for v in ANCHOR_GRID if i < len(FACTORS) else FAMILY_GRID:
                trial = exps[:i] + [v] + exps[i + 1:]
                got = mean_rho(trial)
                if got > best + 1e-9:
                    best, exps, moved = got, trial, True
        if not moved:
            break
    return exps


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("files", nargs="+", type=Path, help="--best-weapons --rank moveset --json outputs")
    ap.add_argument("-o", "--out", type=Path, help="write the weights fitted on every bracket here")
    a = ap.parse_args()
    rec = _load_rec()
    sp, mm = rec.dump_site_tables(rec.CACHE / "site-tables")
    data = rec.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    rec.apply_regulation(data)
    corpus, _ = rec.load_corpus(data)
    adopted = rec.adoption(data, corpus)
    brackets = {}
    for f in a.files:
        for sl, rows in json.loads(f.read_text()).items():
            rows = [r for r in rows if (r.get("metrics") or {}).get("families")]
            n, _, by = adopted.get(rec.sl_bracket(int(sl)), [0, None, {}])
            if n < 50 or len(rows) < 20:
                print(f"SL {sl}: {n} builds, {len(rows)} rows -- left out (too few to rank)")
                continue
            share = [by[r["key"]] / n for r in rows]
            brackets[int(sl)] = (rows, features(rows), percentile(share), share, n)
    if len(brackets) < 3:
        raise SystemExit("need at least 3 brackets with builds to hold one out")
    print(f"{'held out':>9} {'builds':>7} {'ridge':>7} {'anchor':>7} {'window':>7} {'hand':>7}  anchored exponents")
    held = []
    for sl, (rows, X, y, share, n) in sorted(brackets.items()):
        rest = [b for s, b in brackets.items() if s != sl]
        w, mu, sd = ridge(np.vstack([b[1] for b in rest]), np.concatenate([b[2] for b in rest]))
        rho = rec.spearman(list(((X - mu) / sd) @ w), share)
        e = fit_anchored(rec, [(b[0], b[3]) for b in rest])
        anc = rec.spearman(anchored(rows, e), share)
        win = rec.spearman([r["metrics"].get("damage_with_status", 0.0) for r in rows], share)
        hand = rec.spearman([r["score"] for r in rows], share)
        held.append((rho, anc, win, hand))
        print(f"{sl:>9} {n:>7} {rho:+7.3f} {anc:+7.3f} {win:+7.3f} {hand:+7.3f}  {e}")
    m = np.mean(held, axis=0)
    print(f"{'mean':>9} {'':>7} {m[0]:+7.3f} {m[1]:+7.3f} {m[2]:+7.3f} {m[3]:+7.3f}")
    anchor = fit_anchored(rec, [(b[0], b[3]) for b in brackets.values()])
    print("\nanchored exponents fitted on every bracket: "
          + ", ".join(f"{f} {e:+.2f}" for f, e in zip(FACTORS + WEIGHTED_FAMILIES, anchor)))
    w, mu, sd = ridge(np.vstack([b[1] for b in brackets.values()]), np.concatenate([b[2] for b in brackets.values()]))
    exps = w / sd  # the exponent on each feature's raw value: score = prod(feature ^ exponent)
    print("\nexponents fitted on every bracket (score = product of feature ^ exponent):")
    for name, e in zip(FEATURES, exps):
        print(f"  {name:16} {e:+.3f}")
    if a.out:
        a.out.write_text(json.dumps({"factor_exponents": dict(zip(FACTORS, anchor)),
                                     "family_weights": dict(zip(WEIGHTED_FAMILIES, anchor[len(FACTORS):])),
                                     "ridge": {"features": FEATURES, "exponents": [round(float(e), 4) for e in exps]},
                                     "held_out_rho": {"ridge": round(float(m[0]), 4), "anchored": round(float(m[1]), 4),
                                                      "window": round(float(m[2]), 4), "hand": round(float(m[3]), 4)},
                                     "brackets": sorted(brackets)}, indent=1) + "\n")
        print(f"wrote {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
