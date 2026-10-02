#!/usr/bin/env python3
"""Which infusions the game lets each weapon take, read from the shipped regulation.

    python3 scripts/ds2-infusion-allowed.py            # id, name, letters, one row per weapon

Read-only. The gate is `WeaponParam.weaponReinforceId` -> `WeaponReinforceParam.attrSpec`
(属性強化可否, "infusion allowed") -> a `CustomAttrSpecParam` row of one-bit flags. Letters use
`crates/ds2-build-recommender-core/data/weapons.tsv`'s legend so the two can be diffed:

    physical N   fire F   magic M   thunder L   dark D   poison P   bleed B
    physicalByCrude R (Raw)   physicalByEnchanted E   physicalByAbyss U (Mundane)

`physicalByAbyss` = Mundane is by elimination: it is the tenth flag and Mundane the tenth
infusion. Every other flag's name says its infusion.
"""
from __future__ import annotations

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

FLAGS = [("physical", "N"), ("fire", "F"), ("magic", "M"), ("thunder", "L"), ("dark", "D"),
         ("poison", "P"), ("bleed", "B"), ("physicalByCrude", "R"),
         ("physicalByEnchanted", "E"), ("physicalByAbyss", "U")]


def main() -> int:
    import importlib.util
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    sys.modules["ds2attacks"] = ex
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
        "WeaponParam": "WEAPON_PARAM", "WeaponReinforceParam": "WEAPON_REINFORCE_PARAM",
        "CustomAttrSpecParam": "CUSTOM_ATTR_SPEC_PARAM"})
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    print("#id\tname\tinfusions\treinforce\tattrSpec")
    for wid, wp in d["WeaponParam"].items():
        rid = str(wp["weaponReinforceId"])
        r = d["WeaponReinforceParam"].get(rid)
        spec_row = r and d["CustomAttrSpecParam"].get(str(r["attrSpec"]))
        letters = "".join(c for f, c in FLAGS if spec_row and spec_row.get(f)) if spec_row else "?"
        print(f"{wid}\t{names.get(wid, '<no ItemParam row>')}\t{letters}\t{rid}\t"
              f"{r['attrSpec'] if r else '?'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
