#!/usr/bin/env python3
"""Where every armour piece comes from, when a player can have it, and the defender that implies.

    python3 scripts/ds2-armor-accessibility.py sources --grep 'Elite Knight'
    python3 scripts/ds2-armor-accessibility.py table > data/ds2-armor-acquisition.tsv
    python3 scripts/ds2-armor-accessibility.py defender
    python3 scripts/ds2-armor-accessibility.py --selftest

Method, sources and gaps: docs/DS2-ARMOR-ACCESSIBILITY.md. Nothing here changes the recommender's
scoring. `defender` reports what an accessibility-weighted opponent would look like beside the two
opponents the recommender can use now: the posted-build bracket mean (`bracket_defense`) and the
unweighted mean over every piece.

# Read from the game

* `enc_regulation.bnd.dcx` (scripts/ds2-regulation.py): ShopLineupParam (what is sold, at the
  ItemParam price times `priceRate`, and the event flags that open and close the line),
  ItemLotParam2_Other (pickups, chests, NPC gifts, covenant rewards), ItemLotParam2_Chr (what an
  enemy or NPC drops, with each item's weight), EnemyParam (`itemLotId`), ArmorParam
  (requirements, weight), ItemParam.
* Every map's `generatorparam_*` and `generatorregistparam_*` out of GameDataEbl. A generator names
  a GeneratorRegistParam row, the row an `EnemyId`, and EnemyParam that enemy's drop lot, so the
  maps a drop lot occurs in are read off the game's own placement.
* An ItemLotParam2_Other row id starting `AABB` sits in map `mAA_BB` (10105070 -> m10_10). Every
  labelled pickup's Smithbox area agrees with the map this gives; `table` counts any that do not.
* itemname.fmg names, joined to the planner's keys by normalized name.

# Community data, named in the `provenance` column

* Smithbox's DS2S row names (MIT, community-written) label lots and shop lines:
  "[Forest of Fallen Giants - Chest - Soldier's Rest] ...", "[Maughlin the Armourer - Looking
  Glass Knight] ...". They name the area inside a map that holds several, and the condition a shop
  line opens on. Smithbox's MapNames.json names the maps.
* AREA_SL: the soul level a player enters each area at, the lower bound of the ranges in "Dark
  Souls 2 Soul Levels by Zone Guide" (GAMINGREALITY). The three DLC areas are not in it; their
  numbers are set at or above the area that gates them (Fextralife's DLC pages) and are the
  weakest numbers here.
* SHOP_RULES: where each merchant line is first bought and what opens it, from Fextralife's NPC
  pages and the Smithbox labels.

Writes nothing: `table` prints the TSV to stdout.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import re
import struct
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
SMITHBOX = Path.home() / ".local/share/smithbox/app/Assets"
ROW_NAMES = SMITHBOX / "PARAM/DS2S/Param Row Names/English"

SLOTS = ["head", "chest", "hands", "legs"]
LOT_PARAMS = ["ItemLotParam2_Other", "ItemLotParam2_Chr", "ItemLotParam2_SvrEvent"]
DMG = ["physical", "magic", "fire", "lightning", "dark"]
#: Every map with a generator, from Smithbox's DS2S file dictionary (`/param/generatorparam_*`).
MAPS = ["m10_02", "m10_04", "m10_10", "m10_14", "m10_15", "m10_16", "m10_17", "m10_18", "m10_19",
        "m10_23", "m10_25", "m10_27", "m10_29", "m10_30", "m10_31", "m10_32", "m10_33", "m10_34",
        "m20_10", "m20_11", "m20_21", "m20_24", "m20_26", "m40_03", "m50_35", "m50_36", "m50_37",
        "m50_38"]

GAMINGREALITY = "https://www.gamingreality.com/2014/04/dark-souls-2-suggested-levels-by-zone.html"
FEX = "https://darksouls2.wiki.fextralife.com/"

#: Area -> the soul level a player enters it at: the lower bound of GAMINGREALITY's range.
AREA_SL = {
    "Things Betwixt": 1, "Majula": 1,
    "Forest of Fallen Giants": 10,
    "Heide's Tower of Flame": 30, "Cathedral of Blue": 30,
    "No-man's Wharf": 35,
    "The Pit": 45, "Grave of Saints": 45,
    "Lost Bastille": 55, "Belfry Luna": 55, "Huntsman's Copse": 55, "Undead Purgatory": 55,
    "Harvest Valley": 60,
    "Earthen Peak": 65, "The Gutter": 65, "Black Gulch": 65,
    "Iron Keep": 80, "Belfry Sol": 80, "Shaded Woods": 80, "Doors of Pharros": 80,
    "Brightstone Cove": 90,
    "Sinners' Rise": 95, "Shrine of Winter": 95, "Drangleic Castle": 95, "King's Passage": 95,
    "Shrine of Amana": 95, "Dark Chasm of Old": 95,
    "Undead Crypt": 100, "Aldia's Keep": 100,
    "Dragon Aerie": 105, "Dragon Shrine": 105,
    "Memories": 110, "Throne of Want": 110,
    # DLC, entered from Black Gulch (Shulva: Dragon Talon behind the Forgotten Key), Iron Keep
    # (Brume Tower: Heavy Iron Key, Forest of Fallen Giants) and the Shrine of Winter (Eleum Loyce:
    # Frozen Flower). Not in GAMINGREALITY: set at or above the gating area's own number.
    "Shulva": 80, "Brume Tower": 90, "Eleum Loyce": 100,
}
DLC_AREAS = {"Shulva", "Brume Tower", "Eleum Loyce"}
#: Map -> its earliest area (Smithbox MapNames, which names several maps after two areas).
MAP_AREA = {
    "m10_02": "Things Betwixt", "m10_04": "Majula", "m10_10": "Forest of Fallen Giants",
    "m10_14": "Brightstone Cove", "m10_15": "Aldia's Keep", "m10_16": "Lost Bastille",
    "m10_17": "Harvest Valley", "m10_18": "No-man's Wharf", "m10_19": "Iron Keep",
    "m10_23": "Huntsman's Copse", "m10_25": "The Gutter", "m10_27": "Dragon Aerie",
    "m10_29": "Shaded Woods", "m10_30": "Heide's Tower of Flame", "m10_31": "Heide's Tower of Flame",
    "m10_32": "Shaded Woods", "m10_33": "Doors of Pharros", "m10_34": "Grave of Saints",
    "m20_10": "Memories", "m20_11": "Shrine of Amana", "m20_21": "Drangleic Castle",
    "m20_24": "Undead Crypt", "m20_26": "Memories", "m40_03": "Dark Chasm of Old",
    "m50_35": "Shulva", "m50_36": "Brume Tower", "m50_37": "Eleum Loyce", "m50_38": "Eleum Loyce",
}
#: Smithbox's label spellings, normalized, -> AREA_SL keys.
AREA_ALIAS = {
    "thelostbastille": "Lost Bastille", "brightstonecovetseldora": "Brightstone Cove",
    "frozeneleumloyce": "Eleum Loyce", "shulvasanctumcity": "Shulva",
    "sinnersrise": "Sinners' Rise", "giantsmemory": "Memories", "memoryofjeigh": "Memories",
    "memoryoforro": "Memories", "memoryofvammar": "Memories", "memoryoftheking": "Eleum Loyce",
    "memoryoftheoldironking": "Brume Tower", "dragonmemories": "Memories",
    "unseenpathtoheide": "Heide's Tower of Flame", "heidestower": "Heide's Tower of Flame",
}
#: NG+ starts after the Throne of Want, whose GAMINGREALITY range is 110-120.
NG_PLUS_SL = 120
#: A cumulative 15000 souls spent at Maughlin. No source gives a level; an assumption, said so.
MAUGHLIN_15000_SL = 30

#: Shop label (Smithbox's text inside the brackets) -> (area or "@npc", soul level or None for the
#: area's, kind, provenance). "@npc" means the area the named NPC is first placed in (npc_area).
SHOP_RULES = [
    (r"^Maughlin the Armourer$", "Majula", None, "shop", FEX + "Maughlin+the+Armourer"),
    (r"^Maughlin the Armourer - 1000 Souls$", "Majula", None, "shop", FEX + "Maughlin+the+Armourer"),
    (r"^Maughlin the Armourer - 15000 Souls$", "Majula", MAUGHLIN_15000_SL, "shop",
     FEX + "Maughlin+the+Armourer; SL assumed"),
    (r"^Maughlin.*NG\+", "Majula", NG_PLUS_SL, "ng+", "smithbox label"),
    (r"^Magerold.*NG\+", "Iron Keep", NG_PLUS_SL, "ng+", "smithbox label"),
    (r"Smelter Demon", "Iron Keep", None, "boss-soul", "smithbox label"),
    (r"Looking Glass Knight", "Drangleic Castle", None, "boss-soul", "smithbox label"),
    (r"Lost Sinner", "Sinners' Rise", None, "boss-soul", "smithbox label"),
    (r"Velstadt", "Undead Crypt", None, "boss-soul", "smithbox label"),
    (r"Thorne Defenders", "Throne of Want", None, "boss-soul", "smithbox label"),
    (r"Burnt Ivory King", "Eleum Loyce", None, "boss-soul", "smithbox label"),
    (r"Fume Knight", "Brume Tower", None, "boss-soul", "smithbox label"),
    (r"Sir Alonne", "Brume Tower", None, "boss-soul", "smithbox label"),
    (r"^Titchy Gren - Skeleton Lords$", "Huntsman's Copse", None, "boss-soul", "smithbox label"),
    (r"^Titchy Gren - Shrine of Winter$", "Shrine of Winter", None, "shop", "smithbox label"),
    (r"^Merchant Hag Melentia$", "Forest of Fallen Giants", None, "shop",
     FEX + "Merchant+Hag+Melentia"),
    # Melentia sells an NPC's set once that NPC is dead (Fextralife): killed where first met.
    (r"^Merchant Hag Melentia - (.+)$", "@npc", None, "npc", FEX + "Merchant+Hag+Melentia"),
    (r"^Laddersmith Gilligan", "Earthen Peak", None, "shop", FEX + "Laddersmith+Gilligan"),
    (r"^Magerold of Lanafir$", "Iron Keep", None, "shop", FEX + "Magerold+of+Lanafir"),
    (r"^Royal Sorcerer Navlaan$", "Aldia's Keep", None, "shop", FEX + "Royal+Sorcerer+Navlaan"),
    (r"^Cromwell the Pardoner$", "Brightstone Cove", None, "shop", FEX + "Cromwell+the+Pardoner"),
    (r"^Straid of Olaphis - \d Forlorn$", "Lost Bastille", None, "npc", "smithbox label"),
    (r"Royal Rat Authority", "Doors of Pharros", None, "boss-soul", "smithbox label"),
]
#: Other-lot labels that are not an area. A gift at the end of a quest is placed at the NPC's last
#: map (npc_area(last=True)); any other NPC lot is a kill drop, placed where the NPC is first met.
OTHER_RULES = [
    (r"^Covenant - Bell Keepers", "Belfry Luna", None, "covenant"),
    (r"^Covenant - Pilgrims of Dark", "Dark Chasm of Old", None, "covenant"),
    (r"^NPC - Alsanna", "Eleum Loyce", None, "npc"),
    (r"^NPC - Chancellor Wellager", "Drangleic Castle", None, "npc"),
    (r"^NPC - Head of Vengarl", "Shaded Woods", None, "npc"),
    # Maughlin's gift (the "(Transparent)" Aurous set): talk to him with no souls once 15,000 are
    # spent at his shop (Fextralife, Maughlin the Armourer).
    (r"^NPC - Maughlin the Armourer$", "Majula", MAUGHLIN_15000_SL, "npc"),
    (r"^Event - Licia", "@npc:Licia of Lindeldt", None, "npc"),
    (r"^NPC - (.+?)( - (Gift|Survives|All flames).*)$", "@last", None, "npc"),
    (r"^NPC - (.+)$", "@npc", None, "npc"),
]
#: The `[Event]` lots that hand out the three crowns sit in no map; each DLC's crown is that DLC's.
CROWN_AREA = {"Crown of the Old Iron King": "Brume Tower", "Crown of the Sunken King": "Shulva",
              "Crown of the Ivory King": "Eleum Loyce"}
#: Pieces no lot or shop line hands out, from Fextralife. The Black Dragon set's only lots
#: (the Dragon Aerie Black Dragon Knights') give it weight 0; Fextralife: Dragon Remnants rank 1
#: (helm) and rank 2 (the rest), a covenant Magerold of Lanafir leads in Iron Keep.
COMMUNITY = {
    21330100: ("covenant", "Iron Keep", "Dragon Remnants rank 1", FEX + "Black+Dragon+Set"),
    21330101: ("covenant", "Iron Keep", "Dragon Remnants rank 2", FEX + "Black+Dragon+Set"),
    21330102: ("covenant", "Iron Keep", "Dragon Remnants rank 2", FEX + "Black+Dragon+Set"),
    21330103: ("covenant", "Iron Keep", "Dragon Remnants rank 2", FEX + "Black+Dragon+Set"),
}
#: How much a source of each kind counts for, against one anybody can buy or pick up. A drop is a
#: per-kill chance from a respawn-limited enemy, an NPC's set needs the NPC dead or their quest
#: done, a covenant reward needs rank 3; each is taken to reach half the players a pickup does.
#: Not measured: the doc says so and says how it could be.
KIND_WEIGHT = {"pickup": 1.0, "shop": 1.0, "boss-soul": 1.0, "drop": 0.5, "npc": 0.5,
               "covenant": 0.5, "ng+": 0.5}
#: The logistic's width in soul levels: GAMINGREALITY's ranges are ten levels wide.
TAU = 10.0
TARGET_SL = [30, 60, 100, 150, 200, 250]
#: Raw beats a 50/50 split infusion against elemental defense above about this (from the infusion
#: analysis this feeds; quoted, not derived here).
RAW_SPLIT_DEF = 366


def load_module(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def norm(s: str) -> str:
    return re.sub(r"[^a-z0-9]", "", s.lower())


def area_of(label_area: str) -> str | None:
    """A Smithbox label's area, as an AREA_SL key, or None when the text is not an area."""
    n = norm(label_area)
    if n in AREA_ALIAS:
        return AREA_ALIAS[n]
    for a in AREA_SL:
        if norm(a) == n:
            return a
    return None


def lot_map(lot_id: int) -> str | None:
    """`AABBxxxx` -> `mAA_BB` for an 8-digit ItemLotParam2_Other id in a known map."""
    s = str(lot_id)
    m = f"m{s[:2]}_{s[2:4]}" if len(s) == 8 else None
    return m if m in MAP_AREA else None


def chr_lot_map(lot_id: int) -> str | None:
    """`ABBxxxx` -> `mA0_BB` for a 7-digit ItemLotParam2_Chr id. These lots are not any EnemyParam's
    `itemLotId`, so no generator places them; they are per-map drops (1153000 is the Aldia's Keep
    mimic, 2210700 the Drangleic Castle Ruin Sentinel). `table` counts how many Smithbox labels
    that name an area agree with the map this gives."""
    s = str(lot_id)
    if len(s) != 7:
        return None
    # The DLC maps keep both digits of `50`: 5037200 is the Frozen Eleum Loyce mimic.
    m = f"m50_{s[2:4]}" if s[0] == "5" else f"m{s[0]}0_{s[1:3]}"
    return m if m in MAP_AREA else None


def sigmoid(x: float) -> float:
    return 1.0 / (1.0 + math.exp(-x))


def availability(sl: float, avail_sl: float, kind: str) -> float:
    """The share of players at `sl` taken to own a piece first had at `avail_sl` from `kind`."""
    return KIND_WEIGHT[kind] * sigmoid((sl - avail_sl) / TAU)


class Game:
    """The decoded params this needs, from the regulation and from GameDataEbl."""

    def __init__(self):
        self.ex = ex = load_module("ds2attacks", "ds2-attacks-extract.py")
        self.reg = reg = load_module("ds2regulation", "ds2-regulation.py")
        self.p = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
            "ShopLineupParam": "SHOP_LINEUP_PARAM", "ItemLotParam2_Other": "ITEM_LOT_PARAM2",
            "ItemLotParam2_Chr": "ITEM_LOT_PARAM2", "ItemLotParam2_SvrEvent": "ITEM_LOT_PARAM2",
            "ArmorParam": "ARMOR_PARAM", "ItemParam": "ITEM_PARAM", "EnemyParam": "CHR_PARAM"})
        self.names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
        self.armor = sorted(int(k) for k, r in self.p["ItemParam"].items()
                            if r["armorParamId"] > 0 and self.names[k] != "<no name in fmg>")
        self._ebl = None

    def ebl(self, path: str) -> bytes:
        """One GameDataEbl entry, as `ds2-attacks-extract.py`'s `ebl_read`, with the archive header
        decrypted once instead of once per entry."""
        if self._ebl is None:
            ebl = self.ex.load_module("ds2ebl", "ds2-ebl.py")
            bhd, bdt, pem = ebl.archive_paths(self.reg.GAME_DIR, ebl.DEFAULT_ARCHIVE)
            self._ebl = (ebl, ebl.Bhd5(ebl.decrypt_bhd(bhd, pem)), bdt)
        ebl, header, bdt = self._ebl
        size, offset, aes_key, _ = header.lookup(path)
        with bdt.open("rb") as handle:
            handle.seek(offset)
            data = bytearray(handle.read(size))
        if aes_key:
            key = header.blob[aes_key: aes_key + 16]
            count = struct.unpack_from("<i", header.blob, aes_key + 16)[0]
            for i in range(count):
                start, end = struct.unpack_from("<qq", header.blob, aes_key + 20 + 16 * i)
                if start == -1 or end <= start:
                    continue
                data[start:end] = subprocess.run(
                    ["openssl", "enc", "-d", "-aes-128-ecb", "-nopad", "-K", key.hex()],
                    input=bytes(data[start:end]), capture_output=True, check=True).stdout
        return ebl.dcx_decompress(bytes(data))

    def decode(self, name: str, blob: bytes, def_name: str) -> dict:
        """`{row id: {field: value}}`, refusing a param whose stride is not the paramdef's size."""
        size, fields = self.ex.layout(self.ex.DEFAULT_DEFS, def_name)
        p = self.reg.Param(name, blob)
        if p.stride != size:
            raise SystemExit(f"{name}: stride {p.stride} != paramdef {def_name} size {size}")
        rows = {}
        for k, rid in enumerate(p.ids):
            b, r = p.row(k), {}
            for nm, ty, off, bit_pos, bits in fields:
                v = struct.unpack_from("<" + self.ex.FORMATS[ty], b, off)[0]
                r[nm] = (v >> bit_pos) & ((1 << bits) - 1) if bits else v
            rows[rid] = r
        return rows

    def placements(self) -> dict[int, Counter]:
        """Chr item lot id -> {map: generators spawning an enemy that drops it}."""
        out: dict[int, Counter] = defaultdict(Counter)
        enemy = self.p["EnemyParam"]
        for m in MAPS:
            gen = self.decode(m, self.ebl(f"/param/generatorparam_{m}_00_00.param"), "GENERATOR_PARAM")
            regist = self.decode(m, self.ebl(f"/param/generatorregistparam_{m}_00_00.param"),
                                 "GENERATOR_REGIST_PARAM")
            for g in gen.values():
                r = regist.get(g["GeneratorRegistParam"])
                e = enemy.get(str(r["EnemyId"])) if r else None
                if e and e["itemLotId"]:
                    out[e["itemLotId"]][m] += 1
        return out


def row_names(param: str) -> dict[int, str]:
    f = ROW_NAMES / f"{param}.json"
    if not f.exists():
        return {}
    return {e["ID"]: (e["Entries"] or [""])[0] for e in json.loads(f.read_text())["Entries"]}


def label_parts(label: str) -> tuple[str, str]:
    """`"[A - B] items"` -> ("A - B", "items")."""
    m = re.match(r"\s*\[([^\]]*)\]\s*(.*)", label or "")
    return (m.group(1), m.group(2)) if m else ("", label or "")


def lot_items(r: dict) -> list[tuple[int, float]]:
    """`[(item id, share of the lot's weight)]` for the slots that hand something out."""
    total = sum(r[f"itemWeight{j}"] for j in range(1, 11) if r[f"itemNum{j}"] > 0)
    return [(r[f"itemId{j}"], r[f"itemWeight{j}"] / total) for j in range(1, 11)
            if r[f"itemNum{j}"] > 0 and r[f"itemWeight{j}"] > 0 and total > 0]


class Source:
    __slots__ = ("kind", "area", "sl", "what", "prov")

    def __init__(self, kind, area, sl, what, prov):
        self.kind, self.area, self.sl, self.what, self.prov = kind, area, sl, what, prov

    def text(self) -> str:
        return f"{self.kind}@{self.area}({self.sl}): {self.what}"


def collect(game: Game) -> tuple[dict[int, list[Source]], dict]:
    """Every armour piece's sources, and counts of what could not be placed."""
    armor = set(game.armor)
    place = game.placements()
    rn = {p: row_names(p) for p in LOT_PARAMS + ["ShopLineupParam"]}
    notes = Counter()

    # NPC name -> maps their own drop lot is placed in (first met = lowest AREA_SL, last = highest).
    npc_maps: dict[str, Counter] = defaultdict(Counter)
    for lot, maps in place.items():
        head, _ = label_parts(rn["ItemLotParam2_Chr"].get(lot, ""))
        npc_maps[norm(head.split(" - ")[0])].update(maps)

    def npc_area(name: str, last: bool = False) -> str | None:
        maps = npc_maps.get(norm(name))
        if not maps:
            return None
        areas = sorted({MAP_AREA[m] for m in maps}, key=lambda a: AREA_SL[a])
        if not last and len(areas) > 1 and areas[0] in ("Majula", "Things Betwixt"):
            areas = areas[1:]  # NPCs move to Majula; that is not where they are met
        return areas[-1] if last else areas[0]

    out: dict[int, list[Source]] = defaultdict(list)
    items = game.p["ItemParam"]
    for k, r in game.p["ShopLineupParam"].items():
        item = r["saleItem"]
        if item not in armor:
            continue
        head, _ = label_parts(rn["ShopLineupParam"].get(int(k), ""))
        price = round(items[str(item)]["price"] * r["priceRate"])
        for pat, area, sl, kind, prov in SHOP_RULES:
            m = re.search(pat, head)
            if not m:
                continue
            if area == "@npc":
                area = npc_area(m.group(1))
            if area is None and sl is None:
                notes["shop line with no area"] += 1
                break
            sl = sl if sl is not None else AREA_SL[area]
            out[item].append(Source(kind, area or "-", sl, f"{head}, {price} souls (shop {k})",
                                    f"game:ShopLineupParam {k} price; {prov}"))
            break
        else:
            notes[f"shop label with no rule: {head}"] += 1

    for p in LOT_PARAMS:
        for k, r in game.p[p].items():
            lot = int(k)
            label = rn[p].get(lot, "")
            head, _ = label_parts(label)
            for item, share in lot_items(r):
                if item not in armor:
                    continue
                if p == "ItemLotParam2_Chr":
                    maps = place.get(lot)
                    prov = "game:EnemyParam+generators; smithbox label"
                    if not maps and chr_lot_map(lot):
                        maps = {chr_lot_map(lot): 1}
                        prov = "game:Chr lot id's map digits; smithbox label"
                        named = area_of(head.split(" - ")[-1]) if " - " in head else None
                        first = MAP_AREA[chr_lot_map(lot)]
                        # A map's later areas (Earthen Peak in Harvest Valley's) open no earlier.
                        if named and (named == first or AREA_SL[named] >= AREA_SL[first]):
                            notes["7-digit drop lot: label area agrees with the id's map"] += 1
                        elif named:
                            notes[f"7-digit drop lot: label area disagrees: {lot} {head}"] += 1
                    if not maps:
                        notes["drop lot (armour slot) placed nowhere"] += 1
                        continue
                    area = min((MAP_AREA[m] for m in maps), key=lambda a: AREA_SL[a])
                    kind = "npc" if norm(head) in {norm(n) for n in NPC_KILL_NAMES} else "drop"
                    where = ",".join(sorted(maps))
                    out[item].append(Source(kind, area, AREA_SL[area],
                                            f"{head or 'unlabelled'} drop {share:.0%} ({where}, lot {lot})",
                                            prov))
                    continue
                kind, area, sl = "pickup", None, None
                ng = "NG+" in head
                crown = CROWN_AREA.get(game.names[str(item)]) if head == "Event" else None
                for pat, a, s, kd in [] if crown else OTHER_RULES:
                    m = re.search(pat, head)
                    if not m:
                        continue
                    kind, sl = kd, s
                    if a == "@npc":
                        area = npc_area(m.group(1))
                    elif a.startswith("@npc:"):
                        area = npc_area(a[5:])
                    elif a == "@last":
                        area = npc_area(m.group(1), last=True)
                    else:
                        area = a
                    break
                else:
                    area = crown or (area_of(head.split(" - ")[0]) if head else None)
                    mp = lot_map(lot)
                    if mp and area is None:
                        area = MAP_AREA[mp]
                    if mp and area and MAP_AREA[mp] != area and AREA_SL[area] < AREA_SL[MAP_AREA[mp]]:
                        notes["label area earlier than its map's first area"] += 1
                if area is None:
                    notes[f"lot with no area: {p} {lot} {head}"] += 1
                    continue
                sl = sl if sl is not None else AREA_SL[area]
                if ng:
                    kind, sl = "ng+", max(sl, NG_PLUS_SL)
                out[item].append(Source(kind, area, sl, f"{head or 'unlabelled'} (lot {lot})",
                                        f"game:{p} {lot}; smithbox label"))
    for item, (kind, area, what, prov) in COMMUNITY.items():
        if not out.get(item):
            out[item].append(Source(kind, area, AREA_SL[area], what, prov))
    return out, notes


#: Chr lots whose label is a named NPC: killing them is an NPC source, not a farmable drop.
NPC_KILL_NAMES = [
    "Mild Mannered Pate", "Creighton the Wanderer", "Benhart of Jugo", "Cale the Cartographer",
    "Lucatiel of Mirrah", "Grave Warden Agdayne", "Straid of Olaphis", "Licia of Lindeldt",
    "Felkin the Outcast", "Royal Sorcerer Navlaan", "Titchy Gren", "Blue Sentinel Targray",
    "Captain Drummond", "Vengarl", "Maughlin the Armourer", "Magerold of Lanafir",
    "Cromwell the Pardoner", "Aslatiel of Mirrah", "Bowman Guthry", "Fencer Sharron",
    "Rhoy the Explorer", "Dragonfang Villard", "Oliver the Collector", "Melinda the Butcher",
    "Merciless Roenna", "Vorgel the Sinner", "Maldron the Assassin", "Peculiar Kindalur",
    "Body of Vengarl",
]

KIND_ORDER = ["pickup", "shop", "boss-soul", "drop", "npc", "covenant", "ng+"]


def best(sources: list[Source]) -> Source | None:
    """The source the table names: the earliest one of full weight (pickup, shop, boss soul) when
    there is one, else the earliest of any kind. The weighting does not use this; it takes the
    largest availability over all of a piece's sources (piece_availability)."""
    full = [s for s in sources if KIND_WEIGHT[s.kind] >= 1]
    pool = full or sources
    return min(pool, key=lambda s: (s.sl, KIND_ORDER.index(s.kind))) if pool else None


def piece_availability(sl: float, sources: list[Source]) -> float:
    return max((availability(sl, s.sl, s.kind) for s in sources), default=0.0)


def tier_of(sl: int) -> int:
    """Ordinal of `sl` among AREA_SL's distinct values (0 = start), NG+ last."""
    ladder = sorted(set(AREA_SL.values()) | {NG_PLUS_SL})
    return ladder.index(sl) if sl in ladder else sum(1 for v in ladder if v < sl)


# --------------------------------------------------------------------------------------------
# the planner side: keys, corpus, the defense arithmetic


def recommender():
    rec = load_module("ds2buildsrecommend", "ds2-builds-recommend.py")
    sp_json, mm_json = rec.dump_site_tables(rec.CACHE / "site-tables")
    data = rec.Data(json.loads(sp_json.read_text()), json.loads(mm_json.read_text()))
    print(rec.apply_regulation(data)[:120] + " ...", file=sys.stderr)
    return rec, data


def key_join(data, game: Game) -> tuple[dict[tuple[str, str], int], list[str]]:
    """(slot, planner key) -> game item id, by normalized name with the slot from the id's last
    digit, then the name with a plural `s` (the planner's `Wanderer Manchette`). Where two game ids
    share a name (the Aurous set: 21360101 and 21361101 are both "Armor of Aurous") the one whose
    ArmorParam weight is nearest the planner's is taken, which is how the planner's
    "(Transparent)" pieces (1.2 against 3.8 for the helm) land on the Maughlin gift set.
    Unmatched planner pieces are returned, not guessed."""
    by_name: dict[tuple[str, str], list[int]] = defaultdict(list)
    for i in game.armor:
        slot = SLOTS[i % 10] if i % 10 < 4 else None
        if slot:
            by_name[(slot, norm(game.names[str(i)]))].append(i)

    def weight(i):
        return game.p["ArmorParam"][str(game.p["ItemParam"][str(i)]["armorParamId"])]["weight"]

    out, miss = {}, []
    for slot in SLOTS:
        for k, v in data.armor[slot].items():
            if k == "Naked":
                continue
            name = norm(re.sub(r"\(Transparent\)", "", v.get("name", k)))
            cands = by_name.get((slot, name)) or by_name.get((slot, name + "s")) or by_name.get((slot, norm(k)))
            if not cands:
                miss.append(f"{slot}:{k}")
                continue
            out[(slot, k)] = min(cands, key=lambda i: abs(weight(i) - v.get("weight", 0)))
    return out, miss


def corpus_sl(rec, data, corpus) -> dict[tuple[str, str], list[int]]:
    out = defaultdict(list)
    for b in corpus:
        sl = rec.soul_level(data, b)
        for slot, piece in zip(SLOTS, b.armor):
            out[(slot, piece)].append(sl)
    return out


def bracket_builds(rec, data, corpus, sl: int) -> list:
    """The corpus builds `bracket_defense` would average at `sl` (its bracket or the nearest with 20)."""
    by = defaultdict(list)
    for b in corpus:
        by[rec.sl_bracket(rec.soul_level(data, b))].append(b)
    i = rec.sl_bracket(sl)
    for j in sorted(range(len(rec.SL_BRACKETS)), key=lambda j: (abs(j - i), j)):
        if len(by.get(j, [])) >= 20:
            return by[j]
    return []


def wearable(rec, data, builds, slot: str, piece: str) -> float:
    """Share of `builds` that meet the piece's requirements and could put it on in place of what
    they wear in that slot without passing EQUIP_CAP (the recommender's 70%), counting the rings
    and weapons each carries."""
    if not builds:
        return 0.0
    row = data.armor[slot][piece]
    req = row.get("require") or {}
    w = row.get("weight", 0)
    ok = 0
    for b in builds:
        st, room = b._acc
        if all(v <= st.get(s, 0) for s, v in req.items()) and w <= room[slot]:
            ok += 1
    return ok / len(builds)


def prepare(rec, data, builds) -> None:
    """Cache per build: stats with rings, and the load room per slot (cap less everything else)."""
    for b in builds:
        if hasattr(b, "_acc"):
            continue
        rings = [r for r in b.rings if r in data.rings]
        eff = rec.gear_stats(data, rec.effective(data, b), rings)
        try:
            cap = rec.max_load(data, rec.effective(data, b), rings) * rec.EQUIP_CAP
        except TypeError:  # VIT 0: the planner's load table has no row 0, so no load to spend
            cap = 0.0
        carried = sum(data.weapons.get(w, {}).get("weight", 0) for w, _ in b.weapons())
        carried += sum(data.rings.get(r, {}).get("weight", 0) for r in rings)
        worn = {s: data.armor[s].get(p, {}).get("weight", 0) for s, p in zip(SLOTS, b.armor)}
        total = sum(worn.values())
        b._acc = (eff, {s: cap - carried - (total - worn[s]) for s in SLOTS})


def weighted_defense(rec, data, stats: dict, weights: dict[str, dict[str, float]]) -> dict:
    """build_defense's arithmetic with each slot an average over pieces under `weights`: the
    defense is linear in each piece's fields, so the expected set is the per-slot expectation."""
    pb = rec._tab(data, "physicalDEFBonus", stats["endurance"] + stats["vitality"] + stats["strength"]
                  + stats["dexterity"])
    out = {k: 0.0 for k in DMG}
    for slot in SLOTS:
        ws = weights[slot]
        tot = sum(ws.values())
        if tot <= 0:
            continue
        for piece, w in ws.items():
            p = data.armor[slot][piece]
            f = w / tot
            out["physical"] += f * (p.get("physicalDEF", 0) + p.get("physicalDEFBonus", 0) * pb)
            for k in DMG[1:]:
                out[k] += f * p.get(k + "DEF", 0)
    base = {"magic": ("magicDEFBonus", stats["intelligence"]),
            "fire": ("fireDEFBonus", stats["intelligence"] + stats["faith"]),
            "lightning": ("lightningDEFBonus", stats["faith"]),
            "dark": ("darkDEFBonus", min(stats["intelligence"], stats["faith"]))}
    for k, (tab, i) in base.items():
        out[k] += rec._tab(data, tab, i)
    return out


# --------------------------------------------------------------------------------------------
# commands


def sources_cmd(grep: str | None) -> None:
    game = Game()
    srcs, notes = collect(game)
    for i in game.armor:
        n = game.names[str(i)]
        if grep and not re.search(grep, n, re.I):
            continue
        b = best(srcs[i])
        print(f"{i} {n}  -> {b.text() if b else 'NO SOURCE'}")
        for s in sorted(srcs[i], key=lambda s: s.sl):
            print("    " + s.text())
    for k, v in notes.most_common():
        print(f"# {v:4d} {k}", file=sys.stderr)


HEADER = ["key", "slot", "item_id", "name", "area", "tier", "avail_sl", "kind", "source",
          "dlc", "ng_plus_only", "req_str", "req_dex", "req_int", "req_fth", "weight",
          "corpus_n", "corpus_sl_p05", "corpus_sl_p10", "corpus_sl_median", "other_sources",
          "provenance"]


def table_rows(game, srcs, data, joined, sls):
    import numpy as np
    rows = []
    for (slot, key), i in sorted(joined.items(), key=lambda t: (SLOTS.index(t[0][0]), t[0][1])):
        s = srcs.get(i, [])
        b = best(s)
        ap = game.p["ArmorParam"].get(str(game.p["ItemParam"][str(i)]["armorParamId"]), {})
        v = sls.get((slot, key), [])
        pct = [int(np.percentile(v, q)) if v else "" for q in (5, 10, 50)]
        others = "; ".join(x.text() for x in sorted(s, key=lambda x: x.sl) if x is not b)
        rows.append({
            "key": key, "slot": slot, "item_id": i, "name": game.names[str(i)],
            "area": b.area if b else "", "tier": tier_of(b.sl) if b else "",
            "avail_sl": b.sl if b else "", "kind": b.kind if b else "",
            "source": b.what if b else "", "dlc": int(bool(b and b.area in DLC_AREAS)),
            "ng_plus_only": int(bool(s) and all(x.kind == "ng+" for x in s)),
            "req_str": ap.get("prerequisiteStrength", ""), "req_dex": ap.get("prerequisiteDexterity", ""),
            "req_int": ap.get("prerequisiteIntelligence", ""), "req_fth": ap.get("prerequisiteFaith", ""),
            "weight": round(ap["weight"], 2) if ap else "", "corpus_n": len(v),
            "corpus_sl_p05": pct[0], "corpus_sl_p10": pct[1], "corpus_sl_median": pct[2],
            "other_sources": others,
            "provenance": (b.prov + "; area SL " + GAMINGREALITY) if b else "no source found"})
    return rows


def table_cmd() -> None:
    game = Game()
    srcs, notes = collect(game)
    rec, data = recommender()
    joined, miss = key_join(data, game)
    corpus, _ = rec.load_corpus(data)
    rows = table_rows(game, srcs, data, joined, corpus_sl(rec, data, corpus))
    print("#" + "\t".join(HEADER))
    for r in rows:
        print("\t".join(str(r[h]).replace("\t", " ") for h in HEADER))
    print(f"# {len(rows)} planner pieces joined to a game id; unjoined: {', '.join(miss) or 'none'}",
          file=sys.stderr)
    print(f"# with a source: {sum(1 for r in rows if r['kind'])}; by kind: "
          f"{dict(Counter(r['kind'] for r in rows))}", file=sys.stderr)
    for k, v in notes.most_common():
        print(f"# {v:4d} {k}", file=sys.stderr)


def defender_cmd() -> None:
    import numpy as np
    game = Game()
    srcs, _ = collect(game)
    rec, data = recommender()
    joined, _ = key_join(data, game)
    corpus, _ = rec.load_corpus(data)
    avail = {k: srcs[i] for k, i in joined.items() if srcs.get(i)}
    sls = corpus_sl(rec, data, corpus)
    print("SL   opponent            phys   magic  fire   light  dark   (n builds)")
    for sl in TARGET_SL:
        builds = bracket_builds(rec, data, corpus, sl)
        prepare(rec, data, builds)
        stats, n = rec.bracket_stats(data, corpus, sl)
        posted, _ = rec.bracket_defense(data, corpus, sl)
        flat = {s: {k: 1.0 for k in data.armor[s] if k != "Naked"} for s in SLOTS}
        acc = {s: {} for s in SLOTS}
        for (slot, key), src in avail.items():
            w = piece_availability(sl, src) * wearable(rec, data, builds, slot, key)
            if w > 0:
                acc[slot][key] = w
        # The empirical check: weight each piece by how often bracket builds wear it.
        worn = {s: Counter() for s in SLOTS}
        for bb in builds:
            for s, p in zip(SLOTS, bb.armor):
                if p != "Naked":
                    worn[s][p] += 1
        rows = [("posted builds (now)", posted), ("all pieces, unweighted", weighted_defense(rec, data, stats, flat)),
                ("accessibility", weighted_defense(rec, data, stats, acc)),
                ("posted pieces, median stats", weighted_defense(rec, data, stats, worn))]
        for label, d in rows:
            print(f"{sl:<4} {label:<29}" + " ".join(f"{d[k]:6.0f}" for k in DMG) + f"  ({n})")
        top = max(rows[2][1][k] for k in DMG[1:])
        print(f"     highest weighted elemental {top:.0f} vs {RAW_SPLIT_DEF}: "
              f"{'at or over' if top >= RAW_SPLIT_DEF else f'{RAW_SPLIT_DEF - top:.0f} short'}")
    first = [(np.percentile(v, 10), best(src).sl, k) for k, src in avail.items() if len(v := sls.get(k, [])) >= 20]
    if first:
        r = np.corrcoef([a for a, _, _ in first], [b for _, b, _ in first])[0, 1]
        print(f"\npieces worn by >= 20 corpus builds: {len(first)}; correlation of their 10th-percentile "
              f"SL with availability SL: {r:.2f}")
        early = sorted(first, key=lambda t: t[0] - t[1])[:8]
        print("worn earliest relative to availability (p10 SL, avail SL, piece):")
        for p10, a, k in early:
            print(f"    {p10:5.0f} {a:4d}  {k[0]}:{k[1]}")


def selftest() -> int:
    checks = [
        ("lot id -> map", lot_map(10105070), "m10_10"),
        ("short lot id has no map", lot_map(1744010), None),
        ("label area alias", area_of("The Lost Bastille"), "Lost Bastille"),
        ("label area exact", area_of("Forest of Fallen Giants"), "Forest of Fallen Giants"),
        ("label not an area", area_of("NPC"), None),
        ("label split", label_parts("[A - B] c, d"), ("A - B", "c, d")),
        ("availability at its own SL is half the kind weight", availability(55, 55, "shop"), 0.5),
        ("a drop counts half a pickup", availability(1000, 0, "drop"), 0.5),
        ("tier of the start", tier_of(1), 0),
        ("NG+ is the last tier", tier_of(NG_PLUS_SL), len(set(AREA_SL.values()) | {NG_PLUS_SL}) - 1),
        ("a shop beats an earlier drop that only reaches half", best([
            Source("drop", "a", 10, "", ""), Source("shop", "b", 15, "", "")]).kind, "shop"),
        ("the table names a late shop over an early drop", best([
            Source("drop", "a", 10, "", ""), Source("shop", "b", 95, "", "")]).kind, "shop"),
        ("but the weight at SL 40 is the early drop's", piece_availability(40, [
            Source("drop", "a", 10, "", ""), Source("shop", "b", 95, "", "")]),
         availability(40, 10, "drop")),
    ]
    row = {f"itemNum{j}": 0 for j in range(1, 11)} | {f"itemWeight{j}": 0.0 for j in range(1, 11)} \
        | {f"itemId{j}": 10 for j in range(1, 11)}
    row.update(itemNum1=1, itemWeight1=1.0, itemId1=5, itemNum2=1, itemWeight2=3.0, itemId2=6,
               itemNum3=0, itemWeight3=9.0, itemId3=7)
    checks.append(("lot shares skip empty slots", lot_items(row), [(5, 0.25), (6, 0.75)]))
    bad = 0
    for name, got, want in checks:
        ok = abs(got - want) < 1e-9 if isinstance(want, float) else got == want
        bad += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {name}: {got!r}" + ("" if ok else f" (want {want!r})"))
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--selftest", action="store_true", help="offline checks; no game files needed")
    sub = ap.add_subparsers(dest="cmd")
    s = sub.add_parser("sources", help="each armour piece's sources and the one taken as first")
    s.add_argument("--grep", help="only pieces whose name matches this regex")
    sub.add_parser("table", help="the per-piece TSV, to stdout")
    sub.add_parser("defender", help="the weighted defender against the posted and unweighted ones")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.cmd == "sources":
        sources_cmd(a.grep)
    elif a.cmd == "table":
        table_cmd()
    elif a.cmd == "defender":
        defender_cmd()
    else:
        ap.print_help()
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
