#!/usr/bin/env python3
"""Recommend weapons, rings, spells and armor for a DARK SOULS II PvP build.

    python3 scripts/ds2-builds-recommend.py --mugen 33402
    python3 scripts/ds2-builds-recommend.py --soulsplanner 23874
    python3 scripts/ds2-builds-recommend.py --eval

Design: docs/DS2-BUILD-EMBEDDINGS.md. This is the first working cut, and it deviates from that
design in ways that are said out loud here rather than hidden:

* ITEM DATA IS THE PLANNER SITES', NOT THE GAME'S. Weapon/armor requirements, infusion attack and
  scaling, armor defenses and weights come from SoulsPlanner's `ds2planner.min.js`; spell INT/FTH
  requirements from MugenMonkey's `ds2application-*.js` (SoulsPlanner has none). Both are dumped to
  JSON by `--sp-data` / `--mm-data` (see `dump_site_tables`). The design wants regulation params
  and executable-derived formulas; neither is wired in yet.
* ARMOR DAMAGE MODEL IS LINEAR. A piece's value is its per-type defense weighted by the share of
  each damage type the corpus's weapons deal. The game's defense curve is not linear; the
  executable's formula replaces this once it is recovered.
* CORPUS IS SOULSPLANNER ONLY: the mirror at ~/.cache/soulsplanner (6618 builds, the whole public
  list). MugenMonkey builds can be queried but are not in the training set yet.
* Ring stat bonuses (Ring of the Embedded etc.) are ignored: the site encodes them as functions of
  the build. Armor stat bonuses are counted.

Pipeline: filter the corpus (complete, not untouched class stats, every item usable) -> dedupe ->
fit EASE over {weapon, weapon|infusion, ring, spell} tokens -> for the query, score candidates by
EASE + stat-neighbour frequency + scaling fit, drop anything the build cannot use, rank per slot.
Armor is chosen separately by the defense optimizer under a 70% equip-load cap.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.request
from collections import Counter
from pathlib import Path

import numpy as np

CACHE = Path.home() / ".cache/soulsplanner"
STATS = ["vigor", "endurance", "vitality", "attunement", "strength", "dexterity", "adaptability",
         "intelligence", "faith"]
REQ_STATS = ["strength", "dexterity", "intelligence", "faith"]
ARMOR_SLOTS = ["head", "chest", "hands", "legs"]
HAND_SLOTS = ["lh1", "rh1", "lh2", "rh2", "lh3", "rh3"]
DMG = ["physical", "magic", "fire", "lightning", "dark"]
PHYS_TYPES = ["slash", "strike", "thrust"]  # physical subtypes, each with its own armor defense
CATALYST = re.compile(r"Staff|Chime|Pyromancy_Flame|Talisman|Scepter|Branch|Bellvine|Sunlight_Blade|Rod", re.I)
EMPTY = {"", "Bare_Fists", "No_Spell", "No_Ring", "Naked", "No_Item", "None"}
EQUIP_CAP = 0.70
# The user's brackets. SL: inclusive ranges. Stats: 0-10, 11-20, ..., 81-90, 91-99.
SL_BRACKETS = [(1, 20), (21, 40), (41, 70), (71, 100), (101, 125), (126, 155), (156, 200), (201, 250),
               (251, 10**6)]


def sl_bracket(sl: int) -> int:
    return next(i for i, (lo, hi) in enumerate(SL_BRACKETS) if sl <= hi)


def stat_bracket(v: int) -> int:
    return 0 if v <= 10 else min(9, (v - 1) // 10)


def agility(adp: int, att: int) -> int:
    """Displayed AGL. Both planner sites' formula (SoulsPlanner getAgility, MugenMonkey Agility),
    and the community rule investment = floor(0.75 ADP + 0.25 ATT). Linear to 110 at
    3*ADP+ATT = 120, then one point per 28 of 3*ADP+ATT; 120 only at 99/99; floor 85."""
    if adp >= 99 and att >= 99:
        return 120
    x = 3 * adp + att
    return max(85, 80 + x // 4 if x <= 120 else 110 + (x - 120) // 28)


def soul_level(data, b) -> int:
    base = data.classes.get(b.cls, {})
    return base.get("level", 0) + sum(b.stats[s] - base.get(s, 0) for s in STATS)


def norm(name: str) -> str:
    return re.sub(r"[^a-z0-9]", "", name.lower())


# --------------------------------------------------------------------------------------------
# site tables

SP_DUMP_JS = r"""
const fs=require('fs');const s=fs.readFileSync(process.argv[1],'utf8');
const a=s.indexOf('var DarkSouls2={}'),b=s.indexOf('DarkSouls2.Attribute=');
const D=new Function(s.slice(a,b)+';return DarkSouls2;')();
const ST=%s;
function p(o){if(o===null||typeof o!=='object')return o;if(Array.isArray(o))return o.map(p);const r={};
 for(const[k,v]of Object.entries(o)){if(typeof v==='function'){if(ST.includes(k)){try{r[k]=v();}catch(e){}}continue;}r[k]=p(v);}return r;}
const out={};for(const k of ['classes','spells','head','chest','hands','legs','weapons','rings','attunementSlots','equipmentLoad',
 'physicalATKBonus','magicATKBonus','fireATKBonus','lightningATKBonus','darkATKBonus','auxATKBonus','mundaneATKBonus',
 'physicalDEFBonus','magicDEFBonus','fireDEFBonus','lightningDEFBonus','darkDEFBonus'])out[k]=p(D[k]);
process.stdout.write(JSON.stringify(out));
""" % json.dumps(STATS)

MM_DUMP_JS = r"""
const fs=require('fs');const s=fs.readFileSync(process.argv[1],'utf8');
function grab(n){const key='window.'+n+'=';let i=s.indexOf(key);i+=key.length;let d=0,q=null;
 for(let j=i;j<s.length;j++){const c=s[j];if(q){if(c==='\\'){j++;continue;}if(c===q)q=null;continue;}
  if(c==='"'||c==="'")q=c;else if(c==='{'||c==='[')d++;else if(c==='}'||c===']'){d--;if(d===0)return new Function('return '+s.slice(i,j+1))();}}}
function rows(t){const sc=t.schema,o={};for(const[k,v]of Object.entries(t)){if(k==='schema')continue;const r={name:k};sc.forEach((f,i)=>r[f]=v[i]);o[k]=r;}return o;}
const out={};for(const n of ['darkSouls2Armor','darkSouls2Rings','darkSouls2Spells','darkSouls2WeaponDetails'])out[n]=rows(grab(n));
process.stdout.write(JSON.stringify(out));
"""


def fetch(url: str) -> str:
    req = urllib.request.Request(url, headers={"User-Agent": "ds2-mods-rs build research"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode("utf-8", "replace")


def dump_site_tables(out_dir: Path) -> tuple[Path, Path]:
    """Fetch both planners' JS and dump their item tables to JSON with node. Cached on disk."""
    sp_json, mm_json = out_dir / "sp-tables.json", out_dir / "mm-tables.json"
    if sp_json.exists() and mm_json.exists():
        return sp_json, mm_json
    sp_js = out_dir / "ds2planner.min.js"
    sp_js.write_text(fetch("https://soulsplanner.com/public/scripts/release/DarkSouls2/ds2planner.min.js"))
    page = fetch("https://mugenmonkey.com/darksouls2/33402")
    src = re.search(r'src="(/assets/ds2application-[^"]+\.js)"', page).group(1)
    mm_js = out_dir / "ds2application.js"
    mm_js.write_text(fetch("https://mugenmonkey.com" + src))
    for js, code, dst in ((sp_js, SP_DUMP_JS, sp_json), (mm_js, MM_DUMP_JS, mm_json)):
        out = subprocess.run(["node", "-e", code, str(js)], check=True, capture_output=True, text=True).stdout
        dst.write_text(out)
    return sp_json, mm_json


class Data:
    def __init__(self, sp: dict, mm: dict):
        self.sp = sp
        self.weapons = sp["weapons"]
        self.armor = {s: sp[s] for s in ARMOR_SLOTS}
        self.rings = sp["rings"]
        self.spells = sp["spells"]
        self.classes = sp["classes"]
        self.att_slots = sp["attunementSlots"]
        self.equip_load = sp["equipmentLoad"]
        # SoulsPlanner has no spell requirements; MugenMonkey does. Joined by normalized name.
        mm_sp = {norm(k): v for k, v in mm["darkSouls2Spells"].items()}
        mm_sp.setdefault("perseverance", mm_sp.get("perserverance"))  # MugenMonkey's spelling
        self.spell_req = {}
        for key, sp_row in self.spells.items():
            m = mm_sp.get(norm(sp_row["name"])) or mm_sp.get(norm(key))
            if m:
                self.spell_req[key] = {"intelligence": int(m["intReq"] or 0), "faith": int(m["faithReq"] or 0)}
        self.mm = mm
        # weapon class (Dagger, Greatsword, ...): MugenMonkey only, joined by normalized name
        mm_w = {}
        for k, v in mm["darkSouls2WeaponDetails"].items():
            for pat, full in MugenIds.ABBREV:
                k = pat.sub(full, k)
            mm_w[norm(k)] = v["type"]
        self.weapon_class = {k: mm_w.get(norm(v.get("name", k))) or mm_w.get(norm(k)) for k, v in self.weapons.items()}
        self.key_by_name = {v["name"]: k for k, v in self.weapons.items()}
        # name -> SoulsPlanner key, for translating MugenMonkey builds
        self.sp_key = {}
        for table in [self.weapons, self.rings, self.spells, *self.armor.values()]:
            for k, v in table.items():
                self.sp_key.setdefault(norm(v.get("name", k)), k)
                self.sp_key.setdefault(norm(k), k)


# --------------------------------------------------------------------------------------------
# builds

class Build:
    def __init__(self, cls, stats, armor, hands, grip, rings, spells, level=None, label=""):
        self.cls, self.stats, self.armor, self.hands = cls, stats, armor, hands
        self.grip, self.rings, self.spells, self.level, self.label = grip, rings, spells, level, label

    def weapons(self):
        return [(w, i) for w, i in self.hands if w not in EMPTY]


def sp_build(raw: dict, level=None, label="") -> Build:
    w = raw["weapons"].split(";")
    hands = [(w[2 * k], w[2 * k + 1] or "No_Infusion") for k in range(6)]
    spells = [s for s in raw["spells"].split(";") if s not in EMPTY]
    return Build(raw["class_"], {s: int(raw[s]) for s in STATS}, raw["armor"].split(";"), hands,
                 int(raw["grip"]), raw["rings"].split(";"), spells, level, label)


class MugenIds:
    """MugenMonkey numeric ids -> SoulsPlanner keys. Unmapped names are counted, not guessed."""

    def __init__(self, data: Data):
        mm = data.mm
        self.data = data
        self.armor = {(v["type"], v["id"]): v["name"] for v in mm["darkSouls2Armor"].values()}
        self.weapon = {v["id"]: v["name"] for v in mm["darkSouls2WeaponDetails"].values()}
        self.ring = {v["id"]: v["name"] for v in mm["darkSouls2Rings"].values()}
        self.spell = {v["id"]: v["name"] for v in mm["darkSouls2Spells"].values()}
        self.unmapped = Counter()

    # MugenMonkey's own misspellings and abbreviations, seen in its build data
    ALIAS = {"sanctumsolidergauntlets": "Sanctum Soldier Gauntlet",
             "penalstraighjacket": "Penal Straightjacket",
             "tightsofjudgement": "Tights of Judgment",
             "desertsorcesresshood": "Desert Sorceress Hood",
             "desertsoceressgloves": "Desert Sorceress Gloves",
             "wanderermanchettes": "Wanderer Manchette",
             "bonekingarmor": "Bone King Robe",
             "royalsoliderhelm": "Royal Soldier Helm",
             "hardleatherleggings": "Hard Leather Boots",
             "bersekerblade": "Berserker Blade",
             "creightonshelm": "Creightons Steel Mask",
             "royalsolidersleggings": "Royal Soldier Leggings",
             "madwarriorgauntlets": "Mad Warrior Gauntlet",
             "monastarylonggloves": "Monastery Long Gloves",
             "prisonerswaistloth": "Prisoners Waistcloth",
             "astrologistsrobes": "Astrologists Robe",
             "perserverance": "Perseverance",
             "hollowsoliderhelm": "Hollow Soldier Helm",
             "whitepriestheadset": "White Priest Headpiece"}
    ABBREV = [(re.compile(r"\bUGS\b"), "Ultra Greatsword"), (re.compile(r"\bGS\b"), "Greatsword"),
              (re.compile(r"\bGA\b"), "Greataxe")]

    def key(self, name):
        if not name or name.strip() in ("No Ring", "No Spell", "Naked", "No Armor") or name.startswith("default"):
            return ""  # "defaulthead" etc. is MugenMonkey's empty armor slot
        # A trailing note in parentheses is the author's, not the item's: "Ruler's Sword (1mil)",
        # "Curved Nil Greatsword (NG)", "Prisoner's Tatters (2)".
        name = re.sub(r"\s*\([^)]*\)\s*$", "", name)
        name = self.ALIAS.get(norm(name), name)
        for pat, full in self.ABBREV:
            name = pat.sub(full, name)
        k = self.data.sp_key.get(norm(name))
        if k is None:
            self.unmapped[name] += 1
        return k or "?" + name  # "?" keys fail every completeness check downstream


def mugen_from_stats(ids: MugenIds, st: dict, label: str = "") -> Build:
    armor = [ids.key(ids.armor.get((s, st[s]))) for s in ARMOR_SLOTS]
    hands = [(ids.key(ids.weapon.get(st[s])) if st[s] else "", "?")  # MugenMonkey stores no infusion
             for s in HAND_SLOTS]
    rings = [ids.key(ids.ring.get(st[f"ring{i}"])) for i in range(1, 5)]
    spells = [ids.key(ids.spell.get(st.get(f"spell{i}"))) for i in range(10) if st.get(f"spell{i}")]
    stats = {s: int(st[s.capitalize()]) for s in STATS}
    return Build(str(st["starting_class"]).lower(), stats, armor, hands, 1 if st.get("two_hand") else 0,
                 rings, [s for s in spells if s], label=label)


def mugen_build(data: Data, build_id: int) -> Build:
    st = None
    cached = MUGEN_CACHE / "builds.jsonl"
    if cached.exists():
        for line in cached.read_text().splitlines():
            if line.startswith(f'{{"id": "{build_id}"'):
                st = json.loads(line)["build"]
                break
    if st is None:
        st = json.loads(fetch(f"https://mugenmonkey.com/darksouls2/{build_id}.json"))
    ids = MugenIds(data)
    b = mugen_from_stats(ids, st, f"MugenMonkey {build_id}: {st.get('title', '')}")
    for name in ids.unmapped:
        print(f"  [unmapped] {name!r}", file=sys.stderr)
    return b


def effective(data: Data, b: Build) -> dict:
    eff = dict(b.stats)
    for slot, piece in zip(ARMOR_SLOTS, b.armor):
        for s, v in (data.armor[slot].get(piece, {}).get("alter") or {}).items():
            if s in eff and isinstance(v, (int, float)):
                eff[s] += v
    return eff


def weapon_ok(data: Data, name: str, eff: dict, two_hand: bool) -> bool:
    req = data.weapons.get(name, {}).get("require") or {}
    for s, v in req.items():
        have = eff.get(s, 0)
        need = v // 2 if (two_hand and s == "strength") else v  # game halves STR req when two-handed
        if need > have:
            return False
    return True


def armor_ok(data: Data, slot: str, piece: str, eff: dict) -> bool:
    req = data.armor[slot].get(piece, {}).get("require") or {}
    return all(v <= eff.get(s, 0) for s, v in req.items())


def spell_ok(data: Data, spell: str, eff: dict) -> bool:
    req = data.spell_req.get(spell)
    return req is not None and all(v <= eff.get(s, 0) for s, v in req.items())


def slots_of(data: Data, eff: dict) -> int:
    return data.att_slots[min(eff["attunement"], len(data.att_slots) - 1)]


def keep(data: Data, b: Build) -> str | None:
    """None if the build is kept, else the reason it is dropped."""
    if any(p in EMPTY or p not in data.armor[s] for s, p in zip(ARMOR_SLOTS, b.armor)):
        return "armor incomplete"
    if not b.weapons():
        return "no weapon"
    if any(r in EMPTY or r not in data.rings for r in b.rings):
        return "rings incomplete"
    base = data.classes.get(b.cls, {})
    if all(b.stats[s] == base.get(s) for s in STATS) or b.level == 1:
        return "untouched class stats / SL1"
    eff = effective(data, b)
    for idx, (w, _) in enumerate(b.hands):
        if w in EMPTY:
            continue
        if w not in data.weapons:
            return "unknown weapon"
        if not weapon_ok(data, w, eff, two_hand=(b.grip == 1 and HAND_SLOTS[idx] == "rh1")):
            return "weapon requirement unmet"
    for s, p in zip(ARMOR_SLOTS, b.armor):
        if not armor_ok(data, s, p, eff):
            return "armor requirement unmet"
    has_catalyst = any(CATALYST.search(w) for w, _ in b.weapons())
    if has_catalyst and not b.spells:
        return "catalyst but no spells"
    if b.spells:
        if any(s not in data.spells for s in b.spells):
            return "unknown spell"
        if any(not spell_ok(data, s, eff) for s in b.spells):
            return "spell requirement unmet"
        if sum(data.spells[s]["slots"] for s in b.spells) > slots_of(data, eff):
            return "too many spell slots"
    return None


def tokens(b: Build) -> set[str]:
    t = set()
    for w, inf in b.weapons():
        t.add("W:" + w)
        if inf != "?":
            t.add("WI:" + w + "|" + inf)
    t.update("R:" + r for r in b.rings if r not in EMPTY)
    t.update("S:" + s for s in b.spells)
    return t


# --------------------------------------------------------------------------------------------
# model

def ease(X: np.ndarray, lam: float) -> np.ndarray:
    G = X.T @ X + lam * np.eye(X.shape[1])
    P = np.linalg.inv(G)
    B = -P / np.diag(P)
    np.fill_diagonal(B, 0.0)
    return B


class Model:
    def __init__(self, data: Data, corpus: list[Build], lam: float = 50.0):
        self.data = data
        self.builds = corpus
        vocab = Counter(t for b in corpus for t in tokens(b))
        self.vocab = sorted(t for t, c in vocab.items() if c >= 2)
        self.ix = {t: i for i, t in enumerate(self.vocab)}
        self.X = np.zeros((len(corpus), len(self.vocab)), dtype=np.float64)
        for r, b in enumerate(corpus):
            for t in tokens(b):
                if t in self.ix:
                    self.X[r, self.ix[t]] = 1.0
        self.B = ease(self.X, lam)
        self.pop = self.X.sum(0)
        self.SB = np.array([[stat_bracket(effective(data, b)[s]) for s in STATS] for b in corpus])
        self.SLB = np.array([sl_bracket(soul_level(data, b)) for b in corpus])

    def vec(self, toks) -> np.ndarray:
        x = np.zeros(len(self.vocab))
        for t in toks:
            if t in self.ix:
                x[self.ix[t]] = 1.0
        return x

    def neighbours(self, eff: dict, sl: int, k: int = 50) -> np.ndarray:
        """Items used by the k builds in the same SL bracket whose stat brackets are closest."""
        q = np.array([stat_bracket(eff[s]) for s in STATS])
        same = np.flatnonzero(self.SLB == sl_bracket(sl))
        if len(same) == 0:
            return np.zeros(len(self.vocab))
        d = np.abs(self.SB[same] - q).sum(1)
        idx = same[np.argsort(d, kind="stable")[:k]]
        return self.X[idx].mean(0)

    def scale_fit(self, tok: str, eff: dict, base: dict) -> float:
        """How much of the weapon+infusion's stat scaling sits on stats this build invested in."""
        if not tok.startswith("WI:"):
            return 0.0
        w, inf = tok[3:].split("|")
        sc = (self.data.weapons[w]["infusions"].get(inf) or {}).get("atkScale") or {}
        inv = {s: max(0, eff[s] - base.get(s, 0)) for s in STATS}
        tot = sum(inv.values()) or 1
        # physical scaling follows STR/DEX; elemental scaling follows the stat that feeds it
        feed = {"strength": ["strength"], "dexterity": ["dexterity"], "magic": ["intelligence"],
                "fire": ["intelligence", "faith"], "lightning": ["faith"], "dark": ["intelligence", "faith"]}
        return sum(v * sum(inv[s] for s in feed.get(k, [])) / tot for k, v in sc.items() if k in feed)


# --------------------------------------------------------------------------------------------
# armor

def threat_mix(data: Data, corpus: list[Build]) -> dict:
    tot = Counter()
    for b in corpus:
        for w, inf in b.weapons():
            atk = (data.weapons[w]["infusions"].get(inf) or {}).get("atk") or {}
            for k in DMG:
                tot[k] += atk.get(k, 0)
    s = sum(tot.values()) or 1
    return {k: tot[k] / s for k in DMG}


def best_armor(data: Data, b: Build, eff: dict, mix: dict, top: int = 3):
    cap = data.equip_load[min(eff["vitality"], len(data.equip_load) - 1)] * EQUIP_CAP
    carried = sum(data.weapons.get(w, {}).get("weight", 0) for w, _ in b.weapons())
    carried += sum(data.rings.get(r, {}).get("weight", 0) for r in b.rings if r in data.rings)
    budget = cap - carried

    def value(p):
        return sum(mix[k] * p.get(k + "DEF", 0) for k in DMG)

    fronts = []
    for slot in ARMOR_SLOTS:
        cands = sorted(((v.get("weight", 0), value(v), k) for k, v in data.armor[slot].items()
                        if armor_ok(data, slot, k, eff)), key=lambda t: (t[0], -t[1]))
        front, best = [], -1
        for w, val, k in cands:  # pareto: keep only pieces that beat every lighter one
            if val > best:
                front.append((w, val, k))
                best = val
        fronts.append(front)
    sets = []
    for h in fronts[0]:
        for c in fronts[1]:
            if h[0] + c[0] > budget:
                break
            for g in fronts[2]:
                if h[0] + c[0] + g[0] > budget:
                    break
                for l in fronts[3]:
                    w = h[0] + c[0] + g[0] + l[0]
                    if w > budget:
                        break
                    sets.append((h[1] + c[1] + g[1] + l[1], w, (h[2], c[2], g[2], l[2])))
    sets.sort(reverse=True)
    return cap, carried, sets[:top]


# --------------------------------------------------------------------------------------------

MUGEN_CACHE = Path.home() / ".cache/mugenmonkey"


def raw_builds(data: Data, mugen: bool):
    raw = json.loads((CACHE / "builds-darksouls2.json").read_text())
    index = {str(r["id"]): r for r in json.loads((CACHE / "index-darksouls2.json").read_text())}
    for bid, r in raw.items():
        meta = index.get(bid, {})
        yield "sp", sp_build(r, meta.get("level"), meta.get("name", ""))
    path = MUGEN_CACHE / "builds.jsonl"
    if not (mugen and path.exists()):
        return
    levels = {}
    if (MUGEN_CACHE / "index.jsonl").exists():
        for line in (MUGEN_CACHE / "index.jsonl").read_text().splitlines():
            m = json.loads(line)
            levels[m["id"]] = m.get("level")
    ids = MugenIds(data)
    for line in path.read_text().splitlines():
        r = json.loads(line)
        try:
            b = mugen_from_stats(ids, r["build"], f"MugenMonkey {r['id']}")
        except (KeyError, TypeError, ValueError):
            continue
        b.level = levels.get(r["id"])
        yield "mm", b
    if ids.unmapped:
        top = ", ".join(f"{n} x{c}" for n, c in ids.unmapped.most_common(8))
        print(f"MugenMonkey names with no SoulsPlanner match ({len(ids.unmapped)}): {top}", file=sys.stderr)


def load_corpus(data: Data, mugen: bool = True) -> tuple[list[Build], Counter]:
    kept, why, seen = [], Counter(), set()
    for src, b in raw_builds(data, mugen):
        reason = keep(data, b)
        if reason:
            why[f"{src}: {reason}"] += 1
            continue
        sig = (frozenset(tokens(b)), tuple(b.stats[s] for s in STATS))
        if sig in seen:
            why[f"{src}: duplicate"] += 1
            continue
        seen.add(sig)
        why[f"{src}: kept"] += 1
        kept.append(b)
    why["kept"] = len(kept)
    return kept, why


def recommend(model: Model, b: Build, k: int = 10):
    data = model.data
    eff = effective(data, b)
    base = data.classes.get(b.cls, {})
    have = tokens(b)
    x = model.vec(have)
    s_ease = x @ model.B
    s_nb = model.neighbours(eff, soul_level(data, b))
    z = lambda v: (v - v.mean()) / (v.std() + 1e-9)
    s = z(s_ease) + 0.5 * z(s_nb)
    fit = np.array([model.scale_fit(t, eff, base) for t in model.vocab])
    s = s + 0.5 * z(fit)
    owned_weapons = {w for w, _ in b.weapons()}
    out = {"weapon": [], "catalyst": [], "ring": [], "spell": []}
    for i in np.argsort(-s):
        t = model.vocab[i]
        if t in have:
            continue
        if t.startswith("WI:"):
            w = t[3:].split("|")[0]
            if w in owned_weapons or not weapon_ok(data, w, eff, two_hand=bool(b.grip)):
                continue
            bucket = "catalyst" if CATALYST.search(w) else "weapon"
        elif t.startswith("R:"):
            r = t[2:]
            grp = data.rings[r].get("group", r)
            if any(data.rings.get(o, {}).get("group", o) == grp for o in b.rings):
                continue
            bucket = "ring"
        elif t.startswith("S:"):
            if not spell_ok(data, t[2:], eff):
                continue
            bucket = "spell"
        else:
            continue
        if len(out[bucket]) < k:
            out[bucket].append((t, s[i], s_ease[i], s_nb[i], fit[i]))
    return eff, out


def evaluate(model: Model, corpus: list[Build], data: Data, folds: int = 5, lam: float = 50.0):
    """Leave-one-out per held-out build: hide one token, rank it. EASE vs popularity."""
    rng = np.random.default_rng(0)
    order = rng.permutation(len(corpus))
    hits = {"ease": Counter(), "pop": Counter()}
    n = Counter()
    for f in range(folds):
        test = set(order[f::folds].tolist())
        train = [b for i, b in enumerate(corpus) if i not in test]
        m = Model(data, train, lam)
        for i in test:
            toks = [t for t in tokens(corpus[i]) if t in m.ix and not t.startswith("W:")]
            if len(toks) < 2:
                continue
            hide = toks[rng.integers(len(toks))]
            kind = hide.split(":")[0]
            rest = [t for t in tokens(corpus[i]) if t != hide]
            if kind == "WI":
                rest = [t for t in rest if t != "W:" + hide[3:].split("|")[0]]
            x = m.vec(rest)
            same = np.array([t.split(":")[0] == kind for t in m.vocab])
            for name, sc in (("ease", x @ m.B), ("pop", m.pop.astype(float))):
                sc = np.where(same & (x == 0), sc, -np.inf)
                rank = int((sc > sc[m.ix[hide]]).sum())
                if rank < 10:
                    hits[name][kind] += 1
            n[kind] += 1
    return {k: (n[k], hits["ease"][k] / n[k], hits["pop"][k] / n[k]) for k in n}


def ring_effects(data: Data) -> dict:
    """SoulsPlanner ring key -> (weight, {stat: +n}, load multiplier), from MugenMonkey's ring table
    (SoulsPlanner encodes these as functions). SITE data, not yet checked against the game."""
    stat_of = {"Strength": "strength", "Dexterity": "dexterity", "Intelligence": "intelligence",
               "Faith": "faith", "Adaptability": "adaptability", "Attunement": "attunement",
               "Vitality": "vitality", "Endurance": "endurance", "Vigor": "vigor"}
    out = {}
    for name, v in data.mm["darkSouls2Rings"].items():
        key = data.sp_key.get(norm(name))
        if not key or key in EMPTY:
            continue
        add, mul = {}, 1.0
        for e in v["effects"] or []:
            if not isinstance(e, dict):
                continue
            if e.get("alters") in stat_of and e.get("method") == "add":
                add[stat_of[e["alters"]]] = add.get(stat_of[e["alters"]], 0) + e["value"]
            elif e.get("alters") == "Equip Load" and e.get("method") == "mul":
                mul *= e["value"]
        if add or mul != 1.0:
            out[key] = (float(v["weight"]), add, mul)
    return out


MECHANICS = Path.home() / ".cache/ds2-builds/mechanics.json"
FLOOR_STATS = ["vigor", "vitality", "adaptability", "attunement"]


def stamina_r1(data: Data) -> dict:
    """SoulsPlanner weapon key -> one-handed R1 stamina cost (game data, via the mechanics dump)."""
    m = json.loads(MECHANICS.read_text())
    by = {norm(w["name"]): w["stamina"]["r1_1h"] for w in m["stamina"]["weapons"] if w.get("stamina")}
    return {k: by[norm(v["name"])] for k, v in data.weapons.items() if norm(v["name"]) in by}


def high_stamina_cut(r1: dict) -> float:
    """The 'high end' of stamina use: 90th percentile of one-handed R1 cost across weapons."""
    return float(np.percentile(sorted(r1.values()), 90))


def bracket_floors(data: Data, corpus: list[Build], r1: dict) -> dict:
    """Per SL bracket, the corpus median of each floor stat; END only from builds whose weapons
    are high-stamina. Brackets with fewer than 20 builds borrow the nearest bracket that has them."""
    cut = high_stamina_cut(r1)
    rows, end_rows = {}, {}
    for b in corpus:
        i = sl_bracket(soul_level(data, b))
        rows.setdefault(i, []).append([b.stats[s] for s in FLOOR_STATS])
        if any(r1.get(w, 0) >= cut for w, _ in b.weapons()):
            end_rows.setdefault(i, []).append(b.stats["endurance"])

    def pick(table, i):
        for d in sorted(range(len(SL_BRACKETS)), key=lambda j: (abs(j - i), j)):
            if len(table.get(d, [])) >= 20:
                return np.median(np.array(table[d]), axis=0)
        return None

    out = {}
    for i in range(len(SL_BRACKETS)):
        med = pick(rows, i)
        f = {s: int(np.ceil(v)) for s, v in zip(FLOOR_STATS, med)} if med is not None else {}
        e = pick(end_rows, i)
        f["endurance"] = int(np.ceil(e)) if e is not None else 0
        out[i] = f
    return out


def minimum_build(data: Data, weapon: str, two_hand: bool, agl_target: int, floors: dict | None = None,
                  high_stamina: bool = False):
    """Lowest SL per class that can wield `weapon`: requirements met (armor/ring stat bonuses
    count, STR halved two-handed), weapon + armor + rings under 70% equip load, AGL >= target.
    Armor slots may be naked and ring slots empty; helpful rings/armor are searched exhaustively."""
    from itertools import combinations, product
    w = data.weapons[weapon]
    req = dict(w.get("require") or {})
    if two_hand and "strength" in req:
        req["strength"] //= 2
    rings = ring_effects(data)
    ring_opts = [()] + [c for n in range(1, 5) for c in combinations(sorted(rings), n)]
    # per armor slot: naked, plus every piece that adds a stat we could use
    useful = set(req) | {"vitality", "adaptability", "attunement"}
    armor_opts = []
    for slot in ARMOR_SLOTS:
        opts = [("Naked", 0.0, {}, {})]
        for k, v in data.armor[slot].items():
            alt = {s: x for s, x in (v.get("alter") or {}).items() if isinstance(x, (int, float))}
            if any(s in useful and x > 0 for s, x in alt.items()) and k != "Naked":
                opts.append((k, float(v["weight"]), alt, v.get("require") or {}))
        armor_opts.append(opts)
    results = []
    for cls, base in data.classes.items():
        best = None
        for arm in product(*armor_opts):
            for rs in ring_opts:
                bonus = Counter()
                for _, _, alt, _ in arm:
                    bonus.update(alt)
                mul = 1.0
                for r in rs:
                    bonus.update(rings[r][1])
                    mul *= rings[r][2]
                stats = {s: base[s] for s in STATS}
                # raise each requirement stat until effective >= requirement (armor reqs too)
                need = dict(req)
                for _, _, _, areq in arm:
                    for s, v in areq.items():
                        need[s] = max(need.get(s, 0), v)
                for s, v in need.items():
                    stats[s] = max(stats[s], v - bonus.get(s, 0))
                load = float(w["weight"]) + sum(a[1] for a in arm) + sum(rings[r][0] for r in rs)
                req_stats, used = dict(stats), None
                for _ in range(8):  # floors depend on the SL bracket, which depends on the floors
                    stats = dict(req_stats)
                    if used is not None:
                        f = floors[used]
                        for s in FLOOR_STATS + (["endurance"] if high_stamina else []):
                            stats[s] = max(stats[s], f.get(s, 0))  # corpus medians are base stats
                    while data.equip_load[min(stats["vitality"] + bonus.get("vitality", 0), 99)] * mul * EQUIP_CAP < load:
                        stats["vitality"] += 1
                        if stats["vitality"] > 99:
                            break
                    while agility(stats["adaptability"] + bonus.get("adaptability", 0),
                                  stats["attunement"] + bonus.get("attunement", 0)) < agl_target and stats["adaptability"] < 99:
                        stats["adaptability"] += 1
                    sl = base["level"] + sum(stats[s] - base[s] for s in STATS)
                    if floors is None or sl_bracket(sl) == used:
                        break
                    used = sl_bracket(sl)
                if best is None or sl < best[0]:
                    best = (sl, stats, [a[0] for a in arm], list(rs), load, mul)
        results.append((best[0], cls, best))
    results.sort(key=lambda t: t[0])
    return results


FREE_STATS = ["vigor", "endurance", "vitality", "attunement", "adaptability"]
ELEMENTS = ["magic", "fire", "lightning", "dark"]


def _tab(data: Data, name: str, i: int) -> float:
    t = data.sp[name]
    return t[max(0, min(i, len(t) - 1))] or 0


#: WeaponStatsAffectParam row offset of the Enchanted infusion, `statsAffectId + t[8][1]`: the
#: executable's 10x3 infusion table at 0x1410c3e10 has t[8] = {0, 8, 2} (EXE, read from
#: darksoulsii-deobf.bin; docs/DS2-DPS-MECHANICS.md "Attack rating").
ENCHANTED_STATS_AFFECT_OFFSET = 8


def apply_regulation(data: Data) -> str:
    """Replace two SoulsPlanner numbers with the game's, where the executable and regulation were
    read (docs/DS2-BUILD-MECHANICS.md section 6, docs/DS2-DPS-MECHANICS.md "Attack rating"):

    * `physicalDEFBonus[sum]` becomes PhysicalStatsPerLevelStatValuesParam
      `row[trunc((END+VIT+STR+DEX)/4)].defense`, the index the stats builder 0x14038d790 uses.
      SoulsPlanner's table is off by one at 180 of 393 sums.
    * The Enchanted infusion's STR, DEX and INT (physicalByEnchant) coefficients become
      WeaponStatsAffectParam[statsAffectId + 8] at the weapon's max reinforce level, times the
      physical rate the infusion leaves (WeaponReinforceParam physicalRate +
      addPhysicalRateByEnchanted, / 100). SoulsPlanner rounds them to 2 dp and has the Dagger's STR
      at 0.06; the regulation says 0.053.

    Weapons join by normalized itemname.fmg name (WeaponParam id == ItemParam id == text id). A
    weapon whose regulation DEX/INT coefficients do not round to SoulsPlanner's is left alone, as a
    join that picked the wrong row. Returns a one-line summary; SoulsPlanner's numbers stay when
    the game's files cannot be read."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("ds2attacks", Path(__file__).parent / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    try:
        d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
            "WeaponParam": "WEAPON_PARAM", "WeaponReinforceParam": "WEAPON_REINFORCE_PARAM",
            "WeaponStatsAffectParam": "WEAPON_STATS_AFFECT_PARAM",
            "PhysicalStatsPerLevelStatValuesParam": "PHYS_STATS_PER_LEVEL_STAT_PARAM"})
        names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    except (OSError, SystemExit, KeyError) as e:
        return f"regulation unreadable ({e}); SoulsPlanner's defense table and Enchanted coefficients kept"
    rows = d["PhysicalStatsPerLevelStatValuesParam"]
    top = max(map(int, rows))
    old = data.sp["physicalDEFBonus"]
    new = [None if s < 4 else rows[str(min(top, s // 4))]["defense"] for s in range(len(old))]
    moved = sum(1 for a, b in zip(old, new) if b is not None and a != b)
    data.sp["physicalDEFBonus"] = new
    by_name: dict[str, dict] = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    fixed, refused = 0, 0
    for key, w in data.weapons.items():
        row = (w.get("infusions") or {}).get("Enchanted")
        sc = (row or {}).get("atkScale")
        wp = by_name.get(norm(w.get("name", key)))
        if not sc or not wp:
            continue
        r = d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
        c = r and d["WeaponStatsAffectParam"].get(str(r["statsAffectId"] + ENCHANTED_STATS_AFFECT_OFFSET))
        if not c:
            continue
        lv, rate = r["maxLevel"], (r["physicalRate"] + r["addPhysicalRateByEnchanted"]) / 100
        game = {"strength": c[f"physicalByStrength{lv}"] * rate, "dexterity": c[f"physicalByDexterity{lv}"] * rate,
                "magic": c[f"physicalByEnchant{lv}"] * rate}
        if any(abs(round(game[k], 2) - sc.get(k, 0)) > 0.011 for k in ("dexterity", "magic")):
            refused += 1
            continue
        sc.update({k: round(v, 4) for k, v in game.items()})
        fixed += 1
    return (f"regulation: physical stat defense from the game's table ({moved} sums moved), Enchanted "
            f"coefficients from WeaponStatsAffectParam for {fixed} weapons ({refused} left: join disagreed)")


def attack_rating(data: Data, weapon: str, inf: str, eff: dict) -> dict:
    """Per-type attack rating at full upgrade: SoulsPlanner's getPhysicalATK/getMagicATK/... (SITE),
    without ring bonuses. Physical scales STR and DEX off one table; fire reads INT+FTH, dark
    min(INT, FTH)."""
    row = data.weapons[weapon]["infusions"].get(inf) or {}
    atk, sc = row.get("atk") or {}, row.get("atkScale") or {}
    out = {}
    if atk.get("physical"):
        v = (atk["physical"] + sc.get("strength", 0) * _tab(data, "physicalATKBonus", eff["strength"])
             + sc.get("dexterity", 0) * _tab(data, "physicalATKBonus", eff["dexterity"]))
        if inf == "Enchanted" and sc.get("magic"):
            v += sc["magic"] * _tab(data, "magicATKBonus", eff["intelligence"])
        if inf == "Mundane":
            v += sc.get("modifier", 1) * _tab(data, "mundaneATKBonus", min(eff.values()))
        out["physical"] = int(v)
    feed = {"magic": ("magicATKBonus", eff["intelligence"]),
            "fire": ("fireATKBonus", eff["intelligence"] + eff["faith"]),
            "lightning": ("lightningATKBonus", eff["faith"]),
            "dark": ("darkATKBonus", min(eff["intelligence"], eff["faith"]))}
    for k, (tab, i) in feed.items():
        if atk.get(k):
            out[k] = int(atk[k] + sc.get(k, 0) * _tab(data, tab, i))
    return out


def build_defense(data: Data, b: Build) -> dict:
    """Per-type defense of a build: SoulsPlanner's getPhysicalDEF/getMagicDEF/... (SITE). Armor
    physical defense grows with END+VIT+STR+DEX via each piece's bonus coefficient; elemental
    defense is a stat-driven base plus the armor's flat values. Ring defense effects ignored."""
    eff = effective(data, b)
    for r in b.rings:
        for s, v in ring_effects(data).get(r, (0, {}, 1))[1].items():
            eff[s] = eff.get(s, 0) + v
    pieces = [data.armor[s][p] for s, p in zip(ARMOR_SLOTS, b.armor) if p in data.armor[s]]
    pb = _tab(data, "physicalDEFBonus", eff["endurance"] + eff["vitality"] + eff["strength"] + eff["dexterity"])
    out = {"physical": sum(p.get("physicalDEF", 0) + p.get("physicalDEFBonus", 0) * pb for p in pieces)}
    # slash/strike/thrust (SITE values; equal ArmorReinforceParam's max values for 422 of 427 pieces,
    # REGULATION) plus the same stat bonus the general physical defense gets (SITE)
    for t in PHYS_TYPES:
        out[t] = sum(p.get(t + "DEF", p.get("physicalDEF", 0)) + p.get("physicalDEFBonus", 0) * pb for p in pieces)
    base = {"magic": ("magicDEFBonus", eff["intelligence"]),
            "fire": ("fireDEFBonus", eff["intelligence"] + eff["faith"]),
            "lightning": ("lightningDEFBonus", eff["faith"]),
            "dark": ("darkDEFBonus", min(eff["intelligence"], eff["faith"]))}
    for k, (tab, i) in base.items():
        out[k] = _tab(data, tab, i) + sum(p.get(k + "DEF", 0) for p in pieces)
    return out


def bracket_defense(data: Data, corpus: list[Build], sl: int) -> tuple[dict, int]:
    """Mean per-type defense of corpus builds in `sl`'s bracket (nearest bracket with >= 20)."""
    i = sl_bracket(sl)
    by = {}
    for b in corpus:
        by.setdefault(sl_bracket(soul_level(data, b)), []).append(b)
    for j in sorted(range(len(SL_BRACKETS)), key=lambda j: (abs(j - i), j)):
        if len(by.get(j, [])) >= 20:
            ds = [build_defense(data, b) for b in by[j]]
            return {k: float(np.mean([d[k] for d in ds])) for k in DMG + PHYS_TYPES}, len(ds)
    return {k: 0.0 for k in DMG + PHYS_TYPES}, 0


def damage(kind: str, ar: float, df: float) -> float:
    """Damage one type deals to a player defender, motion value and hand 1 (COMMUNITY formula,
    darksouls2.wiki.gg/wiki/Defense; the 10 and 12 match DamageAdjustParam row 0
    pcAttributeAdjustNormalMul/Div in the regulation; the elemental cut (DEF + 100) / 1000 is
    confirmed from the EXE, docs/DS2-DPS-MECHANICS.md "Elemental cut").
    physical: (AR*10 - DEF) / 12;  elemental: AR * (1 - min(0.99, (DEF + 100) / 1000))."""
    if not ar:
        return 0.0
    if kind == "physical":
        return max(0.0, (ar * 10 - df) / 12)
    return ar * (1 - min(0.99, (df + 100) / 1000))


# Both written by scripts/ds2-attacks-extract.py (--unpack-tae for TAE_DIR, then a plain run).
ATTACKS = Path.home() / ".cache/ds2-builds/attacks.json"  # per weapon attack: hitbox frames, motion values
TAE_DIR = Path.home() / ".cache/ds2-builds/tae"  # c000100_pl.tae unpacked, one XML per animation
_chain_cache: dict = {}


def chain_open(anim: int) -> float | None:
    """Seconds (animation time) at which the next chained attack can start: the first TAE event
    111500 (INFERRED chain/cancel window, docs/DS2-DPS-MECHANICS.md), else the animation's end."""
    if anim not in _chain_cache:
        p = TAE_DIR / f"anim-{anim:09d}.xml"
        t = None
        if p.exists():
            ev = re.findall(r"<type>(\d+)</type>.*?<startTime>([-\d.]+)</startTime>\s*<endTime>([-\d.]+)</endTime>",
                            p.read_text(), re.S)
            opens = [float(s) for ty, s, _ in ev if ty == "111500"]
            t = min(opens) if opens else max((float(e) for _, _, e in ev), default=None)
        _chain_cache[anim] = t
    return _chain_cache[anim]


def live_hits(a: dict, distinct: bool = False) -> list[dict]:
    """An attack's hitboxes that can hurt a player: a live TAE 2200 window with a motion value.
    `distinct` drops a hitbox whose window, tick count and interval repeat an earlier one's on a
    different damage row -- the Old Whip's 1.0 + 0.35 pair on frames 16-20, two damageGroups on one
    swing, which docs/DS2-DPS-MECHANICS.md section 4 leaves at "possibly 2" until runtime proof. Windows
    that only overlap (Channeler's Trident R2: opening hit 19-24, spin 23-35) stay separate hits."""
    hs = [h for h in a.get("hits") or [] if h.get("live") and h.get("rate")]
    if not distinct:
        return hs
    seen, out = set(), []
    for h in hs:
        k = (h["start"], h["end"], max(1, h.get("n") or 1), h.get("interval") or 0)
        if k not in seen:
            seen.add(k)
            out.append(h)
    return out


def attack_hits(a: dict | None) -> int:
    """How many times one attack can hit one target: each distinct live hitbox times its repeat
    ticks. REGULATION + TAE via attacks.json: a tick is PlayerDamageParam.hitDistance (INFERRED a
    re-hit interval in seconds) across its 2200 window, n = floor(window / interval) + 1 -- Channeler's
    Trident 1H R2 is its opening hit plus 3 spin ticks = 4 (docs/DS2-DPS-MECHANICS.md section 4)."""
    return sum(max(1, h.get("n") or 1) for h in live_hits(a, distinct=True)) if a else 0


def chain_timeline(attacks: dict, name: str, two_hand: bool, kind: str = "Normal", horizon: float = 3.0,
                   distinct: bool = False, with_start: bool = False) -> list[tuple]:
    """(seconds from input, motion value, physical type, damage floor) of every hit a repeated
    attack lands, alternating its 1st and 2nd chain attacks: `kind` "Normal" is the R1 chain,
    "Strong" the R2 chain. Play speed is the mean of start/end speeds (where one hands over to
    the other is unknown); re-hitting hitboxes add a tick per interval. `distinct`: see live_hits.
    `with_start` appends the second its attack began, which is what `horizon` cuts on: the hits of
    a longer horizon whose attack began before a shorter one are exactly the shorter one's hits."""
    g = "Single2Hand" if two_hand else "Single1Hand"
    seq = [attacks.get((norm(name), g + kind + "1st")), attacks.get((norm(name), g + kind + "2nd"))]
    if not seq[0]:
        return []
    seq[1] = seq[1] or seq[0]
    out, t0, k = [], 0.0, 0
    while t0 < horizon:
        a = seq[k % 2]
        spd = sum(a.get("spd") or [1.0, 1.0]) / 2
        for h in live_hits(a, distinct):
            n = max(1, h.get("n") or 1)
            for i in range(n):
                out.append((t0 + (h["start"] / 30 + i * (h.get("interval") or 0)) / spd, h["rate"],
                            h.get("type") or "physical", h.get("lower", 0)) + ((t0,) if with_start else ()))
        step = chain_open(a["anim"])
        if not step or step <= 0:
            break
        # never chain before this attack's own hitbox has closed (katana anims open 111500 at 0.2 s,
        # before their frame-14 hit, so 111500 alone is not a chain window there)
        live = [h["end"] / 30 for h in a.get("hits") or [] if h.get("live") and h.get("rate")]
        step = max(step, max(live, default=0.0))
        t0 += step / spd
        k += 1
    return sorted(out)


def r1_timeline(attacks: dict, name: str, two_hand: bool, horizon: float = 3.0) -> list[tuple[float, float, str, int]]:
    """The R1 chain's hits (chain_timeline); what `--window` and the exported backend rank by."""
    return chain_timeline(attacks, name, two_hand, "Normal", horizon)


HYPERARMOR = Path.home() / ".cache/ds2-builds/hyperarmor.json"  # weapon name -> WeaponParam.uninterruptibleRate
CRIT = Path.home() / ".cache/ds2-builds/crit.json"  # per weapon: counter, backstab, riposte multipliers
_tae_cache: dict = {}


def hyperarmor(attacks: dict, rates: dict, name: str, two_hand: bool) -> float:
    """Incoming poise damage multiplier during the R1's hyperarmor window, 0 when it has none. EXE
    (docs/DS2-DPS-MECHANICS.md): TAE event 111900 copies WeaponParam.uninterruptibleRate for its
    window and the defender side multiplies incoming poise damage by it."""
    rate = rates.get(name, 0.0)
    a = attacks.get((norm(name), ("Single2Hand" if two_hand else "Single1Hand") + "Normal1st"))
    if not rate or not a:
        return 0.0
    if a["anim"] not in _tae_cache:
        p = TAE_DIR / f"anim-{a['anim']:09d}.xml"
        _tae_cache[a["anim"]] = p.exists() and "<type>111900</type>" in p.read_text()
    return rate if _tae_cache[a["anim"]] else 0.0


ATTACK_TYPES = Path.home() / ".cache/ds2-builds/attack-type.json"  # per weapon slot: slash/strike/thrust per hit
DAMAGE_LOWER = Path.home() / ".cache/ds2-builds/damage-lower.json"  # PlayerDamageParam row -> damageLower


def load_attacks() -> dict:
    """Attacks by (weapon, slot). Each hit gets its physical type from DamageCtrlParam.attackType
    (REGULATION, docs/DS2-DPS-MECHANICS.md) when the per-slot hit lists line up."""
    types = {norm(k): v for k, v in json.loads(ATTACK_TYPES.read_text()).items()} if ATTACK_TYPES.exists() else {}
    lower = json.loads(DAMAGE_LOWER.read_text()) if DAMAGE_LOWER.exists() else {}
    by = {}
    for a in json.loads(ATTACKS.read_text()):
        tl = (types.get(norm(a["name"]), {}).get(a["slot"]) or {}).get("hits") or []
        if tl and len(tl) == len(a.get("hits") or []):
            for h, t in zip(a["hits"], tl):
                h["type"] = t
        for h in a.get("hits") or []:
            h["lower"] = lower.get(str(h.get("dmg")), 0)
        by.setdefault((norm(a["name"]), a["slot"]), a)
    return by


def hit_damage(ar: dict, dfn: dict, mv: float, kind: str = "physical", lower: int = 0) -> float:
    """One hit's damage against a player (EXE, ChrDamageActionCtrl slot 32 0x140138d50; notes in
    docs/DS2-DPS-MECHANICS.md): physical max(AR*10 - DEF_type, lower) / 12, each element
    max(AR*6, lower) / 6 * (1 - cut), summed, times MV. DEF_type is the hit's slash/strike/thrust
    defense (general physical when it has none), `lower` is PlayerDamageParam.damageLower, and
    cut = min(0.99, (DEF + 100) / 1000); the +100 is the stat table's 10% resistance floor that the
    displayed defense leaves out (EXE, docs/DS2-DPS-MECHANICS.md "Elemental cut")."""
    tot = 0.0
    for k, v in ar.items():
        if not v:
            continue
        if k == "physical":
            tot += max(v * 10 - dfn.get(kind, dfn[k]), lower) / 12
        else:
            tot += max(v * 6, lower) / 6 * (1 - min(0.99, (dfn[k] + 100) / 1000))
    return tot * mv


def status_hits(attacks: dict, names: list[str], grips: list[bool], window: float = 0.0) -> tuple[int, str]:
    """The most hits one target takes from a weapon's R1 or R2 chain, and a label for it. With
    `window`: hits landed within that many seconds of repeating the chain (chain_timeline, so the
    attack's length counts -- a 4-hit R2 that takes twice as long as an R1 gains nothing). Without:
    hits of one attack (attack_hits). Ties keep 1H over 2H and R1 over R2. (0, "") without data."""
    best, label = 0, ""
    for two_hand in grips:
        for kind, tag in (("Normal", "R1"), ("Strong", "R2")):
            for nm in names:
                if window:
                    tl = chain_timeline(attacks, nm, two_hand, kind, max(3.0, window), distinct=True)
                    n = sum(1 for t, *_ in tl if t <= window)
                else:
                    n = attack_hits(attacks.get((norm(nm), ("Single2Hand" if two_hand else "Single1Hand") + kind + "1st")))
                if n:
                    break
            if n > best:
                best, label = n, f"{'2H' if two_hand else '1H'} {tag} {n} hit{'s' if n > 1 else ''}"
    return best, label


def weapons_for(data: Data, stats: dict, sl: int, corpus: list[Build], top: int = 25, within: float = 0.10,
                raw_ar: bool = False, one_hand: bool = False, weapon_class: str | None = None, per_class: bool = False,
                window: float = 0.0, objective: str = "damage", weapon: str | None = None,
                every_infusion: bool = False):
    """Weapons (per infusion) ranked by expected damage against the average defender at this SL,
    or with `objective` "bleed"/"poison" by status build-up: build-up per hit (objective_value,
    SITE formula) times the hits of the weapon's best R1/R2 chain attack (status_hits; within
    `window` seconds when given). That every hit and every re-hit tick applies the weapon's full
    build-up is INFERRED: DamageParam has no status field (REGULATION) and the exe's status
    code was not traced (docs/DS2-DPS-MECHANICS.md section 4). The defender's resistance and the
    proc's damage are not modelled.
    Usable only: requirements met (STR halved when that is what makes it usable, flagged 2H).
    Per weapon: the best infusion, plus the 2nd and 3rd only while within `within` of the best.
    `every_infusion`: every infusion of every weapon instead, and no `top` cut (infusion_gaps).
    `weapon` (a key): that weapon alone, every infusion, and no high-stamina END gate -- the
    question is which infusion, not whether to carry it (best_infusion)."""
    every_infusion = every_infusion or weapon is not None
    dfn, n = bracket_defense(data, corpus, sl)
    floors, r1, cut = build_floors(data, corpus, sl)
    attacks = load_attacks()
    rates = json.loads(HYPERARMOR.read_text()) if HYPERARMOR.exists() else {}
    # counter-hit multiplier: WeaponTypeParam.counterDamageScale (REGULATION; equals the menu's
    # Counter Strength for 202 of 204 weapons). Crits are left out: their tick count is unsettled.
    crit = {norm(k): v for k, v in json.loads(CRIT.read_text()).items()} if CRIT.exists() else {}
    rows, skipped = [], []
    for key, w in data.weapons.items():
        if key in EMPTY or CATALYST.search(key) or w.get("isShield") or (weapon and key != weapon):
            continue
        req = w.get("require") or {}
        one = all(stats.get(s, 0) >= v for s, v in req.items())
        two = all(stats.get(s, 0) >= (v // 2 if s == "strength" else v) for s, v in req.items())
        if not two or (one_hand and not one):
            continue
        if weapon_class and norm(data.weapon_class.get(key) or "") != norm(weapon_class):
            continue
        if not weapon and r1.get(key, 0) >= cut and stats["endurance"] < floors.get("endurance", 0):
            continue  # a high-stamina weapon needs END at the bracket median of builds that carry one
        lines = {}
        if objective in ("bleed", "poison"):
            hits, hlabel = status_hits(attacks, [w["name"], key.replace("_", " ")], ([False] if one else []) + [True],
                                       window)
            if not hits:
                skipped.append(w["name"])
                continue
            scored = []
            for inf in w["infusions"]:
                per = objective_value(data, key, inf, stats, objective, dfn)
                if per > 0:
                    scored.append((per * hits, inf, attack_rating(data, key, inf, stats),
                                   hlabel + ("" if one or hlabel.startswith("1H") else " (2H only)")))
            scored.sort(key=lambda s: -s[0])
            for s in scored if every_infusion else [s for s in scored[:3] if s[0] >= scored[0][0] * (1 - within)]:
                rows.append((s[0], w["name"], *s[1:]))
            continue
        if window:  # R1 chain hits landing within `window` seconds, per grip the build can use
            for two_hand in ([False] if one else []) + [True]:
                tl = r1_timeline(attacks, w["name"], two_hand) or r1_timeline(attacks, key.replace("_", " "), two_hand)
                if tl:
                    lines["2H" if two_hand else "1H"] = [(mv, ty, lo) for t, mv, ty, lo in tl if t <= window]
            if not lines:
                skipped.append(w["name"])
                continue
        scored = []
        for inf in w["infusions"]:
            ar = attack_rating(data, key, inf, stats)
            if not ar:
                continue
            if window:
                grip, mvs = max(lines.items(), key=lambda g: sum(hit_damage(ar, dfn, mv, ty, lo) for mv, ty, lo in g[1]))
                dmg = sum(hit_damage(ar, dfn, mv, ty, lo) for mv, ty, lo in mvs)
                label = f"{grip} {len(mvs)} hits" + ("" if one or grip == "1H" else " (2H only)")
            else:
                dmg = sum(ar.values()) if raw_ar else sum(damage(k, v, dfn[k]) for k, v in ar.items())
                label = "1H" if one else "2H only"
            ha = hyperarmor(attacks, rates, w["name"], label.startswith("2H"))
            ctr = (crit.get(norm(w["name"])) or {}).get("counter")
            scored.append((dmg, inf, ar, label + (f" HA x{ha:g}" if ha else "") + (f" ctr x{ctr:g}" if ctr else "")))
        scored.sort(key=lambda s: -s[0])
        if not scored:
            continue
        keep = scored if every_infusion else [s for s in scored[:3] if s[0] >= scored[0][0] * (1 - within)]
        for dmg, inf, ar, label in keep:
            rows.append((dmg, w["name"], inf, ar, label))
    rows.sort(key=lambda r: -r[0])
    if skipped:
        print(f"no attack timing for {len(skipped)} weapons (left out): {', '.join(sorted(skipped)[:8])}...",
              file=sys.stderr)
    if per_class:  # only each class's top row, tagged with the class
        best = {}
        for r in rows:
            best.setdefault(data.weapon_class.get(data.key_by_name[r[1]]) or "?", r)
        return [(*r[:4], f"{r[4]:8} {c}") for c, r in best.items()], dfn, n
    return (rows if every_infusion else rows[:top]), dfn, n


def infusion_margin(values: list[float]) -> float | None:
    """How far the best of `values` (best first) is ahead of the runner-up, as a fraction of the
    runner-up: 0.25 is 25% more. None with no runner-up or a runner-up of 0."""
    if len(values) < 2 or values[1] <= 0:
        return None
    return values[0] / values[1] - 1


def best_infusion(data: Data, weapon: str, stats: dict, sl: int, corpus: list[Build], raw_ar: bool = False,
                  window: float = 0.0, objective: str = "damage"):
    """Every infusion `weapon` takes, best first, by weapons_for's own score at these stats and SL
    (the rows it ranks, uncut). Empty when the stats cannot wield it even two-handed, or for
    bleed/poison when no infusion deals it or the weapon has no attack timing. Full upgrade only:
    attack_rating reads the planner's max-level rows, so the upgrade level is not modelled."""
    return weapons_for(data, stats, sl, corpus, raw_ar=raw_ar, window=window, objective=objective, weapon=weapon)


def infusion_gaps(data: Data, stats: dict, sl: int, corpus: list[Build], raw_ar: bool = False,
                  window: float = 0.0, objective: str = "damage", one_hand: bool = False,
                  weapon_class: str | None = None, top: int = 25):
    """The reverse of best_infusion: across the weapon table, the weapons whose best infusion is
    furthest ahead of their runner-up at these stats. Rows: (margin, weapon name, best row,
    runner-up row, infusions scored), a row being weapons_for's (score, name, infusion, ar, grip)."""
    rows, dfn, n = weapons_for(data, stats, sl, corpus, raw_ar=raw_ar, one_hand=one_hand,
                               weapon_class=weapon_class, window=window, objective=objective,
                               every_infusion=True)
    by = {}
    for r in rows:  # best first overall, so each weapon's own rows are best first too
        by.setdefault(r[1], []).append(r)
    out = []
    for name, rs in by.items():
        m = infusion_margin([r[0] for r in rs])
        if m is not None:
            out.append((m, name, rs[0], rs[1], len(rs)))
    out.sort(key=lambda g: -g[0])
    return out[:top], dfn, n


def build_floors(data: Data, corpus: list[Build], sl: int) -> tuple[dict, dict, float]:
    """The floors every build output must meet at `sl`: VGR/VIT/ADP/ATT at the corpus median of
    the SL bracket, END at the high-stamina builds' median (applies only to high-stamina weapons)."""
    r1 = stamina_r1(data)
    return bracket_floors(data, corpus, r1)[sl_bracket(sl)], r1, high_stamina_cut(r1)


def hit_points(vgr: int) -> int:
    """HP from VGR (SoulsPlanner getHP, SITE): 30 per point to 20, 20 to 50, 5 after. The small
    per-point HP from the other stats is left out."""
    return 500 + 30 * min(vgr, 20) + 20 * max(0, min(vgr, 50) - 20) + 5 * max(0, vgr - 50)


def floor_violations(stats: dict, floors: dict) -> list[str]:
    return [f"{s[:3].upper()} {stats[s]} < {floors[s]}" for s in FLOOR_STATS if stats[s] < floors.get(s, 0)]


def objective_value(data: Data, weapon: str, inf: str, st: dict, objective: str, dfn: dict) -> float:
    row = data.weapons[weapon]["infusions"].get(inf) or {}
    atk, sc = row.get("atk") or {}, row.get("atkScale") or {}
    if objective in ("bleed", "poison"):  # SoulsPlanner getBleedATK / getPoisonATK (SITE)
        i = 3 * st["dexterity"] + (st["faith"] if objective == "bleed" else st["adaptability"])
        return (atk.get(objective) or 0) + sc.get(objective, 0) * _tab(data, "auxATKBonus", i)
    return sum(damage(k, v, dfn[k]) for k, v in attack_rating(data, weapon, inf, st).items())


#: The grip optimize_build builds for, per `--grip`. "two" (the default) halves the STR requirement,
#: even when one-handing would fit; "one" needs the full requirement. The old behaviour -- one-handed
#: whenever it fits, two-handed only when it does not -- is gone: it made a two-handed build
#: unreachable for any weapon the stats could one-hand, and trying two-handed first is the same as
#: always two-handing, since a halved requirement fits whenever the full one does.
#: The objective is scored the same for either grip: AR (or status build-up per hit) does not depend
#: on the grip in this model. The 1.5x STR a two-handed grip is said to give is unproven here
#: (docs/DS2-DPS-MECHANICS.md section 1), so it is not applied.
GRIP_TRIES = {"two": (True,), "one": (False,)}


def optimize_build(data: Data, corpus: list[Build], weapon: str, inf: str, sl: int, objective: str,
                   grip: str = "two"):
    """A valid build at `sl` that maximizes `objective` for weapon+infusion: floors first (bracket
    medians, END only for a high-stamina weapon), then requirements (STR halved for grip "two",
    in full for "one"; see GRIP_TRIES), then every remaining point where it raises the objective
    most."""
    floors, r1, cut = build_floors(data, corpus, sl)
    dfn, _ = bracket_defense(data, corpus, sl)
    req = data.weapons[weapon].get("require") or {}
    best = None
    for cls, base in data.classes.items():
        for two in GRIP_TRIES[grip]:
            st = {s: int(base[s]) for s in STATS}
            for s in FLOOR_STATS:
                st[s] = max(st[s], floors.get(s, 0))
            if r1.get(weapon, 0) >= cut:
                st["endurance"] = max(st["endurance"], floors.get("endurance", 0))
            for s, v in req.items():
                st[s] = max(st[s], (v + 1) // 2 if (two and s == "strength") else v)
            free = sl + 53 - sum(st.values())
            if free < 0:
                continue
            # A stat is weighted by its own curve: gain per point at the current value over the
            # curve's early rate (its mean gain per point from 5 to 25, before any soft cap). Past
            # a soft cap the weight falls (DEX past 40, VGR past 20 and 50, ADP past AGL 110), so
            # points go to a stat still under its cap. VIT is left at its floor: equip load is
            # the armor search's job, not a damage or survival curve.
            obj = lambda s_: objective_value(data, weapon, inf, s_, objective, dfn)
            agl = lambda s_: agility(s_["adaptability"], s_["attunement"])
            curves = {s: obj for s in ("strength", "dexterity", "intelligence", "faith")}
            curves["adaptability"] = obj if objective == "poison" else agl
            curves["vigor"] = lambda s_: hit_points(s_["vigor"])
            curves["attunement"] = agl  # a third of ADP's agility per point; slots only matter with spells
            peak = {}
            for s, f in curves.items():
                peak[s] = max((f({**st, s: 25}) - f({**st, s: 5})) / 20, 1e-9)
            if curves["adaptability"] is agl:
                peak["attunement"] = peak["adaptability"]
            while free > 0:
                pick, best_w = None, 0.0
                for s, f in curves.items():
                    cur = f(st)
                    for n in range(1, min(free, 99 - st[s], 8) + 1):  # look past table plateaus
                        w = (f({**st, s: st[s] + n}) - cur) / n / peak[s]
                        if w > best_w + 1e-12:
                            pick, best_w = (s, n), w
                if pick is None:  # every curve is flat: the points go to vigor
                    pick = (next(s for s in ("vigor", "vitality", "endurance", "attunement") if st[s] < 99), 1)
                st[pick[0]] += pick[1]
                free -= pick[1]
            val = objective_value(data, weapon, inf, st, objective, dfn)
            if best is None or val > best[0]:
                best = (val, cls, two, st)
            break  # one grip per class
    return best, floors


def calibrate_infusions(data: Data, corpus: list[Build]) -> dict:
    """How often the damage model's best infusion is the one a real build chose. Only builds that
    record an infusion (SoulsPlanner) count; MugenMonkey's '?' is skipped. The yardstick for every
    change to the damage formula: a formula that players' choices contradict is missing something."""
    by_sl, hit, total, miss = {}, 0, 0, Counter()
    top2 = 0
    for b in corpus:
        sl = soul_level(data, b)
        i = sl_bracket(sl)
        if i not in by_sl:
            by_sl[i] = bracket_defense(data, corpus, sl)[0]
        dfn, eff = by_sl[i], effective(data, b)
        for w, inf in set(b.weapons()):
            if inf == "?" or w not in data.weapons or data.weapons[w].get("isShield") or CATALYST.search(w):
                continue
            scored = sorted(((sum(damage(k, v, dfn[k]) for k, v in attack_rating(data, w, f, eff).items()), f)
                             for f in data.weapons[w]["infusions"]), reverse=True)
            if len(scored) < 2 or inf not in data.weapons[w]["infusions"]:
                continue
            total += 1
            hit += scored[0][1] == inf
            top2 += inf in (scored[0][1], scored[1][1])
            if scored[0][1] != inf:
                miss[(inf, scored[0][1])] += 1
    return {"n": total, "top1": hit / max(total, 1), "top2": top2 / max(total, 1), "confusions": miss.most_common(8)}


# --------------------------------------------------------------------------------------------
# MugenMonkey infusion inference (docs/DS2-BUILD-EMBEDDINGS.md "MugenMonkey infusion inference")
#
# MugenMonkey records no infusion. A conditional-logit model picks one per weapon from what a
# MugenMonkey build does record: the build's effective STR/DEX/INT/FTH (per infusion, so "INT high"
# can mean Magic), the damage model's score for each infusion, and how often SoulsPlanner builds
# chose each infusion on that same weapon. It is fitted on SoulsPlanner builds and measured on
# SoulsPlanner builds whose infusion is hidden (`--infusion-eval`, split by build).

INFUSIONS = ["No_Infusion", "Fire", "Magic", "Lightning", "Dark", "Poison", "Bleed", "Raw", "Enchanted", "Mundane"]
INF_STATS = ["strength", "dexterity", "intelligence", "faith"]


def infusion_cases(data: Data, corpus: list[Build], recorded: bool = True,
                   builds: list[Build] | None = None) -> list[dict]:
    """One case per distinct weapon of a build in `builds` (default: the corpus): the population
    calibrate_infusions scores (no shields, no catalysts, at least two infusions). recorded=True:
    SoulsPlanner weapons, with the chosen infusion as the label; False: MugenMonkey's '?' weapons,
    unlabelled. Defenses are the corpus's bracket averages; "build" is the index into `builds`."""
    by_sl, cases = {}, []
    for bi, b in enumerate(corpus if builds is None else builds):
        ws = [(w, inf) for w, inf in dict.fromkeys(b.weapons())
              if (inf != "?") == recorded and w in data.weapons and not data.weapons[w].get("isShield")
              and not CATALYST.search(w) and len(data.weapons[w]["infusions"]) >= 2
              and (not recorded or inf in data.weapons[w]["infusions"])]
        if not ws:
            continue
        sl = soul_level(data, b)
        i = sl_bracket(sl)
        if i not in by_sl:
            by_sl[i] = bracket_defense(data, corpus, sl)[0]
        dfn, eff = by_sl[i], effective(data, b)
        for w, inf in ws:
            infs = list(data.weapons[w]["infusions"])
            dmg = [sum(damage(k, v, dfn[k]) for k, v in attack_rating(data, w, f, eff).items()) for f in infs]
            cases.append({"build": bi, "weapon": w, "infusion": inf if recorded else None, "candidates": infs,
                          "damage": dmg, "stats": [eff[s] / 99 for s in INF_STATS]})
    return cases


class InfusionModel:
    """Conditional logit over a weapon's own infusions. Features of candidate f: one-hot f;
    one-hot f x effective STR/DEX/INT/FTH / 99; log smoothed share of f among SoulsPlanner builds
    carrying this weapon (leave-one-out while fitting, so a case never sees its own label); the
    damage model's score for f relative to the weapon's best; whether f is the damage model's best."""

    def __init__(self, cases: list[dict], prior: bool = True, alpha: float = 2.0, l2: float = 1e-3,
                 steps: int = 600, lr: float = 0.5):
        self.use_prior, self.alpha = prior, alpha
        self.count = Counter((c["weapon"], c["infusion"]) for c in cases)
        self.total = Counter(c["weapon"] for c in cases)
        g = Counter(c["infusion"] for c in cases)
        self.base = {f: (g[f] + 1) / (len(cases) + len(INFUSIONS)) for f in INFUSIONS}
        F, y, starts = self._matrix(cases, loo=True)
        self.theta = np.zeros(F.shape[1])
        m, v = np.zeros_like(self.theta), np.zeros_like(self.theta)
        for t in range(1, steps + 1):  # Adam on the mean negative log-likelihood + L2
            p = self._softmax(F @ self.theta, starts)
            grad = F.T @ (p - y) / len(starts) + l2 * self.theta
            m = 0.9 * m + 0.1 * grad
            v = 0.999 * v + 0.001 * grad * grad
            self.theta -= lr * 0.1 * (m / (1 - 0.9 ** t)) / (np.sqrt(v / (1 - 0.999 ** t)) + 1e-8)

    def _rows(self, c: dict, loo: bool) -> np.ndarray:
        best = max(c["damage"]) or 1.0
        top = int(np.argmax(c["damage"]))
        n_w = self.total[c["weapon"]] - (1 if loo else 0)
        out = []
        for j, f in enumerate(c["candidates"]):
            x = np.zeros(len(INFUSIONS) * (1 + len(INF_STATS)) + 3)
            k = INFUSIONS.index(f)
            x[k] = 1.0
            x[len(INFUSIONS) + k * len(INF_STATS): len(INFUSIONS) + (k + 1) * len(INF_STATS)] = c["stats"]
            if self.use_prior:
                n_wf = self.count[(c["weapon"], f)] - (1 if loo and f == c["infusion"] else 0)
                x[-3] = np.log((n_wf + self.alpha * self.base[f]) / (n_w + self.alpha))
            x[-2] = c["damage"][j] / best
            x[-1] = float(j == top)
            out.append(x)
        return np.array(out)

    def _matrix(self, cases: list[dict], loo: bool):
        rows, y, starts = [], [], []
        for c in cases:
            starts.append(sum(len(r) for r in rows))
            rows.append(self._rows(c, loo))
            y.extend(float(f == c["infusion"]) for f in c["candidates"])
        return np.vstack(rows), np.array(y), np.array(starts)

    @staticmethod
    def _softmax(s: np.ndarray, starts: np.ndarray) -> np.ndarray:
        s = s - np.repeat(np.maximum.reduceat(s, starts), np.diff(np.append(starts, len(s))))
        e = np.exp(s)
        return e / np.repeat(np.add.reduceat(e, starts), np.diff(np.append(starts, len(s))))

    def predict(self, c: dict) -> list[tuple[float, str]]:
        """(probability, infusion), most likely first."""
        s = self._rows(c, loo=False) @ self.theta
        p = np.exp(s - s.max())
        p /= p.sum()
        return sorted(zip(p.tolist(), c["candidates"]), reverse=True)


def evaluate_infusion_inference(data: Data, corpus: list[Build], folds: int = 5, seed: int = 0) -> dict:
    """Hide each SoulsPlanner weapon's infusion and predict it from what MugenMonkey would also have,
    `folds`-fold split by build. Baselines are fitted on the same training folds."""
    cases = infusion_cases(data, corpus)
    builds = sorted({c["build"] for c in cases})
    fold_of = dict(zip(np.random.default_rng(seed).permutation(builds).tolist(), range(len(builds))))
    names = ["majority class", "per-weapon majority", "damage model best", "logit, stats + damage only",
             "logit (shipped)"]
    hits, top2, conf = Counter(), Counter(), Counter()
    cal = []  # (predicted probability, correct) of the shipped model, for the confidence table
    for k in range(folds):
        train = [c for c in cases if fold_of[c["build"]] % folds != k]
        test = [c for c in cases if fold_of[c["build"]] % folds == k]
        glob = Counter(c["infusion"] for c in train)
        per_w: dict = {}
        for c in train:
            per_w.setdefault(c["weapon"], Counter())[c["infusion"]] += 1
        ablate, full = InfusionModel(train, prior=False), InfusionModel(train)
        for c in test:
            maj = max(c["candidates"], key=lambda f: glob[f])
            pw = per_w.get(c["weapon"])
            wmaj = max(c["candidates"], key=lambda f: (pw[f], glob[f])) if pw else maj
            dm = c["candidates"][int(np.argmax(c["damage"]))]
            pa, pf = ablate.predict(c), full.predict(c)
            for name, guess in zip(names, [maj, wmaj, dm, pa[0][1], pf[0][1]]):
                hits[name] += guess == c["infusion"]
            top2["logit (shipped)"] += c["infusion"] in (pf[0][1], pf[1][1])
            cal.append((pf[0][0], pf[0][1] == c["infusion"]))
            if pf[0][1] != c["infusion"]:
                conf[(c["infusion"], pf[0][1])] += 1
    n = len(cases)
    bands = []
    for lo, hi in ((0.0, 0.5), (0.5, 0.7), (0.7, 0.9), (0.9, 1.01)):
        sel = [ok for p, ok in cal if lo <= p < hi]
        bands.append((lo, min(hi, 1.0), len(sel), sum(sel) / max(len(sel), 1)))
    return {"n": n, "builds": len(builds), "folds": folds, "acc": {m: hits[m] / max(n, 1) for m in names},
            "top2": top2["logit (shipped)"] / max(n, 1), "bands": bands,
            "labels": Counter(c["infusion"] for c in cases).most_common(), "confusions": conf.most_common(8)}


def infer_mugen_infusions(data: Data, corpus: list[Build], builds: list[Build]) -> list[list[tuple[str, str, float]]]:
    """Per build in `builds` (MugenMonkey, infusions '?'): (weapon, most likely infusion, its
    probability) for every weapon the model covers, from an InfusionModel fitted on every labelled
    corpus weapon. Defenses are the corpus's bracket averages."""
    model = InfusionModel(infusion_cases(data, corpus))
    out = [[] for _ in builds]
    for c in infusion_cases(data, corpus, recorded=False, builds=builds):
        p, f = model.predict(c)[0]
        out[c["build"]].append((c["weapon"], f, p))
    return out


def neighbour_weapons(data: Data, stats: dict, sl: int, corpus: list[Build], k: int = 50, top: int = 15,
                      status: list[str] | None = None):
    """Weapons carried by the k corpus builds in `sl`'s bracket nearest `stats` (per-stat bracket
    L1 distance). Only weapons `stats` can wield are listed (STR halved when two-handing is what
    makes it usable, flagged 2H only); shields are left out."""
    q = np.array([stat_bracket(stats[s]) for s in STATS])
    same = [b for b in corpus if sl_bracket(soul_level(data, b)) == sl_bracket(sl)]
    dist = [int(np.abs(np.array([stat_bracket(effective(data, b)[s]) for s in STATS]) - q).sum()) for b in same]
    near = [same[i] for i in np.argsort(dist, kind="stable")[:k]]
    used, infs = Counter(), {}
    for b in near:
        mine = {}
        for w, inf in b.weapons():
            if w in data.weapons and not data.weapons[w].get("isShield"):
                if status:  # the carried infusion must deal it; MugenMonkey's unknown '?' counts if the base weapon does
                    atk = (data.weapons[w]["infusions"].get("No_Infusion" if inf == "?" else inf) or {}).get("atk") or {}
                    if not any(atk.get(s) for s in status):
                        continue
                # dict, not set: a set of strings iterates in hash order, which Python randomizes per
                # process, so infusions tied on count below ranked differently from run to run
                mine.setdefault(w, {})[inf] = None
        for w, fs in mine.items():  # a build counts once per weapon, however many copies it carries
            used[w] += 1
            for inf in fs:
                infs.setdefault(w, Counter())[inf] += 1
    rows = []
    for w, c in used.most_common():
        grip = "1H" if weapon_ok(data, w, stats, False) else "2H only" if weapon_ok(data, w, stats, True) else None
        if grip:
            rows.append((c, data.weapons[w]["name"], infs[w].most_common(3), grip))
    return rows[:top], len(near), (min(dist), sorted(dist)[len(near) - 1]) if dist else (0, 0)


def nearest_builds(data: Data, stats: dict, sl: int, corpus: list[Build], k: int = 50) -> list[Build]:
    """The k corpus builds in `sl`'s bracket nearest `stats` by per-stat bracket L1 distance."""
    q = np.array([stat_bracket(stats[s]) for s in STATS])
    same = [b for b in corpus if sl_bracket(soul_level(data, b)) == sl_bracket(sl)]
    dist = [int(np.abs(np.array([stat_bracket(effective(data, b)[s]) for s in STATS]) - q).sum()) for b in same]
    return [same[i] for i in np.argsort(dist, kind="stable")[:k]]


COMMON_RING = 0.10  # a ring worn by at least this share of every corpus build counts as common to all builds
#: Rings a generated build never suggests or grants, however many builds wear them. Agape Ring
#: ("Absorbs souls.") is worn by 15.8% of the corpus to hold soul memory down, not for a fight.
NO_USE_RINGS = ("Agape_Ring",)


def suggest_rings(data: Data, near: Counter, every: Counter | None = None, n: int = 4) -> list[str]:
    """The `n` rings `near` counts most, except that each NO_USE_RINGS ring's place goes, in
    place, to the ring the nearest builds wear most (then all builds) that is neither a no-use
    ring nor in the upgrade group of a ring already in the list: never a second Ring of Blades."""
    top = [r for r, _ in near.most_common(n)]
    group = lambda r: data.rings[r].get("group", r)
    taken = {group(r) for r in top if r not in NO_USE_RINGS}
    out = []
    for r in top:
        if r in NO_USE_RINGS:
            r = next((c for c, _ in [*near.most_common(), *(every or Counter()).most_common()]
                      if c not in NO_USE_RINGS and group(c) not in taken), None)
            if r is None:
                continue
            taken.add(group(r))
        out.append(r)
    return out


def generate_armor(data: Data, corpus: list[Build], weapon: str, inf: str, two: bool, stats: dict,
                   rings: list[str]) -> tuple[list[str], str | None]:
    """The generated build's armour: best_armor's top set for a build holding only the primary and
    wearing `rings`, as display names head/chest/hands/legs ("Naked" for a slot left bare), and a
    note when the load cap left a slot bare or left no set at all -- never a silent naked build."""
    wearer = Build("", stats, ["Naked"] * 4, [(weapon, inf)], int(two), rings, [])
    cap, carried, sets = best_armor(data, wearer, dict(stats), threat_mix(data, corpus), top=1)
    if not sets:
        return [], (f"no armor fits: the weapon and rings weigh {carried:.1f}, over the {cap:.1f} a "
                    f"{EQUIP_CAP:.0%} load allows at VIT {stats['vitality']}")
    pieces = sets[0][2]
    bare = [s for s, p in zip(ARMOR_SLOTS, pieces) if p == "Naked"]
    note = (f"{', '.join(bare)} left bare: nothing wearable there fits the {cap - carried:.1f} of load "
            f"left under {EQUIP_CAP:.0%}") if bare else None
    return [data.armor[s][p]["name"] for s, p in zip(ARMOR_SLOTS, pieces)], note


def generate_build(data: Data, corpus: list[Build], weapon: str, inf: str, sl: int, objective: str = "damage",
                   window: float = 1.5, k: int = 50, allow_naked: bool = False,
                   grip: str = "two") -> dict | None:
    """A whole valid build for weapon+infusion at `sl`: optimize_build's class and stats with the
    weapon as primary; the top 15 one-handable and top 5 two-hand-only other weapons for those
    stats (damage over `window` seconds); 3 copies of each of the 4 rings the nearest-stat builds
    wear most (suggest_rings: no NO_USE_RINGS ring), plus one of every other ring at least
    COMMON_RING of all builds wear; and armour, the
    best_armor set under 70% load with the primary and those four rings carried. `allow_naked`
    skips the armour, as every generated build did before it had any."""
    best, floors = optimize_build(data, corpus, weapon, inf, sl, objective, grip)
    if best is None:
        return None
    val, cls, two, stats = best
    rows, _, _ = weapons_for(data, stats, sl, corpus, top=10_000, window=window)
    one, only2, seen = [], [], {data.weapons[weapon]["name"]}
    for dmg, name, winf, ar, label in rows:
        if name in seen:
            continue
        seen.add(name)
        (only2 if "2H only" in label else one).append((name, winf, round(dmg)))
    # dict.fromkeys, not set: most_common breaks ties by first-seen order, and a set of strings
    # iterates in the per-process hash order, so tied rings came out differently on every run
    near = Counter(r for b in nearest_builds(data, stats, sl, corpus, k) for r in dict.fromkeys(b.rings)
                   if r and r in data.rings)
    every = Counter(r for b in corpus for r in dict.fromkeys(b.rings) if r and r in data.rings)
    suggested = suggest_rings(data, near, every)
    common = [r for r, c in every.most_common() if c >= COMMON_RING * len(corpus) and r not in suggested
              and r not in NO_USE_RINGS]
    armor, armor_note = ([], None) if allow_naked else generate_armor(data, corpus, weapon, inf, two, stats,
                                                                     suggested)
    return {"class": cls, "sl": sl, "stats": stats, "two_handed": two, "objective": objective, "value": round(val),
            "primary": (data.weapons[weapon]["name"], inf), "weapons_1h": one[:15], "weapons_2h_only": only2[:5],
            "rings": [data.rings[r]["name"] for r in suggested for _ in range(3)]
            + [data.rings[r]["name"] for r in common], "armor": armor, "armor_note": armor_note}


def gear_bonus(data: Data, arm: list, rs: list) -> tuple[Counter, float]:
    rings = ring_effects(data)
    bonus, mul = Counter(), 1.0
    for slot, piece in zip(ARMOR_SLOTS, arm):
        for s, v in (data.armor[slot].get(piece, {}).get("alter") or {}).items():
            if isinstance(v, (int, float)):
                bonus[s] += v
    for r in rs:
        bonus.update(rings[r][1])
        mul *= rings[r][2]
    return bonus, mul


TOP_PCTS = [1, 25, 50, 75]  # "top N%": the value only N% of builds in the bracket reach


def bracket_values(data: Data, corpus: list[Build], r1: dict) -> dict:
    """Per SL bracket, each free stat's sorted base values across corpus builds. END comes only
    from high-stamina-weapon builds, matching how END is floored."""
    cut = high_stamina_cut(r1)
    out = {}
    for b in corpus:
        i = sl_bracket(soul_level(data, b))
        d = out.setdefault(i, {s: [] for s in FREE_STATS})
        for s in FREE_STATS:
            if s == "endurance" and not any(r1.get(w, 0) >= cut for w, _ in b.weapons()):
                continue
            d[s].append(b.stats[s])
    return out


def top_pct(values: list, pct: int) -> int | None:
    return int(np.ceil(np.percentile(values, 100 - pct))) if len(values) >= 20 else None


def spread(data: Data, cls: str, hard: dict, bonus: Counter, mul: float, load: float, sl: int,
           floors: dict, high: bool, agl_target: int, values: dict | None = None):
    """Free-stat options at a clamped SL, on top of the hard minimum `hard` (requirement stats fixed).

    Returns (leftover, ranges, presets). ranges[stat] = (min, max) that stat can take at this SL while
    every other free stat stays at its hard minimum -- the slider's bounds. presets are whole builds."""
    base = data.classes[cls]
    hard_sl = base["level"] + sum(hard[s] - base[s] for s in STATS)
    left = sl - hard_sl
    if left < 0:
        return left, {}, []
    ranges = {s: (hard[s], min(99, hard[s] + left)) for s in FREE_STATS}
    med = floors[sl_bracket(sl)]
    targets = {s: med.get(s, 0) for s in FREE_STATS}
    if not high:
        targets["endurance"] = hard["endurance"]

    def fill(start, n, order_fn):
        st = dict(start)
        while n > 0:
            s = order_fn(st)
            if s is None:
                break
            st[s] += 1
            n -= 1
        return st, n

    def corpus_shape(pinned=None):
        def pick(st):
            # biggest shortfall against the bracket median first; once all met, keep median proportions
            free = [s for s in FREE_STATS if s != pinned and st[s] < 99]
            if not free:
                return None
            gap, s = max((targets[s] - st[s], s) for s in free)
            if gap > 0:
                return s
            return min((st[s] / max(1, targets[s]), s) for s in free)[1]
        return pick

    presets = [("corpus-typical", fill(hard, left, corpus_shape()))]
    def stat_values(s):
        """This bracket's values for s, or the nearest bracket's with at least 20 builds."""
        i = sl_bracket(sl)
        for j in sorted(range(len(SL_BRACKETS)), key=lambda j: (abs(j - i), j)):
            v = (values or {}).get(j, {}).get(s, [])
            if len(v) >= 20:
                return v
        return []

    for s in FREE_STATS:
        if s == "endurance" and not high:
            continue
        for pct in TOP_PCTS:
            want = top_pct(stat_values(s), pct)
            if want is None:
                continue
            start = dict(hard)
            cost = max(0, want - hard[s])
            start[s] = hard[s] + min(cost, left)
            short = cost - min(cost, left)
            st, n = fill(start, left - min(cost, left), corpus_shape(pinned=s))
            name = f"{s[:3].upper()} top {pct}% ({want})" + (f" short {short}" if short else "")
            presets.append((name, (st, n)))
    out = []
    for name, (st, unspent) in presets:
        eff = {k: st[k] + bonus.get(k, 0) for k in STATS}
        cap = data.equip_load[min(eff["vitality"], 99)] * mul
        out.append({
            "name": name, "stats": st, "unspent": unspent,
            "agl": agility(eff["adaptability"], eff["attunement"]),
            "load_pct": 100 * load / cap, "armor_budget_70": cap * EQUIP_CAP - load,
            "slots": data.att_slots[min(eff["attunement"], len(data.att_slots) - 1)],
        })
    return left, ranges, out


def recommended_minimum(data: Data, corpus: list[Build], weapon: str, two: bool, agl: int = 0,
                        sl: int | None = None) -> dict:
    """--minimum's answer: the cheapest class's hard minimum (requirements and 70% load, gear
    allowed), clamped at `sl` or else at that class's corpus-typical SL, and the free stats spread
    there. `presets[0]` ("corpus-typical") is the recommended build."""
    r1 = stamina_r1(data)
    cut = high_stamina_cut(r1)
    high = r1.get(weapon, 0) >= cut
    floors = bracket_floors(data, corpus, r1)
    hard = minimum_build(data, weapon, two, agl)
    typical = minimum_build(data, weapon, two, agl, floors, high)
    typ = {cls: s for s, cls, _ in typical}
    _, cls, (_, stats, arm, rs, load, mul) = hard[0]
    target_sl = sl or typ[cls]
    bonus, mul = gear_bonus(data, arm, rs)
    left, ranges, presets = spread(data, cls, stats, bonus, mul, load, target_sl, floors, high, agl,
                                   bracket_values(data, corpus, r1))
    return {"r1": r1, "cut": cut, "high": high, "hard": hard, "typ": typ, "target_sl": target_sl,
            "bonus": bonus, "left": left, "ranges": ranges, "presets": presets}


# --------------------------------------------------------------------------------------------
# the in-game panel's data file
#
# `--export-backend PATH` writes everything `CorpusBackend` in crates/ds2-build-recommender-core
# needs to answer the panel the way this script answers the command line: the weapon tables, each
# weapon's R1 timeline, the per-bracket floors and average defender, the gear --minimum searches,
# and the corpus cut down to what nearest_builds and the ring counts read. Line-oriented text, one
# record per line, tab-separated, tagged by its first field. Floats are written with repr, which is
# the shortest text that parses back to the same double, so the Rust side computes on the same bits.
#
#   C key name level vig end vit att str dex adp int fth      a starting class, in the table's order
#   T table v0 v1 ...                                         an attack-bonus or equip-load table
#   W key name class flags weight require ha1h ha2h counter   a weapon; flags: S shield, C catalyst,
#                                                             H high-stamina R1; require: stat:value,..
#   I code atk(7) scale(9)                                    one infusion of the last W
#   L grip t:mv:type:lower ...                                the last W's R1 chain, grip 1 or 2
#   B bracket floors(5) defense(8)                            one SL bracket
#   R key name weight group                                   a ring; group: its upgrade line
#   N key                                                     a ring in NO_USE_RINGS
#   E ring weight mul add(9)                                  a ring --minimum may wear
#   A slot key name weight alter(9) require                   armour --minimum may wear
#   P slot key name weight def(5) require                     every armour piece, for best_armor
#   H physical magic fire lightning dark                      the corpus threat mix (threat_mix)
#   K n top1 top2                                             calibrate_infusions
#   M ring count                                              a ring worn by >= COMMON_RING of builds
#   X bracket stat-brackets ring,ring.. weapon:code,..        one corpus build

BACKEND_DATA_NAME = "ds2-build-recommender.dat"
BACKEND_DATA = Path.home() / ".cache/ds2-builds" / BACKEND_DATA_NAME
BACKEND_FORMAT = "ds2-build-recommender-data 4"
#: How far the exported R1/R2 chains run, in seconds: the panel clamps its window to 10.0
#: (crates/ds2-build-recommender-ui/src/panel.rs), and status_hits runs to max(3, window).
STATUS_HORIZON = 10.0
#: Mirrors `infusion_for_code` in crates/ds2-build-recommender-core/src/weapons.rs; `?` is
#: MugenMonkey's unrecorded infusion.
INFUSION_CODE = {"No_Infusion": "N", "Magic": "M", "Fire": "F", "Lightning": "L", "Dark": "D", "Poison": "P",
                 "Bleed": "B", "Raw": "R", "Enchanted": "E", "Mundane": "U", "?": "?"}
#: A hit's physical type; anything else (none, "standard") is read against general physical defense.
HIT_CODE = {"slash": "s", "strike": "k", "thrust": "t"}
ATK_KEYS = ["physical", "magic", "fire", "lightning", "dark", "bleed", "poison"]
SCALE_KEYS = ["strength", "dexterity", "magic", "fire", "lightning", "dark", "bleed", "poison", "modifier"]
BACKEND_TABLES = ["physicalATKBonus", "magicATKBonus", "fireATKBonus", "lightningATKBonus", "darkATKBonus",
                  "auxATKBonus", "mundaneATKBonus"]


def _num(v) -> str:
    if isinstance(v, float):
        return repr(v)
    assert isinstance(v, int) and not isinstance(v, bool), v
    return str(v)


def _stat_pairs(d: dict) -> str:
    """`index:value,...` over STATS, in the dict's own order (key presence matters to --minimum)."""
    assert all(s in STATS for s in d), d
    return ",".join(f"{STATS.index(s)}:{_num(v)}" for s, v in d.items())


def export_backend(data: Data, corpus: list[Build]) -> str:
    attacks = load_attacks()
    rates = json.loads(HYPERARMOR.read_text()) if HYPERARMOR.exists() else {}
    crit = {norm(k): v for k, v in json.loads(CRIT.read_text()).items()} if CRIT.exists() else {}
    r1 = stamina_r1(data)
    cut = high_stamina_cut(r1)
    out = [BACKEND_FORMAT, f"# {len(corpus)} builds; written by scripts/ds2-builds-recommend.py --export-backend"]
    for key, c in data.classes.items():
        out.append("\t".join(["C", key, c["name"], _num(c["level"]), *(_num(c[s]) for s in STATS)]))
    for t in BACKEND_TABLES:
        out.append("\t".join(["T", t, *(_num(v or 0) for v in data.sp[t])]))
    out.append("\t".join(["T", "equipmentLoad", *(_num(v or 0) for v in data.equip_load)]))
    for key, w in data.weapons.items():
        flags = ("S" if w.get("isShield") else "") + ("C" if CATALYST.search(key) else "") \
            + ("H" if r1.get(key, 0) >= cut else "")
        ctr = (crit.get(norm(w["name"])) or {}).get("counter") or 0
        out.append("\t".join(["W", key, w["name"], data.weapon_class.get(key) or "", flags or "-",
                              _num(w.get("weight", 0)), _stat_pairs(w.get("require") or {}) or "-",
                              _num(float(hyperarmor(attacks, rates, w["name"], False))),
                              _num(float(hyperarmor(attacks, rates, w["name"], True))), _num(ctr)]))
        for inf, row in w["infusions"].items():
            atk, sc = row.get("atk") or {}, row.get("atkScale") or {}
            out.append("\t".join(["I", INFUSION_CODE[inf], *(_num(atk.get(k, 0)) for k in ATK_KEYS),
                                  *(_num(sc.get(k, 1 if k == "modifier" else 0)) for k in SCALE_KEYS)]))
        for two in (False, True):
            tl = r1_timeline(attacks, w["name"], two) or r1_timeline(attacks, key.replace("_", " "), two)
            if tl:
                out.append("\t".join(["L", "2" if two else "1", " ".join(
                    f"{_num(t)}:{_num(mv)}:{HIT_CODE.get(ty, 'p')}:{_num(lo)}" for t, mv, ty, lo in tl)]))
        # status_hits' inputs, per grip and R1/R2 chain, one row per name it would try (in its order,
        # repeats of one normalized name dropped): the hits of one attack, and every hit of the
        # repeated chain out to STATUS_HORIZON as `attack start:hit time`, so a window up to the
        # panel's cap cuts it as the script's own max(3, window) horizon does.
        for two in (False, True):
            for kind, tag in (("Normal", "R1"), ("Strong", "R2")):
                for nm in dict.fromkeys([w["name"], key.replace("_", " ")], None):
                    first = attacks.get((norm(nm), ("Single2Hand" if two else "Single1Hand") + kind + "1st"))
                    if first is None or (nm != w["name"] and norm(nm) == norm(w["name"])):
                        continue
                    tl = chain_timeline(attacks, nm, two, kind, STATUS_HORIZON, distinct=True, with_start=True)
                    out.append("\t".join(["S", "2" if two else "1", tag, str(attack_hits(first)),
                                          " ".join(f"{_num(h[4])}:{_num(h[0])}" for h in tl) or "-"]))
    floors = bracket_floors(data, corpus, r1)
    for i, (lo, _) in enumerate(SL_BRACKETS):
        dfn, _ = bracket_defense(data, corpus, lo)
        f = floors[i]
        out.append("\t".join(["B", str(i), *(_num(f.get(s, 0)) for s in FLOOR_STATS + ["endurance"]),
                              *(_num(float(dfn[k])) for k in DMG + PHYS_TYPES)]))
    ring_ix = {k: i for i, k in enumerate(data.rings)}
    for key, r in data.rings.items():
        out.append("\t".join(["R", key, r.get("name", key), _num(r.get("weight", 0)), r.get("group", key)]))
    out.extend("\t".join(["N", key]) for key in NO_USE_RINGS if key in data.rings)
    for slot in ARMOR_SLOTS:
        for key, v in data.armor[slot].items():
            out.append("\t".join(["P", slot, key, v.get("name", key), _num(v.get("weight", 0)),
                                  *(_num(v.get(k + "DEF", 0)) for k in DMG),
                                  _stat_pairs(v.get("require") or {}) or "-"]))
    mix = threat_mix(data, corpus)
    out.append("\t".join(["H", *(_num(float(mix[k])) for k in DMG)]))
    for key, (weight, add, mul) in sorted(ring_effects(data).items()):
        out.append("\t".join(["E", key, _num(weight), _num(float(mul)), *(_num(add.get(s, 0)) for s in STATS)]))
    helpful = {"strength", "dexterity", "intelligence", "faith", "vitality", "adaptability", "attunement"}
    for slot in ARMOR_SLOTS:
        for key, v in data.armor[slot].items():
            alt = {s: x for s, x in (v.get("alter") or {}).items() if isinstance(x, (int, float))}
            # minimum_build offers a piece only when it raises a stat it can use: a superset of that
            if key == "Naked" or not any(s in helpful and x > 0 for s, x in alt.items()):
                continue
            out.append("\t".join(["A", slot, key, v.get("name", key), _num(float(v["weight"])),
                                  *(_num(alt.get(s, 0)) for s in STATS), _stat_pairs(v.get("require") or {}) or "-"]))
    c = calibrate_infusions(data, corpus)
    out.append("\t".join(["K", str(c["n"]), _num(float(c["top1"])), _num(float(c["top2"]))]))
    every = Counter(r for b in corpus for r in dict.fromkeys(b.rings) if r and r in data.rings)
    for r, n in every.most_common():
        if n >= COMMON_RING * len(corpus):
            out.append("\t".join(["M", r, str(n)]))
    weapon_ix = {k: i for i, k in enumerate(data.weapons)}
    for b in corpus:
        eff = effective(data, b)
        out.append("\t".join(["X", str(sl_bracket(soul_level(data, b))),
                              "".join(str(stat_bracket(eff[s])) for s in STATS),
                              ",".join(str(ring_ix[r]) for r in b.rings if r and r in data.rings) or "-",
                              ",".join(f"{weapon_ix[w]}:{INFUSION_CODE[inf]}" for w, inf in b.weapons()) or "-"]))
    return "\n".join(out) + "\n"


# The questions the Rust tests ask both sides. Stats in STATS order.
EXPECT_WEAPONS_FOR = [
    # stats, sl, one_hand, weapon_class, per_class, window, raw_ar, objective
    ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, False, None, False, 0.0, False, "damage"),
    ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, False, None, False, 1.5, False, "damage"),
    ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, True, None, False, 0.0, True, "damage"),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, False, None, True, 0.0, False, "damage"),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, False, "Katana", False, 2.0, False, "damage"),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, False, None, False, 0.0, False, "bleed"),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, False, None, False, 2.0, False, "bleed"),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, False, None, True, 5.0, False, "bleed"),
    ([20, 20, 15, 10, 20, 30, 30, 9, 9], 100, True, None, False, 0.0, False, "poison"),
]
EXPECT_BUILDS = [  # weapon key, infusion, sl, objective: --optimize and --generate
    ("Demons_Great_Hammer", "Raw", 100, "damage"),
    ("Moonlight_Greatsword", "No_Infusion", 33, "damage"),
    ("Demons_Great_Hammer", "Raw", 20, "damage"),
    ("Demons_Great_Hammer", "Raw", 70, "damage"),
    ("Moonlight_Greatsword", "No_Infusion", 90, "damage"),
    ("Uchigatana", "Bleed", 150, "bleed"),
    ("Dagger", "Poison", 60, "poison"),
]
EXPECT_NAKED = [("Demons_Great_Hammer", "Raw", 100, "damage", True)]  # --generate --allow-naked
EXPECT_ONE_HANDED = [  # --optimize and --generate with --grip one
    ("Murakumo", "Dark", 74, "damage"),
    ("Demons_Great_Hammer", "Raw", 20, "damage"),
    ("Demons_Great_Hammer", "Raw", 100, "damage"),
    ("Uchigatana", "Bleed", 150, "bleed"),
]
EXPECT_MINIMUM = [("Demons_Great_Hammer", True), ("Moonlight_Greatsword", False), ("Uchigatana", False)]
EXPECT_SIMILAR = [  # stats, sl, k, status
    ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, 50, None),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, 30, ["bleed"]),
    ([12, 10, 8, 10, 14, 14, 10, 8, 8], 40, 50, ["bleed", "poison"]),
]
EXPECT_FLOOR_SLS = [1, 33, 60, 100, 150, 200, 838]
EXPECT_BEST_INFUSION = [  # weapon key, stats, sl, window, raw_ar, objective: --best-infusion
    ("Caestus", [20, 20, 15, 10, 40, 40, 15, 9, 9], 125, 0.0, False, "damage"),
    ("Drakeblood_Greatsword", [20, 20, 15, 10, 20, 20, 15, 9, 40], 116, 0.0, False, "damage"),
    ("Drakeblood_Greatsword", [20, 20, 15, 10, 20, 20, 15, 9, 40], 116, 1.5, False, "damage"),
    ("Uchigatana", [25, 20, 15, 12, 12, 40, 15, 9, 20], 150, 0.0, True, "damage"),
    ("Uchigatana", [25, 20, 15, 12, 12, 40, 15, 9, 20], 150, 0.0, False, "bleed"),
    ("Dagger", [20, 20, 15, 10, 20, 30, 30, 9, 9], 100, 2.0, False, "poison"),
    ("Greatsword", [20, 20, 15, 10, 10, 10, 15, 9, 9], 80, 0.0, False, "damage"),  # STR 10 < 28/2: no rows
]


class _Some:
    """A fixture value Rust reads as `Option::Some`."""

    def __init__(self, value):
        self.value = value


def _rs(v) -> str:
    """A Rust literal for a fixture value."""
    if v is None:
        return "None"
    if isinstance(v, _Some):
        return "Some(" + _rs(v.value) + ")"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, float):
        r = repr(v)
        return r if ("." in r or "e" in r) else r + ".0"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, tuple):
        return "(" + ", ".join(_rs(x) for x in v) + ("," if len(v) == 1 else "") + ")"
    if isinstance(v, list):
        return "&[" + ", ".join(_rs(x) for x in v) + "]"
    raise TypeError(v)


def backend_expectations(data: Data, corpus: list[Build]) -> str:
    """This script's own answers to EXPECT_*, as the Rust fixture `CorpusBackend` is tested against."""
    attacks = load_attacks()
    rates = json.loads(HYPERARMOR.read_text()) if HYPERARMOR.exists() else {}
    crit = {norm(k): v for k, v in json.loads(CRIT.read_text()).items()} if CRIT.exists() else {}
    as_dict = lambda st: dict(zip(STATS, st))
    arr = lambda d: [int(d[s]) for s in STATS]
    out = ["// @generated by `scripts/ds2-builds-recommend.py --export-backend ... --expect ...`: the",
           "// script's own answers for the questions in its EXPECT_* lists, over the fixture corpus.", ""]

    rows_out = []
    for st, sl, one, cls, per, window, raw, objective in EXPECT_WEAPONS_FOR:
        rows, _, _ = weapons_for(data, as_dict(st), sl, corpus, raw_ar=raw, one_hand=one, weapon_class=cls,
                                 per_class=per, window=window, objective=objective)
        got = []
        for dmg, name, inf, ar, label in rows:
            key = data.key_by_name[name]
            wclass = data.weapon_class.get(key) or "?"
            base = label[:-(len(wclass) + 1)] if per else label  # per_class appends " <class>"
            base = re.sub(r" (HA|ctr) x\S+", "", base).strip()
            ha = hyperarmor(attacks, rates, name, base.startswith("2H"))
            ctr = (crit.get(norm(name)) or {}).get("counter") or 0
            got.append((name, INFUSION_CODE[inf], float(dmg), [float(ar.get(k, 0)) for k in DMG], base,
                        float(ha), float(ctr), wclass))
        rows_out.append((st, sl, one, cls or "", per, window, raw, objective, got))
    out.append("// stats, sl, one_hand, class, per_class, window, raw_ar, objective, rows: weapon, infusion,")
    out.append("// damage (build-up x hits for bleed/poison), ar by type, grip, hyperarmor, counter, class.")
    out.append(f"pub const WEAPONS_FOR: WeaponsForCases = {_rs(rows_out)};\n")

    mix = threat_mix(data, corpus)
    opt, gen = expect_builds(data, corpus, mix, EXPECT_BUILDS, EXPECT_NAKED, "two")
    opt_one, gen_one = expect_builds(data, corpus, mix, EXPECT_ONE_HANDED, [], "one")
    out.append("// weapon, infusion, sl, objective -> class, two-handed, stats, value.")
    out.append(f"pub const OPTIMIZE: OptimizeCases = {_rs(opt)};\n")
    out.append("// weapon, infusion, sl, objective, allow naked -> class, two-handed, stats, 1H rows, 2H-only")
    out.append("// rows (name, infusion, rounded damage), suggested rings, common rings, armour, armour note,")
    out.append("// whether armour requirements bound the choice, whether the load cap did.")
    out.append(f"pub const GENERATE: GenerateCases = {_rs(gen)};\n")
    out.append("// The same two questions with the grip forced one-handed (--grip one).")
    out.append(f"pub const OPTIMIZE_ONE_HANDED: OptimizeCases = {_rs(opt_one)};\n")
    out.append(f"pub const GENERATE_ONE_HANDED: GenerateCases = {_rs(gen_one)};\n")

    mins = []
    for weapon, two in EXPECT_MINIMUM:
        m = recommended_minimum(data, corpus, weapon, two)
        _, cls, (_, _, arm, rs, _, _) = m["hard"][0]
        mid = m["presets"][0]
        gear = [data.armor[s][p]["name"] for s, p in zip(ARMOR_SLOTS, arm) if p != "Naked"] \
            + [data.rings[r]["name"] for r in rs]
        mins.append((weapon, two, (data.classes[cls]["name"], m["target_sl"], arr(mid["stats"]), gear)))
    return backend_expectations_rest(data, corpus, out, mins)


def expect_builds(data: Data, corpus: list[Build], mix, cases: list, naked_cases: list, grip: str):
    """The script's --optimize and --generate answers for `cases` (plus `naked_cases` generated with
    --allow-naked) at `grip`, as fixture tuples."""
    arr = lambda d: [int(d[s]) for s in STATS]
    opt, gen = [], []
    for weapon, inf, sl, objective in cases:
        best, _ = optimize_build(data, corpus, weapon, inf, sl, objective, grip)
        opt.append((weapon, INFUSION_CODE[inf], sl, objective,
                    None if best is None else _Some((data.classes[best[1]]["name"], best[2], arr(best[3]),
                                                     float(best[0])))))
    for weapon, inf, sl, objective, naked in [(*case, False) for case in cases] + naked_cases:
        g = generate_build(data, corpus, weapon, inf, sl, objective, allow_naked=naked, grip=grip)
        if g is None:
            gen.append((weapon, INFUSION_CODE[inf], sl, objective, naked, None))
            continue
        rings, suggested = g["rings"], []
        while len(rings) >= 3 and rings[0] == rings[1] == rings[2]:
            suggested.append(rings[0])
            rings = rings[3:]
        # Whether the armour's requirements and the load cap each changed the chosen set: the same
        # search with every other stat at 99, and with VIT at 99, picks differently.
        binds = (False, False)
        if not naked:
            worn = [data.sp_key[norm(r)] for r in suggested]
            wearer = Build("", g["stats"], ["Naked"] * 4, [(weapon, inf)], 0, worn, [])
            top = lambda eff: (best_armor(data, wearer, eff, mix, top=1)[2] or [None])[0]
            chosen = top(dict(g["stats"]))
            binds = (chosen != top({**{s: 99 for s in STATS}, "vitality": g["stats"]["vitality"]}),
                     chosen != top({**g["stats"], "vitality": 99}))
        gen.append((weapon, INFUSION_CODE[inf], sl, objective, naked, _Some((
            data.classes[g["class"]]["name"], g["two_handed"], arr(g["stats"]),
            [(n, INFUSION_CODE[i], d) for n, i, d in g["weapons_1h"]],
            [(n, INFUSION_CODE[i], d) for n, i, d in g["weapons_2h_only"]], suggested, rings,
            g["armor"], g["armor_note"] and _Some(g["armor_note"]), binds[0], binds[1]))))
    return opt, gen


def backend_expectations_rest(data: Data, corpus: list[Build], out: list[str], mins: list) -> str:
    """backend_expectations from MINIMUM on."""
    as_dict = lambda st: dict(zip(STATS, st))
    out.append("// weapon, two-handed -> class, sl, stats, gear.")
    out.append(f"pub const MINIMUM: MinimumCases = {_rs(mins)};\n")

    sim = []
    for st, sl, k, status in EXPECT_SIMILAR:
        rows, _, _ = neighbour_weapons(data, as_dict(st), sl, corpus, k=k, status=status)
        sim.append((st, sl, k, status or [], [(c, name, [(INFUSION_CODE[i], m) for i, m in infs], grip)
                                             for c, name, infs, grip in rows]))
    out.append("// stats, sl, k, status -> rows: count, weapon, top infusions with counts, grip.")
    out.append(f"pub const SIMILAR: SimilarCases = {_rs(sim)};\n")

    r1 = stamina_r1(data)
    floors = bracket_floors(data, corpus, r1)
    fl = [(sl, [int(floors[sl_bracket(sl)].get(s, 0)) for s in FLOOR_STATS]) for sl in EXPECT_FLOOR_SLS]
    out.append("// sl -> VIG, VIT, ADP, ATT floors.")
    out.append(f"pub const FLOORS: FloorCases = {_rs(fl)};\n")

    best = []
    for weapon, st, sl, window, raw, objective in EXPECT_BEST_INFUSION:
        rows, _, _ = best_infusion(data, weapon, as_dict(st), sl, corpus, raw_ar=raw, window=window,
                                   objective=objective)
        best.append((weapon, st, sl, window, raw, objective,
                     [(INFUSION_CODE[inf], float(v), [float(ar.get(k, 0)) for k in DMG],
                       re.sub(r" (HA|ctr) x\S+", "", grip).strip()) for v, _, inf, ar, grip in rows]))
    out.append("// weapon, stats, sl, window, raw_ar, objective -> rows best first: infusion, score, AR by type,")
    out.append("// grip.")
    out.append(f"pub const BEST_INFUSION: BestInfusionCases = {_rs(best)};\n")
    c = calibrate_infusions(data, corpus)
    out.append(f"pub const CALIBRATION: (u32, f64, f64) = {_rs((c['n'], float(c['top1']), float(c['top2'])))};")
    return "\n".join(out) + "\n"


def pretty(t: str, data: Data) -> str:
    kind, rest = t.split(":", 1)
    if kind == "WI":
        w, inf = rest.split("|")
        return f"{data.weapons[w]['name']} ({inf.replace('_', ' ')})"
    table = {"R": data.rings, "S": data.spells, "W": data.weapons}[kind]
    return table.get(rest, {}).get("name", rest)


def _hit(start, end, rate, n=1, interval=0.0, dmg=1, live=1):
    return {"start": start, "end": end, "rate": rate, "n": n, "interval": interval, "dmg": dmg, "live": live}


# Rows as ~/.cache/ds2-builds/attacks.json holds them (TAE 2200 windows in 30 fps frames, REGULATION
# damage rows); anim -1 has no TAE file, so chain_open ends each chain after its first attack.
SELFTEST_ATTACKS = {
    ("channelerstrident", "Single1HandNormal1st"):
        {"anim": -1, "spd": [0.8, 1.5], "hits": [_hit(14, 20, 0.96, dmg=10017820)]},
    ("channelerstrident", "Single1HandStrong1st"):
        {"anim": -1, "spd": [1.0, 1.0], "hits": [_hit(19, 24, 0.576, dmg=10018240),
                                                   _hit(19, 30, None, dmg=0, live=False),
                                                   _hit(23, 35, 0.2688, 3, 0.15, dmg=10018241)]},
    ("oldwhip", "Single1HandNormal1st"):
        {"anim": -1, "spd": [1.0, 1.0], "hits": [_hit(16, 20, 1.0, dmg=10011000), _hit(16, 20, 0.35, dmg=3660000)]},
    ("twinblade", "Single2HandStrong2nd"):
        {"anim": -1, "spd": [1.0, 1.0], "hits": [_hit(10, 19, 1.2096, dmg=10016400), _hit(28, 35, 1.2096, dmg=10016400)]},
    ("dagger", "Single1HandNormal1st"): {"anim": -1, "spd": [1.1, 1.2], "hits": [_hit(8, 15, 1.0, dmg=10004100)]},
}


def selftest() -> int:
    """Offline checks of the hits-per-attack model (no site tables, no corpus)."""
    A = SELFTEST_ATTACKS
    RINGS = type("Rings", (), {"rings": {k: {"group": g} for k, g in [
        ("Third_Dragon_Ring", "Dragon_Ring"), ("Agape_Ring", "Agape_Ring"), ("Ring_of_Blades", "Ring_of_Blades"),
        ("Ring_of_Blades_2", "Ring_of_Blades"), ("Flynns_Ring", "Flynns_Ring"), ("Life_Ring", "Life_Ring")]}})
    cases = [
        ("trident R2 = opening hit + 3 spin ticks", attack_hits(A[("channelerstrident", "Single1HandStrong1st")]), 4),
        ("trident R1 = one hit", attack_hits(A[("channelerstrident", "Single1HandNormal1st")]), 1),
        ("old whip same-window pair counts once", attack_hits(A[("oldwhip", "Single1HandNormal1st")]), 1),
        ("old whip pair still both in the damage timeline", len(r1_timeline(A, "Old Whip", False)), 2),
        ("twinblade 2H R2 chain: one row, two windows", attack_hits(A[("twinblade", "Single2HandStrong2nd")]), 2),
        ("no row, no hits", attack_hits(A.get(("dagger", "Single1HandStrong1st"))), 0),
        ("trident per attack: R2 wins", status_hits(A, ["Channeler's Trident"], [False]), (4, "1H R2 4 hits")),
        ("dagger per attack: R1", status_hits(A, ["Dagger"], [False]), (1, "1H R1 1 hit")),
        # timing: in 0.5 s only the R1 (14/30 s at speed 1.15) has landed; by 1.5 s the spin's last
        # tick (23/30 + 2 * 0.15 s) has too
        ("trident in 0.5 s: R1 only", status_hits(A, ["Channeler's Trident"], [False], 0.5), (1, "1H R1 1 hit")),
        ("trident in 1.5 s: R2", status_hits(A, ["Channeler's Trident"], [False], 1.5), (4, "1H R2 4 hits")),
        # best-infusion margin: best over runner-up, as a fraction of the runner-up
        ("margin 300 over 200", infusion_margin([300.0, 200.0, 50.0]), 0.5),
        ("margin needs a runner-up", infusion_margin([300.0]), None),
        ("margin over a zero runner-up is none", infusion_margin([300.0, 0.0]), None),
        ("stats parse into STATS order", list(parse_stats("STR=40,VGR=20,ADP=15").values()),
         [20, 0, 0, 0, 40, 0, 15, 0, 0]),
        # Agape's place goes to the next ring worn, skipping one already worn in another upgrade
        ("agape replaced in place", suggest_rings(RINGS, Counter({"Third_Dragon_Ring": 9, "Agape_Ring": 8,
                                                                  "Ring_of_Blades": 7, "Flynns_Ring": 6,
                                                                  "Ring_of_Blades_2": 5, "Life_Ring": 4})),
         ["Third_Dragon_Ring", "Life_Ring", "Ring_of_Blades", "Flynns_Ring"]),
        ("agape falls back to all builds", suggest_rings(RINGS, Counter({"Agape_Ring": 2, "Flynns_Ring": 1}),
                                                         Counter({"Flynns_Ring": 9, "Life_Ring": 3})),
         ["Life_Ring", "Flynns_Ring"]),
        ("no agape, no change", suggest_rings(RINGS, Counter({"Flynns_Ring": 2, "Ring_of_Blades_2": 1})),
         ["Flynns_Ring", "Ring_of_Blades_2"]),
    ]
    if ATTACKS.exists():  # the real extracted rows agree with the copies above
        real = load_attacks()
        for key in [k for k in A if k in real]:
            cases.append((f"attacks.json {key}", attack_hits(real[key]), attack_hits(A[key])))
    bad = 0
    for what, got, want in cases:
        if got != want:
            bad += 1
            print(f"  FAIL {what}: got {got!r}, want {want!r}")
    print(f"selftest: {len(cases) - bad}/{len(cases)} pass" + ("" if ATTACKS.exists() else f" ({ATTACKS} absent)"))
    return 1 if bad else 0


def parse_stats(text: str) -> dict:
    """ "VGR=10,END=16,..." -> every stat in STATS order, 0 where not given."""
    abbr = {s[:3].upper(): s for s in STATS} | {"VGR": "vigor", "ADP": "adaptability", "FTH": "faith",
                                               "ATN": "attunement"}
    stats = {s: 0 for s in STATS}
    for part in text.split(","):
        k, v = part.split("=")
        stats[abbr[k.strip().upper()[:3]]] = int(v)
    return stats


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--selftest", action="store_true", help="offline checks of the hits-per-attack model")
    g.add_argument("--mugen", type=int, help="MugenMonkey DS2 build id to recommend for")
    g.add_argument("--soulsplanner", type=int, help="SoulsPlanner DS2 build id (from the cache)")
    g.add_argument("--eval", action="store_true", help="leave-one-out recall@10, EASE vs popularity")
    g.add_argument("--minimum", metavar="WEAPON", help="lowest SL per class to wield WEAPON (SoulsPlanner key or name)")
    g.add_argument("--calibrate", action="store_true",
                   help="how often the damage model's best infusion matches real SoulsPlanner builds' choice")
    g.add_argument("--infusion-eval", action="store_true",
                   help="MugenMonkey infusion inference, measured on SoulsPlanner builds with their infusion hidden "
                        "(5-fold by build) against majority-class baselines")
    g.add_argument("--optimize", metavar="WEAPON:INFUSION",
                   help="the valid build at --sl that maximizes --objective for this weapon, then --weapons-for on it")
    g.add_argument("--generate", metavar="WEAPON:INFUSION",
                   help="a whole build at --sl: stats, primary weapon, 15 one-handed + 5 two-hand-only weapons, rings")
    g.add_argument("--export-backend", metavar="PATH", type=Path,
                   help=f"write the in-game panel's data file (install it beside DarkSoulsII.exe as "
                        f"{BACKEND_DATA_NAME}; scripts/ds2-run.py stages {BACKEND_DATA})")
    ap.add_argument("--corpus-every", type=int, default=1, metavar="N",
                    help="with --export-backend: keep every Nth corpus build (the Rust tests' small fixture)")
    ap.add_argument("--expect", type=Path, metavar="PATH",
                    help="with --export-backend: also write this script's answers to the EXPECT_* questions "
                         "over the same corpus, as the Rust fixture CorpusBackend is tested against")
    ap.add_argument("--grip", choices=list(GRIP_TRIES), default="two",
                    help="with --optimize/--generate: 'two' (default) halves the STR requirement even when "
                         "one-handing would fit, 'one' needs it in full. Damage is scored the same for "
                         "either grip (a two-handed STR multiplier is unproven)")
    ap.add_argument("--allow-naked", action="store_true",
                    help="with --generate: no armour (by default the best set under 70%% load is chosen)")
    ap.add_argument("--objective", choices=["damage", "bleed", "poison"], default="damage",
                    help="with --optimize: what the free points maximize. With --weapons-for (and the list "
                         "--optimize prints): bleed/poison rank by build-up per hit x hits of the best R1/R2 "
                         "attack, or hits landed within --window seconds")
    g.add_argument("--weapons-for", metavar="STATS",
                   help='rank weapons for these stats, e.g. "VGR=10,END=16,VIT=7,ATT=9,STR=20,DEX=12,ADP=8,INT=12,FTH=12"')
    g.add_argument("--best-infusion", metavar="WEAPON",
                   help="every infusion WEAPON takes, ranked by --objective at --stats (full upgrade), with the "
                        "best one's margin over the runner-up")
    g.add_argument("--infusion-gaps", action="store_true",
                   help="the weapons whose best infusion is furthest ahead of their runner-up at --stats")
    ap.add_argument("--stats", metavar="STATS", help="with --best-infusion / --infusion-gaps: as --weapons-for takes")
    ap.add_argument("--raw-ar", action="store_true", help="with --weapons-for: rank by total attack rating, before defenses")
    ap.add_argument("--neighbours", action="store_true",
                    help="with --weapons-for: weapons carried by corpus builds with the nearest stats, usable only")
    ap.add_argument("--status", help="with --neighbours: only weapon+infusions dealing these, e.g. bleed,poison")
    ap.add_argument("--k", type=int, default=50, help="with --neighbours: how many nearest builds")
    ap.add_argument("--window", type=float, default=0.0,
                    help="with --weapons-for: rank by R1-chain damage landed within this many seconds (e.g. 1.5)")
    ap.add_argument("--per-class", action="store_true", help="with --weapons-for: the top weapon of each weapon class")
    ap.add_argument("--weapon-class", help="with --weapons-for: only this weapon class (MugenMonkey's, e.g. Dagger)")
    ap.add_argument("--one-hand", action="store_true", help="with --weapons-for: only weapons usable one-handed (drop 2H-only)")
    ap.add_argument("--agl", type=int, default=0, help="optional hard AGL floor for --minimum (default none: AGL is a slider output)")
    ap.add_argument("--sl", type=int, help="with --minimum: clamp SL here and show how free stats can spread")
    ap.add_argument("--json", action="store_true", help="with --minimum: also print the options as JSON")
    ap.add_argument("--lam", type=float, default=50.0, help="EASE L2 penalty")
    ap.add_argument("--site-numbers", action="store_true",
                    help="keep SoulsPlanner's physical stat defense and Enchanted coefficients instead of the "
                         "game's (apply_regulation), to compare against the planner")
    ap.add_argument("--tables", type=Path, default=CACHE / "site-tables", help="where site JS/JSON is cached")
    a = ap.parse_args()
    if a.selftest:
        return selftest()

    a.tables.mkdir(parents=True, exist_ok=True)
    sp_json, mm_json = dump_site_tables(a.tables)
    data = Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    if not a.site_numbers:
        print(apply_regulation(data), file=sys.stderr)
    if a.export_backend:
        corpus, _ = load_corpus(data)
        corpus = corpus[::max(1, a.corpus_every)]
        text = export_backend(data, corpus)
        a.export_backend.parent.mkdir(parents=True, exist_ok=True)
        a.export_backend.write_text(text, encoding="utf-8")
        print(f"{a.export_backend}: {len(text.encode()):,} bytes, {len(corpus)} builds")
        if a.expect:
            a.expect.write_text(backend_expectations(data, corpus), encoding="utf-8")
            print(f"{a.expect}: this script's answers over the same {len(corpus)} builds")
        return 0
    if a.calibrate:
        c = calibrate_infusions(data, load_corpus(data)[0])
        print(f"{c['n']} real weapon+infusion choices: model's best infusion matches {c['top1']:.1%}, "
              f"top two {c['top2']:.1%}")
        for (real, model), k in c["confusions"]:
            print(f"  {k:5}  chose {real:12} model says {model}")
        return 0
    if a.infusion_eval:
        corpus = load_corpus(data)[0]
        e = evaluate_infusion_inference(data, corpus)
        print(f"{e['n']} SoulsPlanner weapons ({e['builds']} builds), infusion hidden, {e['folds']}-fold by build")
        print("  labels: " + ", ".join(f"{f} {k}" for f, k in e["labels"]))
        for name, acc in e["acc"].items():
            print(f"  {acc:6.1%}  {name}")
        print(f"  {e['top2']:6.1%}  logit (shipped), top two")
        print("  shipped model by its own confidence:")
        for lo, hi, n, acc in e["bands"]:
            print(f"    p {lo:.1f}-{hi:.1f}: {n:5} weapons, {acc:6.1%} right")
        for (real, guess), k in e["confusions"]:
            print(f"  {k:5}  chose {real:12} inferred {guess}")
        mm = [b for b in corpus if any(i == "?" for _, i in b.weapons())]
        guessed = Counter(f for rows in infer_mugen_infusions(data, corpus, mm) for _, f, _ in rows)
        print(f"  inferred over {len(mm)} MugenMonkey corpus builds: "
              + ", ".join(f"{f} {k}" for f, k in guessed.most_common()))
        return 0
    if a.generate:
        if not a.sl:
            ap.error("--generate needs --sl")
        name, _, inf = a.generate.partition(":")
        weapon = data.sp_key.get(norm(name))
        if weapon not in data.weapons:
            ap.error(f"unknown weapon {name!r}")
        corpus, _ = load_corpus(data)
        g = generate_build(data, corpus, weapon, inf.replace(" ", "_") or "No_Infusion", a.sl, a.objective,
                           a.window or 1.5, a.k, a.allow_naked, a.grip)
        if g is None:
            print(f"no valid SL {a.sl} build wields {data.weapons[weapon]['name']} (grip {a.grip})")
            return 2
        if a.json:
            print(json.dumps(g, indent=1))
            return 0
        print(f"{g['class']} SL {g['sl']} {'two-handed' if g['two_handed'] else 'one-handed'}, "
              f"{g['objective']} {g['value']}\n  " + " ".join(f"{s[:3].upper()} {v}" for s, v in g["stats"].items()))
        print(f"  primary: {g['primary'][0]} ({g['primary'][1].replace('_', ' ')})")
        for title, key in (("one-handed", "weapons_1h"), ("two-hand only", "weapons_2h_only")):
            print(f"  {title}:")
            for n, i, d in g[key]:
                print(f"    {d:6}  {n} ({i.replace('_', ' ')})")
        print("  rings: " + ", ".join(g["rings"]))
        if a.allow_naked:
            print("  armor: none (--allow-naked)")
        else:
            print("  armor: " + (" / ".join(g["armor"]) or "none"))
        if g["armor_note"]:
            print(f"  armor: {g['armor_note']}")
        return 0
    if a.optimize:
        if not a.sl:
            ap.error("--optimize needs --sl")
        name, _, inf = a.optimize.partition(":")
        weapon = data.sp_key.get(norm(name))
        if weapon not in data.weapons:
            ap.error(f"unknown weapon {name!r}")
        inf = inf.replace(" ", "_") or "No_Infusion"
        corpus, _ = load_corpus(data)
        best, floors = optimize_build(data, corpus, weapon, inf, a.sl, a.objective, a.grip)
        if best is None:
            print(f"no valid SL {a.sl} build wields {data.weapons[weapon]['name']} (grip {a.grip}) with the "
                  "bracket floors "
                  + " ".join(f"{s[:3].upper()} {v}" for s, v in floors.items()))
            return 2
        val, cls, two, stats = best
        print(f"{data.weapons[weapon]['name']} ({inf.replace('_', ' ')}), {a.objective} {val:.0f}: {cls} SL {a.sl}"
              f" {'two-handed' if two else 'one-handed'}\n  "
              + " ".join(f"{s[:3].upper()} {stats[s]}" for s in STATS)
              + "\n  floors (bracket medians): " + " ".join(f"{s[:3].upper()} {v}" for s, v in floors.items()))
        a.weapons_for = ",".join(f"{s[:3].upper()}={stats[s]}" for s in STATS)
    if a.best_infusion or a.infusion_gaps:
        if not a.stats:
            ap.error("--best-infusion and --infusion-gaps need --stats")
        stats = parse_stats(a.stats)
        sl = a.sl or sum(stats.values()) - 53
        corpus, _ = load_corpus(data)
        what = {"damage": "raw AR" if a.raw_ar else "damage"}.get(a.objective, f"{a.objective} x hits")
        if a.window:
            what += f" in {a.window:g}s"
        print(f"stats {' '.join(f'{s[:3].upper()} {v}' for s, v in stats.items())}  ->  SL {sl}; by {what}, "
              "full upgrade")
        if a.best_infusion:
            weapon = data.sp_key.get(norm(a.best_infusion))
            if weapon not in data.weapons:
                ap.error(f"unknown weapon {a.best_infusion!r}")
            rows, _, _ = best_infusion(data, weapon, stats, sl, corpus, raw_ar=a.raw_ar, window=a.window,
                                       objective=a.objective)
            if not rows:
                print(f"{data.weapons[weapon]['name']}: nothing to rank (these stats cannot wield it, or no "
                      f"infusion deals {a.objective})")
                return 2
            m = infusion_margin([r[0] for r in rows])
            print(f"{data.weapons[weapon]['name']}: best {rows[0][2].replace('_', ' ')}"
                  + (f", {m:+.1%} over {rows[1][2].replace('_', ' ')}" if m is not None else ", no runner-up"))
            for v, _, inf, ar, grip in rows:
                print(f"  {inf.replace('_', ' '):12} {v:8.1f}  {v / rows[0][0]:6.1%}   "
                      f"{' '.join(f'{k[:4]} {x}' for k, x in ar.items()):34} {grip}")
            return 0
        gaps, _, _ = infusion_gaps(data, stats, sl, corpus, raw_ar=a.raw_ar, window=a.window,
                                   objective=a.objective, one_hand=a.one_hand, weapon_class=a.weapon_class)
        print(f"\n  {'weapon':32} {'best':12} {'runner-up':12} {'margin':>7}   of")
        for m, name, best, second, count in gaps:
            print(f"  {name:32} {best[2].replace('_', ' '):12} {second[2].replace('_', ' '):12} {m:+7.1%}   "
                  f"{count}  ({best[0]:.0f} vs {second[0]:.0f})")
        return 0
    if a.weapons_for:
        stats = parse_stats(a.weapons_for)
        sl = a.sl or sum(stats.values()) - 53  # every DS2 class satisfies level = stat total - 53
        corpus, _ = load_corpus(data)
        floors, r1, cut = build_floors(data, corpus, sl)
        bad = floor_violations(stats, floors)
        if bad and not a.neighbours:  # never rank for a build that is not a valid one; --neighbours
            # only reports what real builds near these stats carry, so a real build below a floor may query it
            print(f"not a valid SL {sl} build: {', '.join(bad)} (VGR/VIT/ADP/ATT floor at the bracket median; "
                  "use --optimize to get one)", file=sys.stderr)
            return 2
        if a.neighbours:
            status = a.status.split(",") if a.status else None
            rows, n, (lo, hi) = neighbour_weapons(data, stats, sl, corpus, k=a.k, status=status)
            if status:
                print(f"only weapon+infusions that deal {' or '.join(status)}")
            print(f"weapons of the {n} nearest-stat builds at SL {sl} (stat-bracket distance {lo}-{hi}), usable only")
            for c, name, inf, grip in rows:
                print(f"  {c:3}/{n}  {name:32} {grip:8} " + ", ".join(f"{i.replace('_', ' ')} {m}" for i, m in inf))
            return 0
        rows, dfn, n = weapons_for(data, stats, sl, corpus, raw_ar=a.raw_ar, one_hand=a.one_hand, weapon_class=a.weapon_class, per_class=a.per_class, window=a.window, objective=a.objective)
        print(f"stats {' '.join(f'{s[:3].upper()} {v}' for s, v in stats.items())}  ->  SL {sl}")
        print(f"average defender at this SL ({n} builds): "
              + " ".join(f"{k} {v:.0f}" for k, v in dfn.items()))
        what = {"damage": "dmg"}.get(a.objective, a.objective)
        head = "total AR" if a.raw_ar and a.objective == "damage" else f"{what}/{a.window:g}s" if a.window else (
            "damage" if a.objective == "damage" else f"{what}/atk")
        print(f"\n  {'weapon (infusion)':44} {head:>8}   AR by type                         grip")
        for dmg, name, inf, ar, grip in rows:
            print(f"  {name + ' (' + inf.replace('_', ' ') + ')':44} {dmg:7.0f}   "
                  f"{' '.join(f'{k[:4]} {v}' for k, v in ar.items()):34} {grip}")
        return 0
    if a.minimum:
        weapon = data.sp_key.get(norm(a.minimum), a.minimum)
        w = data.weapons[weapon]
        print(f"{w['name']}: requires {w.get('require')}, weight {w['weight']}; AGL floor {a.agl or 'none'}; "
              f"weapon + armor + rings under {EQUIP_CAP:.0%} load; naked/empty slots allowed")
        corpus, _ = load_corpus(data)
        # A STR requirement is always met cheaper two-handed (the game halves it), so that is the
        # only grip shown for it. Weapons without STR show one-handed.
        two = bool((w.get("require") or {}).get("strength"))
        m = recommended_minimum(data, corpus, weapon, two, a.agl, a.sl)
        r1, cut, high = m["r1"], m["cut"], m["high"]
        print(f"  R1 stamina {r1.get(weapon, '?')} vs high-end cut {cut:.1f} -> Endurance "
              f"{'floored to bracket median' if high else 'left at class base'}; VGR/VIT/ADP/ATT floored to "
              f"the corpus median of the build's SL bracket")
        hard, typ, target_sl = m["hard"], m["typ"], m["target_sl"]
        grip = "two-handed" if two else "one-handed"
        sl0, cls, (_, stats, arm, rs, load, mul) = hard[0]
        bonus, left, ranges, presets = m["bonus"], m["left"], m["ranges"], m["presets"]
        if left < 0:
            print(f"\n  SL {target_sl} is below the hard minimum SL {sl0}; nothing is wieldable")
            return 0
        # The recommended default: every free stat at its bracket median ("smack dab in the middle"),
        # requirement stats at their minimum, on the cheapest class and gear.
        mid = presets[0]
        st = mid["stats"]
        eff = {s: st[s] + bonus.get(s, 0) for s in STATS}
        gear = [p for p in arm if p != "Naked"] + rs
        print(f"\n  RECOMMENDED MINIMUM: {cls.capitalize()} SL {target_sl}, {grip}"
              + (f"  (unspent {mid['unspent']})" if mid["unspent"] else ""))
        print("    levelled : " + " ".join(f"{s[:3].upper()} {st[s]}" for s in STATS))
        print("    with gear: " + " ".join(f"{s[:3].upper()} {eff[s]}" for s in STATS))
        print(f"    gear: {', '.join(gear) or 'none'}; other armor slots empty")
        print(f"    AGL {mid['agl']}, load {mid['load_pct']:.1f}%, {mid['armor_budget_70']:.1f} weight left under 70%")
        print(f"\n  {grip}: hard minimum (requirements + 70% load only) and corpus-typical SL per class")
        for sl, c, (_, _st, a_, r_, _l, _m) in hard[:4]:
            g = [p for p in a_ if p != "Naked"] + r_
            print(f"    {c:9} hard SL {sl:3}  typical SL {typ[c]:3}   gear: {', '.join(g) or 'none'}")
        print(f"\n  {cls}, clamped at SL {target_sl} (hard minimum {sl0}): {left} free points")
        fixed = {s: stats[s] for s in STATS if s not in FREE_STATS}
        print("    fixed by requirements: " + " ".join(f"{s[:3].upper()} {v}" for s, v in fixed.items()))
        print("    slider ranges (others at minimum): "
              + "  ".join(f"{s[:3].upper()} {lo}-{hi}" for s, (lo, hi) in ranges.items()))
        print(f"    {"option":28} " + " ".join(f"{s[:3].upper():>4}" for s in FREE_STATS)
              + "   AGL  slots  load%  armor budget@70%")
        for p in presets:
            st = p["stats"]
            print(f"    {p["name"]:28} " + " ".join(f"{st[s]:4}" for s in FREE_STATS)
                  + f"   {p['agl']:3}  {p['slots']:5}  {p['load_pct']:5.1f}  {p['armor_budget_70']:6.1f}"
                  + (f"  ({p['unspent']} unspent, capped)" if p["unspent"] else ""))
        if a.json:
            print(json.dumps({"weapon": weapon, "class": cls, "grip": grip, "sl": target_sl, "hard_sl": sl0,
                              "free_points": left, "fixed": fixed, "ranges": ranges, "presets": presets,
                              "gear": {"armor": arm, "rings": rs}}))
        return 0
    corpus, why = load_corpus(data)
    print("corpus:", ", ".join(f"{k} {v}" for k, v in why.most_common()))

    if a.eval:
        for kind, (n, e, p) in sorted(evaluate(None, corpus, data, lam=a.lam).items()):
            print(f"  {kind:3} held-out {n:5}  recall@10  EASE {e:.3f}   popularity {p:.3f}")
        return 0

    model = Model(data, corpus, a.lam)
    if a.mugen:
        b = mugen_build(data, a.mugen)
    else:
        raw = json.loads((CACHE / "builds-darksouls2.json").read_text())[str(a.soulsplanner)]
        b = sp_build(raw, label=f"SoulsPlanner {a.soulsplanner}")
    eff, out = recommend(model, b)
    print(f"\n{b.label}")
    print("  effective stats:", " ".join(f"{s[:3].upper()} {eff[s]}" for s in STATS))
    print("  has:", ", ".join(pretty(t, data) for t in sorted(tokens(b))))
    if a.mugen:  # display only: inferred infusions do not enter the item model (see --infusion-eval)
        for w, f, p in infer_mugen_infusions(data, corpus, [b])[0]:
            print(f"  inferred infusion: {data.weapons[w]['name']} -> {f.replace('_', ' ')} (p {p:.2f})")
    for bucket, rows in out.items():
        print(f"\n  {bucket}s  (score = EASE + stat-neighbours + scaling fit)")
        for t, sc, e, nb, f in rows:
            print(f"    {pretty(t, data):48} {sc:6.2f}   ease {e:6.3f}  nbr {nb:5.2f}  fit {f:5.2f}")
    mix = threat_mix(data, corpus)
    cap, carried, sets = best_armor(data, b, eff, mix)
    print("\n  armor  threat mix:", " ".join(f"{k} {v:.0%}" for k, v in mix.items()))
    print(f"         70% load cap {cap:.1f}, weapons+rings carry {carried:.1f}")
    for val, w, pieces in sets:
        names = [data.armor[s][p]["name"] for s, p in zip(ARMOR_SLOTS, pieces)]
        print(f"    {val:7.1f} def-score  {w:5.1f} wt   " + " / ".join(names))
    return 0


if __name__ == "__main__":
    sys.exit(main())
