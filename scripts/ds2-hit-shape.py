#!/usr/bin/env python3
"""Print the hitbox shape of a weapon's attacks, read out of the regulation.

    python3 scripts/ds2-hit-shape.py Dagger "Giant Warrior Club"
    python3 scripts/ds2-hit-shape.py --slots Normal Dagger      # only the R1/R2 slots ("Normal"/"Strong")
    python3 scripts/ds2-hit-shape.py --survey                   # every PlayerDamageParam shape value's count

REGULATION: WeaponParam.damageHitRadiusScale / damageHitLengthScale (paramdef display names
"HIT 半径 倍率" / "HIT 長さ 倍率") and, per hit of each attack slot's WeaponAttackMotionParam damage
row, PlayerDamageParam hitDummyPolyType (発生源タイプ, source type), hitDummyPolyId, hitModelType
(形状タイプ, shape type), radius (半径[m]) and length (長さ[m]). The names are Smithbox's DS2S paramdefs;
what the game does with them is not read here. Read-only: prints to stdout.
"""

from __future__ import annotations

import argparse
import importlib.util
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent


def extractor():
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    return ex


SHAPE = ("hitDummyPolyType", "hitDummyPolyId", "hitModelType", "radius", "length", "hitDistance")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("weapons", nargs="*", help="itemname.fmg weapon names")
    ap.add_argument("--slots", default="Normal", help="substring an attack slot name must contain")
    ap.add_argument("--survey", action="store_true", help="count every shape value over all PlayerDamageParam rows")
    a = ap.parse_args()
    ex = extractor()
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS)
    W, A, M, P = d["WeaponParam"], d["WeaponActionCategoryParam"], d["WeaponAttackMotionParam"], d["PlayerDamageParam"]
    if a.survey:
        for f in ("hitDummyPolyType", "hitDummyPolyId", "hitModelType"):
            print(f, sorted(Counter(r[f] for r in P.values()).items()))
        return 0
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    by_name = {}
    for wid in W:
        by_name.setdefault(names.get(wid, "?").lower(), wid)
    for nm in a.weapons:
        wid = by_name.get(nm.lower())
        if not wid:
            print(f"{nm}: no weapon by that name", file=sys.stderr)
            continue
        w = W[wid]
        print(f"{nm} ({wid}) model {w['weaponModelId']} radiusScale {w['damageHitRadiusScale']} "
              f"lengthScale {w['damageHitLengthScale']} actcat {w['weaponActionCategoryId']}")
        cat = A.get(str(w["weaponActionCategoryId"])) or {}
        for k, mid in cat.items():
            if not k.startswith("atkId") or a.slots not in k or mid < 10:
                continue
            m = M.get(str(mid))
            if not m:
                continue
            for n in (1, 2, 3):
                did = m[f"damageId0{n}"]
                p = P.get(str(did)) if did else None
                seen = set()
                while p and did not in seen:  # the row, then its childDamage chain
                    seen.add(did)
                    print(f"  {k[5:]:28} anim {m['attackAnim']} dmg{n} {did}: "
                          + " ".join(f"{f}={p[f]}" for f in SHAPE)
                          + f" group={p['damageGroup']} child={p['childDamage']}")
                    did = p["childDamage"]
                    p = P.get(str(did)) if did else None
    return 0


if __name__ == "__main__":
    sys.exit(main())
