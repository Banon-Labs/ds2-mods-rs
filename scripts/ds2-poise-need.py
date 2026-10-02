#!/usr/bin/env python3
"""How much poise each weapon's R1 needs to trade with the SL bracket, and where more stops paying.

Read-only. The trade model: you and a bracket opponent press R1 at the same instant. The opponent's
counter-hit is their melee weapon's 1H R1 first hit (the weapon bracket_poise's build_counter picks
per build), with its time from input, poise damage and armorBreak. Your R1's first hit lands at its
own time. A counter that lands first interrupts you when it staggers you then: armorBreak 2 always;
armorBreak 1 outside hyperarmor; otherwise when its poise damage -- times the weapon's
uninterruptibleRate inside the R1's hyperarmor window (TAE 111900) -- reaches your poise
(stagger_hits' rule, one hit, no regeneration). A counter that lands after your hit has lost the
trade whatever your poise is.

Per weapon: `exposed` the share of counters that land before your hit (the most poise could ever
win back); `trade(p)` the share of counters you swing through at poise p; `need` the least poise
that wins back all that poise can (armorBreak 2 hits and armorBreak 1 hits outside hyperarmor no
poise wins); `need90` the least poise that wins 90% of it. A weapon whose `exposed` is small needs
no poise; `need` is where more stops paying.

    python3 scripts/ds2-poise-need.py --sl 120 --weapons "Black Flamestone Dagger,Giant Warrior Club"
    python3 scripts/ds2-poise-need.py --sl 120 --generate Black_Flamestone_Dagger --class warrior
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
LEVELS = (0, 20, 40, 60, 80, 100, 120, 140, 160)


def load_recommender():
    spec = importlib.util.spec_from_file_location("ds2recommend", HERE / "ds2-builds-recommend.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ds2recommend"] = mod
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--sl", type=int, required=True)
    ap.add_argument("--weapons", help="comma-separated weapon names or keys")
    ap.add_argument("--generate", help="a primary weapon: measure it and every weapon --generate grants with it")
    ap.add_argument("--infusion", default="No_Infusion")
    ap.add_argument("--class", dest="cls")
    ap.add_argument("--one-hand", action="store_true", help="measure your R1 one-handed (default two-handed)")
    a = ap.parse_args()

    R = load_recommender()
    sp_json, mm_json = R.dump_site_tables(R.CACHE / "site-tables")
    data = R.Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    R.apply_regulation(data)
    corpus, _ = R.load_corpus(data)
    attacks = R.load_attacks(data)

    def names_of(key):
        return [data.weapons[key]["name"], key.replace("_", " ")]

    def first_hit(key, two):
        tl = next((t for t in (R.chain_timeline(attacks, nm, two, "Normal", 3.0, with_row=True)
                               for nm in names_of(key)) if t), None)
        if not tl:
            return None
        hp = R.hit_poise(data, key, tl[0][-1])
        return (tl[0][0], *hp) if hp else None

    # the bracket's counters: (time, poise damage, armorBreak), one per build with a melee weapon
    counters = []
    for b in R.bracket_builds(data, corpus, a.sl):
        w = next((w for w in (b.hands[R.HAND_SLOTS.index(s)][0] for s in R.MELEE_ORDER)
                  if w not in R.EMPTY and w in data.weapons and not data.weapons[w].get("isShield")
                  and not R.CATALYST.search(w) and w not in data.ranged), None)
        c = first_hit(w, False) if w else None
        if c:
            counters.append(c)

    if a.generate:
        weapon = data.sp_key.get(R.norm(a.generate))
        g = R.generate_build(data, corpus, weapon, a.infusion, a.sl, only_class=a.cls)
        by_name = {}
        for key, w in data.weapons.items():
            by_name.setdefault(w["name"], key)
        keys = [weapon] + [by_name[n] for n, _, _ in g["weapons_1h"] + g["weapons_2h_only"]]
        print(f"granted with {data.weapons[weapon]['name']} at SL {a.sl}: {len(keys)} weapons")
    else:
        keys = [data.sp_key.get(R.norm(n.strip())) for n in (a.weapons or "").split(",") if n.strip()]
    print(f"bracket counters: {len(counters)} 1H R1 first hits from SL {a.sl} builds")

    def window(key, two):
        att = next((attacks.get((R.norm(nm), ("Single2Hand" if two else "Single1Hand") + "Normal1st"))
                    for nm in names_of(key)
                    if attacks.get((R.norm(nm), ("Single2Hand" if two else "Single1Hand") + "Normal1st"))), None)
        rate = data.weapon_poise.get(key, (0, 0.0))[1]
        if not att or not rate:
            return [], 0.0
        spd = sum(att.get("spd") or [1.0, 1.0]) / 2
        return [(s / 30 / spd, e / 30 / spd) for s, e in R.tae_windows(att["anim"], R.TAE_HYPERARMOR)], rate

    print(f"{'weapon':32} {'hit s':>6} {'HA':>5} {'exposed':>8} {'need':>6} {'need90':>7}  trade at poise "
          + " ".join(f"{p:>4}" for p in LEVELS))
    rows = []
    for key in keys:
        if key not in data.weapons:
            continue
        two = not a.one_hand
        mine = first_hit(key, two)
        if mine is None:
            print(f"{data.weapons[key]['name'][:32]:32} no attack timing")
            continue
        t_hit = mine[0]
        wins, rate = window(key, two)
        n = len(counters)
        early = [(t, pd, ab) for t, pd, ab in counters if t < t_hit]
        # per early counter, the poise that swings through it (None: no poise does)
        cost = []
        for t, pd, ab in early:
            armored = any(s <= t < e for s, e in wins)
            if ab == 2 or (ab == 1 and not armored):
                cost.append(None)
            else:
                cost.append(pd * (rate if armored else 1.0))
        winnable = sorted(c for c in cost if c is not None)
        trade = lambda p: 1 - sum(1 for c in cost if c is None or c >= p) / max(n, 1)
        need = winnable[-1] + 0.1 if winnable else 0.0
        need90 = winnable[max(0, -(-len(winnable) * 9 // 10) - 1)] + 0.1 if winnable else 0.0
        rows.append((key, need))
        print(f"{data.weapons[key]['name'][:32]:32} {t_hit:6.2f} {rate:5.2f} {len(early) / max(n, 1):8.1%} "
              f"{need:6.1f} {need90:7.1f}  " + " ".join(f"{trade(p):4.0%}" for p in LEVELS))
    if a.generate and rows:
        needs = sorted(nd for _, nd in rows)
        print(f"\nmedian need across the granted weapons: {needs[len(needs) // 2]:.1f}; "
              f"weapons needing under 20 poise: {sum(1 for x in needs if x < 20)}/{len(needs)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
