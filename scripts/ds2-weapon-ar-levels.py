#!/usr/bin/env python3
"""A weapon's attack rating per infusion at chosen upgrade levels, and the fewest stat points that
bring it to a target attack, with scripts/ds2-builds-recommend.py's own AR model.

    python3 scripts/ds2-weapon-ar-levels.py "Pate's Spear"
    python3 scripts/ds2-weapon-ar-levels.py "Pate's Spear" --levels 0,5 --target physical=167,physical=195
    python3 scripts/ds2-weapon-ar-levels.py "Pate's Spear" --grip one --base 10

The recommender scores every weapon at full upgrade only. This rebuilds its per-infusion terms at
any level the same way `infusion_attack` builds them at maxLevel, with the one level-dependent
change the executable makes (docs/DS2-DPS-MECHANICS.md "Base, rates and the sum"):

    base  = lerp(WRP.minimum<T>, WRP.maximum<T>, level / maxLevel) * WSA.baseValueScale * rate
    coef  = WSA.<stat><level> * rate                (0x140397c40: row + 8 + level * 0x24)

and then hands that row to the recommender's `attack_rating` unchanged, so the stat bonus tables,
the STR+DEX physical sum, fire's INT+FTH, dark's min(INT, FTH), Enchanted and Mundane are all its
code, not a copy. The lack-of-stats penalty is not modelled (attack_rating does not either): a
stat line below the wield requirement is never searched.

`--target TYPE=N` (repeatable): for each infusion and level, the fewest added points over the
requirement (STR halved for --grip two) in STR, DEX, INT and FTH that make every listed type's AR
at least N, by exhaustive search over the stats that type reads. Every other stat is --base.
Read-only: prints to stdout.
"""

from __future__ import annotations

import argparse
import importlib.util
import itertools
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load_recommend():
    spec = importlib.util.spec_from_file_location("recommend", HERE / "ds2-builds-recommend.py")
    r = importlib.util.module_from_spec(spec)
    sys.modules["recommend"] = r
    spec.loader.exec_module(r)
    return r


def level_terms(r, d: dict, wrp: dict, inf: str, level: int) -> tuple[dict, dict, dict] | None:
    """`(atk, atkScale, rate)` of WeaponReinforceParam row `wrp` infused with `inf` at `level`,
    keyed as the recommender keys them. Mirrors recommend.infusion_attack with maxLevel replaced
    by `level` and maximum<T> by the lerp."""
    a = d["WeaponStatsAffectParam"].get(str(wrp["statsAffectId"] + r.INFUSION_MOVE[inf][2]))
    if not a:
        return None
    rates, top = r.infused_rates(wrp, inf), wrp["maxLevel"]
    t = min(max(level / top, 0.0), 1.0) if top else 1.0
    atk, sc = {}, {}
    for k, (slot, stem, stems) in r.DAMAGE_TERMS.items():
        rate = rates[slot] / 100
        base = wrp[f"minimum{stem}"] + (wrp[f"maximum{stem}"] - wrp[f"minimum{stem}"]) * t
        if base * rate:
            atk[k] = base * a["baseValueScale"] * rate
        for s, sa in stems.items():
            if a[f"{sa}{level}"] * rate:
                sc[s] = a[f"{sa}{level}"] * rate
    if inf == "Enchanted":
        sc["magic"] = a[f"physicalByEnchant{level}"] * rates[0] / 100
    if inf == "Mundane":
        sc.pop("strength", None), sc.pop("dexterity", None)
    rate = {k: rates[slot] / 100 for k, (slot, _, _) in r.DAMAGE_TERMS.items() if rates[slot]}
    return atk, sc, rate


#: which searched stats each damage type's AR reads in recommend.attack_rating
READS = {"physical": ("strength", "dexterity"), "magic": ("intelligence",), "lightning": ("faith",),
         "fire": ("intelligence", "faith"), "dark": ("intelligence", "faith")}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    ap.add_argument("weapon", help="weapon name or SoulsPlanner key, e.g. \"Pate's Spear\"")
    ap.add_argument("--levels", default=None, help="comma-separated upgrade levels (default 0 and max)")
    ap.add_argument("--target", action="append", default=[], help="TYPE=N, e.g. physical=167 (repeatable)")
    ap.add_argument("--grip", choices=["one", "two"], default="one", help="two halves the STR requirement")
    ap.add_argument("--base", type=int, default=10, help="every stat not searched or required (default 10)")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    r = load_recommend()
    sp_json, mm_json = r.dump_site_tables(r.CACHE / "site-tables")
    data = r.Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    print(r.apply_regulation(data)[:120] + " ...", file=sys.stderr)
    key = a.weapon if a.weapon in data.weapons else data.key_by_name.get(a.weapon) \
        or next((k for k, w in data.weapons.items() if r.norm(w.get("name", k)) == r.norm(a.weapon)), None)
    if key not in data.weapons:
        ap.error(f"unknown weapon {a.weapon!r}")
    w = data.weapons[key]

    spec =importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
        "WeaponParam": "WEAPON_PARAM", "WeaponReinforceParam": "WEAPON_REINFORCE_PARAM",
        "WeaponStatsAffectParam": "WEAPON_STATS_AFFECT_PARAM"})
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    wid = next(i for i in d["WeaponParam"] if r.norm(names.get(i, "")) == r.norm(w.get("name", key)))
    wp = d["WeaponParam"][wid]
    wrp = d["WeaponReinforceParam"][str(wp["weaponReinforceId"])]
    levels = [int(x) for x in a.levels.split(",")] if a.levels else [0, wrp["maxLevel"]]
    targets = [(t.split("=")[0], float(t.split("=")[1])) for t in a.target]

    req = {"strength": wp["requiredStrength"], "dexterity": wp["requiredDexterity"],
           "intelligence": wp["requiredIntelligence"], "faith": wp["requiredFaith"]}
    if a.grip == "two":
        req["strength"] //= 2
    floor = {s: max(a.base, v) for s, v in req.items()}
    eff0 = {s: a.base for s in r.STATS} | floor

    out = {"weapon": w.get("name", key), "weaponId": int(wid), "reinforceId": wp["weaponReinforceId"],
           "maxLevel": wrp["maxLevel"], "require": req, "grip": a.grip, "base": a.base, "rows": []}
    for inf in w.get("infusions") or {}:
        for lv in levels:
            terms = level_terms(r, d, wrp, inf, lv)
            if not terms:
                continue
            row = {"atk": terms[0], "atkScale": dict(terms[1]), "rate": terms[2]}
            if inf == "Mundane":
                row["atkScale"]["modifier"] = (w["infusions"][inf].get("atkScale") or {}).get("modifier", 1)
            saved = w["infusions"][inf]
            w["infusions"][inf] = row
            try:
                ar = r.attack_rating(data, key, inf, eff0)
                res = {"infusion": inf, "level": lv, "ar_at_requirement": ar}
                for typ, n in targets:
                    stats = READS[typ] if inf != "Enchanted" or typ != "physical" else ("strength", "dexterity", "intelligence")
                    best = None
                    for vals in itertools.product(*(range(floor[s], 100) for s in stats)):
                        cost = sum(v - floor[s] for s, v in zip(stats, vals))
                        if best and cost >= best[0]:
                            continue
                        eff = eff0 | dict(zip(stats, vals))
                        if r.attack_rating(data, key, inf, eff).get(typ, 0) >= n:
                            best = (cost, dict(zip(stats, vals)))
                    res[f"min_for_{typ}>={n:g}"] = ({"added": best[0], **best[1]} if best else None)
                out["rows"].append(res)
            finally:
                w["infusions"][inf] = saved
    if a.json:
        print(json.dumps(out))
        return 0
    print(f"{out['weapon']} (WeaponParam {wid}, reinforce {out['reinforceId']}, +{out['maxLevel']} max), "
          f"require {req} ({a.grip}-handed), other stats {a.base}")
    for res in out["rows"]:
        line = f"  {res['infusion']:<12} +{res['level']}  AR {res['ar_at_requirement']}"
        for k, v in res.items():
            if k.startswith("min_for_"):
                line += f"  | {k[8:]}: " + ("unreachable" if v is None else
                                            " ".join(f"{s[:3].upper()} {x}" for s, x in v.items() if s != "added")
                                            + f" (+{v['added']})")
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
