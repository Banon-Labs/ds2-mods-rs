#!/usr/bin/env python3
"""Whether a generated build should trade damage stats for VIT to wear a heavier, higher-poise set.

Read-only. Generates the build as scripts/ds2-builds-recommend.py --generate does, then for its own
armour and for each `--armor` set (head/chest/hands/legs names, '/'-separated) prints:

  * poise: the set's ArmorParam.strong plus the worn rings' (armor_poise);
  * what that poise survives among the SL bracket's counter-hits (bracket_poise: each bracket
    build's melee weapon's 1H R1 first hit, as (poise damage, armorBreak)): the share that stagger
    in one hit, and in two hits back to back (no poise regeneration between them);
  * the VIT the set needs to stay under 70% load with the heaviest weapon the build lists and its
    rings (generate_armor's budget), the points that moves, and the damage per hit against the
    bracket defender (objective_value "damage") when those points come out of STR, DEX, or the
    two damage stats in proportion -- each at the weapon's own requirements or the move is refused.

Poise pays only at thresholds: a set worth wearing turns a counter-hit's one-hit stagger into a
two-hit one, or a two-hit into none. A poise gain that moves no counter-hit across a threshold buys
nothing, whatever the number says.

    python3 scripts/ds2-poise-tradeoff.py Black_Flamestone_Dagger --sl 120 --class warrior \
        --armor "Gyrm Warrior Greathelm/Havel's Armor/Havel's Gauntlets/Havel's Leggings"
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load_recommender():
    spec = importlib.util.spec_from_file_location("ds2recommend", HERE / "ds2-builds-recommend.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ds2recommend"] = mod
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("weapon", help="SoulsPlanner key or name, as --generate takes it")
    ap.add_argument("--infusion", default="No_Infusion")
    ap.add_argument("--sl", type=int, required=True)
    ap.add_argument("--class", dest="cls", help="starting class, as --generate's --class")
    ap.add_argument("--grip", default="two", choices=["two", "one"])
    ap.add_argument("--armor", action="append", default=[], help="a set to compare: Head/Chest/Hands/Legs names")
    a = ap.parse_args()

    R = load_recommender()
    sp_json, mm_json = R.dump_site_tables(R.CACHE / "site-tables")
    data = R.Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    R.apply_regulation(data)
    corpus, _ = R.load_corpus(data)
    weapon = data.sp_key.get(R.norm(a.weapon))
    if weapon not in data.weapons:
        ap.error(f"unknown weapon {a.weapon!r}")
    g = R.generate_build(data, corpus, weapon, a.infusion, a.sl, grip=a.grip, only_class=a.cls)
    if g is None:
        print("no build")
        return 2

    stats = dict(g["stats"])
    ring_key = {v["name"]: k for k, v in reversed(list(data.rings.items()))}
    rings = list(dict.fromkeys(ring_key[n] for n in g["rings"][:12] if n in ring_key))[:4]
    by_name = {}
    for key, w in data.weapons.items():
        by_name.setdefault(w["name"], key)
    weight = lambda k: data.weapons.get(k, {}).get("weight", 0)
    listed = [by_name[n] for n, _, _ in g["weapons_1h"] + g["weapons_2h_only"]]
    heaviest = max([weapon, *listed], key=weight)
    ring_weight = sum(data.rings[r].get("weight", 0) for r in rings)
    attacks = R.load_attacks(data)
    br = R.bracket_poise(data, corpus, a.sl, attacks)
    counters = br["counters"]
    dfn, _ = R.defender_defense(data, corpus, a.sl)
    req = R._grip_req(data, weapon, a.grip == "two")

    def dmg(st):
        return R.objective_value(data, weapon, a.infusion, R.gear_stats(data, st, rings), "damage", dfn, rings)

    def set_weight(keys):
        return sum((data.armor[s].get(p) or {}).get("weight", 0) for s, p in zip(R.ARMOR_SLOTS, keys))

    def vit_needed(keys):
        need = set_weight(keys) + weight(heaviest) + ring_weight
        for vit in range(1, 100):
            if R.max_load(data, {**stats, "vitality": vit}, rings) * R.EQUIP_CAP >= need:
                return vit
        return None

    def take(points, frm):
        st = dict(stats)
        for _ in range(points):
            # one point at a time from whichever listed stat is furthest above its requirement
            s = max(frm, key=lambda s_: st[s_] - req.get(s_, 0))
            if st[s] - 1 < req.get(s, 0):
                return None
            st[s] -= 1
        return st

    base = dmg(stats)
    print(f"build: {g['class']} SL {a.sl} {data.weapons[weapon]['name']} ({a.infusion}) "
          f"{'two' if g['two_handed'] else 'one'}-handed; "
          + " ".join(f"{R.LABEL[s]} {stats[s]}" for s in R.STATS))
    print(f"rings: {', '.join(data.rings[r]['name'] for r in rings)}; heaviest listed weapon: "
          f"{data.weapons[heaviest]['name']} ({weight(heaviest)})")
    print(f"bracket counter-hits: {len(counters)} from {br['n']} SL {a.sl} builds; "
          f"bracket mean poise {br['poise']:.1f}; damage per hit now {base:.1f} vs the bracket defender")
    sets = [("generated", R.armor_keys(data, g["armor"]))]
    sets += [(s, R.armor_keys(data, s.split("/"))) for s in a.armor]
    for label, keys in sets:
        poise = R.armor_poise(data, keys, rings)
        one = sum(1 for pd, ab in counters if ab or pd >= poise) / max(len(counters), 1)
        two = sum(1 for pd, ab in counters if ab or 2 * pd >= poise) / max(len(counters), 1)
        vit = vit_needed(keys)
        names = " / ".join((data.armor[s].get(p) or {}).get("name", p) for s, p in zip(R.ARMOR_SLOTS, keys))
        print(f"\n{label}: {names}")
        print(f"  poise {poise:.1f}, weight {set_weight(keys):.1f}; counter-hits that stagger in one hit "
              f"{one:.1%}, within two hits {two:.1%}")
        if vit is None:
            print("  no VIT carries it under 70% with the heaviest listed weapon")
            continue
        moved = max(0, vit - stats["vitality"])
        print(f"  needs VIT {vit} (has {stats['vitality']}): {moved} points moved")
        if not moved:
            continue
        for frm in (["strength"], ["dexterity"], ["strength", "dexterity"]):
            st = take(moved, frm)
            if st is None:
                print(f"  from {'+'.join(R.LABEL[s] for s in frm)}: refused, under the weapon's requirement")
                continue
            d = dmg(st)
            print(f"  from {'+'.join(R.LABEL[s] for s in frm)}: damage per hit {d:.1f} "
                  f"({(d - base) / base:+.1%})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
