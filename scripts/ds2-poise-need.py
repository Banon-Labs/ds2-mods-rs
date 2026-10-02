#!/usr/bin/env python3
"""How much poise each weapon's R1 needs to trade with the SL bracket, and where more stops paying.

Read-only. The trade model: you and a bracket opponent press R1 at the same instant. The opponent's
counter-hit is their melee weapon's 1H R1 first hit (the weapon bracket_poise's build_counter picks
per build; `--counter-grip build` swings it in the build's own grip instead), with its time from
input, poise damage and armorBreak. Your R1's first hit lands at its
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

Times are chain_timeline's: TAE frame / 30 over the mean of the attack's start/end play speeds.
`--play-speed start|end` plays every attack at one of the two instead, the bounds of the handover
the EXE leaves to the animation network's event tracks (docs/DS2-DPS-MECHANICS.md "Attack play
speed"). `--play-speed game` plays each animation the way those tracks say: at startPlaySpeed
inside its event-150 windows, endPlaySpeed inside its 151 windows and 1.0 elsewhere, read from
the player's anibnd (`scripts/ds2-anibnd.py windows` writes the table this reads); an animation
with no such track plays at 1.0. The EXE multiplies the start speed by sCharacterFlags+0x200, which
both writers found store 0.0 -- taken literally the windup would never play -- so the factor is
taken as 1.0 here and is the one unread number left (docs/DS2-DPS-MECHANICS.md).

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
SPEED_WINDOWS = Path.home() / ".cache/ds2-builds/attack-speed-windows.json"
#: sCharacterFlags+0x200, the EXE's factor on startPlaySpeed inside a 150 window. Not read (see the
#: module docstring); 1.0 is the value under which the start speed is the start speed.
START_FACTOR = 1.0


def game_time(table: dict, anim: int, spd: list, t: float) -> float:
    """Seconds of play to reach animation time `t` (seconds) of `anim`: the rate is startPlaySpeed
    x START_FACTOR while a 150 window is open (it wins when both are, as in 0x14035d580),
    endPlaySpeed while a 151 is, else 1.0."""
    if t <= 0:
        return t
    windows = (table.get(str(anim)) or {}).get("windows") or []
    cuts = sorted({0.0, t, *(x for _, s, e in windows for x in (s, e) if 0.0 < x < t)})
    real = 0.0
    for a, b in zip(cuts, cuts[1:]):
        mid = (a + b) / 2
        open_ = {u for u, s, e in windows if s <= mid < e}
        rate = spd[0] * START_FACTOR if 150 in open_ else spd[1] if 151 in open_ else 1.0
        real += (b - a) / rate
    return real


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
    ap.add_argument("--counter-grip", choices=("1h", "build"), default="1h",
                    help="the grip of each bracket build's counter R1: 1h (bracket_poise's model) or build, "
                         "the build's own: two-handed when its grip is 1 and the counter weapon is in rh1, "
                         "the slot the build two-hands (weapon_ok's rule)")
    ap.add_argument("--play-speed", choices=("mean", "start", "end", "game"), default="mean",
                    help="each attack's play speed: mean of WeaponAttackMotionParam start/endPlaySpeed "
                         "(chain_timeline's rule), all start or all end (the bounds), or game: start "
                         "inside the animation's event-150 windows, end inside its 151 windows, 1.0 "
                         "elsewhere (scripts/ds2-anibnd.py windows)")
    a = ap.parse_args()
    speed_table = None
    if a.play_speed == "game":
        if not SPEED_WINDOWS.exists():
            raise SystemExit(f"{SPEED_WINDOWS} missing: run scripts/ds2-anibnd.py windows")
        speed_table = json.loads(SPEED_WINDOWS.read_text())
    unwindowed = set()

    def real(att, frame):
        """Seconds from input to TAE frame `frame` of attack `att` under --play-speed."""
        spd = att.get("spd") or [1.0, 1.0]
        if speed_table is None:
            return frame / 30 / (sum(spd) / 2)
        if str(att["anim"]) not in speed_table:
            unwindowed.add(att["anim"])
        return game_time(speed_table, att["anim"], spd, frame / 30)

    R = load_recommender()
    sp_json, mm_json = R.dump_site_tables(R.CACHE / "site-tables")
    data = R.Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    R.apply_regulation(data)
    corpus, _ = R.load_corpus(data)
    attacks = R.load_attacks(data)
    if a.play_speed in ("start", "end"):
        # Every attack played at one of its two speeds throughout, for both sides and the 111900
        # window: the bounds of where startPlaySpeed hands over to endPlaySpeed (docs/DS2-DPS-MECHANICS.md
        # "Attack play speed").
        k = 0 if a.play_speed == "start" else 1
        for att in attacks.values():
            s = att.get("spd") or [1.0, 1.0]
            att["spd"] = [s[k], s[k]]

    def names_of(key):
        return [data.weapons[key]["name"], key.replace("_", " ")]

    def first_attack(key, two):
        slot = ("Single2Hand" if two else "Single1Hand") + "Normal1st"
        return next((attacks[(R.norm(nm), slot)] for nm in names_of(key) if (R.norm(nm), slot) in attacks), None)

    def first_hit(key, two):
        if speed_table is not None:
            # the R1's earliest live hitbox, timed through the animation's speed windows
            att = first_attack(key, two)
            hs = R.live_hits(att) if att else []
            if not hs:
                return None
            h = min(hs, key=lambda h: h["start"])
            hp = R.hit_poise(data, key, str(h.get("dmg")))
            return (real(att, h["start"]), *hp) if hp else None
        tl = next((t for t in (R.chain_timeline(attacks, nm, two, "Normal", 3.0, with_row=True)
                               for nm in names_of(key)) if t), None)
        if not tl:
            return None
        hp = R.hit_poise(data, key, tl[0][-1])
        return (tl[0][0], *hp) if hp else None

    # the bracket's counters: (time, poise damage, armorBreak), one per build with a melee weapon
    counters = []
    builds = R.bracket_builds(data, corpus, a.sl)
    no_melee = no_timing = two_handed = 0
    for b in builds:
        slot, w = next(((s, w) for s, w in ((s, b.hands[R.HAND_SLOTS.index(s)][0]) for s in R.MELEE_ORDER)
                        if w not in R.EMPTY and w in data.weapons and not data.weapons[w].get("isShield")
                        and not R.CATALYST.search(w) and w not in data.ranged), (None, None))
        if not w:
            no_melee += 1
            continue
        two = a.counter_grip == "build" and b.grip == 1 and slot == "rh1"
        c = first_hit(w, two)
        if c is None and two:  # a weapon with no 2H R1 row is swung 1H
            c, two = first_hit(w, False), False
        if c:
            counters.append(c)
            two_handed += two
        else:
            no_timing += 1

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
    print(f"bracket counters: {len(counters)} R1 first hits ({two_handed} two-handed, grip {a.counter_grip}) "
          f"from {len(builds)} SL {a.sl} builds; no counter: {no_melee} without a melee weapon, "
          f"{no_timing} without attack timing or poise rows")

    def window(key, two):
        att = first_attack(key, two)
        rate = data.weapon_poise.get(key, (0, 0.0))[1]
        if not att or not rate:
            return [], 0.0
        return [(real(att, s), real(att, e)) for s, e in R.tae_windows(att["anim"], R.TAE_HYPERARMOR)], rate

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
    if speed_table is not None:
        print(f"play speed: anibnd windows, start factor {START_FACTOR}; {len(unwindowed)} animations "
              f"with no 150/151 track played at 1.0: {sorted(unwindowed)[:20]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
