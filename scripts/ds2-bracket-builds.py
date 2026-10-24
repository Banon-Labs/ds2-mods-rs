#!/usr/bin/env python3
"""The recommender's best build at the top level of every Seamless Co-op invasion bracket, as the
JSON `scripts/ds2-sl2-builds.py` writes into a save.

    # 1+2. rank every weapon of every infusion at each level by the moveset rate, then pick the
    # top row per level and generate its whole build (hours: run it detached)
    setsid nohup scripts/ds2-bracket-sweep.sh DIR > DIR/run.log 2>&1 &
    # or step 2 alone over existing --best-weapons --json files
    python3 scripts/ds2-bracket-builds.py --best-dir DIR -o builds.json
    # check the max-HP arithmetic against a live character (scripts/frida/build-check.js)
    python3 scripts/ds2-bracket-builds.py --max-hp "VIG=10,END=8,VIT=8,ATT=6,STR=12,DEX=14,INT=6,FTH=6,ADP=3" \\
        --rings "Life Ring+3,Chloranthy Ring+2"

THE LEVELS are each bracket's highest: Seamless 0.0.5's brackets by soul level are 1-20, 21-40,
41-70, 71-100, 101-125, 126-155, 156-200, 201-250 and 251+ (measured 2026-10-21 with
scripts/frida/bracket-scout.js; `crates/ds2-invadable-hosts/src/census.rs` BRACKET_FIRST_LEVEL),
and the last one's highest is the game's ceiling, 838.

"BEST" is the recommender's own measure, nothing added here: `--best-weapons` ranks each weapon of
an infusion at the build `optimize_build` makes for it, and `scripts/ds2-bracket-sweep.sh` ranks
by `--rank moveset`: the whole moveset's PvP rate in HP per second (`moveset_metrics`: R1/R2
chains, roll/run/backstep/jump attacks, each engagement's damage and status per committed second,
times reach, trade and punish factors whose weights `scripts/ds2-moveset-fit.py` fitted to what
real builds carry) against the bracket's average defender, which answers each attack with its best
counter ring. Files ranked by the R1 window (`--rank window`) work too. Scores
compare across infusions, so the top row over all ten files is the level's weapon, and
`generate_build` makes its whole build: class, stats, rings, armour under 70% load.

WHAT GOES ON THE BODY: the primary in RH1 and nothing in RH2 (a second melee weapon to soft-swap
to is not part of an optimal build), the four rings it wears, its armour, and what the recommender's `choose_kit` carries: the
quick items in slot order (Estus Flask, a heal, a resin when the primary is buffable, the best knife
and urn, a defensive item, then runners-up until at least half the ten slots are full), a bow,
greatbow or crossbow in RH3 when one fits the stats and the load, and ammunition in all four arrow
and bolt slots. Every armour slot holds a piece (the recommender's FULL_ARMOR; `--allow-bare`
lets one go bare). The other weapons it lists are already in the mule's bag.

ITEMS AND ARMOUR THE MULE HOLDS: `--held-from` limits the armour (HELD_ARMOR) and the quick items
and ammunition (HELD_ITEMS) to what the bag holds, as it does the rings below.

RINGS THE MULE HOLDS: with `--held-from SAVE --base-slot N` every ring is one that slot's bag
holds -- a corpus "Chloranthy Ring" comes out as the mule's "Chloranthy Ring+2", chosen before the
armour so its weight counts against 70% (the recommender's HELD_RINGS) -- and an upgrade group the
bag lacks is never suggested. Without them a build can name a ring `scripts/ds2-sl2-builds.py`
cannot equip.

HP is the game's MAX, because the save's HP field is the CURRENT value and the game recomputes
the max on load (measured 2026-10-23: a save written with 800 came up 800/1064): the recommender's
`hit_points` at the stats the rings give, times each worn ring's max-HP factor.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
LEVELS = [20, 40, 70, 100, 125, 155, 200, 250, 838]
INFUSIONS = ["No_Infusion", "Fire", "Magic", "Lightning", "Dark", "Poison", "Bleed", "Raw", "Enchanted",
             "Mundane"]
SHORT = {"No_Infusion": "", "Lightning": "Ltng", "Enchanted": "Ench", "Mundane": "Mund"}
NAME_MAX = 16  # the game's own name field takes 16 characters


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def load_data(rec):
    sp, mm = rec.dump_site_tables(rec.CACHE / "site-tables")
    data = rec.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    print(rec.apply_regulation(data)[:120] + " ...", file=sys.stderr)
    return data


def norm(name: str) -> str:
    return re.sub(r"[^a-z0-9+]", "", name.lower())


def ring_key(data, name: str) -> str:
    for k, v in data.rings.items():
        if norm(v["name"]) == norm(name):
            return k
    raise SystemExit(f"unknown ring {name!r}")


def max_hp(rec, data, stats: dict, rings: list[str]) -> int:
    gear = rec.ring_gear(data)
    factor = math.prod(gear[r]["hp"] for r in rings if r in gear)
    return int(rec.hit_points(data, rec.gear_stats(data, stats, rings)) * factor)


def top_rows(best_dir: Path, level: int) -> list[tuple[float, str, dict]]:
    rows = []
    for inf in INFUSIONS:
        f = best_dir / f"{inf}.json"
        if not f.exists() or not f.stat().st_size:
            raise SystemExit(f"{f}: missing or empty -- the --best-weapons sweep for {inf} has not finished")
        for r in json.loads(f.read_text()).get(str(level), []):
            rows.append((r["score"], inf, r))
    return sorted(rows, key=lambda x: -x[0])


def short_name(level: int, weapon: str, inf: str) -> str:
    tag = SHORT.get(inf, inf)
    name = f"{level} {tag + ' ' if tag else ''}{weapon}"
    return name[:NAME_MAX].rstrip()


def build_for(rec, data, corpus, level: int, slot: int, inf: str, row: dict) -> dict:
    g = rec.generate_build(data, corpus, row["key"], inf, level)
    if g is None:
        raise SystemExit(f"SL {level}: generate_build made no build for {row['weapon']} ({inf})")
    worn = list(dict.fromkeys(g["rings"]))[:4]
    keys = [ring_key(data, n) for n in worn]
    equip = {"rh1": {"name": g["primary"][0], "infusion": g["primary"][1]}}
    for slot_name, piece in zip(("head", "chest", "hands", "legs"), g["armor"]):
        if piece != "Naked":
            equip[slot_name] = {"name": piece}
    for i, ring in enumerate(worn):
        equip[f"ring{i + 1}"] = {"name": ring}
    kit = rec.choose_kit(data, corpus, g, row["key"], inf, level)
    for i, (name, _) in enumerate(kit["items"]):
        equip[f"item{i + 1}"] = {"name": name}
    if kit["launcher"]:
        name, linf, _ = kit["launcher"]
        equip["rh3"] = {"name": name, "infusion": linf}
    for slot_name, (name, _) in kit["ammo"].items():
        equip[slot_name] = {"name": name}
    cls = data.classes[g["class"]]
    return {"slot": slot, "name": short_name(level, g["primary"][0], inf), "class": g["class"],
            "class_level": int(cls["level"]), "soul_level": level, "stats": g["stats"],
            "hp": max_hp(rec, data, g["stats"], keys), "two_handed": g["two_handed"],
            "score": row["score"], "equip": equip,
            "kit": {"items": kit["items"], "launcher": kit["launcher"], "ammo": kit["ammo"]},
            "generated": {k: g[k] for k in ("primary", "weapons_1h", "weapons_2h_only", "rings", "armor",
                                            "armor_note", "value", "objective")}}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--best-dir", type=Path, help="the ten --best-weapons --json files, <infusion>.json")
    ap.add_argument("-o", "--out", type=Path)
    ap.add_argument("--levels", default=",".join(map(str, LEVELS)))
    ap.add_argument("--first-slot", type=int, default=1, help="the save slot the first level goes in")
    ap.add_argument("--max-hp", metavar="STATS", help="print the max HP of these stats and --rings, and stop")
    ap.add_argument("--rings", default="", help="with --max-hp: worn rings, comma-separated names")
    ap.add_argument("--held-from", type=Path, help="give only rings the bag of --base-slot in this save holds")
    ap.add_argument("--base-slot", type=int, help="with --held-from: the slot ds2-sl2-builds.py copies from")
    ap.add_argument("--spend", choices=["curves", "ehp"], default="curves",
                    help="how generate_build spends the free points (the recommender's --spend); "
                         "give the one the --best-dir sweep ranked with")
    ap.add_argument("--allow-bare", action="store_true",
                    help="let an armour slot go bare when no piece is worth its weight (default: every "
                         "slot holds a piece, the recommender's FULL_ARMOR)")
    a = ap.parse_args()
    rec = _load("ds2_builds_recommend", "ds2-builds-recommend.py")
    rec.SPEND.update(mode=a.spend)
    rec.FULL_ARMOR = not a.allow_bare
    data = load_data(rec)
    if a.max_hp:
        stats = {s: int(v) for s, v in (kv.split("=") for kv in a.max_hp.split(","))}
        alias = {"VGR": "VIG", "ADA": "ADP", "FAI": "FTH"}
        stats = {next(n for n in rec.STATS if rec.LABEL[n] == alias.get(s, s)): v for s, v in stats.items()}
        rings = [ring_key(data, n.strip()) for n in a.rings.split(",") if n.strip()]
        print(max_hp(rec, data, stats, rings))
        return 0
    if not (a.best_dir and a.out):
        ap.error("--best-dir and -o are required")
    if a.held_from:
        if a.base_slot is None:
            ap.error("--held-from needs --base-slot")
        writer = _load("ds2_sl2_builds", "ds2-sl2-builds.py")
        held = writer.held_names(a.held_from, a.base_slot, writer.RING_IDS)
        rec.HELD_RINGS = {k for k, v in data.rings.items() if norm(v["name"]) in held}
        held = writer.held_names(a.held_from, a.base_slot, writer.ARMOR_IDS)
        rec.HELD_ARMOR = {k for s in rec.ARMOR_SLOTS for k, v in data.armor[s].items() if norm(v["name"]) in held}
        rec.HELD_ITEMS = {rec.norm(n) for n in writer.held_names(a.held_from, a.base_slot, range(50_000_000, 70_000_000))}
        print(f"rings: {len(rec.HELD_RINGS)}, armour: {len(rec.HELD_ARMOR)}, items: {len(rec.HELD_ITEMS)} "
              f"that the bag of slot {a.base_slot} holds", file=sys.stderr)
    corpus, _ = rec.load_corpus(data)
    builds = []
    for i, level in enumerate(int(x) for x in a.levels.split(",")):
        rows = top_rows(a.best_dir, level)
        if not rows:
            raise SystemExit(f"SL {level}: no ranked weapon in {a.best_dir}")
        score, inf, row = rows[0]
        b = build_for(rec, data, corpus, level, a.first_slot + i, inf, row)
        builds.append(b)
        print(f"SL {level:3}  slot {b['slot']}  {b['name']:16}  {b['class']:9} score {score:7.1f}  "
              f"runner-up {rows[1][2]['weapon']} ({rows[1][1]}) {rows[1][0]:.1f}" if len(rows) > 1 else "")
    a.out.write_text(json.dumps({"levels": a.levels, "builds": builds}, indent=1))
    print(f"wrote {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
