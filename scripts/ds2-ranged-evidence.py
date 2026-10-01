#!/usr/bin/env python3
"""Print the regulation rows behind a ranged attack: every launcher (bow, greatbow, crossbow) and
every piece of ammunition, with the rows the recommender's ranged scoring reads.

Read-only. Decodes the regulation as scripts/ds2-builds-recommend.py does (apply_regulation) and
prints, per subcommand:

  ammo       ArrowParam rows joined to itemname.fmg, with the PlayerDamageParam rows its light/heavy
             damage ids name, per set, and its WeaponParam attack terms if it had a WeaponParam row
             (no ammunition does)
  launchers  every WeaponParam row whose WeaponTypeParam shootCategory is not 0, with its type row's
             shootCategory/weaponType, damageScale and its R1/R2 and two-handed L2 attack motions
             (damage and bullet ids)
  scales     every named weapon whose WeaponTypeParam rightDamageScale is not 1.0
  baseless   every named weapon with an elemental rate (or add rate) and coefficient but no base
  row ID     one row of any decoded param (--param NAME)

docs/DS2-DPS-MECHANICS.md "Ranged attack" is what these rows were read for.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
PARAMS = {"WeaponParam": "WEAPON_PARAM", "WeaponReinforceParam": "WEAPON_REINFORCE_PARAM",
          "WeaponStatsAffectParam": "WEAPON_STATS_AFFECT_PARAM", "WeaponTypeParam": "WEAPON_TYPE_PARAM",
          "WeaponActionCategoryParam": "WEAPON_ACTION_CATEGORY_PARAM",
          "WeaponAttackMotionParam": "WEAPON_ATTACK_MOTION_PARAM", "ArrowParam": "ARROW_PARAM",
          "PlayerDamageParam": "DAMAGE_PARAM", "BulletParam": "BULLET_PARAM", "ItemParam": "ITEM_PARAM"}
DTYPE = {0: "phys", 1: "magic", 2: "light", 3: "fire", 4: "dark", 6: "poison", 7: "bleed"}


def load():
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, PARAMS)
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    return d, names


def dmg_row(d: dict, rid) -> str:
    r = d["PlayerDamageParam"].get(str(rid))
    if not r:
        return f"{rid}:-"
    adds = [f"{DTYPE.get(r[f'damageType0{n}'], r[f'damageType0{n}'])}+{r[f'damage0{n}']}"
            for n in (1, 2, 3) if r[f"damage0{n}"]]
    return f"{rid}:rate={r['damageRate']} lower={r['damageLower']} [{' '.join(adds)}]"


def terms(d: dict, wp: dict) -> str:
    r = d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
    if not r:
        return "no WRP row"
    a = d["WeaponStatsAffectParam"].get(str(r["statsAffectId"])) or {}
    lv = r["maxLevel"]
    base = {k: r[f"maximum{k}"] for k in ("Physical", "Magic", "Thunder", "Fire", "Dark", "Poison", "Bleeding")
            if r[f"maximum{k}"]}
    rates = {k: r[k] for k in ("physicalRate", "magicRate", "thunderRate", "fireRate", "darkRate", "poisonRate",
                               "bleedingRate") if r[k]}
    adds = {k: r[k] for k in r if k.startswith("add") and r[k]}
    coef = {k: a.get(f"{k}{lv}") for k in ("physicalByStrength", "physicalByDexterity", "magic", "thunder", "fire",
                                            "dark") if a.get(f"{k}{lv}")}
    return (f"WRP {wp['weaponReinforceId']} maxLevel={lv} max={base} rates={rates} adds={adds} "
            f"SA {r['statsAffectId']} coef={coef}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("what", choices=["ammo", "launchers", "scales", "baseless", "row"])
    ap.add_argument("id", nargs="?")
    ap.add_argument("--param", default="WeaponParam")
    a = ap.parse_args()
    d, names = load()
    if a.what == "row":
        print(json.dumps(d[a.param].get(a.id), indent=1))
        return 0
    if a.what == "baseless":
        stems = {"Magic": "magic", "Thunder": "thunder", "Fire": "fire", "Dark": "dark"}
        for wid, wp in sorted(d["WeaponParam"].items(), key=lambda x: int(x[0])):
            r = d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
            name = names.get(wid, "?")
            if not r or name[:1] in "?<":
                continue
            for stem, sa in stems.items():
                if r[f"maximum{stem}"]:
                    continue
                coef = [(off, d["WeaponStatsAffectParam"].get(str(r["statsAffectId"] + off), {}).get(f"{sa}{r['maxLevel']}"))
                        for off in range(10)]
                coef = [(o, c) for o, c in coef if c]
                if coef and (r[f"{sa}Rate"] or r[f"add{stem}Rate"]):
                    print(f"{wid} {name!r} {stem}: maximum 0, rate {r[f'{sa}Rate']} add {r[f'add{stem}Rate']}, "
                          f"coefficient by statsAffect offset {coef}")
        return 0
    if a.what == "scales":
        for wid, wp in sorted(d["WeaponParam"].items(), key=lambda x: int(x[0])):
            t = d["WeaponTypeParam"].get(str(wp["weaponTypeId"]))
            if t and t["rightDamageScale"] != 1.0 and names.get(wid, "?")[:1] not in "?<":
                print(f"{wid} {names[wid]!r} rightDamageScale={t['rightDamageScale']} shootCategory={t['shootCategory']}")
        return 0
    if a.what == "ammo":
        for aid, ar in sorted(d["ArrowParam"].items(), key=lambda x: int(x[0])):
            wp = d["WeaponParam"].get(aid)
            print(f"{aid} {names.get(aid, '?')!r} arrowType={ar['arrowType']} menu phys={ar['menuPhysicalAttack']} "
                  f"magic={ar['menuMagicAttack']} light={ar['menuThunderAttack']} fire={ar['menufireAttack']} "
                  f"dark={ar['menudarkAttack']} poison={ar['menuPoisonAttack']} bleed={ar['menuBleedAttack']}")
            for i in range(4):
                lb, hb, ld, hd = (ar[f"lightBulletId{i}"], ar[f"heavyBulletId{i}"], ar[f"lightDamageId{i}"],
                                  ar[f"heavyDamageId{i}"])
                if lb or hb or ld or hd:
                    print(f"   [{i}] light bullet {lb} dmg {dmg_row(d, ld)} | heavy bullet {hb} dmg {dmg_row(d, hd)}")
            if wp:
                print(f"   WeaponParam type={wp['weaponTypeId']} damageScale={wp['damageScale']} {terms(d, wp)}")
        return 0
    for wid, wp in sorted(d["WeaponParam"].items(), key=lambda x: int(x[0])):
        t = d["WeaponTypeParam"].get(str(wp["weaponTypeId"]))
        if not t or not t["shootCategory"]:
            continue
        ac = d["WeaponActionCategoryParam"].get(str(wp["weaponActionCategoryId"])) or {}
        print(f"{wid} {names.get(wid, '?')!r} type={wp['weaponTypeId']} shootCategory={t['shootCategory']} "
              f"weaponType={t['weaponType']} damageScale={wp['damageScale']} rightScale={t['rightDamageScale']} "
              f"req S{wp['requiredStrength']} D{wp['requiredDexterity']} I{wp['requiredIntelligence']} "
              f"F{wp['requiredFaith']}")
        print(f"   {terms(d, wp)}")
        for slot in ("Single1HandNormal1st", "Single1HandStrong1st", "Single2HandNormal1st", "Single2HandStrong1st",
                     "OppositeSingle2HandStrong"):
            mid = ac.get("atkId" + slot)
            m = d["WeaponAttackMotionParam"].get(str(mid))
            if m:
                print(f"   {slot}: motion {mid} anim {m['attackAnim']} spd {m['startPlaySpeed']}/{m['endPlaySpeed']} "
                      f"dmg {[dmg_row(d, m[f'damageId0{n}']) for n in (1, 2, 3) if m[f'damageId0{n}']]} "
                      f"bullets {[(m[f'bulletId0{n}'], dmg_row(d, m[f'bulletDamageId0{n}'])) for n in (1, 2, 3) if m[f'bulletId0{n}']]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
