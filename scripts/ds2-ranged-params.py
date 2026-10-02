#!/usr/bin/env python3
"""Dump the regulation rows behind ranged weapons and their ammunition. Read-only.

    python3 scripts/ds2-ranged-params.py members              # regulation member names
    python3 scripts/ds2-ranged-params.py find <text>          # ItemParam ids whose name contains text
    python3 scripts/ds2-ranged-params.py weapon <id>...       # WeaponParam + reinforce/affect/type rows
    python3 scripts/ds2-ranged-params.py arrow <id>...        # ArrowParam row + its bullets and damages
    python3 scripts/ds2-ranged-params.py row <member> <def> <id>...  # any param row, nonzero fields
"""
from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def load():
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    sys.modules["ds2attacks"] = ex
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    return ex, reg


def show(title: str, row: dict | None, nonzero: bool = True) -> None:
    print(f"== {title}")
    if row is None:
        print("   <no row>")
        return
    for k, v in row.items():
        if nonzero and not v:
            continue
        print(f"   {k} = {v}")


def main() -> int:
    ex, reg = load()
    cmd, args = sys.argv[1], sys.argv[2:]
    if cmd == "members":
        for m in sorted(reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)):
            print(m)
        return 0
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    if cmd == "find":
        for i, n in names.items():
            if args[0].lower() in n.lower():
                print(i, n)
        return 0
    if cmd == "row":
        d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {args[0]: args[1]})
        for rid in args[2:]:
            show(f"{args[0]} {rid}", d[args[0]].get(rid))
        return 0
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
        "WeaponParam": "WEAPON_PARAM", "WeaponReinforceParam": "WEAPON_REINFORCE_PARAM",
        "WeaponStatsAffectParam": "WEAPON_STATS_AFFECT_PARAM", "WeaponTypeParam": "WEAPON_TYPE_PARAM",
        "ArrowParam": "ARROW_PARAM", "BulletParam": "BULLET_PARAM",
        "PlayerDamageParam": "DAMAGE_PARAM", "SystemBulletParam": "BULLET_PARAM",
        "SystemDamageParam": "DAMAGE_PARAM", "CustomAttrSpecParam": "CUSTOM_ATTR_SPEC_PARAM"})
    if cmd == "weapon":
        for wid in args:
            wp = d["WeaponParam"].get(wid)
            show(f"WeaponParam {wid} {names.get(wid)}", wp)
            if not wp:
                continue
            r = d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
            show(f"WeaponReinforceParam {wp['weaponReinforceId']}", r)
            if r:
                show(f"WeaponStatsAffectParam {r['statsAffectId']}", d["WeaponStatsAffectParam"].get(str(r["statsAffectId"])))
                show(f"CustomAttrSpecParam {r['attrSpec']}", d["CustomAttrSpecParam"].get(str(r["attrSpec"])))
            show(f"WeaponTypeParam {wp['weaponTypeId']}", d["WeaponTypeParam"].get(str(wp["weaponTypeId"])))
    elif cmd == "arrow":
        for aid in args:
            a = d["ArrowParam"].get(aid)
            show(f"ArrowParam {aid} {names.get(aid)}", a)
            if not a:
                continue
            seen = set()
            for k, v in a.items():
                if v and ("BulletId" in k or "DamageId" in k) and ("Bullet" in k, v) not in seen:
                    seen.add(("Bullet" in k, v))
                    member = "BulletParam" if "Bullet" in k else "PlayerDamageParam"
                    sysm = "SystemBulletParam" if "Bullet" in k else "SystemDamageParam"
                    row = d[member].get(str(v))
                    show(f"{member} {v} (from {k})" if row else f"{sysm} {v} (from {k})",
                         row or d[sysm].get(str(v)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
