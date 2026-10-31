#!/usr/bin/env python3
"""Evidence for the arsenal-depth score (ds2-mods-rs-c094): what weapon and spell variety corpus
builds carry, and whether the attack data tells movesets apart. Loads ds2-builds-recommend.py as a
module; caches nothing.

    scripts/ds2-arsenal-evidence.py survey      # scaling keys, infusions, weapon classes
"""
import importlib.util
import json
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("rec", HERE / "ds2-builds-recommend.py")
rec = importlib.util.module_from_spec(spec)
sys.modules["rec"] = rec
spec.loader.exec_module(rec)


def load(regulation: bool = True):
    sp, mm = rec.dump_site_tables(rec.CACHE / "site-tables")
    data = rec.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    if regulation:
        print(rec.apply_regulation(data)[:200], file=sys.stderr)
    return data


def survey(data):
    keys, cls, infs = Counter(), Counter(), Counter()
    for k, w in data.weapons.items():
        cls[data.weapon_class.get(k)] += 1
        for inf, r in (w.get("infusions") or {}).items():
            infs[inf] += 1
            keys.update(r.get("atkScale") or {})
    print("atkScale keys", keys.most_common())
    print("infusions", infs.most_common())
    print("classes", cls.most_common())
    for k in ("Uchigatana", "Greatsword", "Mace", "Broadsword"):
        if k in data.weapons:
            w = data.weapons[k]
            print(k, json.dumps(w["infusions"].get("No_Infusion")), json.dumps(w["infusions"].get("Mundane")),
                  w.get("require"))


MAIN_SLOTS = ["Normal1st", "Normal2nd", "Strong1st", "Strong2nd", "Rolling", "Dash", "Step", "Jump"]


def movesets(data):
    """Whether the attack data tells movesets apart: per weapon, the animation ids of its main
    slots in both grips; weapons with the same ids share a moveset. Prints per class how many
    weapons and how many distinct movesets."""
    attacks = rec.load_attacks(data)
    sig = {}
    for key, w in data.weapons.items():
        if key in rec.EMPTY:
            continue
        ids = []
        for g in ("Single1Hand", "Single2Hand"):
            for s in MAIN_SLOTS:
                a = attacks.get((rec.norm(w["name"]), g + s)) or attacks.get((rec.norm(key.replace("_", " ")), g + s))
                ids.append(a["anim"] if a else None)
        if any(ids):
            sig[key] = tuple(ids)
    by = {}
    for key, s in sig.items():
        by.setdefault(data.weapon_class.get(key) or "?", {}).setdefault(s, []).append(key)
    total_w = total_m = 0
    for c, groups in sorted(by.items(), key=lambda x: -len(x[1])):
        n = sum(len(v) for v in groups.values())
        total_w, total_m = total_w + n, total_m + len(groups)
        shared = [v for v in groups.values() if len(v) > 1]
        print(f"{c:20} weapons {n:3} movesets {len(groups):3}  shared: "
              + "; ".join(",".join(v) for v in shared)[:160])
    print(f"total weapons {total_w} movesets {total_m}; CH records carry no anim id")
    return sig


#: The stat line an uninfused weapon's scaling is read at, and how far each stat is raised.
AXIS_BASE, AXIS_STEP = 20, 20


def axis_gains(data, key):
    """How much the uninfused weapon's attack rating rises when each REQ_STAT goes from AXIS_BASE
    to AXIS_BASE + AXIS_STEP, the others held at AXIS_BASE (or its requirement)."""
    req = data.weapons[key].get("require") or {}
    base = {s: AXIS_BASE for s in rec.STATS}
    for s, v in req.items():
        base[s] = max(base.get(s, 0), int(v))
    ar0 = sum(rec.attack_rating(data, key, "No_Infusion", base).values())
    out = {}
    for s in rec.REQ_STATS:
        st = dict(base)
        st[s] += AXIS_STEP
        out[s] = sum(rec.attack_rating(data, key, "No_Infusion", st).values()) - ar0
    return out


def axes(data):
    """Each weapon's scaling gains, and the distribution of its second stat's gain over its
    first's: where 'both' should begin."""
    ratios = []
    rows = {}
    for key, w in data.weapons.items():
        if key in rec.EMPTY or rec.CATALYST.search(key) or w.get("isShield") or "No_Infusion" not in w.get("infusions", {}):
            continue
        g = axis_gains(data, key)
        top = sorted(g.values(), reverse=True)
        if top[0] <= 0:
            continue
        rows[key] = g
        ratios.append((top[1] / top[0], key, {s[:3]: round(v) for s, v in g.items()}))
    ratios.sort()
    for q in (0.1, 0.25, 0.5, 0.75, 0.9):
        print(f"q{q:.2f} second/first {ratios[int(q * (len(ratios) - 1))][0]:.2f}")
    for r, k, g in ratios[-40:]:
        print(f"{r:.2f} {k:28} {data.weapon_class.get(k)} {g}")
    return rows


#: A weapon scales on every stat whose gain is at least this share of its largest.
AXIS_BOTH = 0.75
#: Classes that are not melee movesets: shields, catalysts and launchers.
NOT_ARSENAL = {"Normal Shield", "Small Shield", "Greatshield", "Staff", "Chime", "Flame", "Bow", "Greatbow",
               "Crossbow"}


def axis(data, key, _cache={}):
    if key not in _cache:
        g = axis_gains(data, key)
        top = max(g.values())
        _cache[key] = frozenset(s for s, v in g.items() if top > 0 and v >= AXIS_BOTH * top)
    return _cache[key]


def best_rate(data, corpus, attacks, sc_args, key, eff, infusions=None):
    """The best moveset rate `key` reaches at `eff` over its infusions and the grips `eff` wields:
    (rate, infusion, two_hand). The infusion is picked by one-hit damage first, then the moveset
    is scored for it alone (moveset_metrics is the expensive part)."""
    w = data.weapons[key]
    dfn = sc_args[2]
    best_inf, best_d = None, -1.0
    for inf in infusions or w.get("infusions") or {}:
        ar = rec.attack_rating(data, key, inf, eff)
        if not ar:
            continue
        d = sum(rec.damage(k, v, dfn[k]) for k, v in ar.items())
        if d > best_d:
            best_inf, best_d = inf, d
    if best_inf is None:
        return None
    out = None
    for two in (False, True):
        if not rec.weapon_ok(data, key, eff, two):
            continue
        m = rec.moveset_metrics(data, corpus, attacks, key, best_inf, two, eff, sc_args[0], sc_args[1], dfn)
        if m and (out is None or m["moveset"] > out[0]):
            out = (m["moveset"], best_inf, two)
    return out


def spell_pool(data, eff, rings):
    """The damaging spells `eff` casts: requirement met, a wieldable catalyst of its school, a slot."""
    if rec.slots_of(data, eff, rings) < 1:
        return [], set()
    spells = [s for s in data.spell_hits if s in data.spell_category and rec.spell_ok(data, s, eff)]
    schools = {label for label, *_ in rec.best_catalysts(data, spells, eff)}
    spells = [s for s in spells if rec.SPELL_SCHOOLS[data.spell_category[s]][0] in schools]
    return spells, schools


def arsenal_rows(data, corpus, run: Path, level: int):
    attacks = rec.load_attacks(data)
    dfn, _ = rec.defender_defense(data, corpus, level)
    out = []
    for f in sorted(run.glob("*.json")):
        if f.stem == "builds":
            continue
        for r in json.loads(f.read_text()).get(str(level), []):
            key, st, worn = r["key"], r["stats"], r["rings"]
            eff = rec.gear_stats(data, st, worn)
            sc_args = (worn, level, dfn)
            mine = best_rate(data, corpus, attacks, sc_args, key, eff, [f.stem])
            if not mine:
                continue
            ax = axis(data, key)
            own = data.weapon_class.get(key) or "?"
            per_class = {}
            for k, w in data.weapons.items():
                c = data.weapon_class.get(k)
                if (k in rec.EMPTY or k == key or rec.CATALYST.search(k) or w.get("isShield") or k in data.ranged
                        or c in NOT_ARSENAL or c == own or not rec.weapon_ok(data, k, eff, True)
                        or "No_Infusion" not in (w.get("infusions") or {}) or not axis(data, k) <= ax):
                    continue
                got = best_rate(data, corpus, attacks, sc_args, k, eff)
                if got and got[0] > per_class.get(c or "?", (0,))[0]:
                    per_class[c or "?"] = (got[0], k, got[1])
            spells, schools = spell_pool(data, eff, worn)
            base = data.classes[r["class"]]
            inv = {s: st[s] - base.get(s, 0) for s in rec.STATS}
            out.append({"inf": f.stem, "weapon": key, "score": r["score"], "rate": mine[0], "axis": sorted(ax),
                        "classes": {c: (round(v / mine[0], 3), k, i) for c, (v, k, i) in
                                    sorted(per_class.items(), key=lambda x: -x[1][0])},
                        "spells": len(spells), "schools": sorted(schools),
                        "caster": inv["intelligence"] + inv["faith"] + inv["attunement"] > inv["strength"] + inv["dexterity"],
                        "stats": st})
            print(f"{f.stem:11} {key:28} rate {mine[0]:7.1f} axis {''.join(s[:3] for s in sorted(ax)):9} "
                  f"classes>=0.8 {sum(v[0] >= 0.8 * mine[0] for v in per_class.values()):2} "
                  f"spells {len(spells):3} {','.join(sorted(schools))}", file=sys.stderr, flush=True)
    return out


def rerank(data, corpus, run: Path, level: int, out: Path):
    """The recommender's own arsenal() over every row of a --best-weapons run at `level` (each
    row's score its own rate), its classes kept with every rate so that ARSENAL_K and
    ARSENAL_WEIGHT can be swept offline."""
    import time
    rec.ARSENAL_K = 99
    attacks = rec.load_attacks(data)
    dfn, _ = rec.defender_defense(data, corpus, level)
    rows = []
    for f in sorted(run.glob("*.json")):
        if f.stem == "builds":
            continue
        for r in json.loads(f.read_text()).get(str(level), []):
            t = time.monotonic()
            eff = rec.gear_stats(data, r["stats"], r["rings"])
            a = rec.arsenal(data, corpus, attacks, r["key"], eff, r["rings"], level, dfn, r["score"])
            rows.append({"inf": f.stem, "weapon": r["key"], "score": r["score"],
                         "axis": sorted(rec.scaling_axis(data, r["key"])), "classes": a["arsenal_classes"]})
            print(f"{f.stem:11} {r['key']:26} {r['score']:7.1f} {len(a['arsenal_classes']):2} classes "
                  f"{time.monotonic() - t:4.1f}s  " + ", ".join(f"{c[0]} {c[4] / r['score']:.2f}"
                                                         for c in a["arsenal_classes"][:6]),
                  file=sys.stderr, flush=True)
    out.write_text(json.dumps(rows, indent=1))


def sweep(path: Path, ks=(1, 3, 5, 9, 13), ws=(0.0, 0.1, 0.25, 0.5)):
    """rerank's rows re-ranked per (ARSENAL_K, ARSENAL_WEIGHT): the top five, and where each
    infusion's best row lands."""
    rows = json.loads(path.read_text())
    for k in ks:
        for w in ws:
            def value(r):
                own = r["score"]
                mean = (own + sum(min(own, c[4]) for c in r["classes"][:k - 1])) / k
                return (1 - w) * own + w * mean
            ranked = sorted(rows, key=lambda r: -value(r))
            first = {}
            for i, r in enumerate(ranked):
                first.setdefault(r["inf"], i + 1)
            print(f"K {k:2} W {w:.2f}: " + " | ".join(f"{r['inf'][:5]} {r['weapon'][:14]} {value(r):6.0f}"
                                                  for r in ranked[:5])
                  + "   Mundane first at #" + str(first.get("Mundane")))


def corpus_variety(data, corpus, lo: int, hi: int):
    """What corpus builds at soul levels lo..hi carry: distinct melee weapon classes in their six
    hand slots, attuned spells and spell schools, casters (INT+FTH+ATT put in over the class base
    beats STR+DEX) against the rest."""
    import statistics as stt
    groups = {True: [], False: []}
    for b in corpus:
        sl = rec.soul_level(data, b)
        if not lo <= sl <= hi:
            continue
        base = data.classes.get(b.cls, {})
        inv = {s: b.stats[s] - base.get(s, 0) for s in rec.STATS}
        caster = inv["intelligence"] + inv["faith"] + inv["attunement"] > inv["strength"] + inv["dexterity"]
        classes = {data.weapon_class.get(w) or "?" for w, _ in b.weapons()
                   if w in data.weapons and not rec.CATALYST.search(w) and not data.weapons[w].get("isShield")
                   and data.weapon_class.get(w) not in NOT_ARSENAL}
        spells = [s for s in b.spells if s in data.spells]
        schools = {rec.SPELL_SCHOOLS[data.spell_category[s]][0] for s in spells if s in data.spell_category}
        groups[caster].append((len(classes), len(spells), len(schools)))
    for caster, rows in groups.items():
        if not rows:
            continue
        cols = list(zip(*rows))
        print(f"SL {lo}-{hi} {'casters' if caster else 'non-casters':11} n={len(rows):5} "
              + "  ".join(f"{name} mean {stt.mean(c):.2f} median {stt.median(c):g} p75 {sorted(c)[3 * len(c) // 4]}"
                          for name, c in zip(("classes", "spells", "schools"), cols)))


if __name__ == "__main__":
    what = sys.argv[1] if len(sys.argv) > 1 else "survey"
    if what == "survey":
        survey(load(regulation=False))
    elif what == "movesets":
        movesets(load())
    elif what == "axes":
        axes(load())
    elif what == "corpus":
        data = load()
        corpus, _ = rec.load_corpus(data)
        for lo, hi in rec.SL_BRACKETS[:8]:
            corpus_variety(data, corpus, lo, hi)
    elif what == "sweep":  # sweep RERANK.json
        sweep(Path(sys.argv[2]))
    elif what == "rerank":  # rerank RUN_DIR LEVEL OUT.json
        data = load()
        corpus, _ = rec.load_corpus(data)
        rerank(data, corpus, Path(sys.argv[2]), int(sys.argv[3]), Path(sys.argv[4]))
    elif what == "arsenal":  # arsenal RUN_DIR LEVEL OUT.json
        data = load()
        corpus, _ = rec.load_corpus(data)
        rows = arsenal_rows(data, corpus, Path(sys.argv[2]), int(sys.argv[3]))
        Path(sys.argv[4]).write_text(json.dumps(rows, indent=1))
