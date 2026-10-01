#!/usr/bin/env python3
"""Print the regulation rows behind an attack's stamina cost, per named weapon.

Read-only. Decodes the regulation as scripts/ds2-builds-recommend.py does (apply_regulation) and
prints, per weapon whose itemname.fmg name contains one of the given words (case-insensitive):
WeaponParam meleeAttackBaseCost, rangedAttackBaseCost and costCategoryId, the WeaponStaminaCostParam
row that id names (its 1H/2H R1 first/second and R2 multipliers), and base x multiplier per slot in
the regulation's own units (the menu shows a tenth of it; the recommender keeps the regulation's
units on both sides of a ratio, so the tenth cancels). `--categories` prints every cost category
row and how many weapons use it; `--chr` the player's ChrParam stamina fields.

    python3 scripts/ds2-stamina-evidence.py "giant warrior club" dagger uchigatana "long bow" "light crossbow"

docs/DS2-DPS-MECHANICS.md "Stamina" is what these rows were read for.
"""
from __future__ import annotations

import argparse
import importlib.util
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
PARAMS = {"WeaponParam": "WEAPON_PARAM", "WeaponStaminaCostParam": "WEAPON_STAMINA_COST_PARAM",
          "ChrParam": "CHR_PARAM"}
SLOTS = ("Nnormal1st1H", "normal2nd1H", "strong1st1H", "normal1st2H", "normal2nd2H", "strong1st2H")


def load():
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, PARAMS)
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    return d, names


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("words", nargs="*", help="name fragments to match (case-insensitive)")
    ap.add_argument("--categories", action="store_true", help="every WeaponStaminaCostParam row and its use count")
    ap.add_argument("--chr", action="store_true", help="the player's ChrParam stamina fields (row 100)")
    a = ap.parse_args()
    d, names = load()
    costs = d["WeaponStaminaCostParam"]
    if a.categories:
        use = Counter(w["costCategoryId"] for w in d["WeaponParam"].values())
        for cid, row in sorted(costs.items(), key=lambda kv: int(kv[0])):
            print(f"category {cid} ({use.get(int(cid), 0)} WeaponParam rows): "
                  + " ".join(f"{s}={row[s]:g}" for s in SLOTS))
    if a.chr:
        row = d["ChrParam"].get("100") or {}
        print("ChrParam 100: " + " ".join(f"{k}={v:g}" for k, v in row.items() if "stamina" in k.lower()))
    for wid, w in sorted(d["WeaponParam"].items(), key=lambda kv: int(kv[0])):
        name = names.get(wid, "")
        if not a.words or name.startswith("<") or not any(x.lower() in name.lower() for x in a.words):
            continue
        row = costs.get(str(w["costCategoryId"])) or {}
        per = " ".join(f"{s}={w['meleeAttackBaseCost'] * row.get(s, 0):.1f}" for s in SLOTS)
        print(f"{wid} {name}: meleeAttackBaseCost={w['meleeAttackBaseCost']:g} "
              f"rangedAttackBaseCost={w['rangedAttackBaseCost']:g} costCategoryId={w['costCategoryId']} -> {per}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
