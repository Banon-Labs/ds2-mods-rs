#!/usr/bin/env python3
"""Recommend weapons, rings, spells and armor for a DARK SOULS II PvP build.

    python3 scripts/ds2-builds-recommend.py --mugen 33402
    python3 scripts/ds2-builds-recommend.py --soulsplanner 23874
    python3 scripts/ds2-builds-recommend.py --eval

Design: docs/DS2-BUILD-EMBEDDINGS.md. This is the first working cut, and it deviates from that
design in ways that are said out loud here rather than hidden:

* MOST ITEM DATA IS THE PLANNER SITES', NOT THE GAME'S. Weapon/armor requirements, armor defenses
  and weights come from SoulsPlanner's `ds2planner.min.js`, dumped to JSON by `--sp-data` /
  `--mm-data` (see `dump_site_tables`) with MugenMonkey's tables. The regulation replaces the
  physical stat defense table, max HP, every infusion's attack and scaling (shields aside), bleed
  and poison, and the spells' requirements and slots, and alone supplies catalysts, spell hits,
  weapon buffs and weapon elements (apply_regulation).
* ARMOR IS SCORED PER POINT OF DEFENSE. A piece's value is its per-type defense weighted by what
  one point of that defense takes off one hit of the average opponent (threat_mix): the
  executable's per-hit formula, over every unique build on both mirrors that can hit, with
  MugenMonkey infusions inferred, spells and weapon buffs included. How often a build that can
  both swing and cast does each is not in any build (SPELL_HIT_SHARE).
* CORPUS IS BOTH MIRRORS: SoulsPlanner (~/.cache/soulsplanner, 6618 builds) and MugenMonkey
  (~/.cache/mugenmonkey, 66052 public builds), filtered to complete, usable, unique builds.
* Ring and armor stat bonuses are counted: a ring's from the game's SpEffectRing.emevd (ring_gear),
  except in the corpus's defense average, which reads MugenMonkey's ring table (ring_effects). A
  ring's attack and defense effects are not.

Pipeline: filter the corpus (complete, not untouched class stats, every item usable) -> dedupe ->
fit EASE over {weapon, weapon|infusion, ring, spell} tokens -> for the query, score candidates by
EASE + stat-neighbour frequency + scaling fit, drop anything the build cannot use, rank per slot.
Armor is chosen separately by the defense optimizer under a 70% equip-load cap.
"""

from __future__ import annotations

import argparse
import bisect
import hashlib
import itertools
import json
import math
import os
import re
import subprocess
import sys
import time
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
        # Filled from the regulation by apply_regulation (regulation_spells, regulation_catalysts);
        # empty when it is unreadable or --site-numbers keeps the site's, and then no build is
        # recommended a catalyst.
        self.spell_category = {}  # spell key -> SpellParam.spellCategory
        self.catalysts = {}  # weapon key -> see regulation_catalysts
        self.cast_bonus = {}  # element -> PhysicalStatsPerLevelStatValuesParam column, rows 0-99
        self.spell_hits = {}  # spell key -> the hits one cast deals (regulation_spell_hits)
        self.buffs = {}  # spell key -> the weapon buff it casts (regulation_buffs)
        self.weapon_elements = {}  # weapon key -> element -> (base, coefficient) (regulation_weapon_elements)
        self.hit_flat = {}  # PlayerDamageParam row -> DMG key -> flat attack (regulation_hit_flat)
        self.damage_scale = {}  # weapon key -> WeaponParam.damageScale where not 1.0 (regulation_damage_scale)
        self.hit_shape = {}  # PlayerDamageParam row -> its hitbox shape (regulation_reach)
        self.reach_scale = {}  # weapon key -> (radius scale, length scale) where not 1.0 (regulation_reach)
        self.hit_poise = {}  # PlayerDamageParam row -> (poiseDamage, armorBreak) (regulation_poise)
        self.weapon_poise = {}  # weapon key -> (poiseDamageScalePlayer, uninterruptibleRate) (regulation_poise)
        self.ring_attack = {}  # ring key -> DMG key -> flat attack add (regulation_ring_attack)
        self.ranged = {}  # launcher key -> its ammo type, hand scale and special shots (regulation_ranged)
        self.ammo = {}  # ammo name -> its arrowType and the shot it adds (regulation_ranged)
        self.ring_defense = {}  # ring key -> DMG key -> defense add (regulation_ring_defense)
        self.defense_buffs = {}  # name -> a defender's damage-cutting buff (regulation_defense_buffs)
        # What a poison or bleed proc does, the stat resistance columns and every armour piece's
        # status resistance (regulation_status_procs); empty when the regulation is not read, and
        # then no row carries status damage.
        self.status_procs = {}
        self.status_resist = {}
        self.armor_status = {}
        # PhysicalStatsPerLevelStatValuesParam.staminaMax by END, rows 0-99: what a Dragon ring's
        # stamina factor is weighed against (ring_lift); empty when the regulation is not read.
        self.stamina_max = []
        self.stamina_cost = {}  # weapon key -> base costs and cost-category row (regulation_stamina)
        # PhysicalStatsPerLevelStatValuesParam.hpMax and additionalHp, rows 0-99 (row 0 unread): the
        # max-HP formula's two columns (hit_points); empty when the regulation is not read.
        self.hp_max = []
        self.additional_hp = []
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


def slots_of(data: Data, eff: dict, rings=()) -> int:
    """The attunement slots `eff`'s ATT gives, plus the worn `rings`' (ring_gear)."""
    return data.att_slots[min(eff["attunement"], len(data.att_slots) - 1)] + ring_slots(data, rings)


#: mechanics.json's names for the stats a ring's op 1000 effect adds to.
GEAR_STATS = {"vigor", "endurance", "vitality", "strength", "dexterity", "adaptability", "intelligence", "faith"}


def ring_gear(data: Data) -> dict:
    """SoulsPlanner ring key -> what wearing it does in place of stat points, from the GAME:
    mechanics.json `rings`, decoded from SpEffectRing.emevd op 1000 (event id = item id;
    docs/DS2-BUILD-MECHANICS.md section 2), joined by normalized name. `add` {stat: n}: a flat stat
    bonus (Strength, Dexterity, Knowledge, Prayer, Simpleton's); `scaled` [(stat, low, high,
    bonus at or below low, bonus at or above high)]: Ring of the Embedded's VGR/END/VIT bonus by
    the stat's own value (scaled_bonus); `slots`: attunement slots added (Southern Ritual Band
    +1/+2/+3); `hp`, `load`, `stamina`: max HP, max equip load and max stamina factors (Life Ring,
    Blue Seal, Northern Ritual Band, Royal Soldier's Ring, the Dragon rings). How two factors on
    one quantity combine is not established (mechanics.json `multipliers_source`); they are
    multiplied, as SoulsPlanner does. Poise is left out: nothing here floors it. A ring with none
    of these is not in the table. Cached on `data`."""
    gear = getattr(data, "_ring_gear", None)
    if gear is not None:
        return gear
    gear = {}
    rings = json.loads(MECHANICS.read_text())["rings"] if MECHANICS.exists() else []
    for r in rings:
        key = data.sp_key.get(norm(r["name"]))
        if key not in data.rings:
            continue
        e = {"add": {}, "scaled": [], "slots": 0, "hp": 1.0, "load": 1.0, "stamina": 1.0}
        for x in r["effects"]:
            if x["kind"] == "add" and x["stat"] in GEAR_STATS:
                e["add"][x["stat"]] = e["add"].get(x["stat"], 0) + int(x["value"])
            elif x["kind"] == "add" and x["stat"] == "attunement_slots":
                e["slots"] += int(x["value"])
            elif x["kind"] == "scaled_add" and x["stat"] in GEAR_STATS:
                e["scaled"].append((x["stat"], int(x["low_stat"]), int(x["high_stat"]),
                                    int(x["bonus_at_or_below_low"]), int(x["bonus_at_or_above_high"])))
            elif x["kind"] == "mul" and x["stat"] in ("max_hp", "max_equip_load", "max_stamina"):
                what = {"max_hp": "hp", "max_equip_load": "load", "max_stamina": "stamina"}[x["stat"]]
                e[what] *= float(x["value"])
        if e["add"] or e["scaled"] or e["slots"] or e["hp"] != 1.0 or e["load"] != 1.0 or e["stamina"] != 1.0:
            gear[key] = e
    data._ring_gear = gear
    return gear


def scaled_bonus(s: int, low: int, high: int, at_low: int, at_high: int) -> int:
    """A scaled ring bonus at stat value `s` (docs/DS2-BUILD-MECHANICS.md section 2; the byte
    values are the game's, the interpolation is inferred and reproduces SoulsPlanner's Embedded
    table): `at_low` at or below `low`, `at_high` at or above `high`, floored linear between."""
    if s <= low:
        return at_low
    if s >= high:
        return at_high
    return at_low + (at_high - at_low) * (s - low) // (high - low)


def gear_stats(data: Data, st: dict, rings=()) -> dict:
    """`st` with the worn `rings`' stat bonuses (ring_gear `add` and `scaled`, a scaled bonus read
    at the stat `st` gives), each stat a ring raised capped at 99. `st` itself when no ring adds a
    stat."""
    gear = ring_gear(data)
    out = None
    for r in rings:
        e = gear.get(r)
        if e is None or not (e["add"] or e["scaled"]):
            continue
        if out is None:
            out = dict(st)
        for s, v in e["add"].items():
            out[s] = min(99, out[s] + v)
        for s, lo, hi, at_lo, at_hi in e["scaled"]:
            out[s] = min(99, out[s] + scaled_bonus(st[s], lo, hi, at_lo, at_hi))
    return st if out is None else out


def ring_slots(data: Data, rings=()) -> int:
    gear = ring_gear(data)
    return sum(gear[r]["slots"] for r in rings if r in gear)


def ring_factor(data: Data, rings, what: str) -> float:
    """The product of the worn `rings`' `what` ("hp", "load", "stamina") factors, in ring order."""
    gear = ring_gear(data)
    f = 1.0
    for r in rings:
        if r in gear:
            f *= gear[r][what]
    return f


def spell_floors(data: Data, spells, extra_slots: int = 0) -> dict | None:
    """The least each stat may be for a build to attune and cast `spells` (keys; a repeat is a
    second copy, which costs its slots again): each requirement stat at the highest any of them
    needs, and ATT at the least whose attunement slots, plus `extra_slots` from worn rings, hold
    their summed slot cost. None when no ATT holds them."""
    need = {}
    for s in spells:
        for stat, v in data.spell_req[s].items():
            need[stat] = max(need.get(stat, 0), v)
    cost = sum(data.spells[s]["slots"] for s in spells)
    att = next((a for a, n in enumerate(data.att_slots) if n + extra_slots >= cost), None)
    if att is None:
        return None
    need["attunement"] = att
    return need


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
    # The worn rings count as the game counts them: a stat ring's bonus toward every requirement,
    # a Southern Ritual Band's slots toward the spells (ring_gear).
    eff = gear_stats(data, effective(data, b), b.rings)
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
        if sum(data.spells[s]["slots"] for s in b.spells) > slots_of(data, eff, b.rings):
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

#: The share of an opponent's hits that are spells, for a build that can both swing and cast
#: (one that can only do one lands only that). Not measured: no build records how often its owner
#: casts rather than swings, and only fight data can say. threat_weights takes it as an argument
#: so its effect can be shown at 0 and 1 as well.
SPELL_HIT_SHARE = 0.5
#: The hand slot a build's melee hits come from: the first usable melee weapon in this order.
MELEE_ORDER = ["rh1", "rh2", "rh3", "lh1", "lh2", "lh3"]
_threat_cache: dict = {}


def spell_attack(power: float, hit: dict) -> float:
    """A spell hit's attack rating in its element, the number damageRate multiplies: the casting
    catalyst's cast power in that element (cast_power) plus the hit's flat damage, which on a
    child bullet's hit is still the spell row's (inherited_hits). EXE
    (docs/DS2-DPS-MECHANICS.md "Spell and buff attack"): the per-hand attack builder 0x140391fe0
    prepares the bullet's attack as ((stat bonus + base) x rate + damage of that type) x k per
    type, with the catalyst's rate masked to the types the spell's damage row lists; k (stat
    penalty, WeaponTypeParam right damage scale, ChrParam damageAdjustRate) is 1 for a catalyst
    whose requirements are met, and every catalyst's damageScale is 1.0 (REGULATION)."""
    return power + hit["flat"]


def buff_attack(data: Data, buff: dict, weapon: str, stats: dict) -> float:
    """The attack rating a weapon buff adds to `weapon` in its element. EXE (same builder): the
    buff's 100080[1] adds scale/100 to the weapon's rate in that element and its 100090[2] adds
    flat to that element's stat bonus, so the weapon gains (its own elemental base + coefficient x
    stat bonus + flat) x scale / 100 -- WeaponReinforceParam maximum<Elem> and
    WeaponStatsAffectParam coefficient of the weapon's uninfused row (weapon_elements), not
    anything of the catalyst's. For an infused weapon the infused row's base and coefficient would
    apply; the uninfused row stands in for them."""
    base, coef = data.weapon_elements.get(weapon, {}).get(buff["type"], (0.0, 0.0))
    return (base + coef * element_bonus(data, buff["type"], stats) + buff["flat"]) * buff["scale"] / 100


def threat_opponents(data: Data, corpus: list[Build]) -> list[tuple[dict, list[dict]]]:
    """Every unique build on both mirrors that can hit (a usable melee weapon, or an attack spell
    it can cast with a catalyst it carries and can wield), as (melee attack rating by type, spell
    hits it can cast). Unique: one of each (tokens, stats), as load_corpus dedupes. Unlike the
    recommender's corpus, an incomplete armour set or an empty ring slot does not drop a build:
    what it wears does not change what it hits with.

    MugenMonkey records no infusion, so a MugenMonkey weapon's attack rating is the mean over the
    InfusionModel's probabilities (fitted on `corpus`'s SoulsPlanner builds; 68.3% right when
    measured, docs/DS2-BUILD-EMBEDDINGS.md). The mean rather than the most likely infusion: most of
    the model's misses are an elemental infusion guessed as No_Infusion, and its top guess alone
    would carry that bias into every MugenMonkey weapon. A buff the build can cast is on its melee
    weapon (the strongest one, when it can cast several).

    A catalyst casts at its infusion's rates (regulation_catalysts). The model covers no catalyst,
    so a MugenMonkey catalyst's cast power is the mean over the infusions SoulsPlanner builds
    record for that catalyst, weighted by how often (standard when none carries it)."""
    raw, seen = [], set()
    for _, b in raw_builds(data, True):
        sig = (frozenset(tokens(b)), tuple(b.stats[s] for s in STATS))
        if sig not in seen:
            seen.add(sig)
            raw.append(b)
    mm = [b for b in raw if any(inf == "?" for _, inf in b.hands)]
    model = InfusionModel(infusion_cases(data, corpus))
    dist = {}
    for c in infusion_cases(data, corpus, recorded=False, builds=mm):
        dist[(id(mm[c["build"]]), c["weapon"])] = {f: p for p, f in model.predict(c)}
    cat_seen: dict[str, Counter] = {}
    for b in raw:
        for w, inf in dict.fromkeys(b.weapons()):
            if w in data.catalysts and inf != "?" and inf in data.catalysts[w]["infused"]:
                cat_seen.setdefault(w, Counter())[inf] += 1
    cat_dist = {w: {f: n / sum(c.values()) for f, n in c.items()} for w, c in cat_seen.items()}
    out = []
    for b in raw:
        try:
            eff = gear_stats(data, effective(data, b), [r for r in b.rings if r in data.rings])
        except KeyError:
            continue
        melee, main = {}, None
        for slot in MELEE_ORDER:
            w, inf = b.hands[HAND_SLOTS.index(slot)]
            if (w in EMPTY or w not in data.weapons or data.weapons[w].get("isShield") or CATALYST.search(w)
                    or not weapon_ok(data, w, eff, two_hand=(b.grip == 1 and slot == "rh1"))):
                continue
            infs = data.weapons[w]["infusions"]
            ps = ({inf: 1.0} if inf != "?" else dist.get((id(b), w))
                  or {("No_Infusion" if "No_Infusion" in infs else next(iter(infs))): 1.0})
            for f, p in ps.items():
                for k, v in attack_rating(data, w, f, eff).items():
                    melee[k] = melee.get(k, 0.0) + p * v
            main = w
            break
        power = {}  # (spell category, element) -> the most cast power among wieldable carried catalysts
        for c, inf in dict.fromkeys((w, inf) for w, inf in b.weapons() if w in data.catalysts):
            cat = data.catalysts[c]
            if any(eff[s] < v for s, v in cat["require"].items()):
                continue
            ps = cat_dist.get(c, {"No_Infusion": 1.0}) if inf == "?" else {inf: 1.0}
            for k in cat["categories"]:
                for e in ELEMENTS:
                    p = sum(q * cast_power(data, c, e, eff, f) for f, q in ps.items())
                    power[(k, e)] = max(power.get((k, e), 0.0), p)
        castable = [s for s in b.spells if s in data.spell_category and spell_ok(data, s, eff)
                    and any(k == data.spell_category[s] for k, _ in power)]
        spells = []  # per castable attack spell: its biggest hit's damageRate and attack per type
        for s in castable:
            if s in data.spell_hits:
                top = max(data.spell_hits[s], key=lambda h: h["rate"])
                k = data.spell_category[s]
                spells.append({"rate": top["rate"], "attack": {
                    h["type"]: spell_attack(power.get((k, h["type"]), 0.0), h)
                    for h in data.spell_hits[s] if h["row"] == top["row"]}})
        buffs = [(data.buffs[s]["type"], buff_attack(data, data.buffs[s], main, eff))
                 for s in castable if s in data.buffs] if melee else []
        if buffs:
            k, v = max(buffs, key=lambda t: t[1])
            melee[k] = melee.get(k, 0.0) + v
        if melee or spells:
            out.append((melee, spells, data.damage_scale.get(main, 1.0)))
    return out


def threat_sums(opponents: list[tuple[dict, list[dict], float]]) -> dict:
    """What one point of each defense takes off one hit, summed over `opponents` in four groups:
    "melee" (builds that only swing), "spell" (only cast), and "both_melee" / "both_spell" (the
    swing and the cast of builds that do both). EXE (docs/DS2-DPS-MECHANICS.md "Damage per hit
    against a player"): a hit deals damageRate x max(attack x 10 - physical DEF, floor) / 12
    physical and damageRate x attack x (1 - (D + 100) / 1000) of an element, so a physical point is
    worth damageRate / 12 on any hit that carries physical attack and an element's point damageRate
    x attack / 1000. Floors and the 0.99 cap are left out (the linear range). A melee hit is an R1
    (damageRate 1.0, the median one-handed R1), times its weapon's WeaponParam.damageScale
    (threat_opponents' third field; regulation_damage_scale); a spell hit is the spell's biggest
    hit, each castable attack spell as likely."""
    sums = {g: {k: 0.0 for k in DMG} for g in ("melee", "spell", "both_melee", "both_spell")}
    for melee, spells, scale in opponents:
        if melee:
            g = sums["both_melee" if spells else "melee"]
            g["physical"] += scale / 12 if melee.get("physical", 0) > 0 else 0
            for e in ELEMENTS:
                g[e] += scale * melee.get(e, 0) / 1000
        for sp in spells:
            g = sums["both_spell" if melee else "spell"]
            for t, attack in sp["attack"].items():
                g[t] += sp["rate"] * (1 / 12 if t == "physical" else attack / 1000) / len(spells)
    return sums


def threat_weights(sums: dict, spell_share: float) -> dict:
    """threat_sums as per-type shares that sum to 1, when builds that both swing and cast land
    `spell_share` of their hits as spells."""
    tot = {k: sums["melee"][k] + sums["spell"][k] + (1 - spell_share) * sums["both_melee"][k]
           + spell_share * sums["both_spell"][k] for k in DMG}
    s = sum(tot.values()) or 1
    return {k: tot[k] / s for k in DMG}


THREAT_CACHE = Path.home() / ".cache/ds2-builds/threat-sums.json"


def threat_key(data: Data, corpus: list[Build]) -> str:
    """What threat_sums depends on: this script and the ones it loads, the mirrors, the item and
    regulation data it read, and the corpus the infusion model is fitted on."""
    h = hashlib.sha256()
    here = Path(__file__).parent
    for f in ("ds2-builds-recommend.py", "ds2-attacks-extract.py", "ds2-emevd.py", "ds2-regulation.py"):
        h.update((here / f).read_bytes())
    for p in (CACHE / "builds-darksouls2.json", CACHE / "index-darksouls2.json",
              MUGEN_CACHE / "builds.jsonl", MUGEN_CACHE / "index.jsonl"):
        st = p.stat() if p.exists() else None
        h.update(repr((str(p), st and st.st_size, st and st.st_mtime_ns)).encode())
    h.update(json.dumps([data.sp, data.mm, data.spell_hits, data.buffs, data.catalysts, data.cast_bonus,
                         data.spell_category, data.weapon_elements], sort_keys=True, default=str).encode())
    h.update(repr([(b.cls, b.stats, b.hands, b.spells, b.rings, b.armor) for b in corpus]).encode())
    return h.hexdigest()


def threat_mix(data: Data, corpus: list[Build]) -> dict:
    """Per-type weight of one point of armour defense, for best_armor: threat_weights at
    SPELL_HIT_SHARE. threat_opponents costs about half a minute, so its sums are kept in
    THREAT_CACHE under threat_key and in memory for the run."""
    if id(corpus) not in _threat_cache:
        key, sums = threat_key(data, corpus), None
        try:
            cached = json.loads(THREAT_CACHE.read_text())
            sums = cached["sums"] if cached.get("key") == key else None
        except (OSError, ValueError, KeyError):
            pass
        if sums is None:
            sums = threat_sums(threat_opponents(data, corpus))
            try:
                THREAT_CACHE.write_text(json.dumps({"key": key, "sums": sums}))
            except OSError:
                pass
        _threat_cache[id(corpus)] = threat_weights(sums, SPELL_HIT_SHARE)
    return _threat_cache[id(corpus)]


def max_load(data: Data, eff: dict, rings=()) -> float:
    """Max equip load at `eff` wearing `rings`: equipLoadMax at VIT plus the rings' bonus (Ring of
    the Embedded), times the rings' load factor (ring_gear)."""
    vit = gear_stats(data, eff, rings)["vitality"]
    return data.equip_load[min(vit, len(data.equip_load) - 1)] * ring_factor(data, rings, "load")


def best_armor(data: Data, b: Build, eff: dict, mix: dict, top: int = 3, scarcity: float = 0.0):
    """The `top` armour sets `eff` can wear under EQUIP_CAP beside what `b` carries, best first by
    threat-weighted defense. `scarcity` (load_scarcity, 0..1) trades defense for weight: a set scores
    its defense less `scarcity` x LOAD_PRICE x the best set's own defense per weight x its weight, so
    a build that wields few weapons takes a lighter set of similar defense over the heaviest one.
    The rings `b` wears count (max_load, and their stat bonuses toward the armour's requirements)."""
    worn = [r for r in b.rings if r in data.rings]
    cap = max_load(data, eff, worn) * EQUIP_CAP
    eff = gear_stats(data, eff, worn)
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
    if scarcity > 0 and sets:
        price = sets[0][0] / sets[0][1] if sets[0][1] > 0 else 0.0
        sets.sort(key=lambda s_: (s_[0] - scarcity * LOAD_PRICE * price * s_[1], s_[0], s_[1], s_[2]), reverse=True)
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
    (SoulsPlanner encodes these as functions). SITE data, not yet checked against the game.
    Built once per `data`: build_defense asks for it per ring per corpus build, and rebuilding it
    each time was 88% of an optimize_build (measured 2026-09-30, cProfile, 68 s of 78)."""
    if getattr(data, "_ring_effects", None) is not None:
        return data._ring_effects
    stat_of ={"Strength": "strength", "Dexterity": "dexterity", "Intelligence": "intelligence",
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
    data._ring_effects = out
    return out


MECHANICS = Path.home() / ".cache/ds2-builds/mechanics.json"
FLOOR_STATS = ["vigor", "vitality", "adaptability", "attunement"]


#: Spells a build may attune and still be held to the no-spell floors (floor_stats), by key.
NO_FLOOR_SPELLS = {"Warmth"}


def floor_stats(spells) -> list[str]:
    """The FLOOR_STATS a build casting `spells` is held to: ATT only when it casts something
    other than NO_FLOOR_SPELLS. Without spells ATT buys nothing but agility, which ADP buys three
    times as fast (agility's x is 3*ADP + ATT), so the median ATT of real builds -- most attune
    something -- is no floor for one that does not. Measured 2026-09-30: SL 155's median ATT 6
    levelled +4 ATT onto no-spell builds from classes that start at ATT 2. Warmth alone counts as
    no spells: a melee build attunes it for the heal, and spell_floors already raises ATT to the
    slot it needs."""
    casts = any(s not in NO_FLOOR_SPELLS for s in spells)
    return FLOOR_STATS if casts else [s for s in FLOOR_STATS if s != "attunement"]


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


def apply_regulation(data: Data) -> str:
    """Replace SoulsPlanner numbers with the game's, where the executable and regulation were
    read (docs/DS2-BUILD-MECHANICS.md section 6, docs/DS2-DPS-MECHANICS.md "Attack rating"):

    * `physicalDEFBonus[sum]` becomes PhysicalStatsPerLevelStatValuesParam
      `row[trunc((END+VIT+STR+DEX)/4)].defense`, the index the stats builder 0x14038d790 uses.
      SoulsPlanner's table is off by one at 180 of 393 sums.
    * Max HP (hit_points) reads the same param's hpMax and additionalHp columns.
    * Every infusion's attack bases and coefficients (regulation_attack), bleed and poison
      (regulation_status), and the rest of the regulation_* readers below.

    Weapons join by normalized itemname.fmg name (WeaponParam id == ItemParam id == text id).
    Returns a one-line summary; SoulsPlanner's numbers stay when the game's files cannot be read."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("ds2attacks", Path(__file__).parent / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    try:
        d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
            "WeaponParam": "WEAPON_PARAM", "WeaponReinforceParam": "WEAPON_REINFORCE_PARAM",
            "WeaponStatsAffectParam": "WEAPON_STATS_AFFECT_PARAM",
            "PhysicalStatsPerLevelStatValuesParam": "PHYS_STATS_PER_LEVEL_STAT_PARAM",
            "SpellParam": "SPELL_PARAM", "WeaponTypeParam": "WEAPON_TYPE_PARAM",
            "PlayerDamageParam": "DAMAGE_PARAM", "SystemDamageParam": "DAMAGE_PARAM",
            "BulletParam": "BULLET_PARAM", "SystemBulletParam": "BULLET_PARAM", "ArrowParam": "ARROW_PARAM",
            "WeaponActionCategoryParam": "WEAPON_ACTION_CATEGORY_PARAM",
            "WeaponAttackMotionParam": "WEAPON_ATTACK_MOTION_PARAM", "DamageCtrlParam": "DAMAGE_CTRL_PARAM",
            "ArmorParam": "ARMOR_PARAM", "ItemParam": "ITEM_PARAM",
            "ArmorReinforceParam": "ARMOR_REINFORCE_PARAM",
            "WeaponStaminaCostParam": "WEAPON_STAMINA_COST_PARAM"})
        names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
        members = reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
        emevd = ex.load_module("ds2emevd", "ds2-emevd.py")
    except (OSError, SystemExit, KeyError) as e:
        return f"regulation unreadable ({e}); SoulsPlanner's numbers kept"
    rows = d["PhysicalStatsPerLevelStatValuesParam"]
    top = max(map(int, rows))
    old = data.sp["physicalDEFBonus"]
    new = [None if s < 4 else rows[str(min(top, s // 4))]["defense"] for s in range(len(old))]
    moved = sum(1 for a, b in zip(old, new) if b is not None and a != b)
    data.sp["physicalDEFBonus"] = new
    data.hp_max = [0] + [rows[str(v)]["hpMax"] for v in range(1, top + 1)]
    data.additional_hp = [0] + [rows[str(v)]["additionalHp"] for v in range(1, top + 1)]
    # First, so every join below finds the weapons it renames.
    renamed = regulation_weapon_names(data, d, names)
    return (f"regulation: physical stat defense from the game's table ({moved} sums moved); "
            "max HP from hpMax and every other stat's additionalHp; " + renamed + "; "
            + regulation_spells(data, d, names) + "; " + regulation_spell_hits(data, d, names) + "; "
            + regulation_hit_flat(data, d) + "; "
            + regulation_buffs(data, emevd, members, d, names) + "; " + regulation_ring_attack(data, emevd, members, names)
            + "; " + regulation_ring_defense(data, emevd, members, names)
            + "; " + regulation_defense_buffs(data, emevd, members, d, names)
            + "; " + regulation_weapon_elements(data, d, names)
            + "; " + regulation_damage_scale(data, d, names) + "; " + regulation_reach(data, d, names)
            + "; " + regulation_poise(data, d, names) + "; " + regulation_stamina(data, d, names)
            + "; " + regulation_status(data, d, names)
            + "; " + regulation_status_procs(data, d, emevd, members, names)
            + "; " + regulation_attack(data, d, names) + "; " + regulation_ranged(data, d, names))


def regulation_spells(data: Data, d: dict, names: dict) -> str:
    """Spell slot costs and INT/FTH requirements from SpellParam (equipSlotNum, intelligence,
    faith; SpellParam id == ItemParam id, joined by normalized itemname.fmg name), and attunement
    slots per ATT from PhysicalStatsPerLevelStatValuesParam.spellSlot (rows 1-99; the row the
    stats builder 0x14038d790 writes to PlayerParam+0x36, docs/DS2-ATTUNEMENT.md). Measured
    2026-09-28: all 106 SoulsPlanner spells and all 99 slot rows equal the site's numbers, so this
    changes no answer today; it makes the game the source. A spell with no SpellParam row keeps
    the site's numbers and is counted."""
    rows = d["PhysicalStatsPerLevelStatValuesParam"]
    data.att_slots = [0] + [rows[str(a)]["spellSlot"] for a in range(1, max(map(int, rows)) + 1)]
    data.stamina_max = [0] + [rows[str(a)]["staminaMax"] for a in range(1, max(map(int, rows)) + 1)]
    by_name = {}
    for sid, s in d["SpellParam"].items():
        by_name.setdefault(norm(names.get(sid, "")), s)
    moved, missing = 0, 0
    for key, row in data.spells.items():
        s = by_name.get(norm(row.get("name", key)))
        if s is None:
            missing += 1
            continue
        game = {"intelligence": s["intelligence"], "faith": s["faith"]}
        moved += game != data.spell_req.get(key) or s["equipSlotNum"] != row["slots"]
        data.spell_req[key] = game
        row["slots"] = s["equipSlotNum"]
        data.spell_category[key] = s["spellCategory"]
    return (f"spell slots and INT/FTH from SpellParam for {len(data.spells) - missing} spells ({moved} "
            f"differed from the site, {missing} unmatched), attunement slots from spellSlot; "
            + regulation_catalysts(data, d, names))


#: SpellParam.spellCategory -> (label, the WeaponTypeParam flag that lets a catalyst cast it, the
#: element its cast power is read in). The categories against MugenMonkey's spell types, measured
#: 2026-09-28 over every joined spell: 0 is its 31 sorceries, 1 its 27 miracles, 2 its 24
#: pyromancies, 3 and 4 its 23 hexes, split 12/11. EXE, read 2026-09-30: the cast check 0x140397a30
#: switches on spellCategory and returns the hand's WeaponTypeParam byte +0x0, +0x1, +0x2, +0x3 or
#: +0x53 for 0..4 (any other category: no cast), which are these five flags; the cast request and
#: the HUD's spell icon both go through it (docs/DS2-DPS-MECHANICS.md "Which catalyst casts it").
SPELL_SCHOOLS = {0: ("sorcery", "allowMagic", "magic"), 1: ("miracle", "allowMiracle", "lightning"),
                 2: ("pyromancy", "allowPyromancy", "fire"), 3: ("hex", "allowDarkMagic", "dark"),
                 4: ("hex", "allowDarkMiracle", "dark")}
#: element -> (WeaponReinforceParam base/rate field stem, WeaponStatsAffectParam field stem,
#: PhysicalStatsPerLevelStatValuesParam column).
CAST_ELEMENTS = {"magic": ("Magic", "magic", "magicAttack"), "fire": ("Fire", "fire", "flameAttack"),
                 "lightning": ("Thunder", "thunder", "lightningAttack"), "dark": ("Dark", "dark", "darkAttack")}
#: WeaponReinforceParam's rates, in the slot order the executable moves an infusion's add rate in.
RATE_FIELDS = ["physicalRate", "magicRate", "thunderRate", "fireRate", "darkRate", "poisonRate", "bleedingRate",
               "petrifactionRate", "curseRate"]
#: Infusion -> (the WeaponReinforceParam add rate it moves, the RATE_FIELDS slot it moves it into,
#: its WeaponStatsAffectParam row's offset from statsAffectId). EXE: the 10x3 byte table at
#: 0x1410c3e10 holds slot and offset per infusion index (INFUSIONS order), and 0x14034fe10 picks the
#: add rate (docs/DS2-DPS-MECHANICS.md "Base, rates and the sum").
INFUSION_MOVE = {"No_Infusion": (None, 0, 0), "Fire": ("addFireRate", 3, 3), "Magic": ("addMagicRate", 1, 1),
                 "Lightning": ("addThunderRate", 2, 2), "Dark": ("addDarkRate", 4, 4),
                 "Poison": ("addPoisonRate", 5, 5), "Bleed": ("addBleedRate", 6, 6),
                 "Raw": ("addPhysicalRateByCrude", 0, 7), "Enchanted": ("addPhysicalRateByEnchanted", 0, 8),
                 "Mundane": ("addPhysicalRateByAbyss", 0, 9)}


def infused_rates(r: dict, inf: str) -> list[float]:
    """A WeaponReinforceParam row's rates in percent, per RATE_FIELDS slot, after infusion `inf`
    moves its add rate. EXE, the attack builder's path (0x14034c580 -> 0x14034c760 -> 0x14034c5c0):
    the add rate times 1.0; the target slot gains it, up to 1000 (0x1410ad5f0); every other slot
    with a nonzero rate loses add / (how many such slots there are) (0x14034fed0); then every rate
    is floored at 0. A Magic Staff of Wisdom goes from magic 100 to 110, a Lightning one to magic
    90 and lightning 10."""
    rates = [r[f] for f in RATE_FIELDS]
    field, target, _ = INFUSION_MOVE[inf]
    add = r[field] if field else 0.0
    if add <= 0:
        return rates
    others = sum(1 for i, v in enumerate(rates) if i != target and v != 0)
    out = list(rates)
    if rates[target] < 1000:
        out[target] = min(rates[target] + add, 1000.0)
    for i, v in enumerate(rates):
        if i != target and v > 0:
            out[i] = v - add / others
    return [max(0.0, v) for v in out]


def regulation_catalysts(data: Data, d: dict, names: dict) -> str:
    """Every SoulsPlanner weapon the game lets cast a spell category, from the regulation: which
    categories (WeaponTypeParam allow* of its weaponTypeId), its requirements (WeaponParam
    required*), and per infusion and element its full-upgrade cast power terms -- base =
    WeaponReinforceParam maximum<Elem> x the infusion's WeaponStatsAffectParam baseValueScale x
    rate / 100, scale = that row's <elem><maxLevel> x rate / 100, rate as infused_rates moves it.
    "power" is the standard catalyst's, "infused" every infusion's. The per-stat bonus a scale
    multiplies is PhysicalStatsPerLevelStatValuesParam's magicAttack/flameAttack/lightningAttack/
    darkAttack column (data.cast_bonus); cast_power says which stat indexes it. Measured 2026-09-28
    against SoulsPlanner: all 32 catalysts' requirements equal its require; 30 have its uninfused
    atk/atkScale exactly, Olenford's Staff scales 0.678 where the site rounds to 0.675, and Sanctum
    Shield has no elemental row on the site at all. The magic, lightning and dark columns equal its
    tables at every stat, and fire equals its INT+FTH table at (INT+FTH)//2."""
    rows = d["PhysicalStatsPerLevelStatValuesParam"]
    top = max(map(int, rows))
    data.cast_bonus = {e: [0] + [rows[str(i)][col] for i in range(1, top + 1)]
                       for e, (_, _, col) in CAST_ELEMENTS.items()}
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    data.catalysts = {}
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        t = wp and d["WeaponTypeParam"].get(str(wp["weaponTypeId"]))
        cats = [c for c, (_, flag, _) in SPELL_SCHOOLS.items() if t and t.get(flag)]
        r = cats and d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
        a = r and d["WeaponStatsAffectParam"].get(str(r["statsAffectId"]))
        if not a:
            continue
        infused = {}
        for inf, (_, _, offset) in INFUSION_MOVE.items():
            row = d["WeaponStatsAffectParam"].get(str(r["statsAffectId"] + offset))
            if not row:
                continue
            rates = infused_rates(r, inf)
            infused[inf] = {}
            for e, (stem, sa, _) in CAST_ELEMENTS.items():
                rate = rates[RATE_FIELDS.index(f"{sa}Rate")] / 100
                infused[inf][e] = (r[f"maximum{stem}"] * row["baseValueScale"] * rate,
                                   row[f"{sa}{r['maxLevel']}"] * rate)
        req = {s: wp[f"required{s.capitalize()}"] for s in REQ_STATS if wp[f"required{s.capitalize()}"]}
        data.catalysts[key] = {"categories": cats, "require": req, "power": infused["No_Infusion"],
                               "infused": infused}
    return f"{len(data.catalysts)} catalysts from WeaponParam/WeaponTypeParam/WeaponReinforceParam"


def regulation_weapon_elements(data: Data, d: dict, names: dict) -> str:
    """data.weapon_elements: weapon key -> element -> (base, coefficient) of its uninfused row at
    full upgrade, before any rate: WeaponReinforceParam maximum<Elem> x WeaponStatsAffectParam
    baseValueScale, and WeaponStatsAffectParam <elem><maxLevel>. A standard weapon's elemental
    rates are 0, so these are what a weapon buff's rate multiplies (buff_attack). Every uninfused
    row's baseValueScale is 1.0 (measured 2026-09-29, 322 rows). Joined by name as
    regulation_catalysts joins."""
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    data.weapon_elements = {}
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        r = wp and d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
        a = r and d["WeaponStatsAffectParam"].get(str(r["statsAffectId"]))
        if a:
            data.weapon_elements[key] = {e: (r[f"maximum{stem}"] * a["baseValueScale"], a[f"{sa}{r['maxLevel']}"])
                                         for e, (stem, sa, _) in CAST_ELEMENTS.items()}
    return f"elemental base and scaling for {len(data.weapon_elements)} weapons"


def regulation_damage_scale(data: Data, d: dict, names: dict) -> str:
    """data.damage_scale: weapon key -> WeaponParam.damageScale, where it is not 1.0. EXE
    (docs/DS2-DPS-MECHANICS.md "What else a hit carries"): for every attack that is not a spell the
    attack builder stores 1.0 x WeaponParam +0x80 in the attack block (0x141c5ccfc, 0x141c5cd09),
    the block reaches the defender as sAr+0xb8, and calculateDamage_defense multiplies each type's
    damage by it after the defense (0x140138f8a). Joined by name as regulation_catalysts joins."""
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    data.damage_scale = {}
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        if wp and wp["damageScale"] != 1.0:
            data.damage_scale[key] = wp["damageScale"]
    return f"damageScale off 1.0 on {len(data.damage_scale)} weapons"


def regulation_reach(data: Data, d: dict, names: dict) -> str:
    """data.hit_shape: PlayerDamageParam row -> (hitDummyPolyType, hitDummyPolyId, radius, length,
    childDamage); data.reach_scale: weapon key -> (WeaponParam.damageHitRadiusScale,
    damageHitLengthScale) where either is not 1.0. REGULATION; the paramdef names radius and length
    in metres (半径[m], 長さ[m]). What r1_reach makes of them is INFERRED there.
    scripts/ds2-hit-shape.py prints them per weapon. Joined by name as regulation_damage_scale."""
    data.hit_shape = {rid: (r["hitDummyPolyType"], r["hitDummyPolyId"], r["radius"], r["length"], r["childDamage"])
                      for rid, r in d["PlayerDamageParam"].items()}
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    data.reach_scale = {}
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        if wp and (wp["damageHitRadiusScale"], wp["damageHitLengthScale"]) != (1.0, 1.0):
            data.reach_scale[key] = (wp["damageHitRadiusScale"], wp["damageHitLengthScale"])
    return f"hitbox shape on {len(data.hit_shape)} damage rows, reach scaled on {len(data.reach_scale)} weapons"


def regulation_poise(data: Data, d: dict, names: dict) -> str:
    """What poise_metrics reads (docs/DS2-DPS-MECHANICS.md section 2, "Poise and stagger"):

    * data.hit_poise: PlayerDamageParam row -> (DamageCtrlParam.poiseDamage, .armorBreak) of its
      damageCategory row. EXE: the base poise damage 0x140139160 reads the hit's DamageCtrlParam
      +0xa0 (poiseDamage); the stagger decision 0x140136570 reads +0x2 (armorBreak).
    * data.weapon_poise: weapon key -> (WeaponParam.poiseDamageScalePlayer, .uninterruptibleRate).
      EXE: calculateDamage_attack 0x1401373b0 stores the attacker's PlayerGameParamCalculator slot
      0x190 (0x140381120) in the hit block's +0x74, which 0x140139160 multiplies poiseDamage by;
      that slot returns the hand's WeaponParam +0xb0 (poiseDamageScalePlayer) when the target is a
      player, +0xb4 (poiseDamageScaleEnemy) otherwise. +0xb8 is hyperarmor's (hyperarmor()).
    * Each armour piece's site `poise` replaced by ArmorParam.strong (ItemParam.armorParamId, joined
      by name). EXE: max poise 0x14037fdd0 sums the four worn pieces' ArmorParam +0x40 (strong)."""
    ctrl = d["DamageCtrlParam"]
    data.hit_poise = {rid: (ctrl[str(r["damageCategory"])]["poiseDamage"], ctrl[str(r["damageCategory"])]["armorBreak"])
                      for rid, r in d["PlayerDamageParam"].items() if str(r["damageCategory"]) in ctrl}
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    data.weapon_poise = {}
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        if wp:
            data.weapon_poise[key] = (wp["poiseDamageScalePlayer"], wp["uninterruptibleRate"])
    strong = {}
    for iid, it in d["ItemParam"].items():
        ap = d["ArmorParam"].get(str(it.get("armorParamId")))
        if ap:
            strong.setdefault(norm(names.get(iid, "")), ap["strong"])
    moved = joined = 0
    for slot in ARMOR_SLOTS:
        for key, p in data.armor[slot].items():
            v = strong.get(norm(p.get("name", key))) if isinstance(p, dict) else None
            if v is None:
                continue
            joined += 1
            moved += p.get("poise") != v
            p["poise"] = v
    # Max poise's stat term (EXE 0x14037fdd0 + the stat block's +0x4c): the poise column of
    # PhysicalStatsPerLevelStatValuesParam (+0x90) at min(END, ADP).
    rows = d["PhysicalStatsPerLevelStatValuesParam"]
    data.stat_poise = [rows[str(v)]["poise"] if str(v) in rows else 0.0 for v in range(max(map(int, rows)) + 1)]
    return (f"poise on {len(data.hit_poise)} PlayerDamageParam rows and {len(data.weapon_poise)} weapons; "
            f"armour poise from ArmorParam.strong on {joined} pieces ({moved} differ from the site's); "
            f"stat poise from PhysicalStatsPerLevelStatValuesParam.poise")


def regulation_stamina(data: Data, d: dict, names: dict) -> str:
    """data.stamina_cost: weapon key -> {"melee": WeaponParam.meleeAttackBaseCost, "ranged":
    rangedAttackBaseCost, "rates": the WeaponStaminaCostParam row costCategoryId names}, joined by
    name as regulation_damage_scale joins. What attack_stamina and shot_stamina read; the cost
    formula and what is unproven about it are theirs (docs/DS2-DPS-MECHANICS.md "Stamina")."""
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    costs = d["WeaponStaminaCostParam"]
    data.stamina_cost = {}
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        rates = wp and costs.get(str(wp["costCategoryId"]))
        if rates:
            data.stamina_cost[key] = {"melee": wp["meleeAttackBaseCost"], "ranged": wp["rangedAttackBaseCost"],
                                      "rates": rates}
    return f"stamina cost for {len(data.stamina_cost)} weapons"


#: Status -> (RATE_FIELDS slot, WeaponReinforceParam maximum<...> stem, WeaponStatsAffectParam stem).
STATUS_TERMS = {"poison": (5, "Poison", "poison"), "bleed": (6, "Bleeding", "bleeding")}


def regulation_status(data: Data, d: dict, names: dict) -> str:
    """Every weapon infusion's poison and bleed terms from the regulation, in place of the site's,
    in gauge points per hit before the victim's resistance (a gauge procs at 100). EXE
    (docs/DS2-DPS-MECHANICS.md "Status build-up per hit"): the attack builder computes a status
    attack as it does a damage type's -- WeaponReinforceParam maximum<Status> x the infusion's
    WeaponStatsAffectParam baseValueScale, plus the status bonus x the row's <status><maxLevel>, all
    x the moved rate / 100 (infused_rates) -- and multiplies the block's status entries by 100 / X
    before the hit leaves (0x140299115), where X is the max-HP formula at every stat 99
    (0x14038b9a0 -> 0x14038d290): hpMax[99] + 8 x additionalHp[99], 2505. So `atk` holds the base
    term and `atkScale` the coefficient of the status bonus, both times 100 / X. The site's terms
    are not this (its base is the builder's / 5 and its coefficient / 2) and are dropped where the
    regulation has no row, as numbers in other units. Joined by name as regulation_catalysts joins."""
    rows = d["PhysicalStatsPerLevelStatValuesParam"]
    top = str(max(map(int, rows)))
    unit = 100 / (rows[top]["hpMax"] + 8 * rows[top]["additionalHp"])
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    done, dropped = 0, 0
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        r = wp and d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
        for inf, row in (w.get("infusions") or {}).items():
            atk, sc = row.setdefault("atk", {}), row.setdefault("atkScale", {})
            a = r and d["WeaponStatsAffectParam"].get(str(r["statsAffectId"] + INFUSION_MOVE[inf][2]))
            if not a:
                had = [atk.pop(s, None) for s in STATUS_TERMS] + [sc.pop(s, None) for s in STATUS_TERMS]
                dropped += any(had)
                continue
            rates = infused_rates(r, inf)
            for s, (slot, stem, sa) in STATUS_TERMS.items():
                base = r[f"maximum{stem}"] * a["baseValueScale"] * rates[slot] / 100 * unit
                coef = a[f"{sa}{r['maxLevel']}"] * rates[slot] / 100 * unit
                atk.pop(s, None), sc.pop(s, None)
                if base or coef:
                    atk[s], sc[s] = base, coef
            done += 1
    return (f"poison and bleed from the regulation for {done} weapon infusions, x 100/"
            f"{round(100 / unit)} ({dropped} with no row lost the site's)")


#: Status -> (the SpEffect applyStatusDamage (0x140145c60) applies when its gauge reaches 100,
#: PhysicalStatsPerLevelStatValuesParam resistance column, the stat added to 3 x ADP for its row,
#: ArmorReinforceParam column). EXE for the ids (case 0 -> 900100, case 1 -> 900200) and for the
#: rows: the stats builder 0x14038d784 stores row[trunc((3*ADP + FTH)/4)].bleedingResistance and
#: row[trunc((3*ADP + VIT)/4)].poisonResistance, each x 0.01, into its block.
STATUS_PROC = {"poison": (900100, "poisonResistance", "vitality", "maxPoison"),
               "bleed": (900200, "bleedingResistance", "faith", "maxBleeding")}
#: SpEffect instructions (REGULATION, SpEffectAbnormalState.emevd; `scripts/ds2-status-procs.py`
#: prints them). `100070[0] [1, 872, category, seconds f32, damage row, interval f32, ...]` puts a
#: character in a status category for `seconds`, hitting it with PlayerDamageParam `damage row`
#: every `interval` seconds when there is one; `100170[1] [damage row]` hits it once. The word
#: meanings are INFERRED from the rows: the category word is the byte applyStatusDamage looks the
#: status up by (0 poison at 0x14023dd00, 1 bleed at 0x14023dab0, 8 toxic at 0x14023de40 -- EXE) and
#: equals word 2 of 900100, 900210 and 900600; and 22 s / 0.3 s x 15 = 1095 and 200 are the
#: community's poison and bleed numbers (darksouls2.wiki.fextralife.com Poison, Bleed).
STATUS_PERIODIC, STATUS_ONCE = (100070, 0), (100170, 1)
#: An armour ItemParam id is its ArmorParam id plus this (crates/ds2-rva ARMOR_PARAM_ID_FROM_ITEM_ID;
#: scripts/ds2-infusion-evidence.py joins the same way).
ARMOR_ITEM_OFFSET = 10_000_000


def regulation_status_procs(data: Data, d: dict, emevd, members: dict, names: dict) -> str:
    """data.status_procs, data.status_resist and data.armor_status: what a poison or bleed proc
    does to a player and what resists it.

    | status | proc (REGULATION) | damage | locked out |
    |---|---|---|---|
    | poison | 900100 `100070[0] [1, 872, 0, 22.0, 9010, 0.3, ...]` | row 9010 (15) every 0.3 s for 22 s: 73 ticks, 1095 | 22 s |
    | bleed | 900200 `100170[1] [9020]` | row 9020 (200) once; its spEffectIdWhenHit 900210 is `100070[0] [.., 1, 10.0, ..]` | 10 s |

    Both damage rows are damageType01 5, the attack's slot 5, which calculateDamage_defense
    (0x140138d50) passes through untouched: no defense, cut, motion value or HP percentage (EXE). The
    lockout: applyStatusDamage skips the build-up altogether while the victim has an effect of the
    status's category (0x14014bc80 on the category's slot, called at 0x140145e51; that call's body
    is behind Arxan and was not read, so what it tests is INFERRED from its arguments). On a proc it
    sets the gauge to its floor (0x140146430 with 100.0 at 0x140145e61), so the procing hit's surplus
    is lost (EXE). Resistance: the stat columns of STATUS_PROC by row, and each armour piece's
    ArmorReinforceParam max column (the fully reinforced piece, as the defense model takes it),
    joined by name to the site's pieces: a piece's ItemParam id is its ArmorParam id + ARMOR_ITEM_OFFSET."""
    ev = {e.id: e for e in emevd.Emevd(Path("SpEffectAbnormalState.emevd"),
                                       members["SpEffectAbnormalState.emevd"]).events}
    dmg = d["PlayerDamageParam"]

    def ins(eid, op):
        return next((i.words() for i in (ev[eid].instructions if eid in ev else []) if (i.bank, i.index) == op), None)

    def flat(row):
        r = dmg.get(str(row))
        return r and next((r[f"damage0{n}"] for n in (1, 2, 3) if r[f"damageType0{n}"] == 5 and r[f"damage0{n}"]), 0)

    data.status_procs = {}
    for s, (eid, *_) in STATUS_PROC.items():
        per = ins(eid, STATUS_PERIODIC)
        once = ins(eid, STATUS_ONCE)
        if per and len(per) >= 6 and per[4][0] and per[5][1] > 0:  # a damage-over-time status
            ticks = int(per[3][1] / per[5][1] + 1e-6)
            tick = flat(per[4][0])
            data.status_procs[s] = {"damage": ticks * tick, "lockout": per[3][1], "ticks": ticks, "tick": tick,
                                    "interval": per[5][1], "row": per[4][0]}
        elif once:
            r = dmg.get(str(once[0][0])) or {}
            lock = ins(r.get("spEffectIdWhenHit"), STATUS_PERIODIC)
            data.status_procs[s] = {"damage": flat(once[0][0]), "lockout": lock[3][1] if lock else 0.0,
                                    "ticks": 1, "tick": flat(once[0][0]), "interval": 0.0, "row": once[0][0]}
    rows = d["PhysicalStatsPerLevelStatValuesParam"]
    top = max(map(int, rows))
    data.status_resist = {s: [0.0] + [rows[str(v)][col] for v in range(1, top + 1)]
                          for s, (_, col, _, _) in STATUS_PROC.items()}
    by_name = {}
    for aid, a in d["ArmorParam"].items():
        r = d["ArmorReinforceParam"].get(str(a["armorReinforceId"]))
        if r:
            by_name.setdefault(norm(names.get(str(int(aid) + ARMOR_ITEM_OFFSET), "")),
                               {s: r[c] for s, (_, _, _, c) in STATUS_PROC.items()})
    data.armor_status = {slot: {k: by_name[norm(p.get("name", k))] for k, p in data.armor[slot].items()
                                if norm(p.get("name", k)) in by_name} for slot in ARMOR_SLOTS}
    joined = sum(map(len, data.armor_status.values()))
    return ("status procs from SpEffectAbnormalState.emevd: " + ", ".join(
        f"{s} {p['damage']} ({p['lockout']:g} s)" for s, p in data.status_procs.items())
            + f"; status resistance for {joined} of {sum(map(len, data.armor.values()))} armour pieces")


#: Damage type -> (RATE_FIELDS slot, WeaponReinforceParam maximum<...> stem, atkScale key ->
#: WeaponStatsAffectParam stem). Physical scales off STR and DEX, each element off its own stem.
DAMAGE_TERMS = {"physical": (0, "Physical", {"strength": "physicalByStrength", "dexterity": "physicalByDexterity"}),
                "magic": (1, "Magic", {"magic": "magic"}), "lightning": (2, "Thunder", {"lightning": "thunder"}),
                "fire": (3, "Fire", {"fire": "fire"}), "dark": (4, "Dark", {"dark": "dark"})}
#: The atkScale keys regulation_attack owns; "modifier" (Mundane) and the status keys are not.
DAMAGE_SCALES = ["strength", "dexterity", "magic", "fire", "lightning", "dark"]


def infusion_attack(d: dict, r: dict, inf: str) -> tuple[dict, dict] | None:
    """The damage terms of WeaponReinforceParam row `r` infused with `inf`, keyed as the site keys
    them: `atk` type -> base, `atkScale` stat -> coefficient. None when the regulation has no
    WeaponStatsAffectParam row for the infusion. Enchanted's INT term is physicalByEnchant x the
    physical rate under "magic", which attack_rating adds to physical; Mundane has no STR or DEX
    term."""
    a = d["WeaponStatsAffectParam"].get(str(r["statsAffectId"] + INFUSION_MOVE[inf][2]))
    if not a:
        return None
    rates, lv = infused_rates(r, inf), r["maxLevel"]
    atk, sc = {}, {}
    for k, (slot, stem, stems) in DAMAGE_TERMS.items():
        rate = rates[slot] / 100
        if r[f"maximum{stem}"] * rate:
            atk[k] = r[f"maximum{stem}"] * a["baseValueScale"] * rate
        for s, sa in stems.items():
            if a[f"{sa}{lv}"] * rate:
                sc[s] = a[f"{sa}{lv}"] * rate
    if inf == "Enchanted":
        sc["magic"] = a[f"physicalByEnchant{lv}"] * rates[0] / 100
    if inf == "Mundane":
        # The scaling function's physical term for infusion 9 is the Mundane modifier alone, in
        # place of STR and DEX (docs/DS2-DPS-MECHANICS.md "Scaling function", 0x141bf101e).
        sc.pop("strength", None), sc.pop("dexterity", None)
    return atk, sc


def _terms_differ(game: tuple[dict, dict], site: tuple[dict, dict]) -> bool:
    """Whether the game's terms and the site's differ by more than the site's rounding: a base by
    1 or more (it keeps whole numbers), a coefficient by more than 0.011 (two decimals)."""
    (ga, gs), (sa, ss) = game, site
    return (any(abs(ga.get(k, 0) - sa.get(k, 0)) >= 1 for k in DAMAGE_TERMS)
            or any(abs(gs.get(k, 0) - ss.get(k, 0)) > 0.011 for k in DAMAGE_SCALES))


def regulation_weapon_names(data: Data, d: dict, names: dict) -> str:
    """The game's own name (itemname.fmg) for each weapon whose SoulsPlanner name joins no
    WeaponParam row but whose key does. Every regulation_* reader joins by name, so such a weapon
    kept the site's numbers: SoulsPlanner spells the Black Flamestone Dagger `Black Flamestone
    Dagge`, and the game `Black Flamestone Dagger`, its key's spelling. A name that joins is left as
    the site spells it, and one that joins by neither -- the site's `Santier's Spear (broken)`,
    which the game names `Santier's Spear` like the whole one -- keeps the site's."""
    by_name = {}
    for wid in d["WeaponParam"]:
        by_name.setdefault(norm(names.get(wid, "")), wid)
    renamed = []
    for key, w in data.weapons.items():
        name = w.get("name", key)
        # The site also names one weapon by its key, `Light_Crossbow`: it joins (norm drops the
        # underscore), but no menu or build says it that way, so the game's spelling replaces it.
        leaked = "_" in name and norm(name) in by_name and names[by_name[norm(name)]] != name
        if (norm(name) not in by_name and norm(key) in by_name) or leaked:
            w["name"] = names[by_name[norm(key) if norm(key) in by_name else norm(name)]]
            # Rows carry the name, and the ranking finds a row's weapon by it.
            data.key_by_name[w["name"]] = key
            renamed.append(f"{name} -> {w['name']}")
    return f"{len(renamed)} weapon names from the game's text" + (f" ({', '.join(renamed)})" if renamed else "")


def regulation_attack(data: Data, d: dict, names: dict) -> str:
    """Every weapon infusion's attack bases and coefficients from the regulation, in place of the
    site's (infusion_attack): base = WeaponReinforceParam maximum<Type> x the infusion's
    WeaponStatsAffectParam baseValueScale x the moved rate / 100, coefficient = that row's
    <stat><maxLevel> x the rate / 100. EXE (docs/DS2-DPS-MECHANICS.md "Base, rates and the sum"):
    the attack builder moves the rates as infused_rates does, and nothing else moves them; the rate
    move's two other callers build guard cut. Mundane's modifier stays the site's, which equals the
    game's for every weapon.

    Kept as the site has them: shields, which the site gives no attack (the game gives them a
    bash), and weapons with no regulation row by name. Where the uninfused rows disagree, each name
    has one WeaponReinforceParam row, so the site is the one off: Murakumo STR/DEX 0.07/0.51 against
    0.15/0.59, Foot Soldier Sword STR 0.55 against 0.35, and Ruler's Sword 1.32 x the regulation's
    scaling (its soul bonus is not in these numbers either way). An Enchanted row that also deals
    magic scales that magic by the INT term too, as attack_rating always has (one "magic" key).
    Joined by name as regulation_catalysts joins."""
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    done, moved, kept, both = 0, 0, 0, 0
    for key, w in data.weapons.items():
        wp = not w.get("isShield") and by_name.get(norm(w.get("name", key)))
        r = wp and d["WeaponReinforceParam"].get(str(wp["weaponReinforceId"]))
        for inf, row in (w.get("infusions") or {}).items():
            game = r and infusion_attack(d, r, inf)
            if not game:
                kept += 1
                continue
            atk, sc = row.setdefault("atk", {}), row.setdefault("atkScale", {})
            moved += _terms_differ(game, (atk, sc))
            both += inf == "Enchanted" and "magic" in game[0]
            for k in DAMAGE_TERMS:
                atk.pop(k, None)
            for k in DAMAGE_SCALES:
                sc.pop(k, None)
            atk.update(game[0])
            sc.update(game[1])
            # The moved rates themselves, what a ring's flat add is multiplied by (ring_attack_add).
            rates = infused_rates(r, inf)
            row["rate"] = {k: rates[slot] / 100 for k, (slot, _, _) in DAMAGE_TERMS.items() if rates[slot]}
            done += 1
    return (f"attack from the regulation for {done} weapon infusions ({moved} differ from the site's by more "
            f"than its rounding; {kept} kept the site's: shields and unjoined names; {both} Enchanted rows "
            f"also deal magic)")


def cast_power(data: Data, catalyst: str, element: str, stats: dict, inf: str = "No_Infusion") -> float:
    """A catalyst's full-upgrade cast power in `element` at `stats`, infused with `inf` (standard
    when the regulation has no row for it): base + scale x the element's per-stat bonus, the bonus
    indexed as SoulsPlanner's getMagicATK/... index it (SITE: magic by INT, lightning by FTH, dark
    by min(INT, FTH), fire by (INT+FTH)//2 -- the halving is where its INT+FTH fire table meets the
    game's 99-row column). The attack-rating shape attack_rating uses, and the executable's: a spell
    cast with this catalyst attacks with this plus its damage row's flat damage (spell_attack)."""
    cat = data.catalysts[catalyst]
    base, scale = cat.get("infused", {}).get(inf, cat["power"])[element]
    return base + scale * element_bonus(data, element, stats)


def element_bonus(data: Data, element: str, stats: dict) -> float:
    """The per-stat bonus an elemental scaling coefficient multiplies: data.cast_bonus[element] at
    INT (magic), FTH (lightning), min(INT, FTH) (dark) or (INT+FTH)//2 (fire), as cast_power says."""
    i, f = stats["intelligence"], stats["faith"]
    at = {"magic": i, "lightning": f, "dark": min(i, f), "fire": (i + f) // 2}[element]
    col = data.cast_bonus[element]
    return col[max(0, min(at, len(col) - 1))]


def best_catalysts(data: Data, spells, stats: dict) -> list[tuple[str, str, float, str | None]]:
    """For each spell category `spells` need, in category order, the catalyst with the most cast
    power in that category's element at `stats` among those that cast it and whose every
    requirement `stats` meet (in full: a catalyst is held one-handed), first in table order on a
    tie: (school label, catalyst key, power, passed over), where passed over is the catalyst that
    would have cast harder had its requirements been met, or None. A category no wieldable
    catalyst covers is left out."""
    out = []
    for c in sorted({data.spell_category[s] for s in spells if s in data.spell_category}):
        label, _, element = SPELL_SCHOOLS[c]
        best, top = None, None
        for key, cat in data.catalysts.items():
            if c not in cat["categories"]:
                continue
            p = cast_power(data, key, element, stats)
            if top is None or p > top[1]:
                top = (key, p)
            if any(stats[s] < v for s, v in cat["require"].items()):
                continue
            if best is None or p > best[1]:
                best = (key, p)
        if best is not None:
            out.append((label, best[0], best[1], top[0] if top[1] > best[1] else None))
    return out


#: DAMAGE_PARAM damageType -> DMG key. 5 is no damage and 6 and up are status build-ups
#: (DAMAGE_ATTRIBUTE in the DS2S paramdefs), which no point of armour defense reduces.
DAMAGE_TYPE = {0: "physical", 1: "magic", 2: "lightning", 3: "fire", 4: "dark"}
#: SpellParam ids whose SpEffect event spawns their damage (`100120[3]`) instead of a bullet child;
#: the damaging bullet is the spell id + 10, which is also its PlayerDamageParam row (INFERRED from
#: the id pattern: 31070010, 31080010 and 34040010 are the only magic/dark rows next to these ids).
SCRIPTED_SPELL_BULLET = {31070000: 31070010, 31080000: 31080010, 34040000: 34040010}


def bullet_hits(d: dict, bullet: int, dmg: int, depth: int = 0, seen: set | None = None) -> list[dict]:
    """Every hit a player bullet deals, depth first: the damage row it names and the rows its
    child bullets name (BulletParam childeBulletId/childeDamageId 01..03), where a row counts when
    it deals physical or elemental damage with a positive damageRate. A row is PlayerDamageParam,
    or SystemDamageParam when PlayerDamageParam has no such id; bullets likewise from BulletParam,
    then SystemBulletParam. Each hit: its damage type and flat damage (damage01..03 of that type),
    damageRate and damageLower. REGULATION."""
    seen = set() if seen is None else seen
    if (bullet, dmg) in seen or depth > 8:
        return []
    seen.add((bullet, dmg))
    out = []
    row = d["PlayerDamageParam"].get(str(dmg)) or d["SystemDamageParam"].get(str(dmg))
    if row and row["damageRate"] > 0:
        for n in (1, 2, 3):
            kind = DAMAGE_TYPE.get(row[f"damageType0{n}"])
            if kind:
                out.append({"type": kind, "flat": row[f"damage0{n}"], "rate": row["damageRate"],
                            "lower": row["damageLower"], "row": dmg})
    b = d["BulletParam"].get(str(bullet)) or d["SystemBulletParam"].get(str(bullet)) or {}
    for sfx in ("", "02", "03"):
        child, child_dmg = b.get("childeBulletId" + sfx), b.get("childeDamageId" + sfx)
        if child or child_dmg:
            out += bullet_hits(d, child, child_dmg, depth + 1, seen)
    return out


def inherited_hits(hits: list[dict], root: dict) -> list[dict]:
    """`hits` (bullet_hits of one spell's bullet tree) as a player's spell lands them: every hit
    row keeps its damageRate and damageLower, but its types and flat damage are the spell's own
    damage row's (`root`, SpellParam baseSpellDamageId), summed per type. EXE: the attack block is
    built once from that row (0x1403936a0 -> 0x140391fe0), and a child bullet of a player-type
    owner copies its parent's block whole (0x140445ec0; only slot 5 comes from the child row);
    only a non-player owner's child gets its own row's damage01..03. So Wrath of the Gods'
    damaging child row (lightning, flat 0) hits with the spell row's lightning 300."""
    types: dict[str, float] = {}
    for n in (1, 2, 3):
        kind = DAMAGE_TYPE.get(root[f"damageType0{n}"])
        if kind:
            types[kind] = types.get(kind, 0) + root[f"damage0{n}"]
    out, rows = [], set()
    for h in hits:
        if h["row"] not in rows:
            rows.add(h["row"])
            out += [{"type": t, "flat": f, "rate": h["rate"], "lower": h["lower"], "row": h["row"]}
                    for t, f in types.items()]
    return out


def regulation_spell_hits(data: Data, d: dict, names: dict) -> str:
    """data.spell_hits: spell key -> the hits one cast can deal (bullet_hits from SpellParam
    baseSpellBulletId / baseSpellDamageId as inherited_hits lands them, joined by normalized name
    as regulation_spells joins). A spell with no damaging hit (a heal, a buff, a status mist) has
    none. The SCRIPTED_SPELL_BULLET hits keep their own rows' damage: no parent bullet spawns them.
    A soul-consuming spell (Climax, the Resonant hexes) is taken at its own ids, which EXE
    0x140390050 uses only while the caster holds at least SpellSoulConsumeParam.consumeSoul souls
    (5,000 for Climax; fewer souls pick a weaker pair, docs/DS2-DPS-MECHANICS.md)."""
    by_name = {}
    for sid, s in d["SpellParam"].items():
        by_name.setdefault(norm(names.get(sid, "")), (int(sid), s))
    data.spell_hits = {}
    for key, row in data.spells.items():
        sid, s = by_name.get(norm(row.get("name", key)), (None, None))
        if s is None:
            continue
        hits = bullet_hits(d, s["baseSpellBulletId"], s["baseSpellDamageId"])
        dmg = str(s["baseSpellDamageId"])
        root = d["PlayerDamageParam"].get(dmg) or d["SystemDamageParam"].get(dmg)
        if root:
            hits = inherited_hits(hits, root)
        if sid in SCRIPTED_SPELL_BULLET:
            hits += bullet_hits(d, SCRIPTED_SPELL_BULLET[sid], SCRIPTED_SPELL_BULLET[sid])
        if hits:
            data.spell_hits[key] = hits
    return f"{len(data.spell_hits)} damaging spells from PlayerDamageParam/BulletParam"


def regulation_hit_flat(data: Data, d: dict) -> str:
    """data.hit_flat: PlayerDamageParam row -> {DMG key: flat attack} for every row whose damage01..03
    add physical or elemental attack. EXE (docs/DS2-DPS-MECHANICS.md "Base, rates and the sum"): the
    attack builder adds a hit's row's damage0n to that type's attack before the defender's side
    (0x14038fee0 -> 0x14038f2d0, whose jump table sends damageType 0-4 to physical, magic,
    lightning, fire and dark). Its poison and bleed adds are left out: among weapon attacks only the
    Bat Staff's R2s carry one, and the ranking skips catalysts. The jump table drops type 11 (toxic),
    so Mytha's Bent Blade's and the Umbral Dagger's toxic adds reach no attack."""
    data.hit_flat = {}
    for rid, row in d["PlayerDamageParam"].items():
        flat: dict[str, float] = {}
        for n in (1, 2, 3):
            kind = DAMAGE_TYPE.get(row[f"damageType0{n}"])
            if kind and row[f"damage0{n}"]:
                flat[kind] = flat.get(kind, 0) + row[f"damage0{n}"]
        if flat:
            data.hit_flat[rid] = flat
    return f"flat attack on {len(data.hit_flat)} PlayerDamageParam rows"


#: A weapon buff in its spell's SpEffect event (SpEffectSpell.emevd, event id == SpellParam id):
#: `100080[1] [seconds f32, element, scale f32]` and `100090[2] [seconds f32, flat << 16 | element]`,
#: element as DAMAGE_TYPE. REGULATION for the numbers; what the game does with them is below.
BUFF_SCALED, BUFF_FLAT = (100080, 1), (100090, 2)


def regulation_buffs(data: Data, emevd, members: dict, d: dict, names: dict) -> str:
    """data.buffs: spell key -> {"type", "scale", "flat", "seconds"} for every spell whose own
    SpEffect event carries a weapon-attack instruction (BUFF_SCALED or BUFF_FLAT) in an element.
    A physical attack add is left out: it multiplies the weapon's own physical rate, which
    buff_attack does not model (no spell's own event has one today). `emevd` is
    scripts/ds2-emevd.py, `members` the regulation's files."""
    events = {e.id: e for e in emevd.Emevd(Path("SpEffectSpell.emevd"), members["SpEffectSpell.emevd"]).events}
    by_name = {}
    for sid in d["SpellParam"]:
        by_name.setdefault(norm(names.get(sid, "")), int(sid))
    data.buffs = {}
    for key, row in data.spells.items():
        ev = events.get(by_name.get(norm(row.get("name", key)), -1))
        if ev is None:
            continue
        buff = {}
        for ins in ev.instructions:
            w = ins.words()
            if (ins.bank, ins.index) == BUFF_SCALED and len(w) >= 3:
                buff.update(type=DAMAGE_TYPE.get(w[1][0]), scale=w[2][1], seconds=w[0][1])
            elif (ins.bank, ins.index) == BUFF_FLAT and len(w) >= 2:
                buff.setdefault("type", DAMAGE_TYPE.get(w[1][2] & 0xFFFF))
                buff.update(flat=w[1][2] >> 16, seconds=w[0][1])
        if buff.get("type") in ELEMENTS:
            data.buffs[key] = {"scale": 0.0, "flat": 0, **buff}
    return f"{len(data.buffs)} weapon buffs from SpEffectSpell.emevd"


#: The icon a ring's SpEffect event shows while it is worn, `100130[2]` (docs/DS2-SPEFFECT-VISUAL.md,
#: "Display Icon"): no effect on a stat or an attack.
RING_ICON = (100130, 2)


def regulation_ring_attack(data: Data, emevd, members: dict, names: dict) -> str:
    """data.ring_attack: ring key -> DMG key -> flat attack add, for every ring whose SpEffect event
    (SpEffectRing.emevd, event id == item id, joined to the ring by its itemname.fmg name) is
    nothing but BUFF_FLAT adds in a damage type (DAMAGE_TYPE) and its icon. BUFF_FLAT is the
    weapon buff's instruction, and what the game does with it is the builder's (buff_attack): it
    adds `flat` to that type's stat bonus, so the weapon gains flat x its rate in the type
    (ring_attack_add). REGULATION, read 2026-09-30 (`scripts/ds2-emevd.py events`):

    | ring (event) | instruction | add |
    |---|---|---|
    | Ring of Blades (40160000) | `100090[2] [0, 20 << 16 | 0]` | physical +20 |
    | Ring of Blades+1 (40160001) | `100090[2] [0, 35 << 16 | 0]` | physical +35 |
    | Ring of Blades+2 (40160002) | `100090[2] [0, 50 << 16 | 0]` | physical +50 |
    | King's Ring (40510000) | `100090[2] [0, 5 << 16 | 3]` | fire +5 |

    Left out, each for an instruction beside the add: the Sorcery/Lightning/Fire/Dark Clutch Rings
    (41040000-41070000) add 30 to magic/lightning/fire/dark but also carry `100090[5] [0, 80 << 16 |
    0]`, a physical defense change nothing here weighs against the attack; Crest of the Rat
    (40700000, type 5, poison) is gated on `100180[2]`, which is not decoded; Crest of Blood
    (40730000) adds to type 6, bleed, which no damage objective reads. Old Leo Ring's `1000[1]`
    kind 11 and Flynn's Ring's `1000[18]` are kinds no one here has decoded (docs/DS2-BUILD-MECHANICS.md
    "Not established"), so they have no number at all. In event id order: a ring upgrade group's
    last is its strongest tier, as ring_gear's is."""
    events = {e.id: e for e in emevd.Emevd(Path("SpEffectRing.emevd"), members["SpEffectRing.emevd"]).events}
    data.ring_attack = {}
    for eid in sorted(events):
        key = data.sp_key.get(norm(names.get(str(eid), "")))
        if key not in data.rings:
            continue
        add, other = {}, False
        for ins in events[eid].instructions:
            w = ins.words()
            kind = DAMAGE_TYPE.get(w[1][2] & 0xFFFF) if (ins.bank, ins.index) == BUFF_FLAT and len(w) >= 2 else None
            if kind is not None:
                add[kind] = add.get(kind, 0) + (w[1][2] >> 16)
            elif (ins.bank, ins.index) != RING_ICON:
                other = True
        if add and not other:
            data.ring_attack[key] = add
    return f"{len(data.ring_attack)} rings' attack adds from SpEffectRing.emevd"


#: A defense change in a SpEffect event: `100090[4] [seconds f32, value << 16 | type]` adds, `100090[5]`
#: subtracts, type as DAMAGE_TYPE (5-9 are the status resistances, not modelled here). REGULATION
#: for the numbers. EXE for what they do, read 2026-10-01 (docs/DS2-DPS-MECHANICS.md "Defense from
#: a SpEffect"): `SpEffectActionImpl_ChangeAtkDef` (`0x14021b230`) adds (modes 2, 4, 9) or subtracts
#: (3, 5) the u16 value into slot `type` of a float vector when asked with request byte 2 (modes 4
#: and 5), and `0x14014ce90` asks with that byte. `ChrGameParamCalculator` slot +0x250
#: (`0x14031fe20`) reads that vector: its physical entry raw, every other x 0.01 (`0x14034c7d0`).
#: - physical: slot +0x248 (`0x140320380`) returns the raw entry, and the physical DEF builder
#:   (`0x140380070`) adds it once to the defense of whatever slash/strike/thrust type is asked for.
#:   So `value` is flat physical defense, in the units SoulsPlanner shows.
#: - elements: the cut builder (`0x140381350`) calls slot +0x250 once on the summed cut, after the
#:   four armour pieces and before the clamp and the 0.99 cap. So `value` is `value`% cut, which is
#:   `10 * value` in the displayed defense D (`cut = (D + 100) / 1000`, damage()).
#: The dispatch from `0x14014ce90` (`0x14022e1e0` -> `0x14021f100`) to slot 19 of the handler was
#: followed only to the query it builds (request byte at +0x35); that it reaches ChangeAtkDef is
#: INFERRED from the request byte matching that handler's defense branch.
DEF_ADD, DEF_SUB = (100090, 4), (100090, 5)


def speffect_defense(ev) -> tuple[dict, float]:
    """A SpEffect event's DEF_ADD/DEF_SUB changes as defense in displayed units, per DMG key
    (physical flat, an element 10x its percent), and the longest duration they carry (0: while
    worn). Status types are dropped."""
    add, seconds = {}, 0.0
    for ins in ev.instructions:
        w = ins.words()
        if (ins.bank, ins.index) not in (DEF_ADD, DEF_SUB) or len(w) < 2:
            continue
        kind = DAMAGE_TYPE.get(w[1][2] & 0xFF)
        if kind is None:
            continue
        v = (w[1][2] >> 16) * (1 if kind == "physical" else 10)
        add[kind] = add.get(kind, 0) + (v if (ins.bank, ins.index) == DEF_ADD else -v)
        seconds = max(seconds, w[0][1])
    return {k: v for k, v in add.items() if v}, seconds


def regulation_ring_defense(data: Data, emevd, members: dict, names: dict) -> str:
    """data.ring_defense: ring key -> DMG key -> defense add in displayed units (speffect_defense),
    for every ring whose SpEffect event (SpEffectRing.emevd, event id == item id, joined by
    itemname.fmg name) changes a damage type's defense. REGULATION, read 2026-10-01:

    | ring (event) | instruction | defense |
    |---|---|---|
    | Ring of Steel Protection / +1 / +2 (40050000-2) | `100090[4] [0, 50/75/100 << 16 | 0]` | physical +50 / +75 / +100 |
    | Spell Quartz Ring / +1 / +2 / +3 (40060000-3) | `100090[4] [0, 5/8/10/15 << 16 | 1]` | magic cut +5/8/10/15% = D +50/80/100/150 |
    | Thunder Quartz Ring ... +3 (40080000-3) | same values, type 2 | lightning |
    | Flame Quartz Ring ... +3 (40070000-3) | same values, type 3 | fire |
    | Dark Quartz Ring ... +3 (40090000-3) | same values, type 4 | dark |
    | Dispelling Ring / +1 (40140000-1) | `100090[4] [0, 6/12 << 16 | 1..4]`, one per element | all four +6/12% = D +60/120 |
    | Sorcery/Lightning/Fire/Dark Clutch Ring (41040000-41070000) | `100090[5] [0, 80 << 16 | 0]` | physical -80 |

    Ring of Steel Protection also carries `1000[1] [59, 0, 0.9]` (+2: 0.875), a kind nothing here
    decodes; Ring of Resistance and the bite rings change only status types (5-9)."""
    events = {e.id: e for e in emevd.Emevd(Path("SpEffectRing.emevd"), members["SpEffectRing.emevd"]).events}
    data.ring_defense = {}
    for eid in sorted(events):
        key = data.sp_key.get(norm(names.get(str(eid), "")))
        if key not in data.rings:
            continue
        add, _ = speffect_defense(events[eid])
        if add:
            data.ring_defense[key] = add
    return f"{len(data.ring_defense)} rings' defense changes from SpEffectRing.emevd"


def regulation_defense_buffs(data: Data, emevd, members: dict, d: dict, names: dict) -> str:
    """data.defense_buffs: name -> {"source": "spell" | "item", "add": DMG key -> defense in
    displayed units, "seconds": duration} for every spell and usable item whose SpEffect only raises
    a damage type's defense (speffect_defense; a debuff such as Whisper of Despair's physical -200
    is left out). A spell's effect is event id + 10 in SpEffectSpell.emevd: Magic Barrier's own
    event names it (`100120[6] [32190010, ...]`); for the rest the +10 is INFERRED from that
    pattern, as SCRIPTED_SPELL_BULLET's is. Its +11..+14 rows (Magic Barrier 15% for 70-95 s) are
    not chosen by anything traced here and are left out. An item's is its item id in
    SpEffectActiveItem.emevd. REGULATION, read 2026-10-01:

    | buff (event) | defense | seconds |
    |---|---|---|
    | Magic Barrier (32190010) | magic, lightning, fire, dark cut +10% | 60 |
    | Great Magic Barrier (32200010) | all four +25% | 90 |
    | Sacred Oath (32230010) | physical +75 (and its attack adds) | 40 |
    | Flash Sweat (33180010) | fire +30% | 90 |
    | Iron Flesh (33190010) | physical +100, all four +20% (and `1000[1]` kind 26 x0.6, not modelled) | 25 |
    | Small Blue / Yellow / Orange Burr, Dark Troches (60160000-60190000) | magic / lightning / fire / dark +15% | 90 |

    Numbness (34100010) carries only `1000[1]` kind 26 x0.85 and kind 59 x0.8; kind 26 is INFERRED
    to be a damage-taken rate (docs/DS2-DPS-MECHANICS.md, "What else a hit carries") and its reader
    in the damage path was not found, so neither it nor Iron Flesh's x0.6 counts."""
    spell_ev = {e.id: e for e in emevd.Emevd(Path("SpEffectSpell.emevd"), members["SpEffectSpell.emevd"]).events}
    item_ev = {e.id: e for e in emevd.Emevd(Path("SpEffectActiveItem.emevd"),
                                            members["SpEffectActiveItem.emevd"]).events}
    data.defense_buffs = {}
    found = [("spell", spell_ev.get(int(sid) + 10), names.get(sid, "")) for sid in d["SpellParam"]]
    found += [("item", ev, names.get(str(eid), "")) for eid, ev in item_ev.items()]
    for source, ev, name in found:
        if ev is None or not name or name.startswith("<"):
            continue
        add, seconds = speffect_defense(ev)
        if add and min(add.values()) > 0:
            data.defense_buffs[name] = {"source": source, "add": add, "seconds": seconds}
    return f"{len(data.defense_buffs)} defense buffs from SpEffectSpell/SpEffectActiveItem.emevd"


def ring_attack_add(data: Data, row: dict, rings) -> dict:
    """What the worn `rings` add to a weapon infusion `row`'s attack rating, per DMG key: each
    ring's flat (regulation_ring_attack) times the row's rate in that type (regulation_attack), in
    DMG order. EXE (docs/DS2-DPS-MECHANICS.md "Base, rates and the sum"): `AR = (bonus + base) x
    rate`, and the add is in `bonus`. Only a type the row has a base in: attack_rating builds no
    other."""
    atk, rate = row.get("atk") or {}, row.get("rate") or {}
    out = {}
    for k in DMG:
        if not atk.get(k):
            continue
        for r in rings:
            flat = data.ring_attack.get(r, {}).get(k)
            if flat:
                out[k] = out.get(k, 0.0) + flat * rate.get(k, 0.0)
    return out


def attack_rating(data: Data, weapon: str, inf: str, eff: dict, rings=()) -> dict:
    """Per-type attack rating at full upgrade, combined as SoulsPlanner's getPhysicalATK/
    getMagicATK/... combine it (the bases and coefficients are the regulation's, regulation_attack),
    plus what the worn `rings` add (ring_attack_add; none by default). Physical scales STR and DEX
    off one table; fire reads INT+FTH, dark min(INT, FTH)."""
    row = data.weapons[weapon]["infusions"].get(inf) or {}
    atk, sc = row.get("atk") or {}, row.get("atkScale") or {}
    add = ring_attack_add(data, row, rings) if rings else {}
    out = {}
    if atk.get("physical"):
        v = (atk["physical"] + add.get("physical", 0)
             + sc.get("strength", 0) * _tab(data, "physicalATKBonus", eff["strength"])
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
        # A type with a rate and a coefficient but no base still attacks: `AR = (bonus + base) x
        # rate` (EXE, docs/DS2-DPS-MECHANICS.md "Base, rates and the sum"). The Sanctum Crossbows'
        # dark is this: darkRate 100, maximumDark 0, dark coefficient 0.4. Enchanted's "magic" is
        # its INT term on physical, above, unless the row also has a magic base.
        if atk.get(k) or ((row.get("rate") or {}).get(k) and sc.get(k) and not (inf == "Enchanted" and k == "magic")):
            out[k] = int(atk.get(k, 0) + add.get(k, 0) + sc.get(k, 0) * _tab(data, tab, i))
    return out


#: WeaponTypeParam.shootCategory -> the ArrowParam.arrowType it fires and what it is. REGULATION:
#: bows are shootCategory 1, greatbows 2, crossbows 3 (every named launcher, 2026-10-01); arrows
#: are arrowType 1, greatarrows 2, bolts 4. That a bow takes arrows and a crossbow bolts is the
#: game's rule as players know it (COMMUNITY); the pairing is not read from the executable.
SHOOT_AMMO = {1: (1, "bow"), 2: (2, "greatbow"), 3: (4, "crossbow")}
#: The ArrowParam damage set a shot reads. INFERRED: the executable indexes the row's four
#: (light bullet, heavy bullet, light damage, heavy damage) sets, and the index was not read. For
#: arrows and greatarrows set 1 is the only set whose damage ids name PlayerDamageParam rows, so a
#: bow's shot cannot read another. Bolts fill sets 0-2 alike (MV 0.392) and set 3 at MV 0.247;
#: which launcher reads set 3 is UNPROVEN (the triple-shot Avelyn and Sanctum Repeating Crossbow
#: are the guess), so every crossbow is scored on set 1, and those two are flagged.
AMMO_SET = 1
#: The WeaponActionCategoryParam slot of the Sanctum Crossbows' own shot (their two-handed L2).
SPECIAL_SLOT = "atkIdOppositeSingle2HandStrong"


def regulation_ranged(data: Data, d: dict, names: dict) -> str:
    """data.ranged: launcher key -> {"ammo": the arrowType it fires, "kind", "hand": WeaponTypeParam
    rightDamageScale, "special": [its own shots]}; data.ammo: ammo name -> {"type": arrowType,
    "flat": {DMG key: flat attack}, "status": {"poison"/"bleed": flat build-up}, "mv": damageRate,
    "lower": damageLower, "row": PlayerDamageParam id}, from ArrowParam set AMMO_SET. Joined by name
    as regulation_catalysts joins; a launcher is a WeaponParam row whose WeaponTypeParam
    shootCategory is in SHOOT_AMMO.

    What a shot attacks with (EXE, docs/DS2-DPS-MECHANICS.md "Ranged attack"): a
    normal shot calls the per-hand attack builder 0x140391fe0 (from 0x14005210b -> 0x140392d51)
    with the type mask off, and hands it the ammo's ArrowParam light bullet and damage ids; the
    builder's loop is `AR = ((bonus + base) x rate + D) x k` (docs/DS2-DPS-MECHANICS.md "Base, rates
    and the sum"), where D is that damage row's damage01..03 per type (0x14038fee0). So the shot
    carries the launcher's whole attack in every type, infusion included, plus the ammo's flat
    attack, and the ammo's damageRate is the motion value after the defense. `k` holds
    WeaponTypeParam.rightDamageScale, 0.9 on every bow, greatbow and crossbow but Avelyn (0.865) and
    1.0 on every melee weapon (REGULATION, scripts/ds2-ranged-evidence.py scales), so it is kept
    here and nowhere else. ArrowParam's menu*Attack fields are not this sum and are not read
    (INFERRED display-only).

    "special": the shots of SPECIAL_SLOT's attack motion that hurt (bulletDamageId0n rows with a
    damageRate and hitFlagEnemy), each {"types": the row's damage types, "flat", "mv", "lower",
    "shots": BulletParam.automaticShootNum}. EXE: that builder call (0x1406d5502) sets the type mask
    (0x1403910f0), so only the row's own types keep their rate: the Sanctum Crossbow's is pure dark,
    its dark attack + 50 at MV 7.2, the Repeating one's three shots of dark + 5 at MV 1.85
    (REGULATION). That the call is this attack's is INFERRED from the call site; that the special
    fires no bolt and so takes no ammo is INFERRED."""
    by_name = {}
    for wid, w in d["WeaponParam"].items():
        by_name.setdefault(norm(names.get(wid, "")), w)
    pdp, bullets = d["PlayerDamageParam"], d["BulletParam"]

    def row_terms(rid: int):
        r = pdp.get(str(rid))
        if not r or r["damageRate"] <= 0:
            return None
        flat, status, types = {}, {}, []
        for n in (1, 2, 3):
            t, v = r[f"damageType0{n}"], r[f"damage0{n}"]
            if t in DAMAGE_TYPE:
                types.append(DAMAGE_TYPE[t])
                if v:
                    flat[DAMAGE_TYPE[t]] = flat.get(DAMAGE_TYPE[t], 0) + v
            elif t in (6, 7) and v:  # the builder's jump table: 6 poison, 7 bleed (0x14038f2d0)
                k = "poison" if t == 6 else "bleed"
                status[k] = status.get(k, 0) + v
        return {"types": types, "flat": flat, "status": status, "mv": r["damageRate"], "lower": r["damageLower"],
                "row": rid, "enemy": r["hitFlagEnemy"]}

    data.ammo = {}
    for aid, a in d["ArrowParam"].items():
        name = names.get(aid, "")
        terms = row_terms(a[f"lightDamageId{AMMO_SET}"])
        if name and not name.startswith("<") and terms:
            data.ammo[name] = {"type": a["arrowType"], **terms}
    data.ranged = {}
    for key, w in data.weapons.items():
        wp = by_name.get(norm(w.get("name", key)))
        t = wp and d["WeaponTypeParam"].get(str(wp["weaponTypeId"]))
        if not t or t["shootCategory"] not in SHOOT_AMMO:
            continue
        ammo, kind = SHOOT_AMMO[t["shootCategory"]]
        special = []
        ac = d["WeaponActionCategoryParam"].get(str(wp["weaponActionCategoryId"])) or {}
        m = d["WeaponAttackMotionParam"].get(str(ac.get(SPECIAL_SLOT, 0)))
        for n in (1, 2, 3):
            s = m and m[f"bulletId0{n}"] and row_terms(m[f"bulletDamageId0{n}"])
            if s and s["enemy"]:
                b = bullets.get(str(m[f"bulletId0{n}"])) or {}
                special.append({**s, "shots": max(1, b.get("automaticShootNum") or 1)})
        data.ranged[key] = {"ammo": ammo, "kind": kind, "hand": t["rightDamageScale"], "special": special}
    return (f"{len(data.ranged)} bows/greatbows/crossbows and {len(data.ammo)} ammunition from ArrowParam "
            f"(set {AMMO_SET}), {sum(1 for r in data.ranged.values() if r['special'])} with a special shot")


def ranged_options(data: Data, weapon: str, inf: str, eff: dict, rings=(),
                   special: bool = True) -> list[tuple[str, dict, dict]]:
    """Every shot `weapon` (a data.ranged launcher) can fire at stats `eff`: (label, attack per DMG
    key, the hit's {"mv", "lower", "shots", "status"}), one per ammunition of its arrowType and one
    per special shot (regulation_ranged). The attack is the shot's: (launcher's attack + the ammo's
    flat) x the hand scale, the launcher's attack being attack_rating's, rings included. A special
    keeps only its row's types (the type mask). Status is build-up per hit in the launcher's
    regulation_status units: the launcher's term plus the ammo's flat x 100 / 2505 (the builder
    scales the status entries after D is added, docs/DS2-DPS-MECHANICS.md "Status build-up per hit")."""
    r = data.ranged[weapon]
    base = attack_rating(data, weapon, inf, eff, rings)
    out = []
    for name, a in sorted(data.ammo.items()):
        if a["type"] != r["ammo"]:
            continue
        ar = {k: (base.get(k, 0) + a["flat"].get(k, 0)) * r["hand"] for k in DMG
              if base.get(k, 0) + a["flat"].get(k, 0)}
        out.append((name, ar, {"mv": a["mv"], "lower": a["lower"], "shots": 1, "status": a["status"]}))
    for i, s in enumerate(r["special"] if special else ()):
        ar ={k: (base.get(k, 0) + s["flat"].get(k, 0)) * r["hand"] for k in s["types"]
              if base.get(k, 0) + s["flat"].get(k, 0)}
        out.append(("special shot" + (f" {i + 1}" if len(r["special"]) > 1 else ""), ar,
                    {"mv": s["mv"], "lower": s["lower"], "shots": s["shots"], "status": s["status"]}))
    return out


#: Status build-up units: 100 / (hpMax[99] + 8 x additionalHp[99]) (regulation_status), the
#: multiplier the builder puts on a hit's status entries.
STATUS_UNIT = 100 / 2505


def ranged_value(data: Data, weapon: str, inf: str, eff: dict, objective: str, dfn: dict,
                 rings=(), special: bool = True) -> tuple[float, str, dict]:
    """The best shot of a launcher by `objective`, as (value, its label, its attack): "damage" is
    one trigger pull against `dfn` -- hit_damage of the shot at the ammo's damageRate and
    damageLower, against general physical defense (an arrow's slash/strike/thrust type is not
    read), x damageScale after the defense, x the special's automaticShootNum; "ar" the shot's
    summed attack (before the defense and the motion value); "bleed"/"poison" the build-up per
    hit. Per shot, not per second: the fire rate is the aim/fire/reload loop the TAE files here do
    not hold (the bow shot animations 40230010/41230010 are not in c000100_pl.tae's unpacked set),
    so it is not scored. `special` False leaves the special shots out: the best ammunition alone.
    (0.0, "", {}) when nothing fires."""
    row = data.weapons[weapon]["infusions"].get(inf) or {}
    best = (0.0, "", {})
    for label, ar, hit in ranged_options(data, weapon, inf, eff, rings, special):
        if objective == "ar":
            v = sum(ar.values())
        elif objective in ("bleed", "poison"):
            if hit["shots"] > 1 or label.startswith("special"):
                continue  # a special's status rows are not read here
            i = 3 * eff["dexterity"] + (eff["faith"] if objective == "bleed" else eff["adaptability"])
            own = (row.get("atk") or {}).get(objective, 0) + (row.get("atkScale") or {}).get(objective, 0) * _tab(
                data, "auxATKBonus", i)
            v = (own + hit["status"].get(objective, 0) * STATUS_UNIT) * data.ranged[weapon]["hand"]
        else:  # against the defender's answer to this shot's own types (respond): a Sanctum special's
            # pure dark meets a Dark Quartz Ring, an arrow's physical a Ring of Steel Protection
            v = (hit_damage(ar, respond(dfn, ar), hit["mv"], "physical", hit["lower"])
                 * data.damage_scale.get(weapon, 1.0) * hit["shots"])
        if v > best[0]:
            best = (v, label, ar)
    return best


#: Crossbows that fire more than one bolt per pull, as players describe them (COMMUNITY); how many
#: bolts and which ArrowParam set they read is not read (AMMO_SET), so they are scored on one bolt.
VOLLEY_CROSSBOWS = {"Avelyn", "Sanctum Repeating Crossbow"}


def ranged_pick(data: Data, weapon: str, inf: str, eff: dict, objective: str, dfn: dict,
                rings=()) -> tuple[float, str, dict, str]:
    """ranged_value's best shot, and a note naming it for the output: the ammunition, or the special
    shot (two-handed, its shot count) with the best ammunition beside it, and what is unproven.
    (value, note, attack, best ammunition name)."""
    v, shot, ar = ranged_value(data, weapon, inf, eff, objective, dfn, rings)
    av, ammo, _ = ranged_value(data, weapon, inf, eff, objective, dfn, rings, special=False)
    volley = " (1 bolt; volley and ArrowParam set unproven)" if data.weapons[weapon]["name"] in VOLLEY_CROSSBOWS else ""
    if shot.startswith("special"):
        s = data.ranged[weapon]["special"][int(shot.split()[-1]) - 1 if shot[-1].isdigit() else 0]
        times = f" x{s['shots']}" if s["shots"] > 1 else ""
        note = f"2H special shot{times}, no ammo; best ammo {ammo or 'none'} {av:.0f}{volley}"
    else:
        note = shot + volley
    return v, note, ar, ammo


def build_defense(data: Data, b: Build) -> dict:
    """Per-type defense of a build: SoulsPlanner's getPhysicalDEF/getMagicDEF/... (SITE). Armor
    physical defense grows with END+VIT+STR+DEX via each piece's bonus coefficient; elemental
    defense is a stat-driven base plus the armor's flat values. The rings' stat changes count, and
    so do their defense changes (data.ring_defense, EXE + REGULATION, regulation_ring_defense): a
    physical add on the general physical defense and on each of slash/strike/thrust, since the game
    adds it to whichever type the hit asks for; an element's in displayed units (10x its cut %)."""
    eff = effective(data, b)
    for r in b.rings:
        for s, v in ring_effects(data).get(r, (0, {}, 1))[1].items():
            eff[s] = eff.get(s, 0) + v
    out = _armor_defense(data, b, eff)
    for r in b.rings:
        out = with_defense(out, data.ring_defense.get(r, {}))
    return out


def with_defense(dfn: dict, add: dict, times: float = 1.0) -> dict:
    """`dfn` (DMG + PHYS_TYPES keys) plus `times` x a defense change by DMG key, the physical one
    on every physical type as well (build_defense)."""
    out = dict(dfn)
    for k, v in add.items():
        for t in [k] + (PHYS_TYPES if k == "physical" else []):
            out[t] = out.get(t, 0.0) + v * times
    return out


def _armor_defense(data: Data, b: Build, eff: dict) -> dict:
    """build_defense's armour and stat part, at effective stats `eff`."""
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


def bracket_builds(data: Data, corpus: list[Build], sl: int) -> list[Build]:
    """The corpus builds in `sl`'s bracket, or the nearest bracket with >= 20; none if no bracket has."""
    i = sl_bracket(sl)
    by = {}
    for b in corpus:
        by.setdefault(sl_bracket(soul_level(data, b)), []).append(b)
    for j in sorted(range(len(SL_BRACKETS)), key=lambda j: (abs(j - i), j)):
        if len(by.get(j, [])) >= 20:
            return by[j]
    return []


def bracket_defense(data: Data, corpus: list[Build], sl: int) -> tuple[dict, int]:
    """Mean per-type defense (build_defense, worn rings' defense counted) of the corpus builds in
    `sl`'s bracket (bracket_builds). Remembered per corpus and bracket: optimize_build asks once per
    ring set it tries, and every weapon of best_weapons asks again, for the same few thousand builds."""
    i = sl_bracket(sl)
    memo = _bracket_defense_memo.get((id(data), id(corpus), i))
    if memo is not None and memo[0] is data and memo[1] is corpus:  # held, so the ids cannot be reused
        return dict(memo[2]), memo[3]
    d, n = _bracket_defense(data, corpus, sl)
    _bracket_defense_memo[(id(data), id(corpus), i)] = (data, corpus, d, n)
    return dict(d), n


_bracket_defense_memo: dict = {}


def _bracket_defense(data: Data, corpus: list[Build], sl: int) -> tuple[dict, int]:
    ds = [build_defense(data, b) for b in bracket_builds(data, corpus, sl)]
    if not ds:
        return {k: 0.0 for k in DMG + PHYS_TYPES}, 0
    return {k: float(np.mean([d[k] for d in ds])) for k in DMG + PHYS_TYPES}, len(ds)


#: Who the damage objective is scored against. "adaptive" (the default; --static-defender turns it
#: off): every defender swaps one ring slot to the ring that cuts this weapon+infusion's damage
#: most (respond). "static": the corpus builds' rings as worn. `buff` "none" (the default), "item"
#: (a consumable anyone can use) or "any" (spells too, whether or not the build could cast them):
#: the defender also has the one defense_buffs entry that cuts the damage most. Set by main;
#: --export-backend's precomputed numbers and most --expect fixtures are written "static", and the
#: ADAPTIVE_* fixtures set it themselves (under_defender): the Rust port takes it per question
#: (model::Reply), adaptive by default as here.
DEFENDER = {"mode": "adaptive", "buff": "none"}


def counter_rings(data: Data) -> list[str]:
    """The rings a defender may swap in (respond): per ring upgrade group the last in
    data.ring_defense order (event id order, so the strongest tier, as offense_rings takes it)
    whose defense changes are all raises, never a NO_USE_RINGS ring. Today: Ring of Steel
    Protection+2, the four Quartz Rings+3 and Dispelling Ring+1 (regulation_ring_defense)."""
    by_group = {}
    for r, add in data.ring_defense.items():
        if r not in NO_USE_RINGS and min(add.values()) > 0:
            by_group[data.rings[r].get("group", r)] = r
    return list(by_group.values())


class AdaptiveDefense(dict):
    """A defender who answers the attack (DEFENDER "adaptive"). As a dict it is the static defense
    (bracket_defense, or a --defender set's), so everything that reads a defense without an attack
    in hand sees what it saw before. respond() gives the defense against one attack.

    The swap, per defender build, for counter ring c (counter_rings): a ring of c's upgrade group
    already worn is replaced by c (no change when it is c); else c goes into an empty slot, or in
    place of a worn ring that changes no defense; else (all four worn rings change a defense) in
    place of the one that cuts this attack least (_least_useful). The swapped-out ring's other
    effects (stats, HP, attack) are not counted against the defender. Only the last case depends on
    the attack, so the rest is summed once here: `fixed[c]` is the sum over those builds of the
    defense change, `varying` the worn rings' changes of the others, `n` all builds."""

    def __init__(self, data: Data, base: dict, builds_rings: list[list[str]]):
        super().__init__(base)
        self.n = max(len(builds_rings), 1)
        self.counters = [(c, data.rings[c].get("group", c), data.ring_defense[c]) for c in counter_rings(data)]
        self.names = {c: data.rings[c].get("name", c) for c, _, _ in self.counters}
        self.fixed = {c: {} for c, _, _ in self.counters}
        self.varying = []
        for rings in builds_rings:
            worn = [(data.rings.get(r, {}).get("group", r), data.ring_defense.get(r)) for r in rings if r not in EMPTY]
            if len(worn) >= 4 and all(add for _, add in worn):
                self.varying.append(worn)
                continue
            for c, g, add in self.counters:
                same = next((a for wg, a in worn if wg == g), None)
                for k in DMG:
                    v = add.get(k, 0) - ((same or {}).get(k, 0) if same is not None else 0)
                    self.fixed[c][k] = self.fixed[c].get(k, 0.0) + v
        buffs = data.defense_buffs.items()
        self.buffs = [(name, b["add"]) for name, b in buffs
                      if DEFENDER["buff"] == "any" or (DEFENDER["buff"] == "item" and b["source"] == "item")]

    def _delta(self, c: str, g: str, add: dict, weight: dict) -> dict:
        """The mean defense change of swapping in counter c, over all builds."""
        tot = dict(self.fixed[c])
        for worn in self.varying:
            same = next((a for wg, a in worn if wg == g), None)
            out = same if same is not None else _least_useful(worn, weight)
            for k in DMG:
                tot[k] = tot.get(k, 0.0) + add.get(k, 0) - out.get(k, 0)
        return {k: v / self.n for k, v in tot.items() if v}

    def respond(self, ar: dict) -> tuple[dict, list[str]]:
        """The defense against attack ratings `ar` (DMG keys), and what the defender put on for it:
        the counter ring that leaves the least damage (_type_damage), when one leaves less than no
        swap, then the DEFENDER `buff` that does the same. Ties keep counter_rings' order."""
        weight = {"physical": 1 / 12 if ar.get("physical", 0) * 10 > self["physical"] else 0.0}
        weight |= {k: ar.get(k, 0) / 1000 for k in ELEMENTS}
        best, used = dict(self), []
        for c, g, add in self.counters:
            cand = with_defense(self, self._delta(c, g, add, weight))
            if _type_damage(ar, cand) < _type_damage(ar, best) - 1e-9:
                best, used = cand, [self.names[c]]
        pick = None
        for name, add in self.buffs:
            cand = with_defense(best, add)
            if _type_damage(ar, cand) < _type_damage(ar, pick[0] if pick else best) - 1e-9:
                pick = (cand, name)
        if pick:
            best, used = pick[0], used + [pick[1]]
        return best, used


def _least_useful(worn: list[tuple[str, dict]], weight: dict) -> dict:
    """Of worn rings' defense changes, the one that cuts an attack weighted `weight` least: the
    damage is linear in each type's defense, -1/12 per physical point while the attack clears it and
    -AR/1000 per elemental point (damage()). The first on a tie."""
    return min((a for _, a in worn), key=lambda a: sum(weight.get(k, 0) * v for k, v in a.items()))


def _type_damage(ar: dict, dfn: dict) -> float:
    """One hit's summed damage() for attack ratings `ar` against `dfn`, motion value 1."""
    return sum(damage(k, v, dfn[k]) for k, v in ar.items())


def respond(dfn: dict, ar: dict) -> dict:
    """The defense to score attack `ar` against: an AdaptiveDefense's answer to it, else `dfn`."""
    return dfn.respond(ar)[0] if isinstance(dfn, AdaptiveDefense) else dfn


def bracket_stats(data: Data, corpus: list[Build], sl: int) -> tuple[dict, int]:
    """Per-stat median levelled stats of the corpus builds bracket_defense averages over (`sl`'s
    bracket, or the nearest with >= 20 builds), rounded up as bracket_floors rounds its medians, and
    how many builds. Every stat 0 when no bracket has 20."""
    i = sl_bracket(sl)
    by = {}
    for b in corpus:
        by.setdefault(sl_bracket(soul_level(data, b)), []).append([b.stats[s] for s in STATS])
    for j in sorted(range(len(SL_BRACKETS)), key=lambda j: (abs(j - i), j)):
        if len(by.get(j, [])) >= 20:
            med = np.median(np.array(by[j]), axis=0)
            return {s: int(np.ceil(v)) for s, v in zip(STATS, med)}, len(by[j])
    return {s: 0 for s in STATS}, 0


def defender_defense(data: Data, corpus: list[Build], sl: int, defender=None) -> tuple[dict, int]:
    """The defense the damage objective is scored against at `sl`, and how many corpus builds it
    was taken from. `defender` None is the bracket's average defender (bracket_defense). Otherwise
    it is four armour keys, head/chest/hands/legs ("Naked" for a bare slot), worn by a defender
    whose stats are the bracket's median (bracket_stats) with the pieces' own stat changes and no
    rings: build_defense, since a piece's physical defense grows with END+VIT+STR+DEX and the
    elemental base with INT/FTH, so a set has no defense of its own without some stats under it.
    The median is the stand-in for "a typical player at this level" in that set, as the average
    defender is the stand-in for a typical player's set.

    With DEFENDER "adaptive" the defense is an AdaptiveDefense over the same builds (the --defender
    set counts as one build with no rings): the same numbers, plus the ring each one swaps in
    against a given attack. Cached on `data` per corpus, bracket, set and DEFENDER setting."""
    key = (id(corpus), sl_bracket(sl), tuple(defender or ()), DEFENDER["mode"], DEFENDER["buff"])
    cache = data.__dict__.setdefault("_defender_cache", {})
    if key in cache:
        return cache[key]
    if defender is None:
        builds = bracket_builds(data, corpus, sl)
        dfn, n = bracket_defense(data, corpus, sl)
        rings = [b.rings for b in builds]
    else:
        st, n = bracket_stats(data, corpus, sl)
        d = build_defense(data, Build("", st, list(defender), [], 0, [], []))
        dfn, rings = {k: float(d[k]) for k in DMG + PHYS_TYPES}, [[]]
    if DEFENDER["mode"] == "adaptive":
        dfn = AdaptiveDefense(data, dfn, rings)
    cache[key] = (dfn, n)
    return dfn, n


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
                   distinct: bool = False, with_start: bool = False, with_row: bool = False) -> list[tuple]:
    """(seconds from input, motion value, physical type, damage floor, flat attack) of every hit a
    repeated attack lands, alternating its 1st and 2nd chain attacks: `kind` "Normal" is the R1
    chain, "Strong" the R2 chain. Play speed is the mean of start/end speeds (where one hands over
    to the other is unknown); re-hitting hitboxes add a tick per interval. `distinct`: see
    live_hits. The flat attack is the hit's damage row's, per DMG type (NO_FLAT for none; see
    load_attacks). `with_start` puts the second its attack began before the flat attack, which is
    what `horizon` cuts on: the hits of a longer horizon whose attack began before a shorter one are
    exactly the shorter one's hits. `with_row` appends the hit's PlayerDamageParam row id (str)."""
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
                            h.get("type") or "physical", h.get("lower", 0)) + ((t0,) if with_start else ())
                           + (h.get("flat") or NO_FLAT,) + ((str(h.get("dmg")),) if with_row else ()))
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


def r1_timeline(attacks: dict, name: str, two_hand: bool,
                horizon: float = 3.0) -> list[tuple[float, float, str, int, tuple]]:
    """The R1 chain's hits (chain_timeline); what `--window` and the exported backend rank by."""
    return chain_timeline(attacks, name, two_hand, "Normal", horizon)


_anim_len_cache: dict = {}


def anim_length(anim: int) -> float | None:
    """Seconds (animation time) an animation runs: the end of its TAE event 101100, which spans the
    whole animation in every attack anim read (docs/DS2-DPS-MECHANICS.md section 3), else its last
    event's end; None without the TAE file."""
    if anim not in _anim_len_cache:
        p = TAE_DIR / f"anim-{anim:09d}.xml"
        t = None
        if p.exists():
            ev = re.findall(r"<type>(\d+)</type>.*?<startTime>([-\d.]+)</startTime>\s*<endTime>([-\d.]+)</endTime>",
                            p.read_text(), re.S)
            whole = [float(e) for ty, _, e in ev if ty == "101100"]
            t = max(whole) if whole else max((float(e) for _, _, e in ev), default=None)
        _anim_len_cache[anim] = t
    return _anim_len_cache[anim]


#: How long the sustained-damage metric repeats the R1 chain, in seconds.
SUSTAIN_HORIZON = 5.0


def r1_reach(data: Data, attacks: dict, weapon: str, names: list[str], two_hand: bool) -> float | None:
    """Metres the R1's hitbox extends along the weapon: the first chain attack's live hitboxes,
    each followed through PlayerDamageParam.childDamage. Segments on a weapon dummy poly
    (hitDummyPolyType 1, id 100 and up: the whip's 100 -> 101 -> 102) add their lengths in order;
    the reach is that sum x WeaponParam.damageHitLengthScale plus the last segment's radius x
    damageHitRadiusScale, the longest over the attack's hitboxes. REGULATION for every number
    (regulation_reach). INFERRED: that each next dummy poly sits at the end of the previous segment
    and that the WeaponParam scales multiply the row's radius and length -- the 2200 handler copies a
    radius and a length scale into each hitbox it opens (EXE sHitboxData +0x8/+0xc, parseDamageTae
    0x1403269e0), but where they are filled from was not traced. It is the hitbox's extent from the
    weapon's dummy poly 100, not the distance from the attacker: the swing's own travel is in the
    animation, which is not read. None without the shape data."""
    g = "Single2Hand" if two_hand else "Single1Hand"
    a = next((x for x in (attacks.get((norm(nm), g + "Normal1st")) for nm in names) if x), None)
    if not a or not data.hit_shape:
        return None
    rs, ls = data.reach_scale.get(weapon, (1.0, 1.0))
    best = None
    for h in live_hits(a):
        row, rows, segs, used = h.get("dmg"), set(), [], set()
        while row and row not in rows and str(row) in data.hit_shape:
            rows.add(row)
            ptype, pid, rad, ln, child = data.hit_shape[str(row)]
            if ptype == 1 and pid >= 100 and pid not in used:
                used.add(pid)
                segs.append((rad, ln))
            row = child
        if segs:
            r = sum(ln for _, ln in segs) * ls + segs[-1][0] * rs
            best = r if best is None else max(best, r)
    return None if best is None else round(best, 3)


def r1_metrics(data: Data, attacks: dict, weapon: str, names: list[str], two_hand: bool, ar: dict, dfn: dict,
               anim_len=anim_length) -> dict:
    """What the --window score leaves out, for the R1 of the grip the row uses (seconds of game time,
    animation time over the attack's mean play speed as chain_timeline divides it):

    * reach_m: r1_reach.
    * startup_s: the first chain attack's first live hitbox frame (TAE 2200 start).
    * recovery_s: from its last live hitbox frame to the end of the animation (anim_length), the
      commitment of one R1 nothing cancels. INFERRED: that the player is free when the animation
      ends; a roll or block cancel earlier than that is not read (the 111500 window is the INFERRED
      chain input, and its EXE reader was not found).
    * time_to_first_hit_s: the first hit of the R1 chain's timeline, which is startup_s unless the
      first chain attack opens with a dead hitbox.
    * damage_per_5s: the --window score over SUSTAIN_HORIZON seconds: the R1 chain repeated
      (chain_timeline, each next attack at max(111500, its hitbox's close)), every hit landed by then
      against `dfn`, times WeaponParam.damageScale.

    A value the data cannot give is None."""
    out = {"reach_m": r1_reach(data, attacks, weapon, names, two_hand), "startup_s": None, "recovery_s": None,
           "time_to_first_hit_s": None, "damage_per_5s": None}
    g = "Single2Hand" if two_hand else "Single1Hand"
    a = next((x for x in (attacks.get((norm(nm), g + "Normal1st")) for nm in names) if x), None)
    hs = live_hits(a) if a else []
    if hs:
        spd = sum(a.get("spd") or [1.0, 1.0]) / 2
        out["startup_s"] = round(min(h["start"] for h in hs) / 30 / spd, 3)
        end = anim_len(a["anim"])
        if end is not None:
            out["recovery_s"] = round(max(0.0, end - max(h["end"] for h in hs) / 30) / spd, 3)
    tl = next((t for t in (chain_timeline(attacks, nm, two_hand, "Normal", SUSTAIN_HORIZON) for nm in names) if t),
              None)
    if tl:
        out["time_to_first_hit_s"] = round(tl[0][0], 3)
        out["damage_per_5s"] = round(sum(hit_damage(ar, dfn, mv, ty, lo, fl) for t, mv, ty, lo, fl in tl
                                         if t <= SUSTAIN_HORIZON) * data.damage_scale.get(weapon, 1.0), 1)
    return out


#: The WeaponStaminaCostParam field an R1 chain attack's cost reads, by (two-handed, the chain's
#: 2nd attack): chain_timeline alternates the 1st and 2nd attacks, and so does the cost.
R1_COST_FIELD = {(False, False): "Nnormal1st1H", (False, True): "normal2nd1H",
                 (True, False): "normal1st2H", (True, True): "normal2nd2H"}
#: The regulation's stamina is ten times the menu's: staminaMax / 10 is the max stamina SoulsPlanner
#: and the menu show (docs/DS2-BUILD-MECHANICS.md section 4). Every stamina number printed here is in
#: menu points; a ratio of two regulation numbers does not depend on it.
STAMINA_UNIT = 10.0
#: Seconds of R1 chain read for a full bar (stamina_metrics): the cheapest R1 at the largest bar
#: is about 25 attacks, and an R1 takes at least 0.3 s.
BAR_HORIZON = 30.0


def attack_stamina(data: Data, weapon: str, two_hand: bool, second: bool) -> float | None:
    """Menu stamina one R1 chain attack costs: WeaponParam.meleeAttackBaseCost x the
    WeaponStaminaCostParam row its costCategoryId names, at the slot's field (R1_COST_FIELD), / 10.
    The values are REGULATION (scripts/ds2-stamina-evidence.py prints them); that the game multiplies
    the two is INFERRED from the field names (the multiplier's JP name is "attack stamina
    multiplier, one-handed weak 1"), not traced: the attack's hitbox setter, where the EXE hands a
    per-hitbox stamina amount to drainStamina, is behind an Arxan stub (docs/DS2-DPS-MECHANICS.md
    "Stamina"). None when the regulation was not read for this weapon."""
    c = data.stamina_cost.get(weapon)
    if not c:
        return None
    return c["melee"] * c["rates"][R1_COST_FIELD[(two_hand, second)]] / STAMINA_UNIT


def shot_stamina(data: Data, weapon: str) -> float | None:
    """Menu stamina one shot of a bow, greatbow or crossbow costs: WeaponParam.rangedAttackBaseCost
    / 10, no category multiplier (INFERRED from the field name; which WeaponStaminaCostParam field a
    shot reads, if any, is not read -- every launcher names category 10, whose fields are the melee
    attacks', and category 20 "bow", which no weapon names, is 1.0 one-handed)."""
    c = data.stamina_cost.get(weapon)
    return c["ranged"] / STAMINA_UNIT if c else None


def max_stamina(data: Data, st: dict, rings=()) -> float | None:
    """Menu max stamina of a build: PhysicalStatsPerLevelStatValuesParam.staminaMax at the effective
    END (the worn rings' stat adds, gear_stats), x the worn rings' stamina factors (ring_factor; the
    product is SoulsPlanner's, how two combine is not established), / 10. None without the table."""
    if not data.stamina_max:
        return None
    end = gear_stats(data, st, rings)["endurance"]
    return data.stamina_max[min(end, len(data.stamina_max) - 1)] * ring_factor(data, rings, "stamina") / STAMINA_UNIT


def bar_attacks(costs, bar: float) -> int:
    """How many attacks of `costs` (in order) begin from a full bar of `bar`: one begins while
    stamina is above 0 and may take it below. EXE: drainStamina (0x1403335d0) does not refuse a
    drain that crosses 0, it sets stamina to the drained value clamped to [ChrParam staminaLoanMax,
    staminaLoanMin] = [-150, -25] (REGULATION, ChrParam row 100) and starts the out-of-stamina
    timer; that an attack is refused only at or below 0 is INFERRED from that. No regeneration
    between attacks: every one of the 372 player animations with a TAE 2200 hitbox has a TAE 101100
    window, and its handler (parseStaminaTae 0x1403287b0) holds disable_stamina_regen while the
    window is open (EXE); that one attack's window reaches the next attack's is INFERRED."""
    n, left = 0, bar
    for c in costs:
        if left <= 0 or not c or c <= 0:
            break
        left -= c
        n += 1
    return n


def stamina_metrics(data: Data, weapon: str, two_hand: bool, names: list[str], attacks: dict, ar: dict, dfn,
                    window: float, st: dict, rings=()) -> dict:
    """The stamina side of a melee weapon's R1 chain at attack `ar` against `dfn` (hit_damage per
    hit, x damageScale), as best_weapon_row's window score counts it. Keys:

    * `stamina_per_attack`: the chain's 1st and 2nd attack (attack_stamina).
    * `stamina_per_window`: the attacks whose hits land within `window` seconds -- the hits the
      window score counts, so `damage_per_stamina` is that score over what it cost.
    * `max_stamina` (max_stamina), and from a full bar of it: `bar_attacks` (bar_attacks),
      `bar_damage` (every hit of those attacks) and `bar_seconds` (when the last one begins).
    Empty when the weapon has no R1 timing or no stamina cost; the bar keys are left out without
    the staminaMax table."""
    tl = next((t for t in (chain_timeline(attacks, nm, two_hand, "Normal", BAR_HORIZON, with_start=True)
                           for nm in names) if t), None)
    if not tl or weapon not in data.stamina_cost:
        return {}
    scale = data.damage_scale.get(weapon, 1.0)
    by = {}
    for t, mv, ty, lo, t0, fl in tl:
        by.setdefault(t0, []).append((t, hit_damage(ar, dfn, mv, ty, lo, fl) * scale))
    starts = sorted(by)
    cost = [attack_stamina(data, weapon, two_hand, i % 2 == 1) for i in range(len(starts))]
    hits = [by[t0] for t0 in starts]
    win = [i for i, h in enumerate(hits) if any(t <= window for t, _ in h)]
    spent = sum(cost[i] for i in win)
    dealt = sum(d for h in hits for t, d in h if t <= window)
    out = {"stamina_per_attack": [round(c, 2) for c in cost[:2]], "stamina_per_window": round(spent, 2),
           "damage_per_stamina": round(dealt / spent, 3) if spent else 0.0}
    bar = max_stamina(data, st, rings)
    if bar:
        n = bar_attacks(cost, bar)
        out.update(max_stamina=round(bar, 1), bar_attacks=n,
                   bar_damage=round(sum(d for h in hits[:n] for _, d in h), 1),
                   bar_seconds=round(starts[n - 1], 2) if n else 0.0)
    return out


def shot_metrics(data: Data, weapon: str, shot: float, st: dict, rings=()) -> dict:
    """stamina_metrics for one launcher shot of damage `shot` (shot_stamina; ranged_value's per
    shot damage, so a window is one shot). Empty without a cost."""
    cost = shot_stamina(data, weapon)
    if not cost:
        return {}
    out = {"stamina_per_attack": [round(cost, 2)], "stamina_per_window": round(cost, 2),
           "damage_per_stamina": round(shot / cost, 3)}
    bar = max_stamina(data, st, rings)
    if bar:
        n = bar_attacks(itertools.repeat(cost), bar)
        out.update(max_stamina=round(bar, 1), bar_attacks=n, bar_damage=round(n * shot, 1))
    return out


#: --rank: what best_weapon_row's score is, for the damage objective. "window" (the default) the
#: R1 hits landed within --window; "per-stamina" those hits' damage over the stamina their attacks
#: cost; "bar" the damage of the R1 chain a full bar of the build's own max stamina pays for.
RANKS = {"window": None, "per-stamina": "damage_per_stamina", "bar": "bar_damage"}


HYPERARMOR =Path.home() / ".cache/ds2-builds/hyperarmor.json"  # weapon name -> WeaponParam.uninterruptibleRate
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


#: ChrParam row 100 poiseRecoveryValue (REGULATION): what the poise regen 0x140145ba0 adds per dt
#: (EXE); that dt is seconds is INFERRED. Regen stops while staggered and poise refills to max after.
POISE_REGEN = 0.56
#: TAE event that copies WeaponParam.uninterruptibleRate into status+0x7d0 for its window (EXE 0x140326884)
TAE_HYPERARMOR = 111900


def tae_windows(anim: int, event: int) -> list[tuple[float, float]]:
    """(start, end) in anim frames at 30 fps of every `event` in TAE animation `anim`; none without the XML."""
    p = TAE_DIR / f"anim-{anim:09d}.xml"
    if not p.exists():
        return []
    ev = re.findall(r"<type>(\d+)</type>.*?<startTime>([-\d.]+)</startTime>\s*<endTime>([-\d.]+)</endTime>",
                    p.read_text(), re.S)
    return [(float(s) * 30, float(e) * 30) for ty, s, e in ev if ty == str(event)]


def hyperarmor_cover(a: dict, windows: list[tuple[float, float]] | None = None) -> float:
    """The fraction of an attack's windup and active frames (frame 0 to its last live hitbox's
    end) inside a hyperarmor window (TAE 111900; `windows` overrides the TAE read). TAE frames."""
    end = max((h["end"] for h in live_hits(a)), default=0.0)
    if end <= 0:
        return 0.0
    if windows is None:
        windows = tae_windows(a["anim"], TAE_HYPERARMOR)
    return min(1.0, sum(max(0.0, min(e, end) - max(s, 0.0)) for s, e in windows) / end)


def stagger_hits(hits: list[tuple[float, float, int]], poise: float, regen: float = POISE_REGEN) -> int | None:
    """How many of `hits` ((seconds, poise damage, DamageCtrlParam.armorBreak), in time order) it
    takes to stagger a defender with `poise` who is not in hyperarmor; None when they never do.
    EXE (stagger decision 0x140136570): a hit staggers when the defender's poise is <= 0 after it
    (0x1401459d0), or its armorBreak is 2, or 1 while the defender has no hyperarmor. Poise
    regenerates `regen` per second between hits (POISE_REGEN, dt INFERRED seconds)."""
    cur, last = poise, 0.0
    for n, (t, pd, ab) in enumerate(hits, 1):
        cur = min(poise, cur + regen * (t - last)) - pd
        last = t
        if ab or cur <= 0:
            return n
    return None


def ring_poise(data: Data) -> dict:
    """Ring key -> poise it adds (Ring of Giants +10/+20/+30): mechanics.json, decoded from
    SpEffectRing.emevd (REGULATION, docs/DS2-BUILD-MECHANICS.md). That max poise adds it via the
    status ints 0x14037fdd0's caller sums is INFERRED (docs/DS2-DPS-MECHANICS.md section 2)."""
    out = getattr(data, "_ring_poise", None)
    if out is None:
        out = {}
        for r in json.loads(MECHANICS.read_text())["rings"] if MECHANICS.exists() else []:
            key = data.sp_key.get(norm(r["name"]))
            v = sum(x["value"] for x in r["effects"] if x["kind"] == "add" and x["stat"] == "poise")
            if key in data.rings and v:
                out[key] = v
        data._ring_poise = out
    return out


def lack_of_stats(req: dict, stats: dict) -> float:
    """The share of a piece's poise lost to unmet requirements (EXE 0x1403811f0 over
    PlayerLackOfStatsParam row 28 = [30, 80, 0, 30]): v = clamp(sum of max(0, 1 - stat/req) over the
    piece's requirements, 0, 1) (0x14034d150), then clamp((30 + 50 v) / 100, 0, 1) for v > 0, 0 at
    v == 0. So a piece worn under its requirements keeps 0.7 down to 0.2 of its poise."""
    v = min(1.0, sum(max(0.0, 1 - stats.get(s, 0) / r) for s, r in req.items() if r > 0))
    return min(1.0, max(0.0, (30 + 50 * v) * 0.01)) if v > 0 else 0.0


def armor_poise(data: Data, armor, rings=(), stats: dict | None = None) -> float:
    """A build's max poise (EXE 0x14037fdd0, docs/DS2-DPS-MECHANICS.md section 2): its four pieces'
    poise (ArmorParam.strong once regulation_poise ran), each times 1 - lack_of_stats at `stats`,
    plus the stat poise at min(END, ADP) (data.stat_poise), plus its rings'. Without `stats` the
    pieces count in full and there is no stat term."""
    rp = ring_poise(data)
    total = 0.0
    for s, p in zip(ARMOR_SLOTS, armor):
        piece = data.armor[s].get(p) or {}
        lost = lack_of_stats(piece.get("require") or {}, stats) if stats else 0.0
        total += piece.get("poise", 0) * (1 - lost)
    if stats and getattr(data, "stat_poise", None):
        total += data.stat_poise[min(stats["endurance"], stats["adaptability"], len(data.stat_poise) - 1)]
    return total + sum(rp.get(r, 0) for r in rings)


def r1_attack(attacks: dict, names: list[str], two_hand: bool) -> dict | None:
    g = ("Single2Hand" if two_hand else "Single1Hand") + "Normal1st"
    return next((a for a in (attacks.get((norm(nm), g)) for nm in names) if a), None)


def hit_poise(data: Data, weapon: str, row) -> tuple[float, int] | None:
    """(poise damage to a player, armorBreak) of one hit of `weapon` on PlayerDamageParam `row`:
    DamageCtrlParam.poiseDamage x WeaponParam.poiseDamageScalePlayer (EXE 0x140139160 with its
    other terms at their neutral values: point blank, defender-side table term 0, hit +0x70 0 and
    +0x90 absent -- those three are not identified). None without the regulation rows."""
    w, h = data.weapon_poise.get(weapon), data.hit_poise.get(str(row))
    if w is None or h is None:
        return None
    return h[0] * w[0], h[1]


def bracket_poise(data: Data, corpus: list[Build], sl: int, attacks: dict, defender=None) -> dict:
    """The defender poise_metrics staggers and the counter-hits it weighs hyperarmor against, at
    `sl`'s bracket (bracket_builds): `poise` the mean max poise (armor_poise) of the bracket's
    builds, or of the `defender` pieces; `counters` (poise damage, armorBreak) of each build's
    melee weapon's (adoption's pick) 1H R1 first hit; `n` the builds."""
    key = (id(data), id(corpus), sl_bracket(sl), tuple(defender) if defender else None)
    memo = _bracket_poise_memo.get(key)
    if memo is not None and memo[0] is data and memo[1] is corpus:
        return memo[2]
    builds = bracket_builds(data, corpus, sl)
    poise = (armor_poise(data, defender) if defender else
             float(np.mean([armor_poise(data, b.armor, b.rings, b.stats) for b in builds])) if builds else 0.0)
    counters = [hp for hp in (build_counter(data, attacks, b) for b in builds) if hp]
    out = {"poise": poise, "counters": counters, "n": len(builds)}
    _bracket_poise_memo[key] = (data, corpus, out)
    return out


#: The Stone Ring's flat poise damage per hit: SpEffect `1000[0]` kind 2 in SpEffectRing.emevd,
#: event 40230000, value 30 (REGULATION), carried as the hit's s8 +0x70 and added before the
#: hyperarmor and distance multipliers (EXE 0x140139160; docs/DS2-DPS-MECHANICS.md section 2).
STONE_RING_POISE = 30


def wears_stone_ring(data: Data, rings) -> bool:
    return any((data.rings.get(r) or {}).get("name", r) == "Stone Ring" for r in rings)


def build_counter(data: Data, attacks: dict, b: Build) -> tuple[float, int] | None:
    """The counter-hit bracket_poise weighs hyperarmor against for one build: (poise damage,
    armorBreak) of its melee weapon's (adoption's pick) 1H R1 first hit, plus STONE_RING_POISE
    when the build wears a Stone Ring; None without one."""
    w = next((w for w in (b.hands[HAND_SLOTS.index(s)][0] for s in MELEE_ORDER)
              if w not in EMPTY and w in data.weapons and not data.weapons[w].get("isShield")
              and not CATALYST.search(w) and w not in data.ranged), None)
    a = r1_attack(attacks, [data.weapons[w]["name"], w.replace("_", " ")], False) if w else None
    hs = live_hits(a) if a else []
    hp = hit_poise(data, w, hs[0].get("dmg")) if hs else None
    if hp and wears_stone_ring(data, b.rings):
        hp = (hp[0] + STONE_RING_POISE, hp[1])
    return hp


_bracket_poise_memo: dict = {}


def poise_metrics(data: Data, corpus: list[Build], attacks: dict, weapon: str, two_hand: bool, sl: int,
                  defender=None) -> dict:
    """Hyperarmor and poise of `weapon`'s R1 chain in the grip, against `sl`'s bracket defender
    (bracket_poise), for --best-weapons' `metrics`:
    `hyperarmor` the share of the R1's windup+active frames in its hyperarmor window
    (hyperarmor_cover; 0 when WeaponParam.uninterruptibleRate is 0, as then the window does
    nothing); `hyperarmor_rate` that rate, the factor on poise damage the attacker takes inside the
    window (EXE ChrDamageActionCtrl 0x140137aa0: status+0x7d0 > 0 multiplies it);
    `hyperarmor_holds` the share of the bracket's counter-hits (1H R1s) that do not stagger the
    attacker inside the window -- armorBreak 2 always does, others when poise damage x rate reaches
    the attacker's poise, taken as the bracket's mean (the attacker's armour is not chosen here);
    None without hyperarmor; `poise_damage_per_hit` the R1's first hit's (hit_poise);
    `armor_break` that hit's DamageCtrlParam.armorBreak (1 and 2 stagger regardless of poise);
    `hits_to_stagger` R1 chain hits until the bracket defender staggers (stagger_hits), None if
    not within 10 s; `defender_poise` the bracket defender's poise. All None for a launcher or a
    weapon without attack timing or regulation rows."""
    w = data.weapons[weapon]
    names = [w["name"], weapon.replace("_", " ")]
    a = None if weapon in data.ranged else r1_attack(attacks, names, two_hand)
    br = bracket_poise(data, corpus, sl, attacks, defender)
    out = dict.fromkeys(["hyperarmor", "hyperarmor_rate", "hyperarmor_holds", "poise_damage_per_hit",
                         "armor_break", "hits_to_stagger"]) | {"defender_poise": round(br["poise"], 1)}
    tl = next((t for t in (chain_timeline(attacks, nm, two_hand, "Normal", 10.0, with_row=True) for nm in names)
               if t), None) if a else None
    hits = [(t[0], *hp) for t in tl or [] if (hp := hit_poise(data, weapon, t[-1]))]
    if not hits:
        return out
    rate = data.weapon_poise[weapon][1]
    cover = hyperarmor_cover(a) if rate > 0 else 0.0
    out.update(hyperarmor=round(cover, 3), hyperarmor_rate=rate, poise_damage_per_hit=round(hits[0][1], 2),
               armor_break=hits[0][2], hits_to_stagger=stagger_hits(hits, br["poise"]))
    if cover and br["counters"]:
        out["hyperarmor_holds"] = round(sum(ab != 2 and pd * rate < br["poise"] for pd, ab in br["counters"])
                                        / len(br["counters"]), 3)
    return out


# --- The R1 trade: what poise buys a build (generate_armor) ----------------------------------

#: sCharacterFlags+0x200, the EXE's factor on startPlaySpeed inside an event-150 window. Not read:
#: both writers found store 0.0, and taken literally the windup would never play, so 1.0 -- the
#: value under which the start speed is the start speed (docs/DS2-DPS-MECHANICS.md "Attack play
#: speed"). Set here only; scripts/ds2-poise-need.py reads it from this module.
START_FACTOR = 1.0
#: Per animation, its event-150/151 play-speed windows in seconds: scripts/ds2-anibnd.py windows.
SPEED_WINDOWS = Path.home() / ".cache/ds2-builds/attack-speed-windows.json"
_speed_cache: dict = {}


def game_time(table: dict, anim: int, spd: list, t: float) -> float:
    """Seconds of play to reach animation time `t` (seconds) of `anim`: the rate is startPlaySpeed
    x START_FACTOR while a 150 window is open (it wins when both are, as in 0x14035d580),
    endPlaySpeed while a 151 is, else 1.0. An animation `table` has no windows for plays at 1.0."""
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


def speed_windows() -> dict | None:
    """SPEED_WINDOWS, read once; None when it has not been written."""
    if "table" not in _speed_cache:
        try:
            _speed_cache["table"] = json.loads(SPEED_WINDOWS.read_text())
        except (OSError, ValueError):
            _speed_cache["table"] = None
    return _speed_cache["table"]


def attack_seconds(att: dict, frame: float, table: dict | None) -> float:
    """Seconds from input to TAE frame `frame` of attack `att`: through its play-speed windows
    (game_time) when `table` is given, else at the mean of its start/end speeds (chain_timeline's)."""
    spd = att.get("spd") or [1.0, 1.0]
    if table is None:
        return frame / 30 / (sum(spd) / 2)
    return game_time(table, att["anim"], spd, frame / 30)


def first_strike(data: Data, attacks: dict, weapon: str, two_hand: bool, table: dict | None) -> dict | None:
    """`weapon`'s R1 earliest live hit in the grip, for the trade model (trade): `t` seconds from
    input (attack_seconds), `pd` poise damage and `ab` armorBreak (hit_poise), hit_damage's terms
    `mv`/`kind`/`lower`/`flat`, `scale` WeaponParam.damageScale, and `armor` the R1's hyperarmor
    windows (TAE 111900) in seconds with `rate` its uninterruptibleRate (no windows when the rate is
    0: then the window does nothing). A grip with no R1 row swings one-handed. None without attack
    timing or poise rows."""
    names = [data.weapons[weapon]["name"], weapon.replace("_", " ")]
    for two in dict.fromkeys((two_hand, False)):
        a = r1_attack(attacks, names, two)
        hs = live_hits(a) if a else []
        if hs:
            break
    else:
        return None
    h = min(hs, key=lambda h: h["start"])
    hp = hit_poise(data, weapon, h.get("dmg"))
    if not hp:
        return None
    rate = data.weapon_poise.get(weapon, (0, 0.0))[1]
    wins = [(attack_seconds(a, s, table), attack_seconds(a, e, table))
            for s, e in tae_windows(a["anim"], TAE_HYPERARMOR)] if rate else []
    return {"t": attack_seconds(a, h["start"], table), "pd": hp[0], "ab": hp[1], "mv": h["rate"],
            "kind": h.get("type") or "physical", "lower": h.get("lower", 0), "flat": h.get("flat") or NO_FLAT,
            "scale": data.damage_scale.get(weapon, 1.0), "armor": wins, "rate": rate}


def stagger_threshold(hit: dict, defender: dict) -> float | None:
    """The poise damage `hit` (first_strike) deals `defender` (the first_strike it is swinging) at
    hit["t"]: times the defender's uninterruptibleRate inside its hyperarmor window. None when the
    hit staggers whatever the poise: armorBreak 2, or 1 outside hyperarmor (EXE 0x140136570). The
    defender staggers when the result reaches its poise (poise <= 0 after the hit)."""
    armored = any(s <= hit["t"] < e for s, e in defender["armor"])
    if hit["ab"] == 2 or (hit["ab"] == 1 and not armored):
        return None
    return hit["pd"] * (defender["rate"] if armored else 1.0)


def trade(mine: dict, theirs: dict, my_poise: float, their_poise: float) -> tuple[bool, bool]:
    """One R1 trade, both pressing R1 at the same instant: (my hit lands, theirs lands). The hit
    that lands first lands; the later one lands unless the first staggered its swinger
    (stagger_threshold against that swinger's poise and hyperarmor). A tie counts as mine first."""
    if theirs["t"] < mine["t"]:
        th = stagger_threshold(theirs, mine)
        return th is not None and th < my_poise, True
    th = stagger_threshold(mine, theirs)
    return True, th is not None and th < their_poise


def trade_counters(data: Data, corpus: list[Build], sl: int, attacks: dict) -> list[dict]:
    """The bracket's counter-hits for the trade model: per bracket build (bracket_builds) with a
    melee weapon (build_counter's pick), that weapon's first_strike in the build's grip -- two-handed
    when its grip is 1 and the weapon is in rh1, the slot it two-hands (ds2-poise-need.py
    --counter-grip build) -- plus STONE_RING_POISE with a Stone Ring, with the build's own `poise`
    (armor_poise at its stats), `dfn` (build_defense, its rings as worn) and `ar` (attack_rating at
    its effective stats and rings; an infusion the record does not know as No_Infusion). Cached on
    `data` per corpus, bracket and timing."""
    table = speed_windows()
    key = (id(corpus), sl_bracket(sl), table is not None)
    cache = data.__dict__.setdefault("_trade_counters", {})
    if key in cache:
        return cache[key]
    out = [hit for hit in (trade_counter(data, attacks, b, table) for b in bracket_builds(data, corpus, sl))
           if hit]
    cache[key] = out
    return out


def trade_counter(data: Data, attacks: dict, b: Build, table: dict | None) -> dict | None:
    """trade_counters' counter-hit for one build `b` at timing `table` (speed_windows); None when
    it has no melee weapon with a first_strike, or its stats cannot be read. What --export-backend
    writes per corpus build (the XT record), so the Rust port takes the bracket's from the file."""
    melee = lambda w: (w not in EMPTY and w in data.weapons and not data.weapons[w].get("isShield")
                       and not CATALYST.search(w) and w not in data.ranged)
    pick = next(((s, *b.hands[HAND_SLOTS.index(s)]) for s in MELEE_ORDER
                 if melee(b.hands[HAND_SLOTS.index(s)][0])), None)
    if not pick:
        return None
    slot, w, inf = pick
    hit = first_strike(data, attacks, w, b.grip == 1 and slot == "rh1", table)
    worn = [r for r in b.rings if r in data.rings]
    try:
        eff = gear_stats(data, effective(data, b), worn)
    except KeyError:
        return None
    if not hit:
        return None
    if wears_stone_ring(data, b.rings):
        hit["pd"] += STONE_RING_POISE
    infs = data.weapons[w]["infusions"]
    inf = inf if inf in infs else "No_Infusion" if "No_Infusion" in infs else next(iter(infs))
    hit.update(poise=armor_poise(data, b.armor, b.rings, b.stats), dfn=build_defense(data, b),
               ar=attack_rating(data, w, inf, eff, worn))
    return hit


def exchange_pairs(mine: list[dict], counters: list[dict]) -> dict | None:
    """Every (my weapon, counter) R1 trade (trade) reduced to what the armour changes: my poise
    decides only whether I land when the counter hits first, and my defense only what its hits deal.
    `mine` are first_strikes with `ar`, the counters trade_counters'. Per pair, my hit's damage is
    hit_damage against the counter's own defense. Returns `fixed` (my damage in pairs where I hit
    first), `keys`/`cum` (pairs where the counter hits first and poise could hold: the stagger
    thresholds sorted, and the running sum of my damage over them), `taken` per counter (how many of
    my weapons it lands on), `pairs`, `first` (pairs I hit first). None without either side."""
    if not mine or not counters:
        return None
    fixed, first, held, taken = 0.0, 0, [], [0] * len(counters)
    for w in mine:
        for i, c in enumerate(counters):
            out = hit_damage(w["ar"], c["dfn"], w["mv"], w["kind"], w["lower"], w["flat"]) * w["scale"]
            if c["t"] < w["t"]:
                taken[i] += 1
                th = stagger_threshold(c, w)
                if th is not None:
                    held.append((th, out))
            else:
                fixed += out
                first += 1
                taken[i] += trade(w, c, 0.0, c["poise"])[1]
    held.sort(key=lambda x: x[0])
    cum = [0.0]
    for _, out in held:
        cum.append(cum[-1] + out)
    return {"fixed": fixed, "first": first, "keys": [th for th, _ in held], "cum": cum, "taken": taken,
            "counters": counters, "pairs": len(mine) * len(counters)}


def exchange_value(table: dict, poise: float, dfn: dict) -> tuple[float, float]:
    """(exchange value, trade rate) of wearing `poise` and defense `dfn` over exchange_pairs'
    `table`: the mean over its pairs of my damage when my hit lands less the counter's first-hit
    damage against `dfn` (hit_damage at its attack rating) when its hit lands; and the share of
    pairs where mine lands. Poise holds against a threshold strictly below it."""
    k = bisect.bisect_left(table["keys"], poise)
    taken = sum(n * hit_damage(c["ar"], dfn, c["mv"], c["kind"], c["lower"], c["flat"]) * c["scale"]
                for n, c in zip(table["taken"], table["counters"]) if n)
    return (table["fixed"] + table["cum"][k] - taken) / table["pairs"], (table["first"] + k) / table["pairs"]


def armor_poise_steps(data: Data, b: Build, eff: dict, mix: dict, scarcity: float = 0.0) -> list[tuple[float, tuple]]:
    """best_armor's pick at every armour poise its budget reaches: for each distinct sum P of four
    wearable pieces' poise under the same load budget, the set best_armor's score (threat-mix
    defense less `scarcity`'s load price, the price taken from best_armor's own top set) ranks first
    among sets whose pieces' poise is at least P. As (pieces' poise, pieces), poise rising, each set
    once. Per slot only pieces no lighter-or-equal piece matches in defense and poise both count;
    the four slots are combined keeping, per poise sum, the sets no lighter one outscores."""
    worn = [r for r in b.rings if r in data.rings]
    _, _, top = best_armor(data, b, dict(eff), mix, top=1)
    if not top:
        return []
    price = top[0][0] / top[0][1] if top[0][1] > 0 else 0.0
    k = scarcity * LOAD_PRICE * price
    budget = max_load(data, eff, worn) * EQUIP_CAP
    budget -= sum(data.weapons.get(w, {}).get("weight", 0) for w, _ in b.weapons())
    budget -= sum(data.rings.get(r, {}).get("weight", 0) for r in b.rings if r in data.rings)
    st = gear_stats(data, eff, worn)

    def value(p):
        return sum(mix[t] * p.get(t + "DEF", 0) for t in DMG)

    states = {0.0: [(0.0, 0.0, ())]}  # pieces' poise -> [(weight, defense value, pieces)]
    for slot in ARMOR_SLOTS:
        cands = sorted(((v.get("weight", 0), value(v), v.get("poise", 0), key) for key, v in data.armor[slot].items()
                        if armor_ok(data, slot, key, st)), key=lambda t: (t[0], -t[1], -t[2], t[3]))
        front = []
        for c in cands:
            if not any(f[1] >= c[1] and f[2] >= c[2] for f in front):
                front.append(c)
        nxt: dict = {}
        for p, lst in states.items():
            for w, v, pp, key in front:
                for sw, sv, pcs in lst:
                    if sw + w <= budget:
                        nxt.setdefault(round(p + pp, 3), []).append((sw + w, sv + v, pcs + (key,)))
        states = {}
        for p, lst in nxt.items():
            lst.sort(key=lambda s: (s[0], -(s[1] - k * s[0])))
            lean, best = [], None
            for s in lst:
                if best is None or s[1] - k * s[0] > best:
                    lean.append(s)
                    best = s[1] - k * s[0]
            states[p] = lean
    rank = lambda s: (s[1] - k * s[0], s[1], s[0], s[2])
    out, run = [], None
    for p in sorted(states, reverse=True):
        s = max(states[p], key=rank)
        if run is None or rank(s) > rank(run[1]):
            run = (p, s)
            out.append((p, s[2]))
    return sorted(out)


def granted_trades(data: Data, corpus: list[Build], sl: int, stats: dict, rings: list[str], granted) -> dict | None:
    """exchange_pairs for a build at `stats` wearing `rings`, swinging each of `granted` ((key,
    infusion, two-handed)) against `sl`'s bracket counters (trade_counters): each weapon's
    first_strike at its attack_rating there, plus STONE_RING_POISE when `rings` hold a Stone Ring.
    A launcher, or a weapon without attack timing or poise rows, is left out. Adds `basis`, what was
    weighed. None when nothing is left on either side."""
    # read once per `data`: about a second, and generate_build asks per build (first_strike only reads it)
    attacks = data.__dict__.get("_trade_attacks")
    if attacks is None:
        attacks = data._trade_attacks = load_attacks(data)
    timing = speed_windows()
    eff = gear_stats(data, dict(stats), rings)
    stone = wears_stone_ring(data, rings)
    mine = []
    for key, winf, wtwo in granted:
        hit = None if key in data.ranged else first_strike(data, attacks, key, wtwo, timing)
        if hit:
            hit["pd"] += STONE_RING_POISE if stone else 0
            hit["ar"] = attack_rating(data, key, winf, eff, rings)
            mine.append(hit)
    table = exchange_pairs(mine, trade_counters(data, corpus, sl, attacks))
    if table is not None:
        table["basis"] = (f"{len(mine)} of {len(granted)} granted weapons x {len(table['counters'])} SL "
                          f"bracket counter-hits, " + ("game play speed" if timing is not None else
                                                       f"mean play speed (no {SPEED_WINDOWS.name})"))
    return table


def armor_trade(data: Data, table: dict, pieces, stats: dict, rings: list[str], two: bool) -> dict:
    """Armour `pieces` (keys) on a build at `stats` wearing `rings`, over granted_trades' `table`:
    `poise` (armor_poise at the ring-boosted stats), `poise_target` the largest counter poise damage
    it holds through (0 for none), `trade_rate` and `exchange` (exchange_value against its
    build_defense)."""
    poise = armor_poise(data, pieces, rings, gear_stats(data, dict(stats), rings))
    dfn = build_defense(data, Build("", stats, list(pieces), [], int(two), rings, []))
    val, rate = exchange_value(table, poise, dfn)
    k = bisect.bisect_left(table["keys"], poise)
    return {"poise": round(poise, 1), "poise_target": round(table["keys"][k - 1], 1) if k else 0.0,
            "trade_rate": round(rate, 3), "exchange": round(val, 2)}


ATTACK_TYPES =Path.home() / ".cache/ds2-builds/attack-type.json"  # per weapon slot: slash/strike/thrust per hit
DAMAGE_LOWER = Path.home() / ".cache/ds2-builds/damage-lower.json"  # PlayerDamageParam row -> damageLower


#: A hit whose damage row adds no flat attack: one zero per DMG type.
NO_FLAT = (0, 0, 0, 0, 0)


def load_attacks(data: Data | None = None) -> dict:
    """Attacks by (weapon, slot). Each hit gets its physical type from DamageCtrlParam.attackType
    (REGULATION, docs/DS2-DPS-MECHANICS.md) when the per-slot hit lists line up, and, given `data`
    that read the regulation, its damage row's flat attack as a tuple in DMG order when the row has
    any (regulation_hit_flat)."""
    types = {norm(k): v for k, v in json.loads(ATTACK_TYPES.read_text()).items()} if ATTACK_TYPES.exists() else {}
    lower = json.loads(DAMAGE_LOWER.read_text()) if DAMAGE_LOWER.exists() else {}
    flat = data.hit_flat if data is not None else {}
    by = {}
    for a in json.loads(ATTACKS.read_text()):
        tl = (types.get(norm(a["name"]), {}).get(a["slot"]) or {}).get("hits") or []
        if tl and len(tl) == len(a.get("hits") or []):
            for h, t in zip(a["hits"], tl):
                h["type"] = t
        for h in a.get("hits") or []:
            h["lower"] = lower.get(str(h.get("dmg")), 0)
            f = flat.get(str(h.get("dmg")))
            if f:
                h["flat"] = tuple(f.get(k, 0) for k in DMG)
        by.setdefault((norm(a["name"]), a["slot"]), a)
    return by


def hit_damage(ar: dict, dfn: dict, mv: float, kind: str = "physical", lower: int = 0,
               flat: tuple = NO_FLAT) -> float:
    """One hit's damage against a player (EXE, ChrDamageActionCtrl slot 32 0x140138d50; notes in
    docs/DS2-DPS-MECHANICS.md): physical max(AR*10 - DEF_type, lower) / 12, each element
    max(AR*6, lower) / 6 * (1 - cut), summed, times MV. DEF_type is the hit's slash/strike/thrust
    defense (general physical when it has none), `lower` is PlayerDamageParam.damageLower, and
    cut = min(0.99, (DEF + 100) / 1000); the +100 is the stat table's 10% resistance floor that the
    displayed defense leaves out (EXE, docs/DS2-DPS-MECHANICS.md "Elemental cut"). AR is the
    weapon's attack plus `flat`, the hit's damage row's flat attack per DMG type, which the attack
    builder adds before the defense (EXE, regulation_hit_flat): a type the weapon has no attack in
    still hits when the row adds some."""
    tot = 0.0
    for k, f in zip(DMG, flat):
        v = ar.get(k, 0) + f
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


def status_cut(data: Data, b: Build, status: str) -> float:
    """The share of `status` build-up build `b` resists (EXE, docs/DS2-DPS-MECHANICS.md "Status
    build-up per hit"): each worn piece's resistance x 0.01 (data.armor_status) plus the stat
    column x 0.01 at row trunc((3 x ADP + VIT or FTH) / 4) (STATUS_PROC; the rings' stat changes
    count, as build_defense counts them), clamped to [0, 1]. A ring or SpEffect that adds a status
    resistance (Ring of Resistance, the bite rings) is not counted: their type 5-9 defense changes
    are not decoded (regulation_ring_defense)."""
    eff = effective(data, b)
    for r in b.rings:
        for s, v in ring_effects(data).get(r, (0, {}, 1))[1].items():
            eff[s] = eff.get(s, 0) + v
    col = data.status_resist.get(status)
    if not col:
        return 0.0
    _, _, stat, _ = STATUS_PROC[status]
    i = (3 * eff["adaptability"] + eff[stat]) // 4
    cut = col[i if 1 <= i < len(col) else 1] * 0.01
    cut += sum((data.armor_status.get(s, {}).get(p) or {}).get(status, 0.0) for s, p in zip(ARMOR_SLOTS, b.armor)) * 0.01
    return min(1.0, max(0.0, cut))


def defender_status_cuts(data: Data, corpus: list[Build], sl: int, status: str, defender=None) -> list[float]:
    """status_cut of every defender defender_defense scores against at `sl`: the bracket's builds,
    or the --defender set at the bracket's median stats. Cached on `data`."""
    key = ("status", id(corpus), sl_bracket(sl), tuple(defender or ()), status)
    cache = data.__dict__.setdefault("_defender_cache", {})
    if key not in cache:
        if defender is None:
            builds = bracket_builds(data, corpus, sl)
        else:
            builds = [Build("", bracket_stats(data, corpus, sl)[0], list(defender), [], 0, [], [])]
        cache[key] = [status_cut(data, b, status) for b in builds] or [0.0]
    return cache[key]


def status_metrics(proc: dict, per_hit: float, cuts: list[float], hits: int, window: float) -> dict:
    """What `hits` landed in `window` seconds, each carrying `per_hit` gauge points of one status
    before resistance, do through its proc `proc` (data.status_procs) to defenders who resist
    `cuts` of it, averaged over the defenders.

    hits_to_proc (median over defenders): ceil(100 / (per_hit x (1 - cut))). The gauge starts empty
    and a proc sets it back to its floor, so the procing hit's surplus is lost (EXE 0x140146430) and
    every proc takes that many hits again. 0 when no defender procs at all.

    status_damage_per_window: the proc damage per window of a chain repeated without pause. After
    a proc the status is locked out for proc["lockout"] seconds (build-up is skipped while the
    victim is poisoned, or for 900210's 10 s after a bleed; INFERRED, regulation_status_procs), then
    takes hits_to_proc hits at the window's rate, window / hits seconds apiece. So a proc every
    lockout + hits_to_proc x window / hits seconds, and the window's share of it is
    proc damage x window / that cycle. Poison's damage lands over 22 s, not inside the window; the
    rate charges the window with its share of it. Build-up decay between hits is not modelled (its
    writer was not read); with the chain repeated, the gaps are the chain's own.

    status_damage_first_window: the proc damage those hits deal a defender whose gauge starts
    empty: proc damage when hits >= hits_to_proc, else 0, averaged over defenders."""
    if not proc or per_hit <= 0 or hits <= 0 or window <= 0:
        return {"hits_to_proc": 0, "status_damage_per_window": 0.0, "status_damage_first_window": 0.0}
    ks, rate, first = [], 0.0, 0.0
    for cut in cuts:
        b = per_hit * (1 - cut)
        k = math.ceil(100 / b - 1e-9) if b > 0 else math.inf
        ks.append(k)
        if k < math.inf:
            rate += proc["damage"] * window / (proc["lockout"] + k * window / hits)
            first += proc["damage"] if hits >= k else 0.0
    n = len(cuts)
    mid = sorted(ks)[(n - 1) // 2] if ks else math.inf
    return {"hits_to_proc": int(mid) if mid < math.inf else 0,
            "status_damage_per_window": rate / n, "status_damage_first_window": first / n}


def weapons_for(data: Data, stats: dict, sl: int, corpus: list[Build], top: int = 25, within: float = 0.10,
                raw_ar: bool = False, one_hand: bool = False, weapon_class: str | None = None, per_class: bool = False,
                window: float = 0.0, objective: str = "damage", weapon: str | None = None,
                every_infusion: bool = False, use_floors: bool = True, defender=None):
    """Weapons (per infusion) ranked by expected damage against the average defender at this SL,
    or with `objective` "bleed"/"poison" by status build-up: build-up per hit (objective_value, in
    gauge points before the victim's resistance; regulation_status) times the hits of the weapon's
    best R1/R2 chain attack (status_hits; within `window` seconds when given). Each hit carries the
    build-up its attack block holds (EXE, docs/DS2-DPS-MECHANICS.md "Status build-up per hit"); that
    a re-hit tick carries the whole of it again is INFERRED. The victim's resistance multiplies
    every weapon's build-up of one status alike, so it is left out; the proc's damage is not
    modelled.
    Usable only: requirements met (STR halved when that is what makes it usable, flagged 2H).
    Per weapon: the best infusion, plus the 2nd and 3rd only while within `within` of the best.
    `every_infusion`: every infusion of every weapon instead, and no `top` cut (infusion_gaps).
    `weapon` (a key): that weapon alone, every infusion, and no high-stamina END gate -- the
    question is which infusion, not whether to carry it (best_infusion). `use_floors` False
    (--no-floors) drops that END gate as well: it is a floor, not a game rule.
    `objective` "ar" is `raw_ar` by another name, so the Goal the panel shares between its tabs
    ranks the Weapons tab as it optimizes. It drops `window`: attack rating is one hit's, and the
    window counts hits of damage, so a window ranked "by AR" would be damage under the wrong name.
    `defender` is defender_defense's: None for the bracket's average defender, else four armour keys."""
    if objective == "ar":
        raw_ar, window = True, 0.0
    every_infusion = every_infusion or weapon is not None
    dfn, n = defender_defense(data, corpus, sl, defender)
    floors, r1, cut = build_floors(data, corpus, sl)
    attacks = load_attacks(data)
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
        if use_floors and not weapon and r1.get(key, 0) >= cut and stats["endurance"] < floors.get("endurance", 0):
            continue  # a high-stamina weapon needs END at the bracket median of builds that carry one
        lines = {}
        if key in data.ranged:
            # One trigger pull with the best ammunition per infusion (ranged_value), whatever
            # `window` is: the fire rate is not read, so a ranged row is one shot, not a window.
            # A bow fires two-handed (the 1H motions' damage rows have damageRate 0, REGULATION).
            obj = "ar" if raw_ar else objective
            grip = "2H" if data.ranged[key]["kind"] != "crossbow" else "1H" if one else "2H only"
            scored = []
            for inf in w["infusions"]:
                v, note, ar, _ = ranged_pick(data, key, inf, stats, obj, dfn)
                worn = dfn.respond(ar)[1] if isinstance(dfn, AdaptiveDefense) and obj == "damage" else []
                if v > 0:
                    scored.append((v, inf, {k: round(x) for k, x in ar.items()},
                                   ("" if note.startswith("2H special") else grip + " ") + f"1 shot: {note}"
                                   + (f" vs {' + '.join(worn)}" if worn else "")))
            scored.sort(key=lambda s: -s[0])
            for s in scored if every_infusion else [s for s in scored[:3] if s[0] >= scored[0][0] * (1 - within)]:
                rows.append((s[0], w["name"], *s[1:]))
            continue
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
                    lines["2H" if two_hand else "1H"] = [(mv, ty, lo, fl) for t, mv, ty, lo, fl in tl if t <= window]
            if not lines:
                skipped.append(w["name"])
                continue
        scored = []
        scale = data.damage_scale.get(key, 1.0)  # after the defense, so not in raw AR (regulation_damage_scale)
        for inf in w["infusions"]:
            ar = attack_rating(data, key, inf, stats)
            if not ar:
                continue
            # the defender's answer to this weapon's attack (AdaptiveDefense.respond: the counter
            # ring is chosen once from the weapon's attack ratings and worn for every hit)
            d_ar, worn = dfn.respond(ar) if isinstance(dfn, AdaptiveDefense) and not raw_ar else (dfn, [])
            if window:
                grip, mvs = max(lines.items(),
                                key=lambda g: sum(hit_damage(ar, d_ar, mv, ty, lo, fl) for mv, ty, lo, fl in g[1]))
                dmg = sum(hit_damage(ar, d_ar, mv, ty, lo, fl) for mv, ty, lo, fl in mvs) * scale
                label = f"{grip} {len(mvs)} hits" + ("" if one or grip == "1H" else " (2H only)")
            else:
                dmg = sum(ar.values()) if raw_ar else sum(damage(k, v, d_ar[k]) for k, v in ar.items()) * scale
                label = "1H" if one else "2H only"
            ha = hyperarmor(attacks, rates, w["name"], label.startswith("2H"))
            ctr = (crit.get(norm(w["name"])) or {}).get("counter")
            scored.append((dmg, inf, ar, label + (f" HA x{ha:g}" if ha else "") + (f" ctr x{ctr:g}" if ctr else "")
                           + (f" vs {' + '.join(worn)}" if worn else "")))
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
                  window: float = 0.0, objective: str = "damage", defender=None):
    """Every infusion `weapon` takes, best first, by weapons_for's own score at these stats and SL
    (the rows it ranks, uncut). Empty when the stats cannot wield it even two-handed, or for
    bleed/poison when no infusion deals it or the weapon has no attack timing. Full upgrade only:
    attack_rating reads the planner's max-level rows, so the upgrade level is not modelled."""
    return weapons_for(data, stats, sl, corpus, raw_ar=raw_ar, window=window, objective=objective, weapon=weapon,
                       defender=defender)


def infusion_gaps(data: Data, stats: dict, sl: int, corpus: list[Build], raw_ar: bool = False,
                  window: float = 0.0, objective: str = "damage", one_hand: bool = False,
                  weapon_class: str | None = None, top: int = 25, defender=None):
    """The reverse of best_infusion: across the weapon table, the weapons whose best infusion is
    furthest ahead of their runner-up at these stats. Rows: (margin, weapon name, best row,
    runner-up row, infusions scored), a row being weapons_for's (score, name, infusion, ar, grip)."""
    rows, dfn, n = weapons_for(data, stats, sl, corpus, raw_ar=raw_ar, one_hand=one_hand,
                               weapon_class=weapon_class, window=window, objective=objective,
                               every_infusion=True, defender=defender)
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


def best_weapons_jobs(data: Data, inf: str, weapon_class: str | None = None) -> list[str]:
    """The weapon keys best_weapons ranks for infusion `inf`: every weapon that takes it, shields,
    catalysts and EMPTY aside, of `weapon_class` alone when given (MugenMonkey's class, as
    --weapon-class)."""
    return [key for key, w in data.weapons.items()
            if inf in (w.get("infusions") or {}) and key not in EMPTY and not CATALYST.search(key)
            and not w.get("isShield")
            and (not weapon_class or norm(data.weapon_class.get(key) or "") == norm(weapon_class))]


def best_weapon_row(data: Data, corpus: list[Build], weapon: str, inf: str, sl: int, objective: str = "damage",
                    grip: str = "two", flex_weight: float | None = None, spells=(), only_class: str | None = None,
                    use_floors: bool = True, defender=None, window: float = 0.0, attacks: dict | None = None,
                    rank: str = "window"):
    """One row of best_weapons: `weapon`+`inf` at the build optimize_build makes for it at `sl`, scored
    so weapons compare. (score, weapon, value, class, two-handed, stats, rings, label, ammo, metrics),
    or None when no class wields it at `sl`, with `window` a melee weapon has no attack timing, or
    `rank` asks for a stamina metric the weapon has none of. `metrics` is a dict of named
    measurements beside the score: for the damage objective with `window`, stamina_metrics' (a
    launcher's shot_metrics') merged with row_status' (below), empty otherwise. `rank` (RANKS) replaces the damage score with
    one of them: "per-stamina" the window's damage per stamina, "bar" a full bar's damage. The
    build is still the one optimize_build makes for one hit; it is not re-optimized for stamina
    (END raises the bar and nothing else, so a "bar" ranking reads the END the floors give). `ammo` is
    {"ammo": the best ammunition, "shot": the shot scored (ranged_pick)} for a bow, greatbow or
    crossbow, None for any other weapon; a
    launcher's score is one shot whatever `window` is (ranged_value), so ranged rows rank a shot
    against a melee weapon's window.

    `value` is optimize_build's own: one hit for "damage", which ranks a slow weapon's single swing
    against a fast one's (p5z4.14). `window` re-scores the optimized build by what weapons_for's
    `window` counts: for "damage" the R1 chain's hits landing within that many seconds, for
    "bleed"/"poison" the build-up per hit x status_hits. The stats are the ones that maximize one
    hit; they are not re-optimized for the window. "ar" ignores `window`, as weapons_for does.

    `metrics` (row_status, with `window`, melee only; {} otherwise) is the poison and bleed the same
    hits deal through their procs, and for "damage" `damage_with_status`: the score plus
    status_damage_per_window. The score itself does not count status."""
    best, _ = optimize_build(data, corpus, weapon, inf, sl, objective, grip, flex_weight, spells, only_class,
                             use_floors, defender=defender)
    if best is None:
        return None
    val, cls, two, st, worn = best
    w = data.weapons[weapon]
    names = [w["name"], weapon.replace("_", " ")]
    score, label, metrics = val, "2H" if two else "1H", {}
    by = RANKS[rank] if objective == "damage" and window else None
    if weapon in data.ranged:
        # One shot with its best ammunition at the optimized stats (objective_value is ranged_value
        # for a launcher), with or without `window`: the fire rate is not read (ranged_value).
        _, note, _, ammo = ranged_pick(data, weapon, inf, gear_stats(data, st, worn), objective,
                                       defender_defense(data, corpus, sl, defender)[0], worn)
        if objective == "damage" and window:
            metrics = shot_metrics(data, weapon, val, st, worn)
        if by:
            if by not in metrics:
                return None
            score = metrics[by]
        return score, weapon, val, cls, two, st, worn, label + " 1 shot", {"ammo": ammo, "shot": note}, metrics
    if window and objective in ("bleed", "poison"):
        hits, label = status_hits(attacks, names, [two], window)
        if not hits:
            return None
        score = val * hits
        metrics = row_status(data, corpus, sl, defender, weapon, inf, gear_stats(data, st, worn), hits, window)
    elif window and objective == "damage":
        tl = next((t for t in (r1_timeline(attacks, nm, two) for nm in names) if t), None)
        if not tl:
            return None
        dfn, _ = defender_defense(data, corpus, sl, defender)
        ar = attack_rating(data, weapon, inf, gear_stats(data, st, worn), worn)
        mvs = [(mv, ty, lo, fl) for t, mv, ty, lo, fl in tl if t <= window]
        score = sum(hit_damage(ar, dfn, *m) for m in mvs) * data.damage_scale.get(weapon, 1.0)
        label += f" {len(mvs)} hits"
        # the R1 chain's hits for build-up: a same-window hitbox pair counts once (live_hits)
        hits = next((n for n in (sum(1 for t, *_ in chain_timeline(attacks, nm, two, "Normal", max(3.0, window),
                                                                     distinct=True) if t <= window)
                                 for nm in names) if n), 0)
        metrics = row_status(data, corpus, sl, defender, weapon, inf, gear_stats(data, st, worn), hits, window)
        metrics["damage_with_status"] = score + metrics["status_damage_per_window"]
        metrics.update(stamina_metrics(data, weapon, two, names, attacks, ar, dfn, window, st, worn))
        if by:
            if by not in metrics:
                return None
            score = metrics[by]
    return score, weapon, val, cls, two, st, worn, label, None, metrics


def row_status(data: Data, corpus: list[Build], sl: int, defender, weapon: str, inf: str, st: dict,
               hits: int, window: float) -> dict:
    """best_weapon_row's status metrics for weapon+infusion at effective stats `st`, its `hits`
    landed in `window` seconds: per status it builds up (objective_value), the build-up per hit
    before resistance and hits_to_proc against the defenders at `sl` (status_metrics); summed over
    the statuses, status_damage_per_window and status_damage_first_window."""
    out = {"status_hits": hits, "status_buildup_per_hit": {}, "hits_to_proc": {},
           "status_damage_per_window": 0.0, "status_damage_first_window": 0.0}
    for s in STATUS_PROC:
        per = objective_value(data, weapon, inf, st, s, {})
        if per <= 0:
            continue
        m = status_metrics(data.status_procs.get(s), per, defender_status_cuts(data, corpus, sl, s, defender),
                           hits, window)
        out["status_buildup_per_hit"][s] = per
        out["hits_to_proc"][s] = m["hits_to_proc"]
        out["status_damage_per_window"] += m["status_damage_per_window"]
        out["status_damage_first_window"] += m["status_damage_first_window"]
    return out


def best_weapons(data: Data, corpus: list[Build], inf: str, sl: int, objective: str = "damage",
                 grip: str = "two", flex_weight: float | None = None, spells=(), only_class: str | None = None,
                 use_floors: bool = True, defender=None, weapon_class: str | None = None, window: float = 0.0,
                 jobs: int = 1, rank: str = "window") -> list[tuple]:
    """The reverse of best_infusion: every weapon that takes infusion `inf`, each at the build
    optimize_build makes for it at `sl` (its own stats, class and rings), best first by
    best_weapon_row's score (`rank`: RANKS). A weapon no class wields at `sl` is left out. `jobs` > 1
    forks that many workers: one optimize_build is seconds, and there are a few hundred weapons."""
    keys = best_weapons_jobs(data, inf, weapon_class)
    attacks = load_attacks(data) if window else None
    args = (inf, sl, objective, grip, flex_weight, tuple(spells), only_class, use_floors, defender, window, attacks,
            rank)
    if jobs > 1:
        import multiprocessing
        global _BW
        _BW = (data, corpus, args)  # inherited by the forked workers, not pickled per call
        with multiprocessing.get_context("fork").Pool(jobs) as pool:
            rows = pool.map(_best_weapon_job, keys)
    else:
        rows = [best_weapon_row(data, corpus, key, *args) for key in keys]
    return sorted((r for r in rows if r), key=lambda r: -r[0])


_row_attacks: dict = {}  # id(data) -> load_attacks(data), read once for every row_metrics


def row_metrics(data: Data, corpus: list[Build], row: tuple, inf: str, sl: int, defender=None) -> dict | None:
    """r1_metrics for one best_weapons row, at its optimized stats, rings and grip against the
    defender its score used; None for a bow, greatbow or crossbow (its score is one shot)."""
    _, weapon, _, _, two, st, worn, *_ = row
    if weapon in data.ranged:
        return None
    w = data.weapons[weapon]
    ar = attack_rating(data, weapon, inf, gear_stats(data, st, worn), worn)
    if id(data) not in _row_attacks:
        _row_attacks[id(data)] = load_attacks(data)
    return r1_metrics(data, _row_attacks[id(data)], weapon, [w["name"], weapon.replace("_", " ")], two, ar,
                      defender_defense(data, corpus, sl, defender)[0])


def best_weapon_metrics(data: Data, corpus: list[Build], attacks: dict, row: tuple, inf: str, sl: int,
                        defender=None) -> dict:
    """Every measurement --best-weapons --json prints for one row: r1_metrics' (row_metrics; none
    for a launcher), poise_metrics' and the row's own (best_weapon_row's `metrics`)."""
    return {**(row_metrics(data, corpus, row, inf, sl, defender) or {}),
            **(poise_metrics(data, corpus, attacks, row[1], row[4], sl, defender) or {}),
            **(row[9] or {})}


_BW: tuple | None = None


def _best_weapon_job(key: str):
    data, corpus, args = _BW
    return best_weapon_row(data, corpus, key, *args)


def adoption(data: Data, corpus: list[Build]) -> dict:
    """What real builds pick as their melee weapon per SL bracket: {bracket: [builds, Counter of
    weapon class, Counter of weapon key]}. The melee weapon is the one threat_opponents scores a
    build's hits with: the first shield-less, catalyst-less weapon in MELEE_ORDER. Builds with none
    are left out. What best_weapons is checked against, as er-mods-rs checks its ranking against
    primary-weapon adoption (docs/er-mechanics/moveset.md there). Published builds, not match logs;
    the stat requirements are not checked here."""
    out = {}
    for b in corpus:
        w = next((w for w in (b.hands[HAND_SLOTS.index(s)][0] for s in MELEE_ORDER)
                  if w not in EMPTY and w in data.weapons and not data.weapons[w].get("isShield")
                  and not CATALYST.search(w)), None)
        if w is None:
            continue
        row = out.setdefault(sl_bracket(soul_level(data, b)), [0, Counter(), Counter()])
        row[0] += 1
        row[1][data.weapon_class.get(w) or "?"] += 1
        row[2][w] += 1
    return out


def spearman(xs: list[float], ys: list[float]) -> float | None:
    """Spearman's rho of two equal-length lists, ties at their mean rank. None under 3 pairs or
    when either side is constant."""
    if len(xs) < 3:
        return None

    def ranks(v):
        order = sorted(range(len(v)), key=lambda i: v[i])
        r = [0.0] * len(v)
        i = 0
        while i < len(order):
            j = i
            while j + 1 < len(order) and v[order[j + 1]] == v[order[i]]:
                j += 1
            for k in range(i, j + 1):
                r[order[k]] = (i + j) / 2
            i = j + 1
        return r

    rx, ry = np.array(ranks(xs)), np.array(ranks(ys))
    if rx.std() == 0 or ry.std() == 0:
        return None
    return float(np.corrcoef(rx, ry)[0, 1])


def build_floors(data: Data, corpus: list[Build], sl: int) -> tuple[dict, dict, float]:
    """The floors every build output must meet at `sl`: VGR/VIT/ADP/ATT at the corpus median of
    the SL bracket, END at the high-stamina builds' median (applies only to high-stamina weapons).
    ATT applies only to a build with spells (floor_stats)."""
    r1 = stamina_r1(data)
    return bracket_floors(data, corpus, r1)[sl_bracket(sl)], r1, high_stamina_cut(r1)


#: The stats whose additionalHp the max-HP formula adds to VGR's hpMax: words 1-8 of the stat block,
#: END to ADP (0x14038e1e0 skips words 9 and 10; the ids are 0x14038e280's jump table).
HP_STATS = ["endurance", "vitality", "attunement", "strength", "dexterity", "intelligence", "faith",
            "adaptability"]


def hit_points(data: Data, st: dict) -> int:
    """Max HP of the effective stats `st` (EXE 0x14038e1e0, which the stats builder 0x14038d790
    calls first and stores at its block's +0x0): PhysicalStatsPerLevelStatValuesParam hpMax at VGR
    plus additionalHp at each of HP_STATS, a stat outside 1-99 read at row 1. hpMax equals
    SoulsPlanner's getHP at every VGR; additionalHp (2 per point to 20, then 1 to 50, then 70) is
    what the site leaves out. Without the regulation it is getHP from VGR alone (SITE)."""
    if not data.hp_max:
        vgr = st["vigor"]
        return 500 + 30 * min(vgr, 20) + 20 * max(0, min(vgr, 50) - 20) + 5 * max(0, vgr - 50)
    row = lambda v: v if 1 <= v <= 99 else 1
    return data.hp_max[row(st["vigor"])] + sum(data.additional_hp[row(st[s])] for s in HP_STATS)


def floor_violations(stats: dict, floors: dict, spells=()) -> list[str]:
    return [f"{s[:3].upper()} {stats[s]} < {floors[s]}" for s in floor_stats(spells)
            if stats[s] < floors.get(s, 0)]


def objective_value(data: Data, weapon: str, inf: str, st: dict, objective: str, dfn: dict,
                    rings=()) -> float:
    """`objective` for weapon+infusion at stats `st`: build-up per hit for "bleed" and "poison",
    the summed attack rating for "ar", else one hit's damage against defense `dfn`, the worn
    `rings`' attack adds counted (attack_rating; no ring adds a status build-up here,
    regulation_ring_attack).

    "ar" is what the community compares (a Raw Black Dragon Greataxe's 471 against Lightning's
    287 + 287) and what the menu shows: every type summed, before any defense and before the
    weapon's damage_scale, which applies after the defense. A ring's attack add is in it, because
    the game adds it to the attack rating itself (`AR = (bonus + base) x rate`, attack_rating): a
    Ring of Blades raises the menu's AR, so a goal that left it out would not be the menu's number.

    "damage" against an AdaptiveDefense is against its answer to this attack (respond), so a build
    is optimized for the damage left after the defender's counter ring: the best attack given the
    defender's best reply. "ar" stays defense-free. A ranged weapon's shot is answered the same way
    (ranged_value)."""
    if weapon in data.ranged:  # a bow, greatbow or crossbow: its best shot, ammunition included
        return ranged_value(data, weapon, inf, st, objective, dfn, rings)[0]
    row = data.weapons[weapon]["infusions"].get(inf) or {}
    atk, sc = row.get("atk") or {}, row.get("atkScale") or {}
    if objective in ("bleed", "poison"):
        # Build-up per hit in gauge points before the victim's resistance: the base and coefficient
        # regulation_status puts in place of the site's, times the status bonus. The site's auxATKBonus
        # at this index is the game's bleeding/poisonAdditionalEffect at row trunc(i / 4) (EXE
        # 0x14038dcaf / 0x14038dcef).
        i = 3 * st["dexterity"] + (st["faith"] if objective == "bleed" else st["adaptability"])
        return (atk.get(objective) or 0) + sc.get(objective, 0) * _tab(data, "auxATKBonus", i)
    if objective == "ar":
        return sum(attack_rating(data, weapon, inf, st, rings).values())
    ar = attack_rating(data, weapon, inf, st, rings)
    d = respond(dfn, ar)
    return sum(damage(k, v, d[k]) for k, v in ar.items()) * data.damage_scale.get(weapon, 1.0)


#: The grip optimize_build builds for, per `--grip`. "two" (the default) halves the STR requirement,
#: even when one-handing would fit; "one" needs the full requirement. The old behaviour -- one-handed
#: whenever it fits, two-handed only when it does not -- is gone: it made a two-handed build
#: unreachable for any weapon the stats could one-hand, and trying two-handed first is the same as
#: always two-handing, since a halved requirement fits whenever the full one does.
#: The objective is scored the same for either grip: AR (or status build-up per hit) does not depend
#: on the grip once the requirement is met. The game has no two-handed Strength multiplier
#: (docs/DS2-DPS-MECHANICS.md, "Two-handing").
GRIP_TRIES = {"two": (True,), "one": (False,)}


def optimize_build(data: Data, corpus: list[Build], weapon: str, inf: str, sl: int, objective: str,
                   grip: str = "two", flex_weight: float | None = None, spells=(),
                   only_class: str | None = None, use_floors: bool = True, rings=None, defender=None):
    """A valid build at `sl` that maximizes `objective` for weapon+infusion: floors first (bracket
    medians, END only for a high-stamina weapon), then requirements (STR halved for grip "two",
    in full for "one"; see GRIP_TRIES), then what `spells` need (spell_floors: their INT/FTH, and
    the ATT whose slots hold them), then every remaining point where it raises the objective
    most, plus `flex_weight` (default FLEX_WEIGHT) per weapon the point lets the build wield: a soft
    term, never a filter (see FLEX_WEIGHT). None when no class fits the floors, requirements and
    spells into `sl`.

    `only_class` (a class key, `sorcerer`) tries that starting class alone. A build for a character
    that already exists has to be: the game offers no class change after creation, so a build from
    another class's base is one the character cannot have -- measured 2026-09-28, a Sorcerer given
    a Warrior build lost 6 attunement and 9 intelligence below its own starting stats.

    `use_floors` False drops the floors (they are the medians of real builds, not a game rule), and
    the floors returned are then empty: the panel's "ignore typical-build minimums".

    Rings count as stat points (ring_gear, ring_lift): the build may wear rings that stand in for
    points the floors, the requirements or the spells' slots would otherwise take -- a Southern
    Ritual Band's slots for ATT, a Royal Soldier's Ring's load for VIT, a Life Ring's HP for VIG, a
    stat ring's +5 for a requirement -- and the slots they leave hold offensive rings
    (offense_rings), which a stand-in ring has to beat. `rings` None chooses them (choose_rings); a
    list wears exactly those. The best is (value, class, two-handed, levelled stats, worn rings);
    the objective is scored at the stats the worn rings give, with their attack adds. `defender` is
    what the damage objective is scored against (defender_defense; None: the bracket's average)."""
    if flex_weight is None:
        flex_weight = FLEX_WEIGHT
    floors, r1, cut = build_floors(data, corpus, sl)
    if not use_floors:
        floors = {}
    need_floors = {s: floors.get(s, 0) for s in floor_stats(spells)}
    if r1.get(weapon, 0) >= cut:
        need_floors["endurance"] = floors.get("endurance", 0)
    classes = [c for c in data.classes if only_class is None or c == only_class.lower()]
    two = GRIP_TRIES[grip][0]
    req = _grip_req(data, weapon, two)
    run = lambda rs: _optimize_with(data, corpus, weapon, inf, sl, objective, two, flex_weight, spells,
                                    classes, need_floors, req, rs, defender)
    if rings is not None:
        return run(list(rings)), floors
    return choose_rings(data, classes, need_floors, req, spells, run,
                        offense_rings(data, weapon, inf, objective)), floors


#: A ring worn in place of stat points must free at least this many to take a ring slot: one slot
#: as a stat ring buys +5 of one stat (Strength Ring, Ring of Knowledge and the rest; ring_gear),
#: so a ring that frees fewer points is worth less than the stat ring the slot could hold. Still
#: needed now that an offensive ring's effect is decoded: choose_rings weighs a stand-in against
#: the offensive ring it displaces by the objective, but a slot holds a stat ring as well, and
#: that alternative is not run. It needs no run: five free points go where the objective gains
#: most, which is at least what +5 in one stat gains, so a ring freeing five or more beats every
#: stat ring and one freeing fewer loses to one.
RING_SLOT_POINTS = 5


def offense_rings(data: Data, weapon: str, inf: str, objective: str) -> list[str]:
    """The rings that raise `objective` for weapon+infusion by their attack add alone
    (regulation_ring_attack): per ring upgrade group the last, its strongest tier, never a
    NO_USE_RINGS ring, when ring_attack_add gives the infusion some attack; most attack added first
    (summed over DMG), in data.ring_attack order on a tie. None for "bleed" or "poison": no decoded
    ring adds to a build-up. "ar" takes the same rings as "damage": an add raises both."""
    if objective not in ("damage", "ar"):
        return []
    row = data.weapons[weapon]["infusions"].get(inf) or {}
    by_group = {}
    for r in data.ring_attack:
        if r not in NO_USE_RINGS:
            by_group[data.rings[r].get("group", r)] = r
    gain = {r: sum(ring_attack_add(data, row, [r]).values()) for r in by_group.values()}
    return sorted((r for r in by_group.values() if gain[r] > 0), key=lambda r: -gain[r])


def sub_rings(data: Data) -> list[str]:
    """The rings optimize_build may wear in place of stat points: per ring upgrade group
    (data.rings `group`), the ring_gear ring with the highest item id -- the strongest tier, as
    Southern Ritual Band+2 over +1 -- when none of its factors is below 1 (a Northern Ritual Band's
    HP loss), never a NO_USE_RINGS ring; in ring_gear order (mechanics.json's, item id)."""
    by_group = {}
    for r, e in ring_gear(data).items():
        if r in NO_USE_RINGS or min(e["hp"], e["load"], e["stamina"]) < 1.0:
            continue
        by_group[data.rings[r].get("group", r)] = r
    return list(by_group.values())


def ring_lift(data: Data, st: dict, floors: dict, req: dict, spells, rings=()) -> dict | None:
    """`st` (a class's base) raised to what a build wearing `rings` must level to: each `floors`
    stat until what it gives matches what the floor gives with no ring -- VIG by max HP
    (hit_points: hpMax at VGR plus the other stats' additionalHp, read at the stats `out` holds when
    VIG is lifted), VIT by max equip load (equipLoadMax), END by max stamina (staminaMax), each read
    at the stats plus the rings' bonus and times the rings' factor; ADP and ATT by the stat plus
    bonus --; each `req` stat and the spells' INT/FTH until the stat plus the
    rings' bonus meets it; ATT to the least whose slots plus the rings' hold the spells
    (spell_floors). The columns are indexed by those stats per RelatePhysicalStatToLevelStatParam
    row 0 (hpMax 1 VGR, staminaMax 2 END, equipLoadMax 3 VIT, spellSlot 4 ATT; read 2026-09-28).
    None when no ATT holds the spells. With no ring it is the old arithmetic: each stat the most of
    its base, its floor, its requirement."""
    need = spell_floors(data, spells, ring_slots(data, rings))
    if need is None:
        return None
    out = dict(st)
    gear = ring_gear(data)
    worn = [gear[r] for r in rings if r in gear]
    touched = {s for e in worn for s in [*e["add"], *(x[0] for x in e["scaled"])]}

    def eff(s, v):
        if s not in touched:
            return v
        e_ = gear_stats(data, {**out, s: v}, rings)
        return e_[s]

    def least(s, ok):
        v = out[s]
        while v < 99 and not ok(v):
            v += 1
        out[s] = v

    # What a floor stat gives, read off a whole stat block, the rings' factor on it, and the stats
    # it reads: max HP reads VGR and every HP_STATS stat, so a ring raising any of them counts.
    curves = {"vigor": (lambda e: hit_points(data, e), ring_factor(data, rings, "hp"), {"vigor", *HP_STATS}),
              "vitality": (lambda e: data.equip_load[min(e["vitality"], len(data.equip_load) - 1)] or 0,
                           ring_factor(data, rings, "load"), {"vitality"}),
              "endurance": ((lambda e: data.stamina_max[min(e["endurance"], len(data.stamina_max) - 1)])
                            if data.stamina_max else None, ring_factor(data, rings, "stamina"), {"endurance"})}
    for s, f in floors.items():
        tab, fac, reads = curves.get(s, (None, 1.0, {s}))
        if tab is not None and (fac != 1.0 or touched & reads):
            target = tab({**out, s: f})
            least(s, lambda v: tab(gear_stats(data, {**out, s: v}, rings)) * fac >= target)
        else:
            least(s, lambda v: eff(s, v) >= f)
    for s, x in [*req.items(), *((s, x) for s, x in need.items() if s != "attunement")]:
        least(s, lambda v: eff(s, v) >= x)
    out["attunement"] = max(out["attunement"], need["attunement"])
    return out


#: A lift no class can reach: a set of rings whose spells no ATT holds.
UNREACHABLE = 10 ** 6


def lift_points(data: Data, classes: list[str], floors: dict, req: dict, spells, rings) -> list[int]:
    """Per class, the points ring_lift raises its base by wearing `rings` (UNREACHABLE when no ATT
    holds the spells)."""
    out = []
    for c in classes:
        base = {s: int(data.classes[c][s]) for s in STATS}
        up = ring_lift(data, base, floors, req, spells, rings)
        out.append(UNREACHABLE if up is None else sum(up.values()) - sum(base.values()))
    return out


def next_ring(data: Data, classes: list[str], floors: dict, req: dict, spells, worn: list[str]) -> str | None:
    """The sub_rings ring, not in the group of one `worn` and with fewer than four worn, that frees
    the most points beside `worn` -- the most any class's lift_points falls, first in sub_rings
    order on a tie -- when it frees RING_SLOT_POINTS or more; else None."""
    if len(worn) >= 4:
        return None
    group = lambda r: data.rings[r].get("group", r)
    taken = {group(r) for r in worn}
    now = lift_points(data, classes, floors, req, spells, worn)
    pick = None
    for r in sub_rings(data):
        if group(r) in taken:
            continue
        freed = max(a - b for a, b in zip(now, lift_points(data, classes, floors, req, spells, worn + [r])))
        if freed >= RING_SLOT_POINTS and (pick is None or freed > pick[0]):
            pick = (freed, r)
    return pick and pick[1]


def choose_rings(data: Data, classes: list[str], floors: dict, req: dict, spells, run, offense=()):
    """optimize_build's rings, one next_ring at a time, each set run with the slots it leaves
    holding the first of `offense` (offense_rings). While no build fits, the ring is worn and the
    next tried; once one fits, it is worn only when `run` (optimize_build at those rings) scores the
    objective higher -- so a stand-in that takes an offensive ring's slot is kept only when what its
    freed points buy beats what that ring adds. Returns `run`'s best with the rings worn, or None."""
    fill = lambda rs: rs + list(offense[:4 - len(rs)])
    worn, best = [], run(fill([]))
    while (r := next_ring(data, classes, floors, req, spells, worn)) is not None:
        got = run(fill(worn + [r]))
        if best is not None and (got is None or got[0] <= best[0]):
            break
        worn.append(r)
        best = got
    return best


def least_sl_with_rings(data: Data, classes: list[str], floors: dict, req: dict, spells) -> int | None:
    """The least soul level at which optimize_build finds a build under `floors`, in choose_rings'
    arithmetic: while nothing fits it wears each next_ring in turn, and a build exists as soon as
    one class's lift_points fit the `sl + 53 - sum(base)` points the level gives. None when no
    ring sequence fits any level (no ATT holds the spells)."""
    worn, best = [], None
    while True:
        for c, pts in zip(classes, lift_points(data, classes, floors, req, spells, worn)):
            if pts < UNREACHABLE:
                sl = pts - 53 + sum(int(data.classes[c][s]) for s in STATS)
                best = sl if best is None else min(best, sl)
        r = next_ring(data, classes, floors, req, spells, worn)
        if r is None:
            return best
        worn.append(r)


def _optimize_with(data: Data, corpus: list[Build], weapon: str, inf: str, sl: int, objective: str,
                   two: bool, flex_weight: float, spells, classes: list[str], floors: dict, req: dict,
                   rings: list[str], defender=None):
    """optimize_build's search at one set of worn `rings`: per class the base lifted by ring_lift,
    then the free points spent. The best (value, class, two, stats, rings), or None."""
    flex = (lambda s_: sum(flex_counts(data, s_))) if flex_weight else None
    dfn, _ = defender_defense(data, corpus, sl, defender)
    E =(lambda s_: gear_stats(data, s_, rings)) if rings else (lambda s_: s_)
    best = None
    for cls in classes:
        base = data.classes[cls]
        st = ring_lift(data, {s: int(base[s]) for s in STATS}, floors, req, spells, rings)
        if st is None:
            return None
        free = sl + 53 - sum(st.values())
        if free < 0:
            continue
        # A stat is weighted by its own curve: gain per point at the current value over the
        # curve's early rate (its mean gain per point from 5 to 25, before any soft cap). Past
        # a soft cap the weight falls (DEX past 40, VGR past 20 and 50, ADP past AGL 110), so
        # points go to a stat still under its cap. Every curve reads the stats the worn rings give
        # (E); the flexibility term reads the levelled stats, as flexibility does.
        obj = lambda s_: objective_value(data, weapon, inf, E(s_), objective, dfn, rings)
        agl = lambda s_: (lambda e: agility(e["adaptability"], e["attunement"]))(E(s_))
        curves = {s: obj for s in ("strength", "dexterity", "intelligence", "faith")}
        curves["adaptability"] = obj if objective == "poison" else agl
        # Max HP of the whole block. Only VGR's step moves it here, so the other stats'
        # additionalHp cancels out of VGR's weight and is credited to no stat: nothing here
        # weighs a point of HP against a point of damage.
        curves["vigor"] = lambda s_: hit_points(data, E(s_))
        # END by max stamina (staminaMax) and VIT by max equip load (equipLoadMax), each over its
        # own early rate as VGR is: without them a weapon whose damage stops rising (the Black
        # Dragon Greataxe scales with nothing) put every leftover point into VGR -- VGR 99 at SL
        # 155 against a corpus median of 50 -- where real builds spend past VGR's soft cap on
        # END and VIT. VIT's curve is the capacity itself, not the headroom percentage the 70%
        # rule is written in: headroom depends on the armour, which generate_armor picks after
        # the stats are set, and a curve over the capacity alone falls past VIT's soft cap as
        # the capacity's own gain does. Without the regulation's stamina table END has no curve.
        if data.stamina_max:
            curves["endurance"] = lambda s_: data.stamina_max[min(E(s_)["endurance"], len(data.stamina_max) - 1)]
        curves["vitality"] = lambda s_: data.equip_load[min(E(s_)["vitality"], len(data.equip_load) - 1)] or 0
        curves["attunement"] = agl  # a third of ADP's agility per point; slots only matter with spells
        peak = {}
        for s, f in curves.items():
            peak[s] = max((f({**st, s: 25}) - f({**st, s: 5})) / 20, 1e-9)
        # The stats that feed the objective (STR/DEX/INT/FTH, and ADP when the objective is poison)
        # share one unit, the steepest of their early rates: a point of damage is a point of damage
        # whichever stat buys it. Each over its own rate inflated a stat with almost no scaling to
        # parity with the real one -- measured 2026-09-30, Uchigatana Lightning, Bandit, SL 155, no
        # floors: STR 45 at an early rate of 0.083 beside DEX's 0.375.
        shared = max(peak[s] for s, f in curves.items() if f is obj)
        # Dark reads min(INT, FTH) (attack_rating), so a point of INT alone or FTH alone buys no
        # dark and a one-stat step never finds it: a Dark Sanctum Crossbow kept INT/FTH at 14/14 at
        # SL 150. With a dark attack the two also move together, a step of n each costing 2n.
        dark = "dark" in attack_rating(data, weapon, inf, E(st), rings)
        if dark:
            both = lambda s_, n: {**s_, "intelligence": s_["intelligence"] + n, "faith": s_["faith"] + n}
            shared = max(shared, (obj({**st, "intelligence": 25, "faith": 25})
                                  - obj({**st, "intelligence": 5, "faith": 5})) / 40)
        for s, f in curves.items():
            if f is obj:
                peak[s] = shared
        if curves["adaptability"] is agl:
            peak["attunement"] = peak["adaptability"]
        while free > 0:
            pick, best_w = None, 0.0
            for s, f in curves.items():
                cur = f(st)
                # Only a requirement stat changes what the build wields; the term is left off
                # the rest rather than added as a zero, so a weight of 0 is the old optimizer.
                fcur = flex(st) if flex and s in REQ_STATS else None
                for n in range(1, min(free, 99 - st[s], 8) + 1):  # look past table plateaus
                    w = (f({**st, s: st[s] + n}) - cur) / n / peak[s]
                    if fcur is not None:
                        w += flex_weight * (flex({**st, s: st[s] + n}) - fcur) / n
                    if w > best_w + 1e-12:
                        pick, best_w = (s, n), w
            if dark:
                cur = obj(st)
                for n in range(1, min(free // 2, 99 - max(st["intelligence"], st["faith"]), 8) + 1):
                    w = (obj(both(st, n)) - cur) / (2 * n) / shared
                    if flex:
                        w += flex_weight * (flex(both(st, n)) - flex(st)) / (2 * n)
                    if w > best_w + 1e-12:
                        pick, best_w = ("both", n), w
            if pick is None:  # every curve is flat: the points go to vigor
                pick = (next(s for s in ("vigor", "vitality", "endurance", "attunement") if st[s] < 99), 1)
            if pick[0] == "both":
                st, free = both(st, pick[1]), free - 2 * pick[1]
                continue
            st[pick[0]] += pick[1]
            free -= pick[1]
        val = objective_value(data, weapon, inf, E(st), objective, dfn, rings)
        if best is None or val > best[0]:
            best = (val, cls, two, st, list(rings))
    return best


#: The highest soul level the game allows: where the search for the least SL that fits stops.
SL_MAX = 838

#: The short names refusals print, in STATS order; the panel's STAT_LABELS.
LABEL = dict(zip(STATS, ["VIG", "END", "VIT", "ATT", "STR", "DEX", "ADP", "INT", "FTH"]))


def _floors_at(floors: dict, s: str, high_stamina: bool, spells) -> int:
    """The floor optimize_build applies to stat `s`: floor_stats(spells) always (ATT only with
    spells), END for a high-stamina weapon."""
    if s in floor_stats(spells) or (s == "endurance" and high_stamina):
        return int(floors.get(s, 0))
    return 0


def _raises(data: Data, cls: str, floors: dict, high_stamina: bool, req: dict, need: dict, spells) -> list:
    """Per stat in STATS order, what optimize_build lifts class `cls`'s base to before spending any
    free point, and what lifted it: (stat, raise, value, source), source "spells", "weapon" or
    "floors" -- the game's rules before the floors on a tie, since the floors are not one."""
    out = []
    for s in STATS:
        base = int(data.classes[cls][s])
        v, src = base, None
        for name, x in (("spells", need.get(s, 0)), ("weapon", req.get(s, 0)),
                        ("floors", _floors_at(floors, s, high_stamina, spells))):
            if x > v:
                v, src = x, name
        out.append((s, v - base, v, src))
    return out


def _grip_req(data: Data, weapon: str, two: bool) -> dict:
    req = data.weapons[weapon].get("require") or {}
    return {s: ((v + 1) // 2 if (two and s == "strength") else v) for s, v in req.items()}


def refusal(data: Data, corpus: list[Build], weapon: str, inf: str, sl: int, objective: str = "damage",
            grip: str = "two", spells=(), only_class: str | None = None, use_floors: bool = True) -> dict | None:
    """Why optimize_build finds no build, and what would make one: None when it finds one.

    The arithmetic is optimize_build's own: a class's base lifted to the floors, the weapon's
    requirements and the spells' (spell_floors), and a build exists only when that costs no more
    than the `sl + 53 - sum(base)` points the level gives above the base. `kind` says which layer
    no class clears: "slots" (no ATT holds the spells), "weapon" (its requirements alone), "spells"
    (with the spells), "floors" (only with the typical-build floors). The closest class -- the
    least short -- is the one the arithmetic line is about.

    `fixes` are (what, value, label), each one checked by running optimize_build with it applied:
    ("sl", N) the least soul level above `sl` that fits with nothing else changed; ("spell", key)
    per spell whose removal (one copy) fits at `sl`; ("floors", None) when dropping the floors fits;
    ("class", key) when `only_class` is the blocker and another class fits."""
    wname = data.weapons[weapon]["name"]
    run = lambda sl_=sl, spells_=spells, cls_=only_class, fl_=use_floors: optimize_build(
        data, corpus, weapon, inf, sl_, objective, grip, None, spells_, cls_, fl_)[0]
    if run() is not None:
        return None
    names = list(dict.fromkeys(data.spells[s]["name"] for s in spells))
    spell_list = ", ".join(names)
    two = GRIP_TRIES[grip][0]
    req = _grip_req(data, weapon, two)
    r1 = stamina_r1(data)
    cut = high_stamina_cut(r1)
    hs = r1.get(weapon, 0) >= cut
    table = bracket_floors(data, corpus, r1)
    floors_at = lambda sl_: table[sl_bracket(sl_)] if use_floors else {}
    classes = [c for c in data.classes if only_class is None or c == only_class.lower()]
    need = spell_floors(data, spells)
    lines, fixes = [], []
    # No ATT alone holds the spells, but the ring with the most slots (sub_rings) makes one that
    # does: the arithmetic below is then the one with that ring worn, as optimize_build wears it.
    band = max(sub_rings(data), key=lambda r: ring_gear(data)[r]["slots"], default=None)
    band_slots = ring_slots(data, [band]) if band else 0
    band_note = None
    if need is None and band_slots:
        need = spell_floors(data, spells, band_slots)
        if need is not None:
            band_note = (f"no attunement alone holds {spell_list}; wearing a {data.rings[band]['name']} "
                         f"({ring_effect_text(data, band)}) does")

    def cost(cls, sl_, layer):
        """Points above `cls`'s base the layers up to `layer` need at `sl_` (1 weapon, 2 +spells, 3 +floors)."""
        base = data.classes[cls]
        tot = 0
        for s in STATS:
            v = int(base[s])
            v = max(v, req.get(s, 0))
            if layer >= 2:
                v = max(v, need.get(s, 0))
            if layer >= 3:
                v = max(v, _floors_at(floors_at(sl_), s, hs, spells))
            tot += v - int(base[s])
        return tot

    have = lambda cls, sl_: sl_ + 53 - sum(int(data.classes[cls][s]) for s in STATS)

    if need is None:
        total = sum(data.spells[s]["slots"] for s in spells)
        kind, cls, short = "slots", None, None
        lines.append(f"no attunement holds {spell_list}: they cost {total} slots, and ATT 99 gives "
                     f"{max(data.att_slots)}"
                     + (f", {max(data.att_slots) + band_slots} with a {data.rings[band]['name']}"
                        if band_slots else ""))
    else:
        kind = next((k for k, layer in (("weapon", 1), ("spells", 2), ("floors", 3))
                     if all(cost(c, sl, layer) > have(c, sl) for c in classes)), "floors")
        # the least short class; the first in data order on a tie
        cls = min(classes, key=lambda c: cost(c, sl, 3) - have(c, sl))
        short = cost(cls, sl, 3) - have(cls, sl)
        cname = data.classes[cls]["name"]
        grip_word = "two-handed" if two else "one-handed"
        lines.append({
            "weapon": f"{wname} cannot be wielded {grip_word} at SL {sl}",
            "spells": f"{wname} can be wielded at SL {sl}, but not while casting {spell_list}",
            "floors": f"{wname}{' and ' + spell_list + ' fit' if spells else ' fits'} SL {sl}, but not above its "
                      f"typical-build minimums (the median {'/'.join(LABEL[s] for s in floor_stats(spells))} "
                      "of real builds at this level; not a game rule)",
        }[kind])
        if only_class is not None:
            lines[0] = f"as a {cname}: {lines[0]}"
        if band_note:
            lines.append(band_note)
        raises = _raises(data, cls, floors_at(sl), hs, req, need, spells)
        groups = []
        for src, label in (("floors", "floors"), ("spells", spell_list), ("weapon", "weapon")):
            got = [f"{LABEL[s]} {v}" for s, r, v, x in raises if x == src and r > 0]
            if got:
                groups.append(f"{label} {' '.join(got)}")
        lines.append(f"SL {sl} is {short} points short for a {cname}: {' + '.join(groups)} need "
                     f"{cost(cls, sl, 3)} points above its base, SL {sl} gives {have(cls, sl)}")
        top = sorted((r for r in raises if r[1] > 0), key=lambda r: -r[1])[:3]
        src_name = {"floors": "floor", "spells": spell_list, "weapon": "weapon"}
        lines.append("most from " + ", ".join(f"{LABEL[s]} +{r} ({src_name[x]})" for s, r, v, x in top))

    # 1. the least soul level that fits, nothing else changed, wearing what rings optimize_build
    # would (least_sl_with_rings: per floors bracket, the least SL any class's lift fits under)
    least = {}

    def fits(sl_):
        i = sl_bracket(sl_)
        if i not in least:
            fl = {s: _floors_at(floors_at(sl_), s, hs, spells)
                  for s in floor_stats(spells) + (["endurance"] if hs else [])}
            least[i] = least_sl_with_rings(data, classes, fl, req, spells)
        return least[i] is not None and sl_ >= least[i]

    up = next((s_ for s_ in range(sl + 1, SL_MAX + 1) if fits(s_)), None)
    if up is not None and run(sl_=up) is not None:
        fixes.append(("sl", up, f"Raise SL to {up}"))
        # the floors this build is held to (floor_stats: no ATT without spells), as they read
        fl = lambda sl_: " ".join(f"{LABEL[s]} {floors_at(sl_).get(s, 0)}" for s in floor_stats(spells))
        if use_floors and fl(up) != fl(sl):
            lines.append(f"the least SL that fits is {up}: the floors change with soul level "
                         f"({fl(sl)} at SL {sl}, {fl(up)} at SL {up})")
        else:
            lines.append(f"the least SL that fits is {up}")
    elif need is not None:
        lines.append(f"no soul level up to {SL_MAX} fits it")
    # 2. one spell fewer
    for key in dict.fromkeys(spells):
        rest = list(spells)
        rest.remove(key)
        if run(spells_=rest) is not None:
            fixes.append(("spell", key, f"Remove {data.spells[key]['name']}"))
    # 3. no floors
    if use_floors and need is not None and run(fl_=False) is not None:
        binding = " ".join(f"{LABEL[s]} {v}" for s, r, v, x in _raises(data, cls, floors_at(sl), hs, req, need, spells)
                           if x == "floors" and r > 0)
        fixes.append(("floors", None, f"Ignore typical-build minimums ({binding})"))
    # 4. another class, when a class was asked for
    if only_class is not None:
        best = run(cls_=None)
        # checked as that class alone too: the rings chosen for any class can differ from the
        # rings chosen for one (next_ring counts the classes it is given)
        if best is not None and run(cls_=best[1]) is not None:
            fixes.append(("class", best[1], f"Use a {data.classes[best[1]]['name']} instead"))
    return {"kind": kind, "class": cls and data.classes[cls]["name"], "short": short, "lines": lines,
            "fixes": fixes}


def print_refusal(r: dict) -> None:
    """--optimize/--generate's failure: the reason lines, then each verified fix."""
    for line in r["lines"]:
        print(f"  {line}")
    print("  fixes (each checked to build):" if r["fixes"] else "  no single change makes it build")
    for _, _, label in r["fixes"]:
        print(f"    {label}")


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
            by_sl[i] = defender_defense(data, corpus, sl)[0]  # the DEFENDER the damage objective uses
        dfn, eff = by_sl[i], effective(data, b)
        for w, inf in set(b.weapons()):
            if inf == "?" or w not in data.weapons or data.weapons[w].get("isShield") or CATALYST.search(w):
                continue
            scored = sorted(((_type_damage(ar, respond(dfn, ar)), f) for f in data.weapons[w]["infusions"]
                             for ar in [attack_rating(data, w, f, eff)]), reverse=True)
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


# --------------------------------------------------------------------------------------------
# weapon flexibility: how many weapons a stat line can wield, against its stat neighbours

#: How many nearest-stat corpus builds a build's flexibility is ranked among (nearest_builds' k).
FLEX_K = 50

#: The optimizer's soft flexibility term, per weapon unlocked (one-handed and two-handed each count
#: once) per point spent: a step of `n` points that raises the build's 1H+2H count by `d` gains
#: FLEX_WEIGHT * d / n on top of its objective weight, which is itself the objective's gain per point
#: over the curve's early rate (see optimize_build). So at weight w, unlocking 1/w weapons is worth
#: one point of early-rate damage. Never a filter: a point that raises nothing but flexibility is
#: still outbid by one that raises the objective enough. `--flex-weight` overrides it.
#:
#: 0.02, from `--flex-sweep 0,0.01,0.02,0.05,0.1,0.2` over the EXPECT_BUILDS/EXPECT_ONE_HANDED cases
#: (2026-09-28, full corpus). At 0.02 two of the nine buildable cases move and none gets worse
#: flexibility: Uchigatana (Bleed) SL 150 bleed 221 -> 217 (-1.9%) for 26/116 -> 194/258 1H/2H
#: (percentile 0 -> 93); Dagger (Poison) SL 60 poison 198 -> 198 for 64/161 -> 130/224 (33 -> 69).
#: 0.05 already made Demon's Great Hammer (Raw) SL 70 *less* flexible (percentile 49 -> 26, the
#: greedy's path changing), and 0.1 cost the Uchigatana 4.0%.
FLEX_WEIGHT = 0.02

#: Flynn's Ring's physical bonus falls as equip load rises (MugenMonkey: "Damage increases with lower
#: equip load"). The curve is not modelled, so the spare load is shown and this is said beside it.
FLYNN_NOTE = "Flynn's Ring's bonus falls as load rises"


def flex_pool(data: Data) -> tuple[list[str], np.ndarray, np.ndarray, np.ndarray]:
    """Every weapon a hand can hold (EMPTY keys such as Bare_Fists left out; shields and catalysts
    in), as (keys, requirement matrix over REQ_STATS, the same with STR halved as weapon_ok halves
    it for two hands, weights). Cached on `data`: optimize_build asks per candidate point."""
    pool = getattr(data, "_flex_pool", None)
    if pool is None:
        keys = [k for k in data.weapons if k not in EMPTY]
        req = np.array([[int((data.weapons[k].get("require") or {}).get(s, 0)) for s in REQ_STATS]
                        for k in keys], dtype=np.int64).reshape(len(keys), len(REQ_STATS))
        two = req.copy()
        two[:, REQ_STATS.index("strength")] //= 2
        weight = np.array([float(data.weapons[k].get("weight", 0)) for k in keys])
        pool = (keys, req, two, weight)
        data._flex_pool = pool
    return pool


def flex_counts(data: Data, stats: dict) -> tuple[int, int]:
    """How many weapons `stats` wield one-handed and two-handed (weapon_ok, STR halved for 2H). A
    weapon wieldable one-handed is wieldable two-handed too, so the second count includes the first."""
    _, req, two, _ = flex_pool(data)
    have = np.array([stats.get(s, 0) for s in REQ_STATS])
    return int((req <= have).all(axis=1).sum()), int((two <= have).all(axis=1).sum())


#: Below this many weapons wielded one-handed, armour weight starts to count against defense. Measured
#: with --load-evidence over the corpus (21155 builds): builds wielding 81 or more 1H carry a
#: median load of 59% at VIT 20 in 23.7 of armour; 41-80, 41% at VIT 10 in 15.2; 11-40, 27-28% at
#: VIT 6 in about 11. The load falls across 80..40 and is flat below, so the term rises linearly
#: from 0 at LOAD_SCARCE_FROM to its full weight at LOAD_SCARCE_FULL.
LOAD_SCARCE_FROM = 80
LOAD_SCARCE_FULL = 40
#: The fraction of the best set's defense per weight a unit of armour weight costs at full scarcity.
#: Chosen from --load-evidence's sweep over the VIT 30 STR/DEX 8 case, whose best set is 551 defense
#: at 41.2 weight: 0.50 keeps it; 0.55-0.60 take 454 at 27.7 (0.67 of the weight, 0.82 of the
#: defense); 0.65-0.70 take 363 at 16.9; 0.90 takes 257.5 at 6.3. The corpus's builds wielding
#: 41-80 weapons one-handed wear 0.64 of a normal build's armour weight (15.2 against 23.7), and a
#: "similar defense" is the 0.82 set, not the half-defense one: 0.6.
LOAD_PRICE = 0.6


def load_scarcity(data: Data, stats: dict) -> float:
    """How far `stats` fall short of wielding a normal build's share of weapons, 0..1: 0 at
    LOAD_SCARCE_FROM or more one-handed (flex_counts), 1 at LOAD_SCARCE_FULL or fewer."""
    one = flex_counts(data, stats)[0]
    return min(1.0, max(0.0, (LOAD_SCARCE_FROM - one) / (LOAD_SCARCE_FROM - LOAD_SCARCE_FULL)))


def load_evidence(data: Data, corpus: list[Build], spell: str = "Climax") -> list[str]:
    """What real builds do with equip load against how many weapons their stats wield: the
    measurement behind the load weighting. Per group (every build, builds attuning `spell`, and
    builds whose INT/FTH meet `spell`'s requirements -- a proxy, for builds that record no spells),
    how many weapons the stats wield 1H and 2H (flex_counts, mean and median); then, per band of the
    1H count, the median VIT, armour weight, and load used (armour + weapons over the VIT's full
    equip load)."""
    def med(xs):
        return float(np.median(xs)) if xs else float("nan")

    def mean(xs):
        return float(np.mean(xs)) if xs else float("nan")

    counts = {id(b): flex_counts(data, b.stats) for b in corpus}
    req = data.spell_req.get(spell, {})
    groups = [("all builds", corpus),
              ("recording any spell", [b for b in corpus if b.spells]),
              (f"attuning {data.spells[spell]['name']}", [b for b in corpus if spell in b.spells]),
              (f"proxy: stats meet {data.spells[spell]['name']}'s "
               + " ".join(f"{LABEL[s]} {v}" for s, v in req.items()),
               [b for b in corpus if all(b.stats[s] >= v for s, v in req.items())])]
    out = [f"weapon pool {len(flex_pool(data)[0])}; wielded = flex_counts at the build's own stats"]
    for name, bs in groups:
        one = [counts[id(b)][0] for b in bs]
        two = [counts[id(b)][1] for b in bs]
        out.append(f"{name}: n {len(bs)}, 1H mean {mean(one):.1f} median {med(one):.0f}, "
                   f"2H mean {mean(two):.1f} median {med(two):.0f}, "
                   f"1H <= 5: {sum(1 for x in one if x <= 5)}")
    bands = [(0, 5), (6, 10), (11, 20), (21, 40), (41, 80), (81, 10**6)]
    out.append("by 1H count: n, median VIT, median armour weight, median load used")
    for lo, hi in bands:
        bs = [b for b in corpus if lo <= counts[id(b)][0] <= hi]
        vit = [b.stats["vitality"] for b in bs]
        arm = [sum(float(data.armor[s].get(p, {}).get("weight", 0)) for s, p in zip(ARMOR_SLOTS, b.armor))
               for b in bs]
        used = [(a + sum(float(data.weapons.get(w, {}).get("weight", 0)) for w, _ in b.weapons()))
                / data.equip_load[min(max(b.stats["vitality"], 1), len(data.equip_load) - 1)] for a, b in zip(arm, bs)]
        out.append(f"  {lo}-{hi if hi < 10**6 else 'max'}: n {len(bs)}, VIT {med(vit):.0f}, armour {med(arm):.1f}, "
                   f"load {med(used):.0%}")
    out.append("armour at scarcity 0 -> at load_scarcity (EXPECT_ARMOR_SCARCITY): defense, weight, set")
    mix = threat_mix(data, corpus)
    for weapon, st in EXPECT_ARMOR_SCARCITY:
        stats = dict(zip(STATS, st))
        wearer = Build("", stats, ["Naked"] * 4, [(weapon, "No_Infusion")], 0, [], [])
        s = load_scarcity(data, stats)
        got = [(best_armor(data, wearer, stats, mix, top=1, scarcity=sc)[2] or [None])[0] for sc in (0.0, s)]
        show = lambda t: "none" if t is None else f"{t[0]:.1f} {t[1]:.1f} {'/'.join(t[2])}"
        out.append(f"  {data.weapons[weapon]['name']} {st} wields {flex_counts(data, stats)[0]} 1H, scarcity {s:.2f}: "
                   f"{show(got[0])} -> {show(got[1])}")
    weapon, st = EXPECT_ARMOR_SCARCITY[0]
    stats = dict(zip(STATS, st))
    wearer = Build("", stats, ["Naked"] * 4, [(weapon, "No_Infusion")], 0, [], [])
    out.append(f"effective price (scarcity x LOAD_PRICE) against the first case's set: defense, weight")
    for k in (0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.9, 1.0):
        t = (best_armor(data, wearer, stats, mix, top=1, scarcity=k / LOAD_PRICE)[2] or [None])[0]
        out.append(f"  {k:.2f}: {t[0]:.1f} {t[1]:.1f}")
    return out


def flexibility(data: Data, corpus: list[Build], stats: dict, sl: int, armor: list[str] | None = None,
                rings: list[str] | None = None, k: int = FLEX_K) -> dict:
    """A stat line's weapon flexibility and where it sits among its stat neighbours.

    `one`/`two`: weapons wieldable one-handed / two-handed at the levelled stats (armour stat bonuses
    are not counted, for the build or for its neighbours). `score` = one + two. `below`/`equal`: how
    many of the k nearest_builds score below / the same, over their own levelled stats; `percentile`
    = 100 * (below + equal / 2) / n, the mid-rank percentile, so a build tied with every neighbour
    sits at 50. `spare`: equip load left under EQUIP_CAP at the build's VIT (max_load: with the
    rings' load factor and VIT bonus) after `armor` (piece
    keys head/chest/hands/legs, "Naked" for none) and `rings` (keys, each worn once) -- the weapon
    weight the build can still carry; `fits`: how many weapons it wields in either grip that weigh
    no more than that on their own."""
    one, two = flex_counts(data, stats)
    score = one + two
    scores = [sum(flex_counts(data, b.stats)) for b in nearest_builds(data, stats, sl, corpus, k)]
    below = sum(1 for s in scores if s < score)
    equal = sum(1 for s in scores if s == score)
    n = len(scores)
    cap = max_load(data, stats, [r for r in rings or [] if r in data.rings]) * EQUIP_CAP
    # Added one at a time, not with sum(): Python 3.12's sum() of floats is compensated, so it
    # rounds differently from the plain running total the Rust port keeps.
    spare = cap
    for slot, p in zip(ARMOR_SLOTS, armor or []):
        spare -= float(data.armor[slot].get(p, {}).get("weight", 0))
    for r in rings or []:
        spare -= float(data.rings.get(r, {}).get("weight", 0))
    keys, _, halved, weight = flex_pool(data)
    have = np.array([stats.get(s, 0) for s in REQ_STATS])
    fits = int(((halved <= have).all(axis=1) & (weight <= spare)).sum())
    return {"one": one, "two": two, "total": len(keys), "score": score, "below": below, "equal": equal,
            "n": n, "percentile": 100.0 * (below + equal / 2) / n if n else 50.0, "spare": spare, "fits": fits}


def flex_line(f: dict) -> str:
    """The panel's flexibility line, word for word (`flex_line` in ds2-build-recommender-core)."""
    return (f"wields {f['one']}/{f['total']} 1H, {f['two']} 2H -- {ordinal(round(f['percentile']))} "
            f"percentile of {f['n']} similar builds")


def flex_load_line(f: dict) -> str:
    """The panel's load line under the flexibility line, word for word."""
    return f"{f['spare']:.1f} weight left for weapons under {EQUIP_CAP:.0%} load, {f['fits']} fit -- {FLYNN_NOTE}"


def ordinal(n: int) -> str:
    """1st, 2nd, 3rd, 4th, 11th, 12th, 13th, 21st."""
    suffix = "th" if 10 <= n % 100 <= 20 else {1: "st", 2: "nd", 3: "rd"}.get(n % 10, "th")
    return f"{n}{suffix}"


COMMON_RING = 0.10  # a ring worn by at least this share of every corpus build counts as common to all builds
#: Rings a generated build never suggests or grants, however many builds wear them. Agape Ring
#: ("Absorbs souls.") is worn by 15.8% of the corpus to hold soul memory down, not for a fight.
NO_USE_RINGS = ("Agape_Ring",)


def suggest_rings(data: Data, near: Counter, every: Counter | None = None, n: int = 4,
                  worn: list[str] = ()) -> list[str]:
    """The `n` rings `near` counts most, except that each NO_USE_RINGS ring's place goes, in
    place, to the ring the nearest builds wear most (then all builds) that is neither a no-use
    ring nor in the upgrade group of a ring already in the list: never a second Ring of Blades.
    A ring in the group of one already `worn` (optimize_build's) is left out, as a no-use ring is."""
    group = lambda r: data.rings[r].get("group", r)
    held = {group(r) for r in worn}
    near = Counter({r: c for r, c in near.items() if group(r) not in held})
    every = every and Counter({r: c for r, c in every.items() if group(r) not in held})
    top = [r for r, _ in near.most_common(n)]
    taken = {group(r) for r in top if r not in NO_USE_RINGS} | held
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
                   rings: list[str], listed=(), catalysts=(), sl: int | None = None,
                   granted=()) -> tuple[list[str], str | None, dict]:
    """The generated build's armour, for a build wearing `rings` and holding the heaviest one of the
    primary and the `listed` weapons (keys) -- any one of them can be equipped in its place without
    passing 70% -- plus, for a build that casts, the heaviest of its `catalysts` (keys), which is
    held beside the weapon. As display names head/chest/hands/legs ("Naked" for a slot left bare),
    a note when the load cap left a slot bare or left no set at all -- never a silent naked build --
    or when poise drove the pick, and the trade numbers (below; {} without them).

    Which set: best_armor's top set, unless a set that buys poise trades better. With `sl` and
    `granted` ((key, infusion, two-handed) of every weapon the build lists, primary first) the
    candidates are best_armor's top set and, per poise step the budget reaches, best_armor's pick
    among the sets reaching it (armor_poise_steps); each is valued by its exchange value
    (exchange_value) -- the mean, over the granted weapons and the bracket's counter-hits
    (trade_counters), of the damage of my R1's first hit when it lands less the counter's first hit
    against this set's defense when that lands, both pressing R1 at once (trade). Poise decides
    whether I land when the counter hits first; it is worth buying only as far as that damage won
    back outweighs what the lighter, better-defended set saves. The highest value wins; a tie keeps
    best_armor's set. The numbers: `poise` the build's (armor_poise: pieces, stat poise, rings),
    `poise_target` the largest counter poise damage it holds through (0: none), `trade_rate` the
    share of trades where my hit lands, `exchange` the value, the same three for best_armor's set
    under `previous`, `basis` what was weighed, `candidates` every set weighed as (keys, numbers).

    Before this the set was chosen for the primary alone, so a build whose other weapons were
    heavier than its primary could equip only the primary under 70% (user report 2026-10-01)."""
    weight = lambda k: data.weapons.get(k, {}).get("weight", 0)
    held = [(max([weapon, *listed], key=weight), inf)]  # max keeps the first of a tie: the primary
    if catalysts:
        held.append((max(catalysts, key=weight), "No_Infusion"))
    wearer = Build("", stats, ["Naked"] * 4, held, int(two), rings, [])
    mix, scarcity = threat_mix(data, corpus), load_scarcity(data, stats)
    cap, carried, sets = best_armor(data, wearer, dict(stats), mix, top=1, scarcity=scarcity)
    if not sets:
        what = "the heaviest weapon, its catalyst and the rings" if catalysts else "the heaviest weapon and the rings"
        return [], (f"no armor fits: {what} weigh {carried:.1f}, over the {cap:.1f} a "
                    f"{EQUIP_CAP:.0%} load allows at VIT {stats['vitality']}"), {}
    pieces, info, poise_note = sets[0][2], {}, None
    table = granted_trades(data, corpus, sl, stats, rings, granted) if sl is not None and granted else None
    if table is not None:
        prev = armor_trade(data, table, pieces, stats, rings, two)
        best, best_pieces, cands = prev, pieces, [(list(pieces), prev)]
        for _, ps in armor_poise_steps(data, wearer, dict(stats), mix, scarcity):
            if list(ps) == list(pieces):
                continue
            s = armor_trade(data, table, ps, stats, rings, two)
            cands.append((list(ps), s))
            if s["exchange"] > best["exchange"]:
                best, best_pieces = s, ps
        info = best | {"previous": prev, "basis": table["basis"], "candidates": cands}
        if best_pieces != pieces:
            poise_note = (f"poise drove the pick: {best['poise']:.0f} poise over best_armor's "
                          f"{prev['poise']:.0f} holds through counter hits up to {best['poise_target']:.0f} "
                          f"poise damage; exchange {best['exchange']:+.1f} per trade against "
                          f"{prev['exchange']:+.1f}, lands {best['trade_rate']:.0%} of trades against "
                          f"{prev['trade_rate']:.0%}")
        pieces = best_pieces
    bare = [s for s, p in zip(ARMOR_SLOTS, pieces) if p == "Naked"]
    note = (f"{', '.join(bare)} left bare: nothing wearable there fits the {cap - carried:.1f} of load "
            f"left under {EQUIP_CAP:.0%}") if bare else None
    note = "; ".join(n for n in (note, poise_note) if n) or None
    return [data.armor[s][p]["name"] for s, p in zip(ARMOR_SLOTS, pieces)], note, info


def generate_build(data: Data, corpus: list[Build], weapon: str, inf: str, sl: int, objective: str = "damage",
                   window: float = 1.5, k: int = 50, allow_naked: bool = False,
                   grip: str = "two", flex_weight: float | None = None, spells=(),
                   only_class: str | None = None, use_floors: bool = True, defender=None) -> dict | None:
    """A whole valid build for weapon+infusion at `sl`: optimize_build's class and stats with the
    weapon as primary, able to attune and cast every one of `spells` (their names come back with
    the slots they cost and the slots the build's ATT gives, and the catalyst best_catalysts picks
    to cast each category of them with); the top 15 one-handable and top 5
    two-hand-only other weapons for those
    stats (damage over `window` seconds); 3 copies of each of the 4 rings the nearest-stat builds
    wear most (suggest_rings: no NO_USE_RINGS ring), plus one of every other ring at least
    COMMON_RING of all builds wear; and armour, generate_armor's set under 70% load with the
    heaviest listed weapon and those four rings carried: best_armor's, or a higher-poise one when
    its R1 trades against the bracket over every listed weapon (each in the grip its listed damage
    is swung in) are worth more -- `poise`, `poise_target`, `trade_rate`, `exchange`,
    `trade_previous` (best_armor's set's numbers), `trade_basis` and `trade_candidates` (every set
    weighed, as (keys, numbers)) are generate_armor's, over `granted`. `allow_naked`
    skips the armour, as every generated build did before it had any. `only_class` is
    optimize_build's: the build for a character that already has a class.

    The rings optimize_build wears in place of stat points come first, each in the place of one of
    the four (suggest_rings fills the rest), and `ring_trades` says what each stood in for
    (ring_trades). The weapons, the catalysts and the armour's requirements are read at the stats
    those rings give; `slots` counts their attunement slots. `defender` (defender_defense) is what
    the objective and the other weapons' damage are scored against; the armour is still chosen
    against the corpus threat mix, which is what this build's own wearer is hit by."""
    best, floors = optimize_build(data, corpus, weapon, inf, sl, objective, grip, flex_weight, spells,
                                  only_class, use_floors, defender=defender)
    if best is None:
        return None
    val, cls, two, stats, worn = best
    eff = gear_stats(data, stats, worn)
    assert all(spell_ok(data, s, eff) for s in spells)
    slots = (sum(data.spells[s]["slots"] for s in spells), slots_of(data, eff, worn))
    assert slots[0] <= slots[1]
    rows, _, _ = weapons_for(data, eff, sl, corpus, top=10_000, window=window, defender=defender)
    one, only2, seen, grips = [], [], {data.weapons[weapon]["name"]}, {}
    for dmg, name, winf, ar, label in rows:
        if name in seen:
            continue
        seen.add(name)
        grips[name] = label.startswith("2H")  # the grip its listed damage is swung in
        (only2 if "2H only" in label else one).append((name, winf, round(dmg)))
    # dict.fromkeys, not set: most_common breaks ties by first-seen order, and a set of strings
    # iterates in the per-process hash order, so tied rings came out differently on every run
    near = Counter(r for b in nearest_builds(data, stats, sl, corpus, k) for r in dict.fromkeys(b.rings)
                   if r and r in data.rings)
    every = Counter(r for b in corpus for r in dict.fromkeys(b.rings) if r and r in data.rings)
    suggested = worn + suggest_rings(data, near, every, 4 - len(worn), worn)
    common = [r for r, c in every.most_common() if c >= COMMON_RING * len(corpus) and r not in suggested
              and r not in NO_USE_RINGS]
    by_name = {}
    for key, w in data.weapons.items():
        by_name.setdefault(w["name"], key)
    listed = [by_name[name] for name, _, _ in one[:15] + only2[:5]]
    catalysts = [c for _, c, _, _ in best_catalysts(data, spells, eff)]
    granted = [(weapon, inf, two)] + [(by_name[n], i, grips[n]) for n, i, _ in one[:15] + only2[:5]]
    armor, armor_note, trades = ([], None, {}) if allow_naked else generate_armor(
        data, corpus, weapon, inf, two, stats, suggested, listed, catalysts, sl, granted)
    return {"class": cls, "sl": sl, "stats": stats, "two_handed": two, "objective": objective, "value": round(val),
            "primary": (data.weapons[weapon]["name"], inf), "weapons_1h": one[:15], "weapons_2h_only": only2[:5],
            "rings": [data.rings[r]["name"] for r in suggested for _ in range(3)]
            + [data.rings[r]["name"] for r in common], "armor": armor, "armor_note": armor_note,
            "poise": trades.get("poise"), "poise_target": trades.get("poise_target"),
            "trade_rate": trades.get("trade_rate"), "exchange": trades.get("exchange"),
            "trade_previous": trades.get("previous"), "trade_basis": trades.get("basis"),
            "trade_candidates": trades.get("candidates"), "granted": granted,
            "flex": flexibility(data, corpus, stats, sl, armor_keys(data, armor), suggested),
            "spells": [data.spells[s]["name"] for s in spells], "slots": slots,
            "ring_trades": ring_trades(data, corpus, weapon, sl, grip, spells, cls, use_floors, worn),
            "catalysts": [(label, data.weapons[c]["name"], p, over and data.weapons[over]["name"])
                          for label, c, p, over in best_catalysts(data, spells, eff)]}


def ring_effect_text(data: Data, ring: str) -> str:
    """What `ring` does, from ring_gear: "attunement slots +3", "STR +5", "equip load x1.2", ..."""
    e = ring_gear(data)[ring]
    out = [f"{LABEL[s]} +{v}" for s, v in e["add"].items()]
    if e["scaled"]:
        _, _, _, lo, hi = e["scaled"][0]
        out.append("/".join(LABEL[x[0]] for x in e["scaled"]) + f" +{lo} to +{hi} by the stat")
    if e["slots"]:
        out.append(f"attunement slots +{e['slots']}")
    for what, name in (("hp", "max HP"), ("load", "equip load"), ("stamina", "max stamina")):
        if e[what] != 1.0:
            out.append(f"{name} x{e[what]!r}")
    return ", ".join(out)


def ring_trades(data: Data, corpus: list[Build], weapon: str, sl: int, grip: str, spells, cls: str,
                use_floors: bool, worn: list[str]) -> list[tuple[str, str, list[tuple[str, int, int]]]]:
    """Per ring optimize_build wore in place of stat points: (its name, ring_effect_text, the stats
    class `cls` levels less for it, as (label, without the ring, with it)) -- ring_lift with every
    worn ring against the same without that one. An offensive ring (offense_rings) stands in for
    nothing and is not listed."""
    floors, r1, cut = build_floors(data, corpus, sl)
    if not use_floors:
        floors = {}
    need_floors = {s: floors.get(s, 0) for s in floor_stats(spells)}
    if r1.get(weapon, 0) >= cut:
        need_floors["endurance"] = floors.get("endurance", 0)
    req = _grip_req(data, weapon, GRIP_TRIES[grip][0])
    base = {s: int(data.classes[cls][s]) for s in STATS}
    lift = lambda rs: ring_lift(data, base, need_floors, req, spells, rs)
    with_all = lift(worn)
    out = []
    for r in [x for x in worn if x in ring_gear(data)]:
        without = lift([x for x in worn if x != r])
        # None: without it no ATT holds the spells
        moved = None if without is None else [(LABEL[s], without[s], with_all[s]) for s in STATS
                                               if without[s] != with_all[s]]
        out.append((data.rings[r]["name"], ring_effect_text(data, r), moved))
    return out


def trade_line(t: tuple) -> str:
    """One ring_trades entry as the panel shows it, word for word."""
    name, effect, moved = t
    if moved is None:
        return f"{name}: {effect} -> no ATT holds the spells without it"
    return f"{name}: {effect} -> " + (", ".join(f"{s} {a} -> {b}" for s, a, b in moved) or "no stat lowered")


def armor_keys(data: Data, names: list[str]) -> list[str]:
    """Armour display names, head/chest/hands/legs, back to keys: the first piece of the slot with
    that name, as the Rust port looks one up."""
    out = []
    for slot, name in zip(ARMOR_SLOTS, names):
        out.append(next((k for k, v in data.armor[slot].items() if v.get("name", k) == name), "Naked"))
    return out


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
#   T table v0 v1 ...                                         an attack-bonus, defense-bonus,
#                                                             equip-load, attunement-slots,
#                                                             max-stamina or cast-bonus table
#   W key name class flags weight require ha1h ha2h counter   a weapon; flags: S shield, C catalyst,
#     damagescale                                             H high-stamina R1; require: stat:value,..;
#                                                             damagescale: WeaponParam.damageScale
#   I code atk(7) scale(9) rate(5)                            one infusion of the last W; rate: its
#                                                             moved rates in DMG order (0 unread)
#   L grip t:mv:type:lower[:flat(5)] ...                      the last W's R1 chain, grip 1 or 2;
#                                                             flat: the hit's damage row's flat
#                                                             attack in DMG order, comma-separated,
#                                                             only when a value is nonzero
#   B bracket floors(5) defense(8) stats(9)                   one SL bracket: its average
#                                                             defender and median stats
#                                                             (bracket_stats), which a --defender
#                                                             set is worn at
#   R key name weight group                                   a ring; group: its upgrade line
#   N key                                                     a ring in NO_USE_RINGS
#   O ring add(5)                                             data.ring_attack, in its order: a
#                                                             ring's flat attack adds in DMG order
#   E ring weight mul add(9)                                  a ring --minimum may wear
#   G ring slots hp load stamina add(9) scaled                ring_gear, in its order: a ring
#                                                             optimize_build counts; scaled is
#                                                             stat:low:high:bonusLow:bonusHigh,..
#   A slot key name weight alter(9) require                   armour --minimum may wear
#   P slot key name weight def(5) require typed(3) bonus      every armour piece, for best_armor
#     alter(9)                                                and build_defense: typed is its
#                                                             slash/strike/thrust defense, bonus
#                                                             its physicalDEFBonus, alter its
#                                                             stat changes
#   H physical magic fire lightning dark                      the corpus threat mix (threat_mix)
#   K n top1 top2                                             calibrate_infusions
#   M ring count                                              a ring worn by >= COMMON_RING of builds
#   Z key name slots category require                         a spell: its attunement slot cost,
#                                                             SpellParam category (-1 unread) and
#                                                             INT/FTH requirements (spell_floors)
#   Y key name categories require base:scale(4)               a catalyst (regulation_catalysts):
#                                                             the categories it casts, one digit
#                                                             each, and magic fire lightning dark
#                                                             cast power terms
#   X bracket stat-brackets ring,ring.. weapon:code,..        one corpus build
#   RG ammo kind hand volley                                  the last W is a bow, greatbow or
#                                                             crossbow (data.ranged): the arrowType
#                                                             it fires, bow/greatbow/crossbow, its
#                                                             hand scale, 1 when VOLLEY_CROSSBOWS
#                                                             names it
#   RS types flat(5) mv lower shots poison bleed              one special shot of the last RG: its
#                                                             row's DMG types (indices, comma-
#                                                             separated), flat attack in DMG order,
#                                                             damageRate, damageLower, shots per
#                                                             pull, flat status build-up
#   AM name type flat(5) poison bleed mv lower                one ammunition (data.ammo), in name
#                                                             order as ranged_options reads them
#   RD ring type:value,..                                     data.ring_defense, in its order: a
#                                                             ring's defense changes in displayed
#                                                             units, type a DMG index, in the
#                                                             change's own key order
#   DB name source seconds type:value,..                      data.defense_buffs, in its order:
#                                                             source spell or item
#   RM grip reach startup recovery first-hit                  the last W's r1_metrics that do not
#                                                             depend on the build ("-" for None)
#   CH grip step spd hits step spd hits                       the last W's R1 chain, one record per
#                                                             name chain_timeline tries (in order,
#                                                             a repeat of one normalized name
#                                                             dropped): its 1st and 2nd attack,
#                                                             each its chain_open step ("-" None),
#                                                             mean play speed and live hits
#                                                             start:end:rate:n:interval:type:lower:
#                                                             flat(5, comma-separated, or "-");
#                                                             the 2nd attack's fields "-" when it
#                                                             has none, so the 1st repeats
#   PO grip rate cover t:poise:break ..                       the last W's poise_metrics inputs: the
#                                                             R1's uninterruptibleRate and
#                                                             hyperarmor_cover, and its chain's hits
#                                                             out to 10 s with poise data
#   and, trailing: B its bracket_poise mean poise; P the piece's poise; X the build's counter-hit
#   (build_counter) as poise:break, "-" for none
#   SP status damage lockout                                  data.status_procs: what a poison or
#                                                             bleed proc deals, and the seconds the
#                                                             status is locked out after
#   and, trailing: P the piece's poison and bleed resistance (data.armor_status, 0 for none); X the
#   build's status_cut poison:bleed; T poisonResistance/bleedingResistance, data.status_resist
#   SC melee ranged r1-1h r2-1h r1-2h r2-2h                   the last W's data.stamina_cost: its
#                                                             melee and ranged base costs and the
#                                                             R1_COST_FIELD rates, 1H 1st/2nd then
#                                                             2H 1st/2nd
#   FS grip strike                                            the last W's first_strike in the grip
#                                                             (2: a grip with no R1 row swings 1H)
#   XT strike poise defense(8) ar(5)                          the last X's trade_counter: its
#                                                             first_strike (Stone Ring added), its
#                                                             max poise, build_defense and AR
#     where strike is t pd ab mv type lower flat(5) scale rate armor: seconds to the first hit at
#     the game play speed, poise damage, armorBreak, motion value, type (p s k t), damageLower, flat
#     attack in DMG order, damageScale, uninterruptibleRate, hyperarmor windows s:e,.. ("-" none)
#   and, trailing: R the ring's poise (ring_poise); T statPoise, data.stat_poise by min(END, ADP)

BACKEND_DATA_NAME = "ds2-build-recommender.dat"
BACKEND_DATA = Path.home() / ".cache/ds2-builds" / BACKEND_DATA_NAME
BACKEND_FORMAT = "ds2-build-recommender-data 19"
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
                  "auxATKBonus", "mundaneATKBonus",
                  # build_defense's, for a --defender set worn at a bracket's median stats
                  "physicalDEFBonus", "magicDEFBonus", "fireDEFBonus", "lightningDEFBonus", "darkDEFBonus"]


def _num(v) -> str:
    if isinstance(v, float):
        return repr(v)
    assert isinstance(v, int) and not isinstance(v, bool), v
    return str(v)


def _stat_pairs(d: dict) -> str:
    """`index:value,...` over STATS, in the dict's own order (key presence matters to --minimum)."""
    assert all(s in STATS for s in d), d
    return ",".join(f"{STATS.index(s)}:{_num(v)}" for s, v in d.items())


def _opt(v) -> str:
    """`_num`, or `-` for None."""
    return "-" if v is None else _num(v)


def _chain_records(data: Data, attacks: dict, key: str, name: str) -> list[str]:
    """export_backend's RM and CH records for one weapon: r1_metrics' build-independent numbers,
    and per name chain_timeline tries, the R1 chain's two attacks as chain_timeline reads them."""
    out = []
    names = [name, key.replace("_", " ")]
    zero = dict.fromkeys(DMG + PHYS_TYPES, 0.0)
    for two in (False, True):
        m = r1_metrics(data, attacks, key, names, two, {}, zero)
        got = [m["reach_m"], m["startup_s"], m["recovery_s"], m["time_to_first_hit_s"]]
        if any(v is not None for v in got):
            out.append("\t".join(["RM", "2" if two else "1", *map(_opt, got)]))
    for two in (False, True):
        g = "Single2Hand" if two else "Single1Hand"
        for nm in dict.fromkeys(names):
            if nm != name and norm(nm) == norm(name):
                continue
            first = attacks.get((norm(nm), g + "Normal1st"))
            if not first:
                continue
            fields = ["CH", "2" if two else "1"]
            for a in (first, attacks.get((norm(nm), g + "Normal2nd"))):
                if not a:
                    fields += ["-", "-", "-"]
                    continue
                hits = " ".join(
                    ":".join([_num(h["start"]), _num(h["end"]), _num(h["rate"]), _num(max(1, h.get("n") or 1)),
                              _num(h.get("interval") or 0), h.get("type") or "physical", _num(h.get("lower", 0)),
                              ",".join(map(_num, h["flat"])) if h.get("flat") else "-"])
                    for h in live_hits(a))
                fields += [_opt(chain_open(a["anim"])), _num(sum(a.get("spd") or [1.0, 1.0]) / 2), hits or "-"]
            out.append("\t".join(fields))
    return out


def _strike_fields(hit: dict) -> list[str]:
    """A first_strike as the FS and XT records carry it."""
    return [_num(hit["t"]), _num(float(hit["pd"])), _num(int(hit["ab"])), _num(float(hit["mv"])),
            HIT_CODE.get(hit["kind"], "p"), _num(hit["lower"]), ",".join(_num(v) for v in hit["flat"]),
            _num(float(hit["scale"])), _num(float(hit["rate"])),
            ",".join(f"{_num(s)}:{_num(e)}" for s, e in hit["armor"]) or "-"]


def _poise_records(data: Data, attacks: dict, key: str, name: str) -> list[str]:
    """export_backend's PO records for one weapon: per grip, what poise_metrics reads that no
    build changes -- the R1's hyperarmor rate and cover, and its chain's hits out to 10 s as
    (seconds, poise damage, armorBreak) -- when the chain has a hit with poise data."""
    out = []
    names = [name, key.replace("_", " ")]
    for two in (False, True):
        a = r1_attack(attacks, names, two)
        if not a:
            continue
        tl = next((t for t in (chain_timeline(attacks, nm, two, "Normal", 10.0, with_row=True) for nm in names)
                   if t), None)
        hits = [(t[0], *hp) for t in tl or [] if (hp := hit_poise(data, key, t[-1]))]
        if not hits:
            continue
        rate = data.weapon_poise[key][1]
        cover = hyperarmor_cover(a) if rate > 0 else 0.0
        out.append("\t".join(["PO", "2" if two else "1", _num(float(rate)), _num(float(cover)),
                              " ".join(f"{_num(t)}:{_num(float(pd))}:{_num(int(ab))}" for t, pd, ab in hits)]))
    return out


def export_backend(data: Data, corpus: list[Build]) -> str:
    attacks = load_attacks(data)
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
    out.append("\t".join(["T", "attunementSlots", *(_num(v) for v in data.att_slots)]))
    if getattr(data, "stat_poise", None):  # armor_poise's stat term
        out.append("\t".join(["T", "statPoise", *(_num(v) for v in data.stat_poise)]))
    timing = speed_windows()  # first_strike's: the game play speed when the anibnd windows were read
    if data.stamina_max:
        out.append("\t".join(["T", "staminaMax", *(_num(v) for v in data.stamina_max)]))
    if data.hp_max:
        out.append("\t".join(["T", "hpMax", *(_num(v) for v in data.hp_max)]))
        out.append("\t".join(["T", "additionalHp", *(_num(v) for v in data.additional_hp)]))
    for e, col in data.cast_bonus.items():
        out.append("\t".join(["T", "cast" + e.capitalize(), *(_num(v) for v in col)]))
    # status_cut's stat columns, and what a proc does (row_status)
    for s, (_, col, _, _) in STATUS_PROC.items():
        if data.status_resist.get(s):
            out.append("\t".join(["T", col, *(_num(float(v)) for v in data.status_resist[s])]))
    for s, p in data.status_procs.items():
        out.append("\t".join(["SP", s, _num(p["damage"]), _num(float(p["lockout"]))]))
    for key, s in data.spells.items():
        if key in data.spell_req:  # spell_ok refuses a spell with no requirements row
            out.append("\t".join(["Z", key, s["name"], _num(s["slots"]), _num(data.spell_category.get(key, -1)),
                                  _stat_pairs(data.spell_req[key]) or "-"]))
    for key, c in data.catalysts.items():
        out.append("\t".join(["Y", key, data.weapons[key]["name"], "".join(map(str, c["categories"])),
                              _stat_pairs(c["require"]) or "-",
                              *(f"{_num(float(b))}:{_num(float(s))}" for b, s in c["power"].values())]))
    for key, w in data.weapons.items():
        flags = ("S" if w.get("isShield") else "") + ("C" if CATALYST.search(key) else "") \
            + ("H" if r1.get(key, 0) >= cut else "")
        ctr = (crit.get(norm(w["name"])) or {}).get("counter") or 0
        out.append("\t".join(["W", key, w["name"], data.weapon_class.get(key) or "", flags or "-",
                              _num(w.get("weight", 0)), _stat_pairs(w.get("require") or {}) or "-",
                              _num(float(hyperarmor(attacks, rates, w["name"], False))),
                              _num(float(hyperarmor(attacks, rates, w["name"], True))), _num(ctr),
                              _num(float(data.damage_scale.get(key, 1.0)))]))
        for inf, row in w["infusions"].items():
            atk, sc = row.get("atk") or {}, row.get("atkScale") or {}
            rate = row.get("rate") or {}
            out.append("\t".join(["I", INFUSION_CODE[inf], *(_num(atk.get(k, 0)) for k in ATK_KEYS),
                                  *(_num(sc.get(k, 1 if k == "modifier" else 0)) for k in SCALE_KEYS),
                                  *(_num(float(rate.get(k, 0.0))) for k in DMG)]))
        for two in (False, True):
            tl = r1_timeline(attacks, w["name"], two) or r1_timeline(attacks, key.replace("_", " "), two)
            if tl:
                out.append("\t".join(["L", "2" if two else "1", " ".join(
                    f"{_num(t)}:{_num(mv)}:{HIT_CODE.get(ty, 'p')}:{_num(lo)}"
                    + (":" + ",".join(map(_num, fl)) if any(fl) else "") for t, mv, ty, lo, fl in tl)]))
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
        if key not in data.ranged:  # what row_metrics and poise_metrics read; a launcher has none
            out.extend(_chain_records(data, attacks, key, w["name"]))
            out.extend(_poise_records(data, attacks, key, w["name"]))
            for two in (False, True):  # what granted_trades swings
                hit = first_strike(data, attacks, key, two, timing)
                if hit:
                    out.append("\t".join(["FS", "2" if two else "1", *_strike_fields(hit)]))
        r = data.ranged.get(key)
        if r:
            out.append("\t".join(["RG", _num(r["ammo"]), r["kind"], _num(float(r["hand"])),
                                  "1" if w["name"] in VOLLEY_CROSSBOWS else "0"]))
            for s in r["special"]:
                out.append("\t".join(["RS", ",".join(str(DMG.index(k)) for k in s["types"]) or "-",
                                      *(_num(s["flat"].get(k, 0)) for k in DMG), _num(s["mv"]), _num(s["lower"]),
                                      _num(s["shots"]), *(_num(s["status"].get(k, 0)) for k in STATUS_PROC)]))
        c = data.stamina_cost.get(key)
        if c:  # what attack_stamina and shot_stamina read
            out.append("\t".join(["SC", _num(c["melee"]), _num(c["ranged"]),
                                  *(_num(c["rates"][R1_COST_FIELD[(two, second)]])
                                    for two in (False, True) for second in (False, True))]))
    for name, a in sorted(data.ammo.items()):
        out.append("\t".join(["AM", name, _num(a["type"]), *(_num(a["flat"].get(k, 0)) for k in DMG),
                              *(_num(a["status"].get(k, 0)) for k in STATUS_PROC), _num(a["mv"]), _num(a["lower"])]))
    floors = bracket_floors(data, corpus, r1)
    for i, (lo, _) in enumerate(SL_BRACKETS):
        dfn, _ = bracket_defense(data, corpus, lo)
        st, _ = bracket_stats(data, corpus, lo)
        f = floors[i]
        out.append("\t".join(["B", str(i), *(_num(f.get(s, 0)) for s in FLOOR_STATS + ["endurance"]),
                              *(_num(float(dfn[k])) for k in DMG + PHYS_TYPES), *(_num(st[s]) for s in STATS),
                              _num(float(bracket_poise(data, corpus, lo, attacks)["poise"]))]))
    ring_ix = {k: i for i, k in enumerate(data.rings)}
    for key, r in data.rings.items():
        out.append("\t".join(["R", key, r.get("name", key), _num(r.get("weight", 0)), r.get("group", key),
                              _num(ring_poise(data).get(key, 0))]))
    out.extend("\t".join(["N", key]) for key in NO_USE_RINGS if key in data.rings)
    for key, add in data.ring_attack.items():
        out.append("\t".join(["O", key, *(_num(add.get(k, 0)) for k in DMG)]))
    # What an adaptive defender answers with (AdaptiveDefense): every ring's defense changes, from
    # which counter_rings and the swap are worked out over the X builds' rings, and the buffs.
    for key, add in data.ring_defense.items():
        out.append("\t".join(["RD", key, ",".join(f"{DMG.index(k)}:{_num(v)}" for k, v in add.items())]))
    for name, b in data.defense_buffs.items():
        out.append("\t".join(["DB", name, b["source"], _num(float(b["seconds"])),
                              ",".join(f"{DMG.index(k)}:{_num(v)}" for k, v in b["add"].items())]))
    for slot in ARMOR_SLOTS:
        for key, v in data.armor[slot].items():
            # typed and bonus as build_defense reads them (a missing typed defense is the general
            # physical one), alter as effective adds it (numbers only)
            alt = {s: x for s, x in (v.get("alter") or {}).items() if isinstance(x, (int, float))}
            out.append("\t".join(["P", slot, key, v.get("name", key), _num(v.get("weight", 0)),
                                  *(_num(v.get(k + "DEF", 0)) for k in DMG),
                                  _stat_pairs(v.get("require") or {}) or "-",
                                  *(_num(v.get(t + "DEF", v.get("physicalDEF", 0))) for t in PHYS_TYPES),
                                  _num(v.get("physicalDEFBonus", 0)), *(_num(alt.get(s, 0)) for s in STATS),
                                  _num(v.get("poise", 0)),
                                  *(_num(float((data.armor_status.get(slot, {}).get(key) or {}).get(s, 0.0)))
                                    for s in STATUS_PROC)]))
    mix = threat_mix(data, corpus)
    out.append("\t".join(["H", *(_num(float(mix[k])) for k in DMG)]))
    for key, (weight, add, mul) in sorted(ring_effects(data).items()):
        out.append("\t".join(["E", key, _num(weight), _num(float(mul)), *(_num(add.get(s, 0)) for s in STATS)]))
    for key, e in ring_gear(data).items():
        out.append("\t".join(["G", key, _num(e["slots"]), _num(float(e["hp"])), _num(float(e["load"])),
                              _num(float(e["stamina"])), *(_num(e["add"].get(s, 0)) for s in STATS),
                              ",".join(f"{STATS.index(s)}:{lo}:{hi}:{a}:{b}" for s, lo, hi, a, b in e["scaled"])
                              or "-"]))
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
        # The trailing `one:two` is flex_counts over the build's levelled stats, which the file does
        # not otherwise carry: what a neighbour scores in flexibility(). Added without a format bump:
        # a reader that predates it stops at the weapons field, and one that has it treats a file
        # without it as having no flexibility to rank against.
        one, two = flex_counts(data, b.stats)
        hp = build_counter(data, attacks, b)  # what bracket_poise counts as its counter-hit
        out.append("\t".join(["X", str(sl_bracket(soul_level(data, b))),
                              "".join(str(stat_bracket(eff[s])) for s in STATS),
                              ",".join(str(ring_ix[r]) for r in b.rings if r and r in data.rings) or "-",
                              ",".join(f"{weapon_ix[w]}:{INFUSION_CODE[inf]}" for w, inf in b.weapons()) or "-",
                              f"{one}:{two}", f"{_num(float(hp[0]))}:{_num(int(hp[1]))}" if hp else "-",
                              ":".join(_num(float(status_cut(data, b, s))) for s in STATUS_PROC)]))
        c = trade_counter(data, attacks, b, timing)  # what trade_counters counts for the build
        if c:
            out.append("\t".join(["XT", *_strike_fields(c), _num(float(c["poise"])),
                                  ",".join(_num(float(c["dfn"][k])) for k in DMG + PHYS_TYPES),
                                  ",".join(_num(c["ar"].get(k, 0)) for k in DMG)]))
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
    # the AR goal: raw AR with the window dropped, so this ranks as the third case does
    ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, True, None, False, 1.5, False, "ar"),
]
EXPECT_BUILDS = [  # weapon key, infusion, sl, objective: --optimize and --generate
    ("Demons_Great_Hammer", "Raw", 100, "damage"),
    ("Moonlight_Greatsword", "No_Infusion", 33, "damage"),
    ("Demons_Great_Hammer", "Raw", 20, "damage"),
    ("Demons_Great_Hammer", "Raw", 70, "damage"),
    ("Moonlight_Greatsword", "No_Infusion", 90, "damage"),
    ("Uchigatana", "Bleed", 150, "bleed"),
    ("Dagger", "Poison", 60, "poison"),
    # the AR goal: a split infusion that scales with nothing past its requirements, so the AR the
    # worn Ring of Blades+2 adds is what the goal reads; and one whose lightning half scales with FTH
    ("Black_Dragon_Greataxe", "Lightning", 100, "ar"),
    ("Uchigatana", "Lightning", 150, "ar"),
    # a crossbow scored by its best shot; dark reads min(INT, FTH), so the two rise together
    ("Sanctum_Crossbow", "Dark", 150, "damage"),
]
EXPECT_NAKED = [("Demons_Great_Hammer", "Raw", 100, "damage", True)]  # --generate --allow-naked
EXPECT_ONE_HANDED = [  # --optimize and --generate with --grip one
    ("Murakumo", "Dark", 74, "damage"),
    ("Demons_Great_Hammer", "Raw", 20, "damage"),
    ("Demons_Great_Hammer", "Raw", 100, "damage"),
    ("Uchigatana", "Bleed", 150, "bleed"),
]
EXPECT_AS_CLASS = [  # class key, weapon key, infusion, sl, objective: --generate --class
    ("sorcerer", "Demons_Great_Hammer", "Raw", 90, "damage"),  # the SL 90 Sorcerer given a Warrior
    ("sorcerer", "Moonlight_Greatsword", "No_Infusion", 90, "damage"),
    ("sorcerer", "Demons_Great_Hammer", "Raw", 20, "damage"),
    # generate_armor's poise trade: the three SL 120 warriors it was checked on, a fast weapon, a
    # slow one and a mid one
    ("warrior", "Black_Flamestone_Dagger", "No_Infusion", 120, "damage"),
    ("warrior", "Giant_Warrior_Club", "Dark", 120, "damage"),
    ("warrior", "Uchigatana", "No_Infusion", 120, "damage"),
]
EXPECT_SPELLS = [  # weapon key, infusion, sl, objective, spell keys: --generate --spells
    ("Demons_Great_Hammer", "Raw", 100, "damage", []),  # the same build EXPECT_BUILDS generates
    # FTH 42 (miracle): Dragon Chime casts harder but needs FTH 50, so the next chime is picked
    ("Demons_Great_Hammer", "Raw", 200, "damage", ["Great_Lightning_Spear"]),
    # INT 40 (sorcery): Staff of Wisdom casts harder but needs INT 50, so the next staff is picked
    ("Uchigatana", "Bleed", 150, "bleed", ["Soul_Spear"]),
    ("Moonlight_Greatsword", "No_Infusion", 200, "damage", ["Soul_Geyser"]),  # INT 64: Staff of Wisdom wieldable
    ("Demons_Great_Hammer", "Raw", 200, "damage", ["Blinding_Bolt"]),  # FTH 65: Dragon Chime wieldable
    ("Moonlight_Greatsword", "No_Infusion", 90, "damage", ["Soul_Arrow"]),
    ("Moonlight_Greatsword", "No_Infusion", 200, "damage", ["Sacred_Oath", "Denial", "Great_Heal"]),  # 8 slots: ATT 50
    ("Demons_Great_Hammer", "Raw", 200, "damage", ["Heal", "Heal"]),  # a second copy costs its slot again
    ("Demons_Great_Hammer", "Raw", 120, "damage", ["Resonant_Soul", "Dark_Orb", "Fireball"]),  # hex x2 + pyromancy
    ("Demons_Great_Hammer", "Raw", 20, "damage", ["Great_Lightning_Spear"]),  # FTH 42 does not fit SL 20
    # 12 slots: no ATT alone holds them (ATT 99 gives 10), a Southern Ritual Band+2 does (+3)
    ("Demons_Great_Hammer", "Raw", 200, "damage", ["Climax", "Climax", "Sacred_Oath"]),
    # soulsplanner 16581's SL and spell: the floors and Climax's ATT 20 do not fit SL 98 without the
    # rings; Ring of the Embedded stands in for VIG/VIT and the band for ATT
    ("Dagger", "Dark", 98, "damage", ["Climax"]),
    # Warmth alone is held to the no-spell floors (NO_FLOOR_SPELLS): its own slot sets ATT
    ("Uchigatana", "Lightning", 155, "damage", ["Warmth"]),
    ("Black_Dragon_Greataxe", "Raw", 250, "damage", ["Warmth"]),
]
#: --optimize --spells, as Optimize for weapon asks now that it honours the chosen spells; the last
#: field is whether the floors apply (False: --no-floors, the panel's "ignore typical-build minimums").
EXPECT_OPTIMIZE_SPELLS = [(*case, True) for case in EXPECT_SPELLS] + [
    ("Dagger", "No_Infusion", 120, "damage", ["Climax"], True),  # the user's refusal: no build
    ("Dagger", "No_Infusion", 120, "damage", ["Climax"], False),  # the same without the floors: a build
    ("Roaring_Halberd", "No_Infusion", 120, "damage", ["Climax"], False),
    # no ring: SL 78 is the least that fits; SL 72 fits with the band alone (ATT 20 -> 10); SL 60
    # with the band, Ring of Knowledge and Ring of Prayer
    ("Dagger", "No_Infusion", 72, "damage", ["Climax"], False),
    ("Dagger", "No_Infusion", 60, "damage", ["Climax"], False),
]
EXPECT_REFUSALS = [  # weapon key, infusion, sl, objective, grip, spells, class, floors: refusal()
    ("Dagger", "No_Infusion", 120, "damage", "two", ["Climax"], None, True),  # the user's case: floors
    ("Roaring_Halberd", "No_Infusion", 120, "damage", "two", ["Climax"], None, True),
    ("Dagger", "No_Infusion", 130, "damage", "two", ["Climax"], None, True),  # past the bracket edge
    ("Demons_Great_Hammer", "Raw", 20, "damage", "two", ["Great_Lightning_Spear"], None, True),
    ("Demons_Great_Hammer", "Raw", 200, "damage", "two", ["Climax", "Climax", "Sacred_Oath"], None, True),
    ("Giant_Stone_Axe", "No_Infusion", 10, "damage", "one", [], None, True),  # STR 45 one-handed
    ("Demons_Great_Hammer", "Raw", 40, "damage", "two", [], "sorcerer", True),  # the class is the blocker
    ("Demons_Great_Hammer", "Raw", 100, "damage", "two", [], None, True),  # a build: no refusal
    ("Dagger", "No_Infusion", 50, "damage", "two", ["Climax"], None, False),  # the least SL counts the rings
    # SL 120 above builds now, wearing rings in place of points; SL 60 still refuses
    ("Dagger", "No_Infusion", 60, "damage", "two", ["Climax"], None, True),
    ("Roaring_Halberd", "No_Infusion", 60, "damage", "two", ["Climax"], None, True),
    ("Demons_Great_Hammer", "Raw", 20, "damage", "two", [], "sorcerer", True),  # the class is the blocker
    ("Dagger", "No_Infusion", 40, "damage", "two", ["Climax"], "sorcerer", True),
    # 16 slots: not even ATT 99 with a Southern Ritual Band+2 (13)
    ("Demons_Great_Hammer", "Raw", 200, "damage", "two", ["Climax", "Climax", "Climax", "Climax"], None, True),
    # Warmth alone: the floors it is refused on read VIG/VIT/ADP, no ATT (NO_FLOOR_SPELLS)
    ("Demons_Great_Hammer", "Raw", 20, "damage", "two", ["Warmth"], None, True),
]
EXPECT_MINIMUM = [("Demons_Great_Hammer", True), ("Moonlight_Greatsword", False), ("Uchigatana", False)]
EXPECT_SIMILAR = [  # stats, sl, k, status
    ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, 50, None),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, 30, ["bleed"]),
    ([12, 10, 8, 10, 14, 14, 10, 8, 8], 40, 50, ["bleed", "poison"]),
]
EXPECT_FLOOR_SLS = [1, 33, 60, 100, 150, 200, 838]
#: best_armor for a build holding `weapon`, wearing no rings: at scarcity 0 and at load_scarcity.
EXPECT_ARMOR_SCARCITY = [  # weapon key, stats
    ("Dagger", [20, 10, 30, 5, 8, 8, 10, 5, 5]),  # VIT 30, STR/DEX 8: few weapons, room for heavy armour
    ("Dagger", [15, 10, 20, 8, 10, 12, 10, 8, 8]),
    ("Demons_Great_Hammer", [22, 20, 14, 6, 44, 11, 26, 5, 5]),  # many weapons: scarcity 0, no change
]
EXPECT_FLEX = [  # stats, sl, armour names, ring names: flexibility(); the generated builds are added
    ([10, 6, 7, 6, 6, 20, 9, 6, 18], 35, [], []),  # the SL 35 DEX/FTH character this was written for
    ([10, 6, 7, 6, 6, 20, 9, 6, 18], 35, ["Alva Helm", "Alva Armor", "Alva Gauntlets", "Alva Leggings"],
     ["Flynn's Ring", "Chloranthy Ring + 2"]),
    ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, ["Naked", "Drangleic Mail", "Naked", "Naked"], []),
    ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, [], ["Third Dragon Ring"]),
]
EXPECT_BEST_INFUSION = [  # weapon key, stats, sl, window, raw_ar, objective: --best-infusion
    ("Caestus", [20, 20, 15, 10, 40, 40, 15, 9, 9], 125, 0.0, False, "damage"),
    ("Drakeblood_Greatsword", [20, 20, 15, 10, 20, 20, 15, 9, 40], 116, 0.0, False, "damage"),
    ("Drakeblood_Greatsword", [20, 20, 15, 10, 20, 20, 15, 9, 40], 116, 1.5, False, "damage"),
    ("Uchigatana", [25, 20, 15, 12, 12, 40, 15, 9, 20], 150, 0.0, True, "damage"),
    ("Uchigatana", [25, 20, 15, 12, 12, 40, 15, 9, 20], 150, 0.0, False, "bleed"),
    ("Dagger", [20, 20, 15, 10, 20, 30, 30, 9, 9], 100, 2.0, False, "poison"),
    ("Greatsword", [20, 20, 15, 10, 10, 10, 15, 9, 9], 80, 0.0, False, "damage"),  # STR 10 < 28/2: no rows
    # the AR goal: Raw's phys 471 below the split infusions' 287 + 287, tied four ways at 574
    ("Black_Dragon_Greataxe", [20, 20, 15, 10, 35, 15, 15, 9, 14], 100, 0.0, False, "ar"),
    # launchers: three special shots a pull (and the volley note), a shot's summed attack
    ("Sanctum_Repeating_Crossbow", [20, 20, 15, 10, 25, 20, 15, 30, 30], 150, 0.0, False, "damage"),
    ("Avelyn", [20, 20, 15, 10, 25, 20, 15, 9, 9], 120, 1.5, False, "ar"),
    ("Long_Bow", [20, 20, 15, 10, 12, 30, 15, 9, 20], 120, 0.0, False, "bleed"),
]
#: --defender sets, head/chest/hands/legs keys: a heavy one with the best elemental defense of the
#: heavy sets, a bare one, and one whose head and hands change stats the defense reads (Warlock Mask
#: INT +2 raises the magic/fire/dark base; Agdayne's Cuffs FTH +1 the fire/lightning/dark base).
HAVEL = ["Havels_Helm", "Havels_Armor", "Havels_Gauntlets", "Havels_Leggings"]
EXPECT_DEFENDERS = [HAVEL, ["Naked"] * 4, ["Warlock_Mask", "Black_Witch_Robe", "Agdaynes_Cuffs", "Naked"]]
EXPECT_DEFENSE_SLS = [1, 33, 100, 150, 838]  # defender_defense at each, average and every EXPECT_DEFENDERS set
EXPECT_DEFENDER_WEAPONS_FOR = [  # defender, then an EXPECT_WEAPONS_FOR case
    (HAVEL, ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, False, None, False, 0.0, False, "damage")),
    (HAVEL, ([20, 20, 15, 10, 40, 15, 15, 9, 9], 100, False, None, False, 1.5, False, "damage")),
    (EXPECT_DEFENDERS[2], ([25, 20, 15, 12, 12, 40, 15, 9, 20], 150, False, None, True, 0.0, False, "damage")),
]
EXPECT_DEFENDER_BUILDS = [  # defender, then an EXPECT_BUILDS case: --optimize and --generate --defender
    (HAVEL, ("Black_Dragon_Greataxe", "Lightning", 150, "damage")),
    (HAVEL, ("Demons_Great_Hammer", "Raw", 100, "damage")),
    (["Naked"] * 4, ("Uchigatana", "Dark", 150, "damage")),
]
EXPECT_DEFENDER_BEST_INFUSION = [  # defender, then an EXPECT_BEST_INFUSION case
    (HAVEL, ("Black_Dragon_Greataxe", [30, 20, 25, 10, 40, 15, 20, 15, 30], 150, 0.0, False, "damage")),
    (EXPECT_DEFENDERS[2], ("Black_Dragon_Greataxe", [30, 20, 25, 10, 40, 15, 20, 15, 30], 150, 0.0, False,
                           "damage")),
]
#: The adaptive defender (DEFENDER "adaptive", the script's default): the --defender-buff setting,
#: a --defender set (None for the average defender), then a case of the list named.
EXPECT_ADAPTIVE_WEAPONS_FOR = [  # an EXPECT_WEAPONS_FOR case
    ("none", None, EXPECT_WEAPONS_FOR[0]),
    ("none", None, EXPECT_WEAPONS_FOR[1]),
    ("item", None, EXPECT_WEAPONS_FOR[0]),
    ("any", HAVEL, EXPECT_WEAPONS_FOR[3]),
    # crossbows: a bolt's physical and a Sanctum special's pure dark each draw their own answer
    ("none", None, ([20, 20, 15, 10, 25, 20, 15, 30, 30], 150, False, "Crossbow", False, 0.0, False, "damage")),
]
EXPECT_ADAPTIVE_BUILDS = [  # an EXPECT_BUILDS case: --optimize and --generate
    ("none", None, ("Demons_Great_Hammer", "Raw", 100, "damage")),
    ("none", None, ("Uchigatana", "Lightning", 150, "damage")),
    ("item", None, ("Murakumo", "Dark", 74, "damage")),
    ("any", HAVEL, ("Black_Dragon_Greataxe", "Lightning", 150, "damage")),
    ("none", None, ("Sanctum_Crossbow", "Dark", 150, "damage")),
]
EXPECT_ADAPTIVE_BEST_INFUSION = [  # an EXPECT_BEST_INFUSION case
    ("none", None, EXPECT_BEST_INFUSION[1]),
    ("none", None, EXPECT_BEST_INFUSION[2]),
    ("item", HAVEL, EXPECT_DEFENDER_BEST_INFUSION[0][1]),
    ("none", None, EXPECT_BEST_INFUSION[8]),
]


#: --best-weapons: infusion, sl, objective, grip, weapon class, window, rank, --with-status,
#: --defender, and who answers ("static", or the adaptive defender's --defender-buff setting). Small
#: classes, since every row is an optimize_build.
EXPECT_BEST_WEAPONS = [
    ("Dark", 150, "damage", "two", "Katana", 1.5, "window", False, None, "static"),
    ("No_Infusion", 100, "damage", "two", "Crossbow", 1.5, "window", False, None, "static"),
    ("Bleed", 120, "bleed", "two", "Whip", 1.5, "window", False, None, "static"),
    ("Raw", 100, "damage", "one", "Twinblade", 2.0, "bar", False, HAVEL, "static"),
    ("Lightning", 120, "damage", "two", "Lance", 1.5, "per-stamina", False, None, "none"),
    ("Poison", 80, "damage", "two", "Claw", 1.5, "window", True, None, "static"),
    ("Fire", 100, "ar", "two", "Curved Greatsword", 1.5, "window", False, None, "static"),
    ("No_Infusion", 100, "damage", "two", "Bow", 0.0, "window", False, None, "item"),
]


def under_defender(mode: str, buff: str, fn):
    """`fn()` with DEFENDER set to `mode` and `buff`, restored after."""
    saved = dict(DEFENDER)
    DEFENDER.update(mode=mode, buff=buff)
    try:
        return fn()
    finally:
        DEFENDER.update(saved)


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
    attacks = load_attacks(data)
    rates = json.loads(HYPERARMOR.read_text()) if HYPERARMOR.exists() else {}
    crit = {norm(k): v for k, v in json.loads(CRIT.read_text()).items()} if CRIT.exists() else {}
    as_dict = lambda st: dict(zip(STATS, st))
    arr = lambda d: [int(d[s]) for s in STATS]
    out = ["// @generated by `scripts/ds2-builds-recommend.py --export-backend ... --expect ...`: the",
           "// script's own answers for the questions in its EXPECT_* lists, over the fixture corpus.", ""]

    def for_case(case, defender=None):
        st, sl, one, cls, per, window, raw, objective = case
        rows, _, _ = weapons_for(data, as_dict(st), sl, corpus, raw_ar=raw, one_hand=one, weapon_class=cls,
                                 per_class=per, window=window, objective=objective, defender=defender)
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
        return (st, sl, one, cls or "", per, window, raw, objective, got)

    rows_out = [for_case(case) for case in EXPECT_WEAPONS_FOR]
    out.append("// stats, sl, one_hand, class, per_class, window, raw_ar, objective, rows: weapon, infusion,")
    out.append("// damage (build-up x hits for bleed/poison), ar by type, grip, hyperarmor, counter, class.")
    out.append(f"pub const WEAPONS_FOR: WeaponsForCases = {_rs(rows_out)};\n")
    defense = []
    for sl in EXPECT_DEFENSE_SLS:
        for defender in [None] + EXPECT_DEFENDERS:
            dfn, n = defender_defense(data, corpus, sl, defender)
            defense.append((sl, defender or [], [float(dfn[k]) for k in DMG + PHYS_TYPES], n))
    out.append("// sl, defender armour keys head to legs ([] for the average defender) -> physical, magic, fire,")
    out.append("// lightning, dark, slash, strike, thrust defense, and how many builds it was taken from.")
    out.append(f"pub const DEFENSE: DefenseCases = {_rs(defense)};\n")
    out.append("// A defender's armour keys, then a WEAPONS_FOR case scored against it (--defender).")
    out.append(f"pub const DEFENDER_WEAPONS_FOR: DefenderWeaponsForCases = "
               f"{_rs([(d, for_case(case, d)) for d, case in EXPECT_DEFENDER_WEAPONS_FOR])};\n")
    d_opt, d_gen = [], []
    for d, case in EXPECT_DEFENDER_BUILDS:
        opt_d, gen_d = expect_builds(data, corpus, threat_mix(data, corpus), [case], [], "two", defender=d)
        d_opt.append((d, opt_d[0]))
        d_gen.append((d, gen_d[0]))
    out.append("// A defender's armour keys, then an OPTIMIZE and a GENERATE case scored against it.")
    out.append(f"pub const DEFENDER_OPTIMIZE: DefenderOptimizeCases = {_rs(d_opt)};\n")
    out.append(f"pub const DEFENDER_GENERATE: DefenderGenerateCases = {_rs(d_gen)};\n")

    mix = threat_mix(data, corpus)
    adaptive = lambda buff, fn: under_defender("adaptive", buff, fn)
    out.append("// The rings the adaptive defender may swap in (counter_rings), by name.")
    out.append(f"pub const COUNTER_RINGS: &[&str] = "
               f"{_rs([data.rings[c].get('name', c) for c in counter_rings(data)])};\n")
    out.append("// The adaptive defender: its buff setting (none, item, any), its armour keys ([] for the")
    out.append("// average defender), then a WEAPONS_FOR case; a row's grip ends ` vs <what it put on>`.")
    out.append(f"pub const ADAPTIVE_WEAPONS_FOR: AdaptiveWeaponsForCases = "
               f"{_rs([(b, d or [], adaptive(b, lambda: for_case(case, d))) for b, d, case in EXPECT_ADAPTIVE_WEAPONS_FOR])};\n")
    a_opt, a_gen = [], []
    for b, d, case in EXPECT_ADAPTIVE_BUILDS:
        o, g = adaptive(b, lambda: expect_builds(data, corpus, mix, [case], [], "two", defender=d))
        a_opt.append((b, d or [], o[0]))
        a_gen.append((b, d or [], g[0]))
    out.append("// The same, then an OPTIMIZE and a GENERATE case.")
    out.append(f"pub const ADAPTIVE_OPTIMIZE: AdaptiveOptimizeCases = {_rs(a_opt)};\n")
    out.append(f"pub const ADAPTIVE_GENERATE: AdaptiveGenerateCases = {_rs(a_gen)};\n")
    flexes: list = []  # the generated builds, as flexibility() questions: filled by expect_builds
    opt, gen = expect_builds(data, corpus, mix, EXPECT_BUILDS, EXPECT_NAKED, "two", flexes)
    opt_one, gen_one = expect_builds(data, corpus, mix, EXPECT_ONE_HANDED, [], "one", flexes)
    out.append("// weapon, infusion, sl, objective -> class, two-handed, stats, value.")
    out.append(f"pub const OPTIMIZE: OptimizeCases = {_rs(opt)};\n")
    out.append("// weapon, infusion, sl, objective, allow naked -> class, two-handed, stats, 1H rows, 2H-only")
    out.append("// rows (name, infusion, rounded damage), suggested rings, common rings, armour, armour note,")
    out.append("// whether armour requirements bound the choice, whether the load cap did.")
    out.append(f"pub const GENERATE: GenerateCases = {_rs(gen)};\n")
    out.append("// The same two questions with the grip forced one-handed (--grip one).")
    out.append(f"pub const OPTIMIZE_ONE_HANDED: OptimizeCases = {_rs(opt_one)};\n")
    out.append(f"pub const GENERATE_ONE_HANDED: GenerateCases = {_rs(gen_one)};\n")
    gen_class = []
    for cls, weapon, inf, sl, objective in EXPECT_AS_CLASS:
        _, got = expect_builds(data, corpus, mix, [(weapon, inf, sl, objective)], [], "two", only_class=cls)
        gen_class.append((cls, got[0]))
    out.append("// class key, then a GENERATE case asked of that starting class alone (--class).")
    out.append(f"pub const GENERATE_AS_CLASS: ClassGenerateCases = {_rs(gen_class)};\n")
    spell_gen = []
    for weapon, inf, sl, objective, spells in EXPECT_SPELLS:
        g = generate_build(data, corpus, weapon, inf, sl, objective, spells=spells)
        spell_gen.append((weapon, INFUSION_CODE[inf], sl, objective, spells, None if g is None else _Some((
            data.classes[g["class"]]["name"], g["two_handed"], arr(g["stats"]), g["spells"], g["slots"][0],
            g["slots"][1], [(label, name, float(p), over or "") for label, name, p, over in g["catalysts"]],
            [trade_line(t) for t in g["ring_trades"]]))))
    out.append("// weapon, infusion, sl, objective, spells -> class, two-handed, stats, spell names, slots the")
    out.append("// spells cost, slots the build's ATT and rings give, catalysts: school, name, cast power, the")
    out.append("// catalyst passed over for its requirements (\"\" for none), the rings worn in place of points.")
    out.append(f"pub const GENERATE_SPELLS: SpellCases = {_rs(spell_gen)};\n")
    opt_spells = []
    for weapon, inf, sl, objective, spells, use_floors in EXPECT_OPTIMIZE_SPELLS:
        best, _ = optimize_build(data, corpus, weapon, inf, sl, objective, spells=spells, use_floors=use_floors)
        opt_spells.append((weapon, INFUSION_CODE[inf], sl, objective, spells, use_floors,
                           None if best is None else _Some((data.classes[best[1]]["name"], best[2],
                                                            arr(best[3]), float(best[0]),
                                                            [data.rings[r]["name"] for r in best[4]]))))
    out.append("// weapon, infusion, sl, objective, spells, floors apply -> class, two-handed, stats, value,")
    out.append("// the rings worn in place of points.")
    out.append(f"pub const OPTIMIZE_SPELLS: OptimizeSpellCases = {_rs(opt_spells)};\n")
    refusals = []
    for weapon, inf, sl, objective, grip, spells, cls, use_floors in EXPECT_REFUSALS:
        r = refusal(data, corpus, weapon, inf, sl, objective, grip, spells, cls, use_floors)
        refusals.append((weapon, INFUSION_CODE[inf], sl, objective, grip, spells, cls or "", use_floors,
                         None if r is None else _Some((
                             r["kind"], r["class"] or "", r["short"] or 0, r["lines"],
                             [(what, "" if v is None else str(v), label) for what, v, label in r["fixes"]]))))
    out.append("// weapon, infusion, sl, objective, grip, spells, class (\"\" any), floors apply -> kind, closest")
    out.append("// class, points short, the reason lines, fixes: what, its value (\"\" for none), label.")
    out.append(f"pub const REFUSALS: RefusalCases = {_rs(refusals)};\n")

    mins = []
    for weapon, two in EXPECT_MINIMUM:
        m = recommended_minimum(data, corpus, weapon, two)
        _, cls, (_, _, arm, rs, _, _) = m["hard"][0]
        mid = m["presets"][0]
        gear = [data.armor[s][p]["name"] for s, p in zip(ARMOR_SLOTS, arm) if p != "Naked"] \
            + [data.rings[r]["name"] for r in rs]
        mins.append((weapon, two, (data.classes[cls]["name"], m["target_sl"], arr(mid["stats"]), gear)))
    return backend_expectations_rest(data, corpus, out, mins, flexes)


def expect_builds(data: Data, corpus: list[Build], mix, cases: list, naked_cases: list, grip: str,
                  flexes: list | None = None, only_class: str | None = None, defender=None):
    """The script's --optimize and --generate answers for `cases` (plus `naked_cases` generated with
    --allow-naked) at `grip`, and for `only_class` alone when given, against `defender`
    (defender_defense), as fixture tuples."""
    arr = lambda d: [int(d[s]) for s in STATS]
    opt, gen = [], []
    for weapon, inf, sl, objective in cases:
        best, _ = optimize_build(data, corpus, weapon, inf, sl, objective, grip, only_class=only_class,
                                 defender=defender)
        opt.append((weapon, INFUSION_CODE[inf], sl, objective,
                    None if best is None else _Some((data.classes[best[1]]["name"], best[2], arr(best[3]),
                                                     float(best[0])))))
    for weapon, inf, sl, objective, naked in [(*case, False) for case in cases] + naked_cases:
        g = generate_build(data, corpus, weapon, inf, sl, objective, allow_naked=naked, grip=grip,
                           only_class=only_class, defender=defender)
        if g is None:
            gen.append((weapon, INFUSION_CODE[inf], sl, objective, naked, None))
            continue
        rings, suggested = g["rings"], []
        while len(rings) >= 3 and rings[0] == rings[1] == rings[2]:
            suggested.append(rings[0])
            rings = rings[3:]
        if flexes is not None:  # the panel asks with the build's names: armour and suggested rings
            flexes.append((arr(g["stats"]), sl, g["armor"], suggested,
                           flexibility(data, corpus, g["stats"], sl, armor_keys(data, g["armor"]),
                                       [data.sp_key[norm(r)] for r in suggested])))
        # Whether the armour's requirements and the load cap each changed the chosen set: the same
        # search with every other stat at 99, and with VIT at 99, picks differently.
        binds = (False, False)
        if not naked:
            worn = [data.sp_key[norm(r)] for r in suggested]
            wearer = Build("", g["stats"], ["Naked"] * 4, [(weapon, inf)], 0, worn, [])
            scarce = load_scarcity(data, g["stats"])
            top = lambda eff: (best_armor(data, wearer, eff, mix, top=1, scarcity=scarce)[2] or [None])[0]
            chosen = top(dict(g["stats"]))
            binds = (chosen != top({**{s: 99 for s in STATS}, "vitality": g["stats"]["vitality"]}),
                     chosen != top({**g["stats"], "vitality": 99}))
        gen.append((weapon, INFUSION_CODE[inf], sl, objective, naked, _Some((
            data.classes[g["class"]]["name"], g["two_handed"], arr(g["stats"]),
            [(n, INFUSION_CODE[i], d) for n, i, d in g["weapons_1h"]],
            [(n, INFUSION_CODE[i], d) for n, i, d in g["weapons_2h_only"]], suggested, rings,
            g["armor"], g["armor_note"] and _Some(g["armor_note"]), binds[0], binds[1],
            [trade_line(t) for t in g["ring_trades"]],
            None if g["poise"] is None else _Some((tuple(float(g[k]) for k in TRADE_FIELDS),
                                                   tuple(float(g["trade_previous"][k]) for k in TRADE_FIELDS)))))))
    return opt, gen


#: generate_build's trade numbers the fixture carries, in order, for the chosen set and for
#: best_armor's (trade_previous): the armour's poise, the counter poise damage it holds through,
#: the share of trades my R1 lands and the exchange value.
TRADE_FIELDS = ("poise", "poise_target", "trade_rate", "exchange")


def backend_expectations_rest(data: Data, corpus: list[Build], out: list[str], mins: list, flexes: list) -> str:
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

    def infusion_case(case, defender=None):
        weapon, st, sl, window, raw, objective = case
        rows, _, _ = best_infusion(data, weapon, as_dict(st), sl, corpus, raw_ar=raw, window=window,
                                   objective=objective, defender=defender)
        return (weapon, st, sl, window, raw, objective,
                [(INFUSION_CODE[inf], float(v), [float(ar.get(k, 0)) for k in DMG],
                  re.sub(r" (HA|ctr) x\S+", "", grip).strip()) for v, _, inf, ar, grip in rows])

    best = [infusion_case(case) for case in EXPECT_BEST_INFUSION]
    out.append("// weapon, stats, sl, window, raw_ar, objective -> rows best first: infusion, score, AR by type,")
    out.append("// grip.")
    out.append(f"pub const BEST_INFUSION: BestInfusionCases = {_rs(best)};\n")
    out.append("// A defender's armour keys, then a BEST_INFUSION case scored against it.")
    out.append(f"pub const DEFENDER_BEST_INFUSION: DefenderBestInfusionCases = "
               f"{_rs([(d, infusion_case(case, d)) for d, case in EXPECT_DEFENDER_BEST_INFUSION])};\n")
    out.append("// The adaptive defender's buff setting and armour keys, then a BEST_INFUSION case.")
    out.append(f"pub const ADAPTIVE_BEST_INFUSION: AdaptiveBestInfusionCases = "
               f"{_rs([(b, d or [], under_defender('adaptive', b, lambda: infusion_case(case, d))) for b, d, case in EXPECT_ADAPTIVE_BEST_INFUSION])};\n")
    c = calibrate_infusions(data, corpus)
    out.append(f"pub const CALIBRATION: (u32, f64, f64) = {_rs((c['n'], float(c['top1']), float(c['top2'])))};")

    rows = []
    for st, sl, armor, rings in EXPECT_FLEX:
        keys = [data.sp_key[norm(r)] for r in rings]
        f = flexibility(data, corpus, as_dict(st), sl, armor_keys(data, armor), keys)
        rows.append((st, sl, armor, rings, f))
    rows += flexes
    out.append("")
    out.append("// stats, sl, armour names, ring names -> weapons wielded 1H, 2H, of how many, neighbours")
    out.append("// scoring below, the same, how many neighbours, percentile, spare load, weapons that fit it.")
    out.append(f"pub const FLEXIBILITY: FlexCases = {_rs([(st, sl, armor, rings, (f['one'], f['two'], f['total'], f['below'], f['equal'], f['n'], float(f['percentile']), float(f['spare']), f['fits'])) for st, sl, armor, rings, f in rows])};")
    out.append("")
    out.append(f"pub const FLEX_LINES: &[(&str, &str)] = {_rs([(flex_line(f), flex_load_line(f)) for *_, f in rows])};")
    armor_rows = []
    for weapon, st in EXPECT_ARMOR_SCARCITY:
        wearer = Build("", as_dict(st), ["Naked"] * 4, [(weapon, "No_Infusion")], 0, [], [])
        s = load_scarcity(data, as_dict(st))
        pick = lambda sc: list((best_armor(data, wearer, as_dict(st), threat_mix(data, corpus), top=1, scarcity=sc)[2]
                                 or [(0, 0, ["", "", "", ""])])[0][2])
        armor_rows.append((weapon, st, float(s), pick(0.0), pick(s)))
    out.append("")
    out.append("// weapon, stats -> load scarcity, armour keys head to legs at scarcity 0, the same at that scarcity.")
    out.append(f"pub const ARMOR_SCARCITY: ArmorScarcityCases = {_rs(armor_rows)};")
    out.append("")
    out.append("// --best-weapons: infusion, sl, objective, grip, weapon class, window, rank, with status, defender")
    out.append("// keys, who answers (static or the buff setting) -> rows best first: score, weapon key, value,")
    out.append("// class, two-handed, stats, rings, label, (best ammunition, shot) for a launcher; then the metrics:")
    out.append("// R1 (reach, startup, recovery, first hit, 5 s damage); poise (hyperarmor, its rate, the share")
    out.append("// of counter-hits it holds, poise damage, armorBreak, hits to stagger, defender poise); status")
    out.append("// (hits, per status its build-up per hit and hits to proc, damage per window, in the first")
    out.append("// window); damage with status; stamina (per attack, per window, damage per stamina, max")
    out.append("// stamina, attacks, damage and seconds of a full bar).")
    out.append(f"pub const BEST_WEAPONS: BestWeaponsCases = {_rs(expect_best_weapons(data, corpus))};")
    return "\n".join(out) + "\n"


def expect_best_weapons(data: Data, corpus: list[Build]) -> list:
    """EXPECT_BEST_WEAPONS as fixture tuples: per case its rows, each with every metric
    best_weapon_metrics gives (None where the row has none)."""
    attacks = load_attacks(data)
    opt = lambda v: None if v is None else _Some(float(v))
    opt_int = lambda v: None if v is None else _Some(int(v))
    jobs = max(1, (os.cpu_count() or 1) // 4)
    out = []
    for inf, sl, objective, grip, cls, window, rank, with_status, defender, buff in EXPECT_BEST_WEAPONS:
        def run():
            rows = best_weapons(data, corpus, inf, sl, objective, grip, None, (), None, True, defender, cls,
                                window, jobs, rank)
            if with_status:
                rows.sort(key=lambda r: -r[9].get("damage_with_status", r[0]))
            return [(r, best_weapon_metrics(data, corpus, attacks, r, inf, sl, defender)) for r in rows]

        got = run() if buff == "static" else under_defender("adaptive", buff, run)
        rows = []
        for r, m in got:
            score, key, val, c, two, st, worn, label, ammo, _ = r
            status = stamina = None
            if "status_buildup_per_hit" in m:
                status = _Some((int(m["status_hits"]),
                                [(s, float(p), int(m["hits_to_proc"][s])) for s, p in m["status_buildup_per_hit"].items()],
                                float(m["status_damage_per_window"]), float(m["status_damage_first_window"])))
            if "stamina_per_window" in m:
                stamina = _Some(([float(x) for x in m["stamina_per_attack"]], float(m["stamina_per_window"]),
                                 float(m["damage_per_stamina"]), opt(m.get("max_stamina")), opt_int(m.get("bar_attacks")),
                                 opt(m.get("bar_damage")), opt(m.get("bar_seconds"))))
            rows.append((float(score), key, float(val), data.classes[c]["name"], two, [int(st[s]) for s in STATS],
                         [data.rings[x]["name"] for x in worn], label,
                         _Some((ammo["ammo"] or "", ammo["shot"])) if ammo else None,
                         tuple(opt(m.get(k)) for k in ("reach_m", "startup_s", "recovery_s", "time_to_first_hit_s",
                                                         "damage_per_5s")),
                         (opt(m.get("hyperarmor")), opt(m.get("hyperarmor_rate")), opt(m.get("hyperarmor_holds")),
                          opt(m.get("poise_damage_per_hit")), opt_int(m.get("armor_break")),
                          opt_int(m.get("hits_to_stagger")), float(m.get("defender_poise", 0.0))),
                         status, opt(m.get("damage_with_status")), stamina))
        out.append((INFUSION_CODE[inf], sl, objective, grip, cls, float(window), rank, with_status, defender or [],
                    buff, rows))
    return out


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


def trade_selftest_cases() -> list:
    """The R1 trade (trade) and its exchange value (exchange_pairs, exchange_value) on hand-made
    strikes: stagger at poise <= 0 (EXE 0x140136570), armorBreak 2 always, 1 outside hyperarmor,
    poise damage x uninterruptibleRate inside it."""
    def hit(t, pd, ab=0, armor=(), rate=0.0, phys=100, dfn=0.0, poise=0.0):
        return {"t": t, "pd": pd, "ab": ab, "armor": list(armor), "rate": rate, "mv": 1.0, "kind": "physical",
                "lower": 0, "flat": NO_FLAT, "scale": 1.0, "ar": {"physical": phys}, "dfn": {"physical": dfn},
                "poise": poise}
    hammer = hit(0.8, 60, armor=[(0.6, 1.0)], rate=0.3)  # slow, hyperarmor 0.6-1.0 s at x0.3
    dagger = hit(0.3, 35)
    cases = [
        ("counter first, 35 into 36 poise: both land", trade(hit(0.8, 0), dagger, 36, 0), (True, True)),
        ("counter first, 35 into 35 poise: staggered at 0", trade(hit(0.8, 0), dagger, 35, 0), (False, True)),
        ("counter first, armorBreak 2: staggered through any poise",
         trade(hit(0.8, 0), hit(0.3, 1, ab=2), 1000, 0), (False, True)),
        ("armorBreak 1 inside hyperarmor holds on poise: 100 x 0.3 under 31",
         trade(hammer, hit(0.7, 100, ab=1), 31, 0), (True, True)),
        ("armorBreak 1 inside hyperarmor, 30 into 30 poise: staggered", trade(hammer, hit(0.7, 100, ab=1), 30, 0),
         (False, True)),
        ("armorBreak 1 outside hyperarmor: staggered", trade(hammer, hit(0.5, 1, ab=1), 1000, 0), (False, True)),
        ("mine first, 35 into their 30: theirs never lands", trade(dagger, hit(0.8, 0), 0, 30), (True, False)),
        ("mine first, 35 into their 40: both land", trade(dagger, hit(0.8, 0), 0, 40), (True, True)),
        ("mine first into their hyperarmor: 35 x 0.3 under 11",
         trade(dagger, hammer | {"armor": [(0.2, 1.0)]}, 0, 11), (True, True)),
        ("mine first before their hyperarmor opens: 35 into 11", trade(dagger, hammer, 0, 11), (True, False)),
        ("a tie is mine first", trade(hit(0.5, 50), hit(0.5, 50), 0, 10), (True, False)),
    ]
    # one weapon, one counter, by hand: mine 100 AR into 200 DEF = (1000 - 200) / 12; theirs 120
    # AR into 300 DEF = (1200 - 300) / 12 = 75, 40 poise damage landing first
    t = exchange_pairs([hit(0.8, 0)], [hit(0.5, 40, phys=120, dfn=200.0)])
    cases += [
        ("exchange at 30 poise: staggered, takes 75", exchange_value(t, 30, {"physical": 300.0}), (-75.0, 0.0)),
        ("exchange at 41 poise: lands 66.7, takes 75", tuple(round(x, 3) for x in
                                                               exchange_value(t, 41, {"physical": 300.0})),
         (round(800 / 12 - 75, 3), 1.0)),
    ]
    # the table's reduction equals trading every pair at the poise, for poise either side of each
    # threshold (35, 60 x 0.3 = 18 inside the hammer's window, armorBreak 2 never)
    mine = [dagger, hammer, hit(0.45, 20, phys=150)]
    counters = [hit(0.4, 35, phys=110, dfn=150.0, poise=30.0), hit(0.7, 60, phys=200, dfn=250.0, poise=50.0),
                hit(0.5, 10, ab=2, phys=90, dfn=100.0, poise=80.0), hit(1.2, 45, phys=130, dfn=180.0, poise=20.0)]
    t = exchange_pairs(mine, counters)
    me = {"physical": 220.0}
    for poise in (0, 18, 18.5, 35, 35.5, 70):
        want = 0.0
        for w in mine:
            for c in counters:
                landed, taken = trade(w, c, poise, c["poise"])
                want += (landed * hit_damage(w["ar"], c["dfn"], 1.0) - taken * hit_damage(c["ar"], me, 1.0))
        want /= len(mine) * len(counters)
        cases.append((f"exchange table = every pair traded, poise {poise}", round(exchange_value(t, poise, me)[0], 9),
                      round(want, 9)))
    return cases


def metrics_selftest_cases() -> list:
    """r1_reach / r1_metrics on regulation rows (scripts/ds2-hit-shape.py prints them): the Dagger's
    one segment, the Whip's three dummy polys end to end, the Greatsword's child sphere on the same
    dummy poly and the Winged Spear's on a body dummy poly, neither adding length."""
    A = dict(SELFTEST_ATTACKS)
    A[("whip", "Single1HandNormal1st")] = {"anim": -1, "spd": [1.0, 1.0], "hits": [_hit(16, 20, 1.0, dmg=10011000)]}
    A[("greatsword", "Single1HandNormal1st")] = {"anim": -1, "spd": [1.0, 1.0], "hits": [_hit(23, 29, 1.0, dmg=10042700)]}
    A[("wingedspear", "Single1HandNormal1st")] = {"anim": -1, "spd": [1.0, 1.0],
                                                   "hits": [_hit(14, 20, 1.0, dmg=10017820)]}
    fake = type("Reach", (), {"damage_scale": {"Greatsword": 2.0}, "reach_scale": {"Dagger": (1.0, 0.7),
                                                                                   "Greatsword": (1.0, 1.1)},
                              "hit_shape": {"10004100": (1, 100, 0.3, 0.3, 0),
                                            "10011000": (1, 100, 0.25, 1.0, 10011001),
                                            "10011001": (1, 101, 0.25, 1.1, 10011002),
                                            "10011002": (1, 102, 0.25, 0.2, 0),
                                            "10042700": (1, 100, 0.3, 1.55, 10042701),
                                            "10042701": (1, 100, 0.4, 0.01, 0),
                                            "10017820": (1, 100, 0.3, 1.55, 40000010),
                                            "40000010": (1, 1, 0.3, 0.1, 0)}})
    ar, dfn = {"physical": 100}, {"physical": 0}
    dagger = r1_metrics(fake, A, "Dagger", ["Dagger"], False, ar, dfn, anim_len=lambda anim: 1.0)
    return [
        ("dagger reach 0.3 x 0.7 + 0.3", r1_reach(fake, A, "Dagger", ["Dagger"], False), 0.51),
        ("whip reach 1.0 + 1.1 + 0.2 + 0.25", r1_reach(fake, A, "Whip", ["Whip"], False), 2.55),
        ("greatsword child sphere adds no length", r1_reach(fake, A, "Greatsword", ["Greatsword"], False), 2.005),
        ("spear body-dummy-poly child adds none", r1_reach(fake, A, "Winged_Spear", ["Winged Spear"], False), 1.85),
        ("no 2H row, no reach", r1_reach(fake, A, "Dagger", ["Dagger"], True), None),
        # frame 8 at the mean of speeds 1.1/1.2; the anim's 1.0 s ends 0.5 s after frame 15
        ("dagger startup 8/30/1.15", dagger["startup_s"], 0.232),
        ("dagger recovery (1.0 - 15/30)/1.15", dagger["recovery_s"], 0.435),
        ("dagger first hit is its startup", dagger["time_to_first_hit_s"], 0.232),
        # anim -1 has no chain window, so the 5 s repeat ends after one hit: (100*10 - 0)/12
        ("dagger 5 s damage, one hit", dagger["damage_per_5s"], 83.3),
        ("greatsword damageScale x2", r1_metrics(fake, A, "Greatsword", ["Greatsword"], False, ar, dfn,
                                                 anim_len=lambda anim: None)["damage_per_5s"], 166.7),
        ("no TAE, no recovery", r1_metrics(fake, A, "Dagger", ["Dagger"], False, ar, dfn,
                                           anim_len=lambda anim: None)["recovery_s"], None),
    ]


def flex_selftest_cases() -> list:
    """flex_counts/flexibility over a four-weapon table and a five-build corpus, no site tables."""
    from types import SimpleNamespace
    weapons = {
        "Bare_Fists": {"name": "Bare Fists", "weight": 0.0},  # EMPTY: never counted
        "Dagger": {"name": "Dagger", "weight": 1.0, "require": {"strength": 5, "dexterity": 8}},
        "Club": {"name": "Club", "weight": 6.0, "require": {"strength": 20}},  # 2H needs STR 10
        "Chime": {"name": "Chime", "weight": 2.5, "require": {"strength": 3, "faith": 18}},
        "Greataxe": {"name": "Greataxe", "weight": 20.0, "require": {"strength": 31}},  # 2H needs 15
    }
    base = dict.fromkeys(STATS, 5)
    data = SimpleNamespace(
        weapons=weapons, classes={"x": {"level": 156, **base}},  # every build in SL bracket 156-200
        armor={s: {"Naked": {"name": "Naked", "weight": 0.0}, "Plate": {"name": "Plate", "weight": 10.0}}
               for s in ARMOR_SLOTS},
        rings={"Flynns_Ring": {"name": "Flynn's Ring", "weight": 0.5}},
        equip_load=[None] + [40.0 + 1.5 * i for i in range(99)],
        _ring_gear={})  # Flynn's Ring changes no stat, load, HP or slot count
    st = lambda **kw: {**base, **{s: kw.get(s[:3], base[s]) for s in STATS}}
    user = st(str=10, dex=8, fai=18, vit=10)  # Dagger, Chime 1H; Club too 2H: 2 + 3 = 5
    corpus = [Build("x", st(str=s_, dex=d, fai=f), [], [], 0, [], [], label=str(i))
              for i, (s_, d, f) in enumerate([(5, 5, 5), (5, 8, 5), (10, 8, 18), (31, 8, 18), (20, 8, 5)])]
    f = flexibility(data, corpus, user, 180, ["Plate", "Naked", "Naked", "Naked"], ["Flynns_Ring"], k=5)  # sl in that bracket
    # corpus scores: 0+0, 1+1, 2+3, 4+4, 2+3 -> 2 below 5, 2 equal -> 100 * (2 + 1) / 5 = 60
    return [
        ("flex: Bare_Fists is no weapon", len(flex_pool(data)[0]), 4),
        ("flex: STR 10 two-hands a STR 20 club, not one", flex_counts(data, st(str=10)), (0, 1)),
        ("flex: STR 9 wields neither", flex_counts(data, st(str=9)), (0, 0)),
        ("flex: halving floors, STR 15 two-hands a STR 31 axe", flex_counts(data, st(str=15))[1], 2),
        ("flex: counts", (f["one"], f["two"], f["total"], f["score"]), (2, 3, 4, 5)),
        ("flex: neighbours below/equal", (f["below"], f["equal"], f["n"]), (2, 2, 5)),
        ("flex: mid-rank percentile", f["percentile"], 60.0),
        # VIT 10 -> 40 + 1.5 * 9 = 53.5, * 0.7 = 37.45, less 10 plate and 0.5 ring = 26.95: all three fit
        ("flex: spare load after armour and rings", round(f["spare"], 2), 26.95),
        ("flex: fits", f["fits"], 3),
        ("flex: the panel's line", flex_line(f), "wields 2/4 1H, 3 2H -- 60th percentile of 5 similar builds"),
        ("flex: ordinals", [ordinal(n) for n in (1, 2, 3, 4, 11, 12, 13, 21, 22, 100)],
         ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "100th"]),
    ]


def ranged_selftest_cases() -> list[tuple]:
    """A shot over made-up rows (regulation_ranged's shapes): launcher attack + ammo flat, x the hand
    scale; the special keeps its row's types only; a type with a rate and coefficient but no base
    attacks; a bow fires arrows only; damage is per shot at the ammo's MV, x damageScale."""
    flat = lambda v: [v] * 100
    sp = {t: flat(0) for t in ("physicalATKBonus", "magicATKBonus", "fireATKBonus", "lightningATKBonus",
                               "darkATKBonus", "mundaneATKBonus", "auxATKBonus")}
    sp["physicalATKBonus"], sp["darkATKBonus"] = flat(100), flat(50)
    fake = type("Ranged", (), {
        "sp": sp, "damage_scale": {"Sanctum": 1.25}, "ring_attack": {},
        "weapons": {"Bow": {"name": "Bow", "infusions": {"No_Infusion": {
                        "atk": {"physical": 150}, "atkScale": {"dexterity": 0.5}, "rate": {"physical": 1.0}}}},
                    "Sanctum": {"name": "Sanctum", "infusions": {"Dark": {
                        "atk": {"physical": 70}, "atkScale": {"dark": 0.52}, "rate": {"physical": 0.7, "dark": 1.3}}}}},
        "ranged": {"Bow": {"ammo": 1, "kind": "bow", "hand": 0.9, "special": []},
                   "Sanctum": {"ammo": 4, "kind": "crossbow", "hand": 0.9, "special": [
                       {"types": ["dark"], "flat": {"dark": 50}, "mv": 7.2, "lower": 255, "shots": 1, "status": {}}]}},
        "ammo": {"Wood Arrow": {"type": 1, "flat": {}, "status": {}, "mv": 1.176, "lower": 70},
                 "Dark Arrow": {"type": 1, "flat": {"dark": 110}, "status": {}, "mv": 1.176, "lower": 70},
                 "Dark Bolt": {"type": 4, "flat": {"dark": 200, "physical": 200}, "status": {}, "mv": 0.392,
                               "lower": 34}}})
    st = dict.fromkeys(STATS, 20)
    bow = {label: ar for label, ar, _ in ranged_options(fake, "Bow", "No_Infusion", st)}
    san = {label: ar for label, ar, _ in ranged_options(fake, "Sanctum", "Dark", st)}
    zero = dict.fromkeys(DMG + PHYS_TYPES, 0.0)
    return [
        ("bow fires arrows only", sorted(bow), ["Dark Arrow", "Wood Arrow"]),
        ("wood arrow: the bow's own attack x hand 0.9", bow["Wood Arrow"], {"physical": (150 + 50) * 0.9}),
        ("dark arrow adds its flat dark", bow["Dark Arrow"], {"physical": 200 * 0.9, "dark": 110 * 0.9}),
        ("no dark base, dark rate and coefficient: still a dark attack",
         attack_rating(fake, "Sanctum", "Dark", st), {"physical": 70, "dark": int(0.52 * 50)}),
        ("special shot keeps its row's dark only", san["special shot"], {"dark": (26 + 50) * 0.9}),
        ("dark bolt carries both types", san["Dark Bolt"], {"physical": 270 * 0.9, "dark": 226 * 0.9}),
        ("ar objective: the larger summed attack, here the bolt's",
         (round(ranged_value(fake, "Sanctum", "Dark", st, "ar", zero)[0], 3),
          ranged_value(fake, "Sanctum", "Dark", st, "ar", zero)[1]), (round(496 * 0.9, 3), "Dark Bolt")),
        ("damage objective: per shot at MV, x damageScale",
         round(ranged_value(fake, "Sanctum", "Dark", st, "damage", zero)[0], 3),
         round(max(76 * 0.9 * 0.9 * 7.2, hit_damage({"physical": 243.0, "dark": 203.4}, zero, 0.392, "physical", 34))
               * 1.25, 3)),
    ] + _ranged_adaptive_cases(fake, st, zero)


def stamina_selftest_cases() -> list[tuple]:
    """The stamina model over made-up rows: cost = base x the slot's multiplier / 10, alternating the
    chain's 1st and 2nd attack; a full bar starts attacks while stamina is above 0; the window
    charges the attacks whose hits it counts. The chain steps every 0.5 s (a planted chain_open for
    the made-up anim -7), each attack hitting 0.2 s in."""
    rates = {"Nnormal1st1H": 0.95, "normal2nd1H": 1.05, "normal1st2H": 1.187, "normal2nd2H": 1.312}
    fake = type("Stamina", (), {
        "stamina_cost": {"W": {"melee": 200.0, "ranged": 300.0, "rates": rates}},
        "stamina_max": [0] + [1000 + 10 * v for v in range(1, 100)], "damage_scale": {},
        "_ring_gear": {"Dragon": {"add": {}, "scaled": [], "slots": 0, "hp": 1.0, "load": 1.0, "stamina": 1.1},
                       "Endurance": {"add": {"endurance": 5}, "scaled": [], "slots": 0, "hp": 1.0, "load": 1.0,
                                     "stamina": 1.0}}})
    _chain_cache[-7] = 0.5
    attacks = {("w", "Single1HandNormal1st"): {"anim": -7, "spd": [1.0, 1.0], "hits": [_hit(6, 9, 1.0)]},
               ("w", "Single1HandNormal2nd"): {"anim": -7, "spd": [1.0, 1.0], "hits": [_hit(6, 9, 1.0)]}}
    st = dict.fromkeys(STATS, 20)
    zero = dict.fromkeys(DMG + PHYS_TYPES, 0.0)
    m = stamina_metrics(fake, "W", False, ["W"], attacks, {"physical": 100}, zero, 1.5, st)
    return [
        ("R1 1H 1st: 200 x 0.95 / 10", attack_stamina(fake, "W", False, False), 19.0),
        ("R1 2H 2nd: 200 x 1.312 / 10", round(attack_stamina(fake, "W", True, True), 3), 26.24),
        ("no regulation row, no cost", attack_stamina(fake, "X", False, False), None),
        ("shot: rangedAttackBaseCost / 10", shot_stamina(fake, "W"), 30.0),
        ("max stamina: staminaMax[END 20] / 10", max_stamina(fake, st), 120.0),
        ("a dragon ring's factor", round(max_stamina(fake, st, ["Dragon"]), 3), 132.0),
        ("an END ring reads a later row", max_stamina(fake, st, ["Endurance"]), 125.0),
        ("bar: an attack starts above 0 and may end below", bar_attacks([50, 50, 50], 120), 3),
        ("bar: at exactly 0 no attack starts", bar_attacks([50] * 5, 100), 2),
        ("bar: a zero cost ends the count", bar_attacks([50, 0, 50], 120), 1),
        ("window: 1st/2nd/1st attacks land by 1.5 s", m["stamina_per_attack"], [19.0, 21.0]),
        ("window charges the three attacks it counts", m["stamina_per_window"], 59.0),
        ("damage per stamina: 3 x 1000/12 over 59", m["damage_per_stamina"], 4.237),
        ("bar 120: 19+21+19+21+19+21 = 120, six attacks", (m["bar_attacks"], m["bar_damage"], m["bar_seconds"]),
         (6, 500.0, 2.5)),
        ("every --rank metric is in the row", {k for k in RANKS.values() if k} <= set(m), True),
        ("shot: 60 a shot over 30 stamina, four from 120",
         shot_metrics(fake, "W", 60.0, st), {"stamina_per_attack": [30.0], "stamina_per_window": 30.0,
                                             "damage_per_stamina": 2.0, "max_stamina": 120.0, "bar_attacks": 4,
                                             "bar_damage": 240.0}),
    ]


def _ranged_adaptive_cases(fake, st: dict, zero: dict) -> list[tuple]:
    """A shot against an AdaptiveDefense meets the defender's answer to that shot's own types: one
    defender with a free slot, counters Steel_2 (physical +100) and Dark_3 (dark cut +15%, D +150)."""
    rings = type("R", (), {"rings": {"Steel_2": {"group": "Steel", "name": "Steel_2"},
                                     "Dark_3": {"group": "Dark", "name": "Dark_3"}},
                           "ring_defense": {"Steel_2": {"physical": 100}, "Dark_3": {"dark": 150}},
                           "defense_buffs": {}})
    saved = dict(DEFENDER)
    try:
        DEFENDER.update(mode="adaptive", buff="none")
        adp = AdaptiveDefense(rings, zero, [[]])
    finally:
        DEFENDER.update(saved)
    dark, phys = with_defense(zero, {"dark": 150}), with_defense(zero, {"physical": 100})
    # the special's pure dark 68.4 and the Dark Bolt's 243 + 203.4 both draw Dark_3 (its 150 cuts
    # 0.15 x 203.4 = 30.5 a hit, Steel_2's 100 / 12 = 8.3); a Wood Arrow's pure physical draws Steel_2
    special = hit_damage({"dark": 68.4}, dark, 7.2, "physical", 255) * 1.25
    bolt = hit_damage({"physical": 243.0, "dark": 203.4}, dark, 0.392, "physical", 34) * 1.25
    return [
        ("adaptive: the Sanctum special's pure dark draws the Dark Quartz", adp.respond({"dark": 68.4})[1],
         ["Dark_3"]),
        ("adaptive: a wood arrow's physical draws Steel Protection", adp.respond({"physical": 180.0})[1],
         ["Steel_2"]),
        ("adaptive: the shot is scored against its own reply",
         round(ranged_value(fake, "Sanctum", "Dark", st, "damage", adp)[0], 3), round(max(special, bolt), 3)),
        ("adaptive: the wood arrow against Steel Protection's +100",
         round(ranged_value(fake, "Bow", "No_Infusion", st, "damage", adp, special=False)[0], 3),
         round(max(hit_damage({"physical": 180.0}, phys, 1.176, "physical", 70),
                   hit_damage({"physical": 180.0, "dark": 99.0}, dark, 1.176, "physical", 70)), 3)),
    ]


def status_selftest_cases() -> list:
    """Status procs (status_metrics, status_cut) on made-up numbers shaped like the regulation's."""
    bleed = {"damage": 200, "lockout": 10.0}
    m = status_metrics(bleed, 40.0, [0.2], 4, 1.5)  # 32 a hit: 4 to proc, all 4 land in the window
    cut_data = type("D", (), {
        "armor": {s: {} for s in ARMOR_SLOTS},
        "status_resist": {"poison": [0.0] + [10.0 + v for v in range(1, 100)],
                          "bleed": [0.0] + [20.0 + v for v in range(1, 100)]},
        "armor_status": {"head": {"Hat": {"poison": 5.0, "bleed": 0.0}},
                         "chest": {"Coat": {"poison": 7.0, "bleed": 90.0}}}})
    st = dict.fromkeys(STATS, 10) | {"adaptability": 20, "vitality": 8, "faith": 4}
    hat = Build("", st, ["Hat", "Naked", "Naked", "Naked"], [], 0, [], [])
    coat = Build("", st, ["Hat", "Coat", "Naked", "Naked"], [], 0, [], [])
    return [
        ("status: 32 a hit procs on the 4th hit", m["hits_to_proc"], 4),
        ("status: 4 hits in the window proc a fresh gauge", m["status_damage_first_window"], 200),
        # a proc every 10 s lockout + 4 hits x 1.5/4 s = 11.5 s; the 1.5 s window's share of 200
        ("status: steady proc rate charges the lockout", round(m["status_damage_per_window"], 3),
         round(200 * 1.5 / 11.5, 3)),
        ("status: the procing hit's surplus is lost (50 a hit: 2)",
         status_metrics(bleed, 50.0, [0.0], 2, 1.0)["hits_to_proc"], 2),
        ("status: 3 hits short of 4 deal nothing to a fresh gauge",
         status_metrics(bleed, 40.0, [0.2], 3, 1.5)["status_damage_first_window"], 0.0),
        ("status: hits_to_proc is the median defender's (2, 4, never)",
         status_metrics(bleed, 50.0, [0.0, 0.5, 1.0], 2, 1.0)["hits_to_proc"], 4),
        ("status: a defender who resists all of it never procs",
         status_metrics(bleed, 50.0, [1.0], 2, 1.0), {"hits_to_proc": 0, "status_damage_per_window": 0.0,
                                                       "status_damage_first_window": 0.0}),
        ("status: no build-up, no proc", status_metrics(bleed, 0.0, [0.0], 4, 1.5)["status_damage_per_window"], 0.0),
        # poison row trunc((3 x 20 + 8) / 4) = 17 -> 27 %, plus the hat's 5 %
        ("status cut: poison stat row + armour", round(status_cut(cut_data, hat, "poison"), 6), 0.32),
        # bleed row trunc((60 + 4) / 4) = 16 -> 36 %, plus 90 % from the coat: clamped to 1
        ("status cut: clamped to 1", status_cut(cut_data, coat, "bleed"), 1.0),
    ]


def defense_selftest_cases() -> list:
    """Offline checks of the defender's defense changes (speffect_defense, with_defense,
    AdaptiveDefense) on made-up rings and events."""
    class Ins:  # an emevd instruction: (bank, index) and its words as (i32, f32, u32)
        def __init__(self, bank, index, seconds, value, kind):
            self.bank, self.index, self.raw = bank, index, [(0, seconds, 0), (0, 0.0, value << 16 | kind)]

        def words(self):
            return self.raw
    ev = type("Ev", (), {"instructions": [Ins(100090, 4, 90.0, 15, 3), Ins(100090, 5, 0.0, 80, 0),
                                          Ins(100090, 4, 90.0, 20, 5), Ins(100090, 2, 0.0, 30, 1)]})
    # in event id order, so Flame_3 (the last of its group) is the counter, not Flame_1
    rings = {"Steel_2": ("Steel", {"physical": 100}), "Flame_1": ("Flame", {"fire": 80}),
             "Flame_3": ("Flame", {"fire": 150}), "Clutch": ("Clutch", {"physical": -80}),
             "Spell_1": ("Spell", {"magic": 50}), "Dark_1": ("Dark", {"dark": 50}), "Life": ("Life", None)}
    fake = type("D", (), {"rings": {k: {"group": g, "name": k} for k, (g, _) in rings.items()},
                          "ring_defense": {k: a for k, (_, a) in rings.items() if a},
                          "defense_buffs": {"Burr": {"source": "item", "add": {"fire": 150}, "seconds": 90.0},
                                            "Sweat": {"source": "spell", "add": {"fire": 300}, "seconds": 90.0}}})
    base = {"physical": 500.0, "magic": 200.0, "fire": 200.0, "lightning": 200.0, "dark": 200.0,
            "slash": 520.0, "strike": 480.0, "thrust": 500.0}
    # one build with a free slot, one already in Flame_1 (upgraded in place: +70), one whose four
    # rings all change a defense (one goes, the least useful against the attack: the first on a tie)
    builds = [["Life"], ["Flame_1", "Life"], ["Steel_2", "Clutch", "Spell_1", "Dark_1"]]
    saved = dict(DEFENDER)
    try:
        DEFENDER.update(mode="adaptive", buff="none")
        adp = AdaptiveDefense(fake, base, builds)
        fire, fire_used = adp.respond({"fire": 300})
        phys, phys_used = adp.respond({"physical": 300})
        none, none_used = adp.respond({})
        DEFENDER.update(buff="item")
        item, item_used = AdaptiveDefense(fake, base, builds).respond({"fire": 300})
        DEFENDER.update(buff="any")
        spell, spell_used = AdaptiveDefense(fake, base, builds).respond({"fire": 300})
    finally:
        DEFENDER.update(saved)
    r = lambda d: {k: round(v, 3) for k, v in d.items()}
    return [
        ("100090[4] fire 15 is D +150, [5] physical 80 is -80, status and attack dropped",
         speffect_defense(ev), ({"fire": 150, "physical": -80}, 90.0)),
        ("a physical add reaches every physical type", with_defense(base, {"physical": 50})["strike"], 530.0),
        ("quartz +15%: the cut rises 0.15", round(damage("fire", 300, 200 + 150), 6),
         round(300 * (1 - (200 + 100) / 1000 - 0.15), 6)),
        ("as a dict it is the static defense", dict(adp), base),
        # fire: (150 + 70 + 150) / 3 fire, and the third build's Steel_2 out (all tie at 0 vs fire)
        ("fire attack: Flame_3 in, mean over 3 builds", (r(fire), fire_used),
         (r(with_defense(base, {"fire": 370 / 3, "physical": -100 / 3})), ["Flame_3"])),
        # physical: (100 + 100 + 0) / 3, the third already wears Steel_2
        ("physical attack: Steel_2 in", (r(phys), phys_used),
         (r(with_defense(base, {"physical": 200 / 3})), ["Steel_2"])),
        ("no attack, no swap", (none, none_used), (base, [])),
        ("least useful vs physical: the clutch ring",
         _least_useful([("Steel", {"physical": 100}), ("Clutch", {"physical": -80})], {"physical": 1 / 12}),
         {"physical": -80}),
        ("buff 'item': the Burr on top of the ring", (r(item), item_used),
         (r(with_defense(base, {"fire": 370 / 3 + 150, "physical": -100 / 3})), ["Flame_3", "Burr"])),
        ("buff 'any': the stronger spell", spell_used, ["Flame_3", "Sweat"]),
        ("a plain dict answers with itself", respond(base, {"fire": 300}), base),
    ]


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
        # armour poise lost to unmet requirements (PlayerLackOfStatsParam row 28): none when met,
        # 30% + 50% per unit of shortfall, all of it past a full unit
        ("lack of stats: requirements met", lack_of_stats({"strength": 20}, {"strength": 20}), 0.0),
        ("lack of stats: STR 10 of 20", lack_of_stats({"strength": 20}, {"strength": 10}), 0.55),
        ("lack of stats: capped at 0.8", lack_of_stats({"strength": 20, "dexterity": 20},
                                                       {"strength": 0, "dexterity": 0}), 0.8),
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
    # an infusion's rate move (EXE 0x14034fed0), on regulation rows: the Dagger gives half its
    # physical rate to the element; a catalyst whose target is already nonzero only gains
    wrp = dict.fromkeys(RATE_FIELDS, 0.0) | dict.fromkeys([f for f, _, _ in INFUSION_MOVE.values() if f], 0.0)
    dagger = wrp | {"physicalRate": 100.0, "addMagicRate": 50.0, "addPhysicalRateByCrude": 0.0}
    wisdom = wrp | {"magicRate": 100.0, "addMagicRate": 10.0, "addThunderRate": 10.0}
    chime = wrp | {"thunderRate": 100.0, "darkRate": 100.0, "addThunderRate": 15.0, "addMagicRate": 15.0}
    cases += [
        ("magic dagger: physical 50, magic 50", infused_rates(dagger, "Magic")[:2], [50.0, 50.0]),
        ("raw dagger with no crude rate: unchanged", infused_rates(dagger, "Raw")[:2], [100.0, 0.0]),
        ("magic staff of wisdom: magic 110", infused_rates(wisdom, "Magic")[:3], [0.0, 110.0, 0.0]),
        ("lightning staff of wisdom: magic 90, lightning 10", infused_rates(wisdom, "Lightning")[:3],
         [0.0, 90.0, 10.0]),
        ("lightning dragon chime: lightning 115, dark 85", infused_rates(chime, "Lightning")[:5],
         [0.0, 0.0, 115.0, 0.0, 85.0]),
        ("magic dragon chime: 15 split over lightning and dark", infused_rates(chime, "Magic")[:5],
         [0.0, 15.0, 92.5, 0.0, 92.5]),
    ]
    # a player's child bullet hits with the spell row's types and flat damage (EXE 0x140445ec0),
    # at its own row's damageRate; Outcry's row is fire 0 + dark 100
    root = {"damageType01": 3, "damage01": 0, "damageType02": 4, "damage02": 100, "damageType03": 5, "damage03": 7}
    child = [{"type": "fire", "flat": 0, "rate": 1.75, "lower": 0.0, "row": 33320040}]
    cases.append(("child hit takes the spell row's fire + dark", inherited_hits(child, root),
                  [{"type": "fire", "flat": 0, "rate": 1.75, "lower": 0.0, "row": 33320040},
                   {"type": "dark", "flat": 100, "rate": 1.75, "lower": 0.0, "row": 33320040}]))
    # max HP (EXE 0x14038e1e0) over made-up columns: hpMax at VGR plus the other eight stats'
    # additionalHp, a stat outside 1-99 at row 1; with no columns, the site's getHP from VGR alone
    tables = type("HP", (), {"hp_max": [0] + [1000 + v for v in range(1, 100)],
                             "additional_hp": [0] + [2 * v for v in range(1, 100)]})
    ten = dict.fromkeys(STATS, 10)
    cases += [
        ("max HP: hpMax at VGR + 8 x additionalHp", hit_points(tables, {**ten, "vigor": 30}), 1030 + 8 * 20),
        ("max HP: FTH 0 reads row 1", hit_points(tables, {**ten, "faith": 0}), 1010 + 7 * 20 + 2),
        ("max HP without the regulation: getHP", hit_points(type("Site", (), {"hp_max": []}), {**ten, "vigor": 30}),
         500 + 30 * 20 + 20 * 10),
    ]
    # poise (stagger_hits, EXE 0x140136570): poise <= 0 after a hit, or armorBreak 1/2, staggers;
    # 0.56 regen per second between hits
    cases += [
        ("no poise: the first hit staggers", stagger_hits([(0.0, 35.0, 0)], 0.0), 1),
        ("35 poise damage through 30 poise", stagger_hits([(0.0, 35.0, 0), (0.5, 35.0, 0)], 30.0), 1),
        ("50 poise: two 35s", stagger_hits([(0.0, 35.0, 0), (0.5, 35.0, 0)], 50.0), 2),
        ("armorBreak 1 staggers through any poise", stagger_hits([(0.0, 1.0, 1)], 132.0), 1),
        ("132 poise vs 35 every 0.5 s with regen: 4", stagger_hits([(i * 0.5, 35.0, 0) for i in range(6)], 132.0), 4),
        ("regen can outlast a weak chain", stagger_hits([(0.0, 0.5, 0), (10.0, 0.5, 0)], 1.0), None),
        # OKH R1: hitbox 23-28, 111900 frames 20-30 (TAE) -> 8 of its 28 windup+active frames
        ("hyperarmor covers frames 20-28 of 0-28", round(hyperarmor_cover(
            {"anim": 0, "hits": [{"start": 23.0, "end": 28.0, "rate": 0.945, "live": 1}]}, [(20.0, 30.0)]), 4),
         round(8 / 28, 4)),
        ("no window, no hyperarmor", hyperarmor_cover({"anim": 0, "hits": [{"start": 8.0, "end": 15.0, "rate": 1.0,
                                                                           "live": 1}]}, []), 0.0),
    ]
    cases += defense_selftest_cases()
    cases += status_selftest_cases()
    cases += flex_selftest_cases()
    cases += ranged_selftest_cases()
    cases += metrics_selftest_cases()
    cases += trade_selftest_cases()
    cases += stamina_selftest_cases()
    if ATTACKS.exists():  # the real extracted rows agree with the copies above
        real = load_attacks()
        for key in [k for k in A if k in real]:
            cases.append((f"attacks.json {key}", attack_hits(real[key]), attack_hits(A[key])))
        okh = real.get((norm("Old Knight Hammer"), "Single1HandNormal1st"))
        if okh and TAE_DIR.exists():  # the TAE's own 111900 window, docs/DS2-DPS-MECHANICS.md section 3
            cases.append(("OKH R1 hyperarmor from the TAE", round(hyperarmor_cover(okh), 4), round(8 / 28, 4)))
    bad = 0
    for what, got, want in cases:
        if got != want:
            bad += 1
            print(f"  FAIL {what}: got {got!r}, want {want!r}")
    print(f"selftest: {len(cases) - bad}/{len(cases)} pass" + ("" if ATTACKS.exists() else f" ({ATTACKS} absent)"))
    return 1 if bad else 0


def parse_armor(data: Data, text: str, flag: str, ap) -> list[str]:
    """`HEAD/CHEST/HANDS/LEGS`, names or keys, to four armour keys; a missing or empty slot is
    Naked. An unknown piece is an error, not a bare slot."""
    out = []
    names = text.split("/")
    if len(names) > len(ARMOR_SLOTS):
        ap.error(f"{flag}: at most {len(ARMOR_SLOTS)} pieces, head/chest/hands/legs")
    for slot, name in zip(ARMOR_SLOTS, names + [""] * (len(ARMOR_SLOTS) - len(names))):
        name = name.strip()
        key = name if name in data.armor[slot] else next(
            (k for k, v in data.armor[slot].items() if v.get("name", k) == name), "Naked")
        if name not in ("", "Naked") and key == "Naked":
            ap.error(f"{flag}: unknown {slot} armour {name!r}")
        out.append(key)
    return out


def defender_line(data: Data, corpus: list[Build], sl: int, defender, dfn: dict, n: int) -> str:
    """What the damage column is scored against, as --weapons-for, --optimize and --best-infusion
    print it: the average defender, or the --defender set at the bracket's median stats."""
    nums = " ".join(f"{k} {dfn[k]:.0f}" for k in DMG + PHYS_TYPES)
    if isinstance(dfn, AdaptiveDefense):
        nums += ("; adapts: swaps a ring slot to " + ", ".join(dfn.names.values()) + " against each weapon"
                 + (f", plus the best {DEFENDER['buff'] if DEFENDER['buff'] == 'item' else 'item or spell'} "
                    "defense buff" if DEFENDER["buff"] != "none" else "") + " (column 'vs')")
    if defender is None:
        return f"average defender at this SL ({n} builds): {nums}"
    st, _ = bracket_stats(data, corpus, sl)
    names = " / ".join(data.armor[s][p].get("name", p) for s, p in zip(ARMOR_SLOTS, defender))
    return (f"defender {names} at the bracket's median stats ({n} builds: "
            + " ".join(f"{LABEL[s]} {st[s]}" for s in STATS) + f"): {nums}")


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
                         "either grip: the game has no two-handed STR multiplier")
    ap.add_argument("--class", dest="start_class", metavar="CLASS",
                    help="with --optimize/--generate: this starting class only (sorcerer, warrior, ...), as a "
                         "build for an existing character must be -- the game has no class change")
    ap.add_argument("--spells", metavar="SPELL,SPELL",
                    help="with --optimize/--generate: spells the build must attune and cast (SoulsPlanner keys or "
                         "names; repeat one for a second copy). Their INT/FTH requirements raise those stats and "
                         "their summed slot cost raises ATT, as a weapon's requirements do; no build when they "
                         "do not fit the SL")
    ap.add_argument("--no-floors", action="store_true",
                    help="with --optimize/--generate/--weapons-for: drop the SL bracket floors (VIG/VIT/ADP, ATT with "
                         "--spells, END for a high-stamina weapon): they are the medians of real builds, not a game rule")
    ap.add_argument("--allow-naked", action="store_true",
                    help="with --generate: no armour (by default the best set under 70%% load is chosen)")
    ap.add_argument("--objective", choices=["damage", "ar", "bleed", "poison"], default="damage",
                    help="with --optimize: what the free points maximize; ar is the summed attack rating, worn "
                         "rings' adds counted, before any defense. With --weapons-for (and the list --optimize "
                         "prints): ar is --raw-ar and ignores --window; bleed/poison rank by build-up per hit x "
                         "hits of the best R1/R2 attack, or hits landed within --window seconds")
    g.add_argument("--weapons-for", metavar="STATS",
                   help='rank weapons for these stats, e.g. "VGR=10,END=16,VIT=7,ATT=9,STR=20,DEX=12,ADP=8,INT=12,FTH=12"')
    g.add_argument("--flexibility", metavar="STATS",
                   help="how many weapons these stats wield 1H/2H, the weapon weight left under 70%% load after "
                        "--armor, and the build's percentile among its --k nearest stat-neighbour builds")
    g.add_argument("--load-evidence", metavar="SPELL", nargs="?", const="Climax",
                   help="how many weapons real builds wield (all, attuning SPELL, and a stat proxy for it) "
                        "and their VIT, armour weight and load by that count: the load weighting's measurement")
    g.add_argument("--flex-sweep", metavar="W,W,...",
                   help="--optimize every EXPECT_BUILDS/EXPECT_ONE_HANDED case at each flexibility weight and "
                        "print the objective and the flexibility it gives: how FLEX_WEIGHT was chosen")
    ap.add_argument("--armor", metavar="HEAD/CHEST/HANDS/LEGS",
                    help="with --flexibility: the armour worn, names or keys, '/'-separated (Naked for none)")
    ap.add_argument("--defender", metavar="HEAD/CHEST/HANDS/LEGS",
                    help="with --weapons-for/--optimize/--generate/--best-infusion/--infusion-gaps: score damage "
                         "against this set, names or keys, '/'-separated (Naked for none), worn at the SL "
                         "bracket's median stats, instead of the bracket's average defender. Not --armor, which "
                         "is the armour the build itself wears for --flexibility's load")
    ap.add_argument("--static-defender", action="store_true",
                    help="score damage against the defenders' rings as worn, instead of each defender swapping "
                         "one ring slot to the ring that cuts this weapon's damage most (Ring of Steel "
                         "Protection+2, a Quartz Ring+3 or Dispelling Ring+1; AdaptiveDefense). --objective ar ignores both")
    ap.add_argument("--defender-buff", choices=["none", "item", "any"], default="none",
                    help="the defender also uses the damage-cutting buff that cuts this weapon's damage most: "
                         "'item' a consumable anyone can use (the Burrs, Dark Troches), 'any' spells as well "
                         "(Flash Sweat, Iron Flesh, the Magic Barriers, Sacred Oath) whatever the defender's "
                         "stats. Needs the adaptive defender")
    ap.add_argument("--flex-weight", type=float, default=None, metavar="W",
                    help=f"with --optimize/--generate: the soft flexibility term per weapon unlocked per point "
                         f"(default {FLEX_WEIGHT}; 0 is the optimizer without it)")
    g.add_argument("--best-infusion", metavar="WEAPON",
                   help="every infusion WEAPON takes, ranked by --objective at --stats (full upgrade), with the "
                        "best one's margin over the runner-up")
    g.add_argument("--best-weapons", metavar="INFUSION",
                   help="every weapon INFUSION goes on, each at the build --optimize makes for it at --sl (or "
                        "each SL of --sweep), ranked by --objective over the R1 hits landed in --window seconds "
                        "(default 1.5) at those stats")
    g.add_argument("--adoption", action="store_true",
                   help="what real builds carry as their melee weapon (first in rh1..rh3, lh1..lh3), per SL bracket: share by weapon class "
                        "and the --top weapons")
    ap.add_argument("--sweep", metavar="SL,SL,...",
                    help="with --best-weapons: one ranking per soul level, in place of --sl")
    ap.add_argument("--top", type=int, default=15, help="with --best-weapons: rows per soul level")
    ap.add_argument("--with-status", action="store_true",
                    help="with --best-weapons and --objective damage: rank by metrics.damage_with_status, the "
                         "window's damage plus the poison and bleed its hits deal through their procs; the "
                         "score column stays the damage alone")
    ap.add_argument("--rank", choices=list(RANKS), default="window",
                    help="with --best-weapons and --objective damage: window (default) ranks by the R1 hits "
                         "landed in --window; per-stamina by those hits' damage per stamina point their attacks "
                         "cost; bar by the R1 chain damage a full bar of the build's max stamina (END, rings) "
                         "pays for. Every row prints all three")
    ap.add_argument("--jobs", type=int, default=0,
                    help="with --best-weapons: worker processes (default: a quarter of the CPUs)")
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
    ap.add_argument("--json", action="store_true", help="with --minimum: also print the options as JSON; with --best-weapons: print only JSON")
    ap.add_argument("--lam", type=float, default=50.0, help="EASE L2 penalty")
    ap.add_argument("--site-numbers", action="store_true",
                    help="keep SoulsPlanner's stat defense, weapon attack and status numbers instead of the "
                         "game's (apply_regulation), to compare against the planner")
    ap.add_argument("--tables", type=Path, default=CACHE / "site-tables", help="where site JS/JSON is cached")
    a = ap.parse_args()
    # Runs only on CPU nothing else wants: SCHED_IDLE, inherited by --best-weapons' workers. Measured
    # 2026-09-30: 16 workers at the shell's nice -4 took every core from a game in the foreground.
    os.sched_setscheduler(0, os.SCHED_IDLE, os.sched_param(0))
    if a.selftest:
        return selftest()
    if a.static_defender and a.defender_buff != "none":
        ap.error("--defender-buff needs the adaptive defender (drop --static-defender)")
    # --export-backend's bracket defense and calibration are the static defender's (the Rust port
    # works the adaptive answer out per question), and so are its fixtures but the ADAPTIVE_* ones
    DEFENDER.update(mode="static" if a.static_defender or a.export_backend else "adaptive", buff=a.defender_buff)

    a.tables.mkdir(parents=True, exist_ok=True)
    sp_json, mm_json = dump_site_tables(a.tables)
    data = Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    if not a.site_numbers:
        print(apply_regulation(data), file=sys.stderr)
    if a.start_class and a.start_class.lower() not in data.classes:
        ap.error(f"unknown class {a.start_class!r}: one of {', '.join(data.classes)}")
    spells = []
    for name in (a.spells or "").split(","):
        if name.strip():
            key = data.sp_key.get(norm(name))
            if key not in data.spells or key not in data.spell_req:
                ap.error(f"unknown spell {name!r}")
            spells.append(key)
    defender = parse_armor(data, a.defender, "--defender", ap) if a.defender else None
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
                           a.window or 1.5, a.k, a.allow_naked, a.grip, a.flex_weight, spells, a.start_class,
                           not a.no_floors, defender)
        if g is None:
            print(f"no valid SL {a.sl} {a.start_class or ''} build wields {data.weapons[weapon]['name']} "
                  f"(grip {a.grip})"
                  + (f" and casts {', '.join(data.spells[s]['name'] for s in spells)}" if spells else ""))
            print_refusal(refusal(data, corpus, weapon, inf.replace(" ", "_") or "No_Infusion", a.sl, a.objective,
                                  a.grip, spells, a.start_class, not a.no_floors))
            return 2
        if a.json:
            print(json.dumps(g, indent=1))
            return 0
        print(f"{g['class']} SL {g['sl']} {'two-handed' if g['two_handed'] else 'one-handed'}, "
              f"{g['objective']} {g['value']}\n  " + " ".join(f"{s[:3].upper()} {v}" for s, v in g["stats"].items()))
        if defender is not None:
            print("  " + defender_line(data, corpus, a.sl, defender, *defender_defense(data, corpus, a.sl, defender)))
        print(f"  primary: {g['primary'][0]} ({g['primary'][1].replace('_', ' ')})")
        for title, key in (("one-handed", "weapons_1h"), ("two-hand only", "weapons_2h_only")):
            print(f"  {title}:")
            for n, i, d in g[key]:
                print(f"    {d:6}  {n} ({i.replace('_', ' ')})")
        print("  rings: " + ", ".join(g["rings"]))
        for t in g["ring_trades"]:
            print(f"  ring in place of points: {trade_line(t)}")
        if g["spells"]:
            print(f"  spells: {', '.join(g['spells'])} ({g['slots'][0]} of {g['slots'][1]} attunement slots)")
            for label, name, p, over in g["catalysts"]:
                print(f"  {label} catalyst: {name} (cast power {p:.1f} at these stats, full upgrade)"
                      + (f"; {over} casts harder but these stats do not meet its requirements" if over else ""))
            if not g["catalysts"]:
                print("  catalyst: none -- the regulation was not read, or nothing these stats wield casts them")
        if a.allow_naked:
            print("  armor: none (--allow-naked)")
        else:
            print("  armor: " + (" / ".join(g["armor"]) or "none"))
        if g["armor_note"]:
            print(f"  armor: {g['armor_note']}")
        if g["poise"] is not None:
            p = g["trade_previous"]
            print(f"  poise: {g['poise']:.1f}, holds through counter hits up to {g['poise_target']:.1f} poise "
                  f"damage; my R1 lands in {g['trade_rate']:.0%} of trades, exchange {g['exchange']:+.2f} damage "
                  f"per trade (best_armor's set: poise {p['poise']:.1f}, {p['trade_rate']:.0%}, "
                  f"{p['exchange']:+.2f}); over {g['trade_basis']}")
        print(f"  flexibility: {flex_line(g['flex'])}")
        print(f"  load: {flex_load_line(g['flex'])}")
        return 0
    if a.load_evidence:
        spell = data.sp_key.get(norm(a.load_evidence), a.load_evidence)
        if spell not in data.spells:
            ap.error(f"unknown spell {a.load_evidence!r}")
        print("\n".join(load_evidence(data, load_corpus(data)[0], spell)))
        return 0
    if a.flex_sweep:
        weights = [float(w) for w in a.flex_sweep.split(",")]
        corpus, _ = load_corpus(data)
        print("weapon (infusion) SL objective grip: per weight, objective (change) 1H/2H percentile")
        for grip, cases in (("two", EXPECT_BUILDS), ("one", EXPECT_ONE_HANDED)):
            for weapon, inf, sl, objective in cases:
                cells, base = [], None
                for w in weights:
                    best, _ = optimize_build(data, corpus, weapon, inf, sl, objective, grip, w)
                    if best is None:
                        cells.append("none")
                        continue
                    f = flexibility(data, corpus, best[3], sl)
                    base = best[0] if base is None else base
                    cells.append(f"w={w:g}: {best[0]:.0f} ({(best[0] / base - 1) if base else 0:+.1%}) "
                                 f"{f['one']}/{f['two']} p{round(f['percentile'])} "
                                 + "".join(f"{s[:3].upper()}{best[3][s]}" for s in REQ_STATS))
                print(f"{data.weapons[weapon]['name']} ({inf}) SL {sl} {objective} {grip}")
                for c in cells:
                    print(f"    {c}")
        return 0
    if a.flexibility:
        stats = parse_stats(a.flexibility)
        sl = a.sl or sum(stats.values()) - 53
        armor = parse_armor(data, a.armor, "--armor", ap) if a.armor else []
        corpus, _ = load_corpus(data)
        f = flexibility(data, corpus, stats, sl, armor, [], a.k)
        if a.json:
            print(json.dumps(f, indent=1))
            return 0
        print(f"stats {' '.join(f'{s[:3].upper()} {v}' for s, v in stats.items())}  ->  SL {sl}")
        print(f"  {flex_line(f)}")
        print(f"    score {f['score']} (1H + 2H): {f['below']} of the {f['n']} nearest builds score below, "
              f"{f['equal']} the same")
        print(f"  {flex_load_line(f)}")
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
        best, floors = optimize_build(data, corpus, weapon, inf, a.sl, a.objective, a.grip, a.flex_weight, spells,
                                      a.start_class, not a.no_floors, defender=defender)
        if best is None:
            print(f"no valid SL {a.sl} {a.start_class or ''} build wields {data.weapons[weapon]['name']} (grip {a.grip})"
                  + (f" and casts {', '.join(data.spells[s]['name'] for s in spells)}" if spells else "")
                  + " with the bracket floors "
                  + " ".join(f"{s[:3].upper()} {v}" for s, v in floors.items()))
            print_refusal(refusal(data, corpus, weapon, inf, a.sl, a.objective, a.grip, spells, a.start_class,
                                  not a.no_floors))
            return 2
        val, cls, two, stats, worn = best
        print(f"{data.weapons[weapon]['name']} ({inf.replace('_', ' ')}), {a.objective} {val:.0f}: {cls} SL {a.sl}"
              f" {'two-handed' if two else 'one-handed'}\n  "
              + " ".join(f"{s[:3].upper()} {stats[s]}" for s in STATS)
              + "\n  floors (bracket medians): " + " ".join(f"{s[:3].upper()} {v}" for s, v in floors.items()))
        for t in ring_trades(data, corpus, weapon, a.sl, a.grip, spells, cls, not a.no_floors, worn):
            print(f"  ring: {trade_line(t)}")
        for r in worn:
            if r in data.ring_attack:  # an offensive ring (offense_rings): it stands in for nothing
                print(f"  ring: {data.rings[r]['name']}: "
                      + ", ".join(f"{k} attack +{v}" for k, v in data.ring_attack[r].items()))
        print(f"  flexibility: {flex_line(flexibility(data, corpus, stats, a.sl))}")
        eff = gear_stats(data, stats, worn)  # the weapons below are read at what the rings give
        a.weapons_for = ",".join(f"{s[:3].upper()}={eff[s]}" for s in STATS)
    if a.adoption:
        corpus, _ = load_corpus(data)
        for i, (n, by_class, by_weapon) in sorted(adoption(data, corpus).items()):
            lo, hi = SL_BRACKETS[i]
            print(f"\nSL {lo}-{hi if hi < 10**6 else '+'}: {n} builds with a melee weapon")
            print("  classes: " + ", ".join(f"{c} {k / n:.1%}" for c, k in by_class.most_common()))
            print("  weapons: " + ", ".join(f"{data.weapons[w]['name']} {k / n:.1%}"
                                            for w, k in by_weapon.most_common(a.top)))
        return 0
    if a.best_weapons:
        known ={i for w in data.weapons.values() for i in (w.get("infusions") or {})}
        typed = a.best_weapons.replace(" ", "_").lower()
        inf = {"none": "No_Infusion", "uninfused": "No_Infusion", "standard": "No_Infusion"}.get(
            typed, next((k for k in known if k.lower() == typed), a.best_weapons))
        if inf not in known:
            ap.error(f"unknown infusion {a.best_weapons!r}: one of {', '.join(sorted(known))}")
        sls = [int(s) for s in a.sweep.split(",")] if a.sweep else [a.sl] if a.sl else []
        if not sls:
            ap.error("--best-weapons needs --sl or --sweep")
        corpus, _ = load_corpus(data)
        jobs = a.jobs or max(1, (os.cpu_count() or 1) // 4)
        what = {"damage": "damage", "ar": "AR"}.get(a.objective, f"{a.objective} build-up")
        # One hit ranks a greataxe's single swing against a dagger's (p5z4.14), and heavy weapons are
        # rarely what players carry; hits landed in a window charge a slow weapon for its swing time.
        # --generate's default window.
        window = a.window or 1.5
        if a.rank != "window" and a.objective != "damage":
            ap.error(f"--rank {a.rank} needs --objective damage")
        if a.rank != "window" and a.with_status:
            ap.error("--with-status ranks by damage_with_status; it does not combine with --rank " + a.rank)
        what = (f"{what} in {window:g}s" if window and a.objective != "ar"
                else f"{what} per hit" if a.objective != "ar" else what)
        what = {"per-stamina": f"{what} per stamina point",
                "bar": "R1 damage from a full stamina bar"}.get(a.rank, what)
        out = {}
        adopted = adoption(data, corpus)
        for sl in sls:
            t = time.monotonic()
            rows = best_weapons(data, corpus, inf, sl, a.objective, a.grip, a.flex_weight, spells, a.start_class,
                                not a.no_floors, defender, a.weapon_class, window, jobs, a.rank)
            if a.with_status:
                rows.sort(key=lambda r: -r[9].get("damage_with_status", r[0]))
            out[sl] = rows
            if a.json:
                continue
            print(f"\n{inf.replace('_', ' ')} at SL {sl}, grip {a.grip}, by {what}: {len(rows)} weapons "
                  f"({time.monotonic() - t:.0f}s)")
            if a.objective == "damage":
                print("  " + defender_line(data, corpus, sl, defender, *defender_defense(data, corpus, sl, defender)))
            # Checked against what real builds of this SL bracket carry as their melee weapon, as
            # er-mods-rs checks its ranking (Spearman rho over every ranked weapon, adopted or not).
            n, _, by_weapon = adopted.get(sl_bracket(sl), [0, Counter(), Counter()])
            share = {r[1]: by_weapon[r[1]] / n if n else 0.0 for r in rows}
            rho = spearman([r[0] for r in rows], [share[r[1]] for r in rows])
            print(f"  vs {n} real builds of this SL bracket: Spearman rho "
                  + (f"{rho:+.3f}" if rho is not None else "n/a") + " (score against melee-weapon share)")
            if any(r[9].get("status_buildup_per_hit") for r in rows):
                print("  columns: score, +status (poison/bleed proc damage per window, row_status), "
                      "hits to proc per status; defenders resist "
                      + ", ".join(f"{s} {np.mean(c):.0%} (median {np.median(c):.0%})" for s in STATUS_PROC
                                  for c in [defender_status_cuts(data, corpus, sl, s, defender)])
                      + "; procs: " + ", ".join(f"{s} {p['damage']} then {p['lockout']:g} s locked"
                                                for s, p in data.status_procs.items()))
            for score, key, val, cls, two, st, worn, label, ammo, m in rows[:a.top]:
                status = (f"  +{m['status_damage_per_window']:4.0f} "
                          + ",".join(f"{s[0]}{n or '-'}" for s, n in m["hits_to_proc"].items()).ljust(7)
                          if m.get("status_buildup_per_hit") else "")
                print(f"  {score:7.1f}{status}  {share[key]:5.1%}  {data.weapons[key]['name']:30} {label:11} {cls:9} "
                      + " ".join(f"{LABEL[s]} {st[s]}" for s in ("strength", "dexterity", "intelligence", "faith",
                                                                    "endurance"))
                      + (f"  rings {', '.join(data.rings[r]['name'] for r in worn)}" if worn else "")
                      + (f"  stamina {m['stamina_per_window']:g}/window {m['damage_per_stamina']:.2f} dmg/st"
                         if "stamina_per_window" in m else "")
                      + (f" bar {m['max_stamina']:g}: {m['bar_attacks']} attacks {m['bar_damage']:.0f}"
                         if "bar_damage" in m else "")
                      + (f"  shot: {ammo['shot']}" if ammo else ""))
        if a.json:
            attacks = load_attacks(data)
            print(json.dumps({sl: [{"score": r[0], "weapon": data.weapons[r[1]]["name"], "key": r[1], "value": r[2],
                                    "class": r[3], "two_handed": r[4], "stats": r[5], "rings": r[6],
                                    "label": r[7], **(r[8] or {}),
                                    "metrics": best_weapon_metrics(data, corpus, attacks, r, inf, sl, defender)}
                                   for r in rows[:a.top]] for sl, rows in out.items()},
                             indent=1))
        return 0
    if a.best_infusion or a.infusion_gaps:
        if not a.stats:
            ap.error("--best-infusion and --infusion-gaps need --stats")
        stats = parse_stats(a.stats)
        sl = a.sl or sum(stats.values()) - 53
        corpus, _ = load_corpus(data)
        what = {"damage": "raw AR" if a.raw_ar else "damage", "ar": "raw AR"}.get(a.objective,
                                                                               f"{a.objective} x hits")
        if a.window and a.objective != "ar":
            what += f" in {a.window:g}s"
        print(f"stats {' '.join(f'{s[:3].upper()} {v}' for s, v in stats.items())}  ->  SL {sl}; by {what}, "
              "full upgrade")
        if what.startswith("damage"):
            print(defender_line(data, corpus, sl, defender, *defender_defense(data, corpus, sl, defender)))
        if a.best_infusion:
            weapon = data.sp_key.get(norm(a.best_infusion))
            if weapon not in data.weapons:
                ap.error(f"unknown weapon {a.best_infusion!r}")
            rows, _, _ = best_infusion(data, weapon, stats, sl, corpus, raw_ar=a.raw_ar, window=a.window,
                                       objective=a.objective, defender=defender)
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
                                   objective=a.objective, one_hand=a.one_hand, weapon_class=a.weapon_class,
                                   defender=defender)
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
        bad = floor_violations(stats, floors, spells) if not a.no_floors else []
        if bad and not a.neighbours:  # never rank for a build that is not a valid one; --neighbours
            # only reports what real builds near these stats carry, so a real build below a floor may query it
            shown = "/".join(LABEL[s] for s in floor_stats(spells))
            print(f"not a valid SL {sl} build: {', '.join(bad)} ({shown} floor at the bracket median; "
                  "use --optimize to get one, or --no-floors to rank these stats anyway)", file=sys.stderr)
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
        rows, dfn, n = weapons_for(data, stats, sl, corpus, raw_ar=a.raw_ar, one_hand=a.one_hand, weapon_class=a.weapon_class, per_class=a.per_class, window=a.window, objective=a.objective, use_floors=not a.no_floors, defender=defender)
        print(f"stats {' '.join(f'{s[:3].upper()} {v}' for s, v in stats.items())}  ->  SL {sl}")
        print(defender_line(data, corpus, sl, defender, dfn, n))
        if a.objective in ("bleed", "poison"):
            print(f"{a.objective}: gauge points before the defender's resistance; a gauge procs at 100")
        what = {"damage": "dmg"}.get(a.objective, a.objective)
        head = "total AR" if (a.raw_ar and a.objective == "damage") or a.objective == "ar" else f"{what}/{a.window:g}s" if a.window else (
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
