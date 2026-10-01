#!/usr/bin/env python3
"""Evidence for and against the recommender's infusion ranking, measured on the SoulsPlanner builds
that record an infusion: the numbers docs/DS2-INFUSION-EVIDENCE.md quotes.

    python3 scripts/ds2-infusion-evidence.py              # every section
    python3 scripts/ds2-infusion-evidence.py --only margin,breakeven

Sections:
  margin     how far the model's best infusion is ahead of the one the player chose, per miss kind
  breakeven  the extra elemental defense every element would need before the player's choice wins
  scaling    the stats No_Infusion choosers invest, against Lightning and Dark choosers
  buffs      who carries a weapon buff or a resin, and whether a buff on every infusion moves the pick
  intent     agreement split by what a build records about PvP or PvE intent (covenant, name, item)
  fire       Fire choosers' INT and FTH, and Fire against Dark at those stats
  rings      elemental defense rings in the defender corpus, and the pick with them counted
  naked      the bare defender's physical defense, from ArmorParam's per-piece stat scale
  falsify    whether the 0.99 cap, the damageLower floors or slash/strike/thrust defense can bind

The population and the damage formula are calibrate_infusions' own (scripts/ds2-builds-recommend.py
--calibrate): one case per distinct weapon of a SoulsPlanner build, no shields or catalysts, scored
with damage() against the build's SL bracket's average defender, full upgrade. Loads the corpus as
main() does, so a run takes about half a minute."""
from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

HERE = Path(__file__).parent
SECTIONS = ["margin", "breakeven", "scaling", "buffs", "intent", "fire", "rings", "naked", "falsify"]
ELEMENTS = ["magic", "fire", "lightning", "dark"]
#: Covenants whose play is invading or defending against invaders (PvP), and those that are
#: co-operation or offline challenge (PvE). Way of Blue is summoned to defend, so PvP.
PVP_COVENANTS = {"Brotherhood_of_Blood", "Bell_Keepers", "Rat_King_Covenant", "Blue_Sentinels",
                 "Dragon_Remnants", "Way_of_Blue"}
PVE_COVENANTS = {"Heirs_of_the_Sun", "Pilgrims_of_Dark", "Company_of_Champions"}
PVP_NAME = re.compile(r"pvp|invad|duel|arena|gank|fight ?club|twink", re.I)
PVE_NAME = re.compile(r"pve|boss|co-?op|speed ?run|ng\+|sotfs run|playthrough", re.I)
#: Items that are only used to invade.
PVP_ITEMS = {"Cracked_Red_Eye_Orb", "Red_Eye_Orb"}
#: Resins: SoulsPlanner item key -> element. Their effect values were not read from the game.
RESINS = {"Dark_Pine_Resin": "dark", "Gold_Pine_Resin": "lightning", "Charcoal_Pine_Resin": "fire",
          "Rotten_Pine_Resin": "poison"}


def load_recommend():
    spec = importlib.util.spec_from_file_location("recommend", HERE / "ds2-builds-recommend.py")
    r = importlib.util.module_from_spec(spec)
    sys.modules["recommend"] = r
    spec.loader.exec_module(r)
    return r


def sp_corpus(r, data):
    """The SoulsPlanner builds load_corpus keeps, in its order and with its dedupe, each with its raw
    record (covenant and items, which Build drops) and its name. SoulsPlanner builds come first in
    raw_builds, so the dedupe against MugenMonkey builds cannot drop one."""
    raw = json.loads((r.CACHE / "builds-darksouls2.json").read_text())
    index = {str(m["id"]): m for m in json.loads((r.CACHE / "index-darksouls2.json").read_text())}
    out, seen = [], set()
    for bid, rec in raw.items():
        meta = index.get(bid, {})
        b = r.sp_build(rec, meta.get("level"), meta.get("name", ""))
        if r.keep(data, b):
            continue
        sig = (frozenset(r.tokens(b)), tuple(b.stats[s] for s in r.STATS))
        if sig in seen:
            continue
        seen.add(sig)
        out.append((b, rec))
    return out


def score(r, data, w, inf, eff, dfn, add=None):
    """calibrate_infusions' score of one infusion, with `add` (DMG type -> attack) on top of its AR."""
    ar = dict(r.attack_rating(data, w, inf, eff))
    for k, v in (add or {}).items():
        ar[k] = ar.get(k, 0) + v
    return sum(r.damage(k, v, dfn[k]) for k, v in ar.items())


def cases(r, data, corpus, sp):
    """One dict per calibrate case: the build, its record, its stats, defender, and every infusion's score."""
    by_sl, out = {}, []
    for b, rec in sp:
        sl = r.soul_level(data, b)
        i = r.sl_bracket(sl)
        if i not in by_sl:
            by_sl[i] = r.bracket_defense(data, corpus, sl)[0]
        dfn, eff = by_sl[i], r.effective(data, b)
        for w, inf in set(b.weapons()):
            if inf == "?" or w not in data.weapons or data.weapons[w].get("isShield") or r.CATALYST.search(w):
                continue
            infs = list(data.weapons[w]["infusions"])
            if len(infs) < 2 or inf not in infs:
                continue
            s = {f: score(r, data, w, f, eff, dfn) for f in infs}
            best = max(infs, key=lambda f: (s[f], f))  # calibrate's sorted(reverse=True) tie order
            out.append({"b": b, "rec": rec, "w": w, "inf": inf, "eff": eff, "dfn": dfn, "sl": sl,
                        "s": s, "best": best})
    return out


def pct(xs, q):
    return float(np.percentile(xs, q)) if xs else float("nan")


def top_misses(cs, n=8):
    return Counter((c["inf"], c["best"]) for c in cs if c["best"] != c["inf"]).most_common(n)


def section_margin(r, data, cs):
    print("== margin: best infusion's damage over the chosen one's, at the build's stats and SL ==")
    hit = sum(c["best"] == c["inf"] for c in cs)
    print(f"cases {len(cs)}, model's best is the chosen infusion in {hit / len(cs):.1%}")
    miss = [c for c in cs if c["best"] != c["inf"]]
    allm = [c["s"][c["best"]] / c["s"][c["inf"]] - 1 for c in miss if c["s"][c["inf"]] > 0]
    print(f"all misses ({len(miss)}): " + band_line(allm))
    for (real, model), k in top_misses(cs):
        ms = [c["s"][model] / c["s"][real] - 1 for c in miss
              if (c["inf"], c["best"]) == (real, model) and c["s"][real] > 0]
        print(f"  chose {real:12} model {model:10} {k:5}  " + band_line(ms))
    print("  within 5% counts as agreement:",
          f"{(hit + sum(m <= 0.05 for m in allm)) / len(cs):.1%}; within 10%: "
          f"{(hit + sum(m <= 0.10 for m in allm)) / len(cs):.1%}")


def band_line(ms):
    if not ms:
        return "none"
    return (f"median {pct(ms, 50):.1%}, p25 {pct(ms, 25):.1%}, p75 {pct(ms, 75):.1%}; "
            f"<=2% {np.mean([m <= 0.02 for m in ms]):.0%}, <=5% {np.mean([m <= 0.05 for m in ms]):.0%}, "
            f"<=10% {np.mean([m <= 0.10 for m in ms]):.0%}, >25% {np.mean([m > 0.25 for m in ms]):.0%}")


def section_breakeven(r, data, corpus, cs):
    """The defense at which the chosen infusion first scores best, raising the bracket average in
    steps of 10. A physical choice (No_Infusion, Raw, ...) gets the extra in every element; an
    elemental choice only in the model's pick's element, since raising both moves both. Printed as
    the defender's resulting defense in the model's pick's element."""
    print("== breakeven: elemental defense at which the chosen infusion scores best ==")
    steps = list(range(0, 901, 10))
    by = defaultdict(list)
    for c in cs:
        if c["best"] == c["inf"]:
            continue
        ars = {f: r.attack_rating(data, c["w"], f, c["eff"]) for f in c["s"]}
        el = c["best"].lower()
        raise_in = ELEMENTS if c["inf"].lower() not in ELEMENTS or el not in ELEMENTS else [el]
        found = None
        for x in steps:
            d2 = {k: c["dfn"][k] + (x if k in raise_in else 0) for k in c["dfn"]}
            sc = {f: sum(r.damage(k, v, d2[k]) for k, v in a.items()) for f, a in ars.items()}
            if sc[c["inf"]] >= max(sc.values()) - 1e-9:
                found = x + c["dfn"].get(el, 0)
                break
        by[(c["inf"], c["best"])].append(found)
    for (real, model), _ in top_misses(cs):
        xs = by[(real, model)]
        ok = [x for x in xs if x is not None]
        where = "all elements" if real.lower() not in ELEMENTS or model.lower() not in ELEMENTS else model.lower()
        print(f"  chose {real:12} model {model:10} n {len(xs):5}  raised in {where:12}; still loses at +900: "
              f"{xs.count(None):4}; wins at {model.lower()} defense {pct(ok, 50):.0f} median "
              f"(p25 {pct(ok, 25):.0f}, p75 {pct(ok, 75):.0f})")
    print("  for scale, defenders' own elemental defense by bracket (corpus builds, rings left out):")
    by_br = defaultdict(list)
    for b in corpus:
        by_br[r.sl_bracket(r.soul_level(data, b))].append(b)
    for sl in (100, 150, 200):
        i = r.sl_bracket(sl)
        ds = [r.build_defense(data, b) for b in by_br[i]]
        lo, hi = r.SL_BRACKETS[i]
        print(f"    SL {lo}-{hi} n {len(ds)}: " + "; ".join(
            f"{k} p50 {pct([d[k] for d in ds], 50):.0f} p90 {pct([d[k] for d in ds], 90):.0f} "
            f"p99 {pct([d[k] for d in ds], 99):.0f}" for k in ELEMENTS))


STAT_KEYS = ["strength", "dexterity", "intelligence", "faith"]


def stat_line(cs):
    return "  ".join(f"{s[:3].upper()} {np.median([c['eff'][s] for c in cs]):.0f}" for s in STAT_KEYS)


def section_scaling(r, data, cs):
    print("== scaling: median effective stats by choice, and the AR the model credits at them ==")
    groups = [("No_Infusion, model says Lightning", lambda c: c["inf"] == "No_Infusion" and c["best"] == "Lightning"),
              ("No_Infusion, model says Dark", lambda c: c["inf"] == "No_Infusion" and c["best"] == "Dark"),
              ("No_Infusion, model agrees", lambda c: c["inf"] == "No_Infusion" and c["best"] == "No_Infusion"),
              ("chose Lightning", lambda c: c["inf"] == "Lightning"),
              ("chose Dark", lambda c: c["inf"] == "Dark"),
              ("chose Raw", lambda c: c["inf"] == "Raw")]
    for name, f in groups:
        g = [c for c in cs if f(c)]
        if not g:
            continue
        print(f"  {name:36} n {len(g):5}  {stat_line(g)}")
    for model in ("Lightning", "Dark"):
        g = [c for c in cs if c["inf"] == "No_Infusion" and c["best"] == model]
        phys_kept, elem = [], []
        for c in g:
            un = r.attack_rating(data, c["w"], "No_Infusion", c["eff"])
            inf = r.attack_rating(data, c["w"], model, c["eff"])
            if un.get("physical"):
                phys_kept.append(inf.get("physical", 0) / un["physical"])
            elem.append(inf.get(model.lower(), 0))
        base = [c["eff"]["faith" if model == "Lightning" else "intelligence"] for c in g]
        print(f"  No_Infusion vs {model}: the {model} row keeps {pct(phys_kept, 50):.0%} of the uninfused "
              f"physical AR (median), adds {pct(elem, 50):.0f} {model.lower()} AR; "
              f"{np.mean([v <= 15 for v in base]):.0%} of these builds have the element's stat at 15 or less")


def buff_gain(data, d, r, w, inf, buff, eff):
    """What a weapon buff adds to `w` infused with `inf`, in the buff's element: the builder adds
    scale/100 to the weapon's rate there (docs/DS2-DPS-MECHANICS.md "Spell and buff attack"), so the
    gain is the infused row's own elemental base and coefficient, as buff_attack takes the uninfused
    row's. None when the regulation has no row for this weapon and infusion."""
    reg = d["by_name"].get(r.norm(data.weapons[w].get("name", w)))
    rr = reg and d["WeaponReinforceParam"].get(str(reg["weaponReinforceId"]))
    if not rr:
        return None
    a = d["WeaponStatsAffectParam"].get(str(rr["statsAffectId"] + r.INFUSION_MOVE[inf][2]))
    if not a:
        return None
    stem, sa, _ = r.CAST_ELEMENTS[buff["type"]]
    base = rr[f"maximum{stem}"] * a["baseValueScale"]
    coef = a[f"{sa}{rr['maxLevel']}"]
    return (base + coef * r.element_bonus(data, buff["type"], eff) + buff["flat"]) * buff["scale"] / 100


def section_buffs(r, data, d, cs):
    print("== buffs: carried weapon buffs and resins, and the pick with a buff on every infusion ==")
    has_cat = lambda c: any(r.CATALYST.search(w) for w, _ in c["b"].weapons())
    buffs_of = lambda c: [s for s in c["b"].spells if s in data.buffs] if has_cat(c) else []
    resins_of = lambda c: [i for i in c["rec"]["items"].split(";") if i in RESINS]
    for name, f in [("chose No_Infusion", lambda c: c["inf"] == "No_Infusion"),
                    ("  of them, model says Lightning or Dark",
                     lambda c: c["inf"] == "No_Infusion" and c["best"] in ("Lightning", "Dark")),
                    ("chose an elemental infusion", lambda c: c["inf"] in ("Lightning", "Dark", "Magic", "Fire")),
                    ("chose Raw", lambda c: c["inf"] == "Raw")]:
        g = [c for c in cs if f(c)]
        print(f"  {name:40} n {len(g):5}: buff spell + catalyst {np.mean([bool(buffs_of(c)) for c in g]):.1%}, "
              f"resin {np.mean([bool(resins_of(c)) for c in g]):.1%}")
    buffed = [c for c in cs if buffs_of(c)]
    print(f"  builds' cases with a castable buff: {len(buffed)}, by spell "
          + ", ".join(f"{k} {v}" for k, v in Counter(s for c in buffed for s in set(buffs_of(c))).most_common()))
    before = after = equal = flips_to = flips_from = missing = 0
    for c in buffed:
        best_eq, best_own = {}, {}
        for f in c["s"]:
            eq, own = [], []
            for s in set(buffs_of(c)):
                bf = data.buffs[s]
                eq.append(score(r, data, c["w"], f, c["eff"], c["dfn"],
                                {bf["type"]: r.buff_attack(data, bf, c["w"], c["eff"])}))
                g = buff_gain(data, d, r, c["w"], f, bf, c["eff"])
                if g is None:
                    missing += 1
                    g = 0.0
                own.append(score(r, data, c["w"], f, c["eff"], c["dfn"], {bf["type"]: g}))
            best_eq[f], best_own[f] = max(eq), max(own)
        pick_eq = max(best_eq, key=lambda f: (best_eq[f], f))
        pick = max(best_own, key=lambda f: (best_own[f], f))
        before += c["best"] == c["inf"]
        equal += pick_eq == c["inf"]
        after += pick == c["inf"]
        flips_to += pick == c["inf"] and c["best"] != c["inf"]
        flips_from += pick != c["inf"] and c["best"] == c["inf"]
    n = max(len(buffed), 1)
    print(f"  agreement on those cases: unbuffed {before / n:.1%}; the uninfused row's buff on every infusion "
          f"{equal / n:.1%}; each infusion's own row {after / n:.1%} ({flips_to} misses become matches, "
          f"{flips_from} matches become misses; {missing} buff scores had no regulation row and took 0)")
    nores = [c for c in cs if c["inf"] == "No_Infusion" and c["best"] != "No_Infusion"]
    print(f"  No_Infusion misses carrying a resin: {sum(bool(resins_of(c)) for c in nores)} of {len(nores)}")


def intent_of(c):
    rec, name = c["rec"], c["b"].label or ""
    items = set(rec["items"].split(";"))
    pvp = rec["covenant"] in PVP_COVENANTS or bool(PVP_NAME.search(name)) or bool(items & PVP_ITEMS)
    pve = rec["covenant"] in PVE_COVENANTS or bool(PVE_NAME.search(name))
    return "both" if pvp and pve else "PvP" if pvp else "PvE" if pve else "unmarked"


def section_intent(r, data, cs):
    print("== intent: agreement by what the build records (covenant, name, invasion item) ==")
    by = defaultdict(list)
    for c in cs:
        by[intent_of(c)].append(c)
    for k in ("PvP", "PvE", "both", "unmarked"):
        g = by.get(k, [])
        if not g:
            continue
        un = np.mean([c["inf"] in ("No_Infusion", "Raw") for c in g])
        print(f"  {k:9} n {len(g):5}: model agrees {np.mean([c['best'] == c['inf'] for c in g]):.1%}, "
              f"chose No_Infusion or Raw {un:.1%}, chose Lightning/Dark/Magic/Fire "
              f"{np.mean([c['inf'] in ('Lightning', 'Dark', 'Magic', 'Fire') for c in g]):.1%}")


def section_fire(r, data, cs):
    print("== fire: Fire choosers' stats, and Fire against Dark at them ==")
    g = [c for c in cs if c["inf"] == "Fire"]
    print(f"  chose Fire n {len(g)}: {stat_line(g)}; model agrees {np.mean([c['best'] == 'Fire' for c in g]):.1%}; "
          f"model's picks " + ", ".join(f"{k} {v}" for k, v in Counter(c['best'] for c in g).most_common(4)))
    both = [c for c in g if c["eff"]["intelligence"] >= 20 and c["eff"]["faith"] >= 20]
    one = [c for c in g if c not in both]
    for name, h in (("INT and FTH both 20+", both), ("otherwise", one)):
        md = [c["s"]["Dark"] / c["s"]["Fire"] - 1 for c in h if "Dark" in c["s"] and c["s"]["Fire"] > 0]
        print(f"  {name:22} n {len(h):5}: model agrees {np.mean([c['best'] == 'Fire' for c in h]) if h else 0:.1%}; "
              f"Dark over Fire median {pct(md, 50):+.1%}")
    sl150 = [c for c in cs if 126 <= c["sl"] <= 155]
    if sl150:
        d = sl150[0]["dfn"]
        print("  bracket SL 126-155 mean defense: " + ", ".join(f"{k} {d[k]:.0f}" for k in ["physical", *ELEMENTS]))


def section_rings(r, data, corpus, cs):
    print("== rings: defense rings as worn, and the defender who swaps one in against the weapon ==")
    # build_defense counts every worn ring's regulation defense change once (EXE: a 100090[4]
    # reaches the cut once, docs/DS2-DPS-MECHANICS.md "Defense from a SpEffect"), so the cases'
    # defense already holds them; the adaptive rows are calibrate's with DEFENDER "adaptive".
    def ring_add(b):
        add = Counter()
        for ring in b.rings:
            for k, v in data.ring_defense.get(ring, {}).items():
                add[k] += v
        return add
    by = defaultdict(list)
    for b in corpus:
        by[r.sl_bracket(r.soul_level(data, b))].append(b)
    lo, hi = r.SL_BRACKETS[r.sl_bracket(150)]
    g = by[r.sl_bracket(150)]
    worn = [b for b in g if any(v > 0 for k, v in ring_add(b).items() if k in ELEMENTS)]
    print(f"  SL {lo}-{hi}: {len(worn) / max(len(g), 1):.1%} of builds wear an elemental one; mean change "
          + ", ".join(f"{k} {np.mean([ring_add(b)[k] for b in g]):.1f}" for k in ["physical", *ELEMENTS]))
    hit = sum(c["best"] == c["inf"] for c in cs)
    print(f"  agreement with rings as worn: {hit / len(cs):.1%}")
    saved = dict(r.DEFENDER)
    try:
        for buff in ("none", "item", "any"):
            r.DEFENDER.update(mode="adaptive", buff=buff)
            hit = 0
            for c in cs:
                dfn = r.defender_defense(data, corpus, c["sl"])[0]
                sc = {}
                for f in c["s"]:
                    ar = r.attack_rating(data, c["w"], f, c["eff"])
                    sc[f] = r._type_damage(ar, r.respond(dfn, ar))
                hit += max(sc, key=lambda f: (sc[f], f)) == c["inf"]
            print(f"  agreement with the counter ring swapped in, defender buff {buff}: {hit / len(cs):.1%}")
    finally:
        r.DEFENDER.update(saved)


#: The game's bare armour pieces, head to legs (ArmorParam ids; crates/ds2-rva ARMOR_NAKED_IDS).
NAKED_ARMOR_IDS = ["11001100", "11001101", "11001102", "11001103"]
#: An armour ItemParam id is its ArmorParam id plus this (crates/ds2-rva ARMOR_PARAM_ID_FROM_ITEM_ID).
ARMOR_ITEM_OFFSET = 10_000_000
#: PlayerDamageParam.damageLower on most rows (docs/DS2-DPS-MECHANICS.md), and the elemental cut at
#: which FUN_140164af0's cap starts to matter: (D + 100) / 1000 = 0.99.
DAMAGE_LOWER = 70
CAP_D = 890


def armor_rows(r):
    """ArmorParam rows by id, and the planner's armour keys mapped to their ArmorParam row by name."""
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    rows = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {"ArmorParam": "ARMOR_PARAM"})["ArmorParam"]
    by_name = {}
    for iid, name in ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION).items():
        aid = str(int(iid) - ARMOR_ITEM_OFFSET)
        if aid in rows:
            by_name.setdefault(r.norm(name), rows[aid])
    return rows, by_name


def section_naked(r, data, corpus):
    """The bare defender's physical defense. PlayerGameParamCalculator 0x140380070 (Ghidra, read
    2026-09-30) writes DEF[0] = SpEffect add + (sum of the four pieces' ArmorParam +0x1c,
    defenseStatAffectScale) x stat defense (stat block +0x48) + sum of each piece's own defense; an
    empty slot holds the naked pieces. So the stat part is scaled per piece, not added whole."""
    print("== naked: the stat part of physical defense, per piece (ArmorParam.defenseStatAffectScale) ==")
    rows, by_name = armor_rows(r)
    scales = [rows[i]["defenseStatAffectScale"] for i in NAKED_ARMOR_IDS]
    site = [data.armor[s]["Naked"].get("physicalDEFBonus", 0) for s in r.ARMOR_SLOTS]
    print(f"  naked pieces' defenseStatAffectScale {scales} (sum {sum(scales):.3f}); "
          f"the planner's Naked physicalDEFBonus {site}")
    same = differ = unmatched = 0
    for slot in r.ARMOR_SLOTS:
        for key, p in data.armor[slot].items():
            row = by_name.get(r.norm(p.get("name", key)))
            if row is None or key == "Naked":
                unmatched += row is None
                continue
            ok = abs(row["defenseStatAffectScale"] - p.get("physicalDEFBonus", 0)) < 1e-3
            same += ok
            differ += not ok
    print(f"  planner pieces matched by name: physicalDEFBonus equals defenseStatAffectScale {same}, "
          f"differs {differ}, no row by name {unmatched}")
    by = defaultdict(list)
    for b in corpus:
        by[r.sl_bracket(r.soul_level(data, b))].append(b)
    g = by[r.sl_bracket(150)]
    med = {s: int(np.ceil(np.median([b.stats[s] for b in g]))) for s in r.STATS}
    total = med["endurance"] + med["vitality"] + med["strength"] + med["dexterity"]
    stat_def = r._tab(data, "physicalDEFBonus", total)
    print(f"  SL 126-155 median END {med['endurance']} VIT {med['vitality']} STR {med['strength']} "
          f"DEX {med['dexterity']}: stat defense {stat_def}, naked physical {sum(scales) * stat_def:.1f} "
          f"(the game's sum), a whole stat defense would be {stat_def}")


def section_falsify(r, data, corpus, cs):
    """Static checks that could flip a physical choice against a split one: the 0.99 cap, the
    damageLower floors, and slash/strike/thrust defense in place of the general one."""
    print("== falsify: whether the cap, the floors or a typed physical defense can bind ==")
    ds = [r.build_defense(data, b) for b in corpus]
    top = max(max(d[k] for k in ELEMENTS) for d in ds)
    print(f"  highest elemental defense of any corpus build {top:.0f} (worn rings counted), with a Quartz "
          f"Ring+3 swapped in {top + 150:.0f}, and Flash Sweat's +300 on top {top + 450:.0f}; the cap "
          f"binds from {CAP_D}")
    phys_floor = elem_floor = 0
    for c in cs:
        worst = max(c["dfn"][k] for k in ["physical", *r.PHYS_TYPES])
        for f in c["s"]:
            ar = r.attack_rating(data, c["w"], f, c["eff"])
            phys_floor += bool(ar.get("physical")) and ar["physical"] * 10 - worst < DAMAGE_LOWER
            elem_floor += any(0 < ar.get(k, 0) and ar[k] * 6 < DAMAGE_LOWER for k in ELEMENTS)
    print(f"  infusion rows scored: physical floor binds in {phys_floor}, elemental floor in {elem_floor}")

    def floored(k, v, df):  # hit_damage's floors, which damage() leaves at 0
        if not v:
            return 0.0
        if k == "physical":
            return max(v * 10 - df, DAMAGE_LOWER) / 12
        return max(v * 6, DAMAGE_LOWER) / 6 * (1 - min(0.99, (df + 100) / 1000))
    hit = flips = 0
    for c in cs:
        sc = {f: sum(floored(k, v, c["dfn"][k]) for k, v in r.attack_rating(data, c["w"], f, c["eff"]).items())
              for f in c["s"]}
        pick = max(sc, key=lambda f: (sc[f], f))
        hit += pick == c["inf"]
        flips += pick != c["best"]
    print(f"  agreement with damageLower {DAMAGE_LOWER} floors applied: {hit / len(cs):.1%} "
          f"(the model's pick changes in {flips} cases)")
    for t in r.PHYS_TYPES:
        hit = 0
        for c in cs:
            d2 = dict(c["dfn"], physical=c["dfn"][t])
            sc = {f: score(r, data, c["w"], f, c["eff"], d2) for f in c["s"]}
            hit += max(sc, key=lambda f: (sc[f], f)) == c["inf"]
        print(f"  agreement with {t} defense as the physical one: {hit / len(cs):.1%}")


def regulation_rows(r):
    """WeaponParam by item name, WeaponReinforceParam and WeaponStatsAffectParam, decoded as
    apply_regulation decodes them; for each infusion's own buff gain."""
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
        "WeaponParam": "WEAPON_PARAM", "WeaponReinforceParam": "WEAPON_REINFORCE_PARAM",
        "WeaponStatsAffectParam": "WEAPON_STATS_AFFECT_PARAM"})
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    d["by_name"] = {}
    for wid, w in d["WeaponParam"].items():
        d["by_name"].setdefault(r.norm(names.get(wid, "")), w)
    return d


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter,
                                 epilog="\n\n".join(__doc__.split("\n\n")[1:]))
    ap.add_argument("--only", default=",".join(SECTIONS),
                    help=f"comma-separated sections to print (default all): {', '.join(SECTIONS)}")
    a = ap.parse_args()
    only = [s.strip() for s in a.only.split(",") if s.strip()]
    bad = [s for s in only if s not in SECTIONS]
    if bad:
        ap.error(f"unknown section {', '.join(bad)}: one of {', '.join(SECTIONS)}")
    r = load_recommend()
    sp, mm = r.dump_site_tables(r.CACHE / "site-tables")
    data = r.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    print(r.apply_regulation(data), file=sys.stderr)
    corpus = r.load_corpus(data)[0]
    cs = cases(r, data, corpus, sp_corpus(r, data))
    for s in only:
        if s == "margin":
            section_margin(r, data, cs)
        elif s == "breakeven":
            section_breakeven(r, data, corpus, cs)
        elif s == "scaling":
            section_scaling(r, data, cs)
        elif s == "buffs":
            section_buffs(r, data, regulation_rows(r), cs)
        elif s == "intent":
            section_intent(r, data, cs)
        elif s == "fire":
            section_fire(r, data, cs)
        elif s == "rings":
            section_rings(r, data, corpus, cs)
        elif s == "naked":
            section_naked(r, data, corpus)
        elif s == "falsify":
            section_falsify(r, data, corpus, cs)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
