#!/usr/bin/env python3
"""What each character in a DARK SOULS II save is wearing and wielding, read from the save file.

    python3 scripts/ds2-sl2-equipped.py SAVE.sl2            # every occupied slot, a table
    python3 scripts/ds2-sl2-equipped.py SAVE.sl2 --json     # the same, as JSON
    python3 scripts/ds2-sl2-equipped.py SAVE.sl2 --defense  # plus build_defense and the corpus

Read-only. Decrypts with `scripts/ds2-sl2.py`'s key and walks each slot's first entry,
USER_DATA(slot+1), whose stream (past the u32 length) is a run of 0x20-byte section headers
`{u32 type, u32 version, u32 size, ...}`, each followed by `size` bytes.

THE EQUIPPED GEAR IS AN ARRAY OF HANDLES, NOT OF ITEM IDS. Section type 10 is the inventory save
block `SaveDataItemInventory2` streams (`ds2_rva::ITEM_INVENTORY_SAVE_BLOCK_SIZE`, 0x100bc bytes):

* `+0x0000`: 3840 records of 16 bytes, `{u32 item, u32, f32 durability, u8 level, u8 infusion,
  u16}`, record k belonging to the bag entry whose handle is k.
* `+0x10004`: 50 `u16` handles, one per INTERNAL equip slot, `0xFFFF` for empty.

Static, darksoulsii-deobf.bin (SOTFS 9527516):

* `0x1401a5950` (the inventory manager's constructor) and `0x1401aa210` (its reset) memset the
  block at manager `+0x30` for 0x100bc bytes and fill the 0x32 u16 at manager `+0x10034` --
  block `+0x10004` -- with 0xFFFF.
* `0x1401b2980` (the bag's reset) stores `block + 0x10004` at bag `+0x25998`, and the bag's
  virtual at `0x1401b3c50` is `[bag+0x25998][slot] = handle` -- the equip path's record of which
  entry is in which slot.
* The load path at `0x1401a5e5c` walks that array, i = 0..0x31: a handle <= 0xEFF
  (`0x1401b87b0`) is resolved through the bag's entry-by-handle virtual, and for i <= 0x29 the
  entry is equipped into internal slot i (`0x1401b3d50(bag, i, item, handle)`); 42..49 go to the
  gesture list. So the array index IS the internal slot.

Internal slots (`ds2_rva::ITEM_SLOT_FLAT_TO_INTERNAL`): 0 RH1, 1 LH1, 2 RH2, 3 LH2, 4 RH3, 5 LH3,
6..9 head chest hands legs, 10..13 rings, 14..17 ammo, 18..27 hotbar, 28..41 attunement.

Section type 0 is the player: eleven u16 at `+0x00`, the nine stats in the GAME's order (vigor,
endurance, vitality, attunement, strength, dexterity, intelligence, faith, adaptability --
`ds2_rva::PLAYER_PARAM_STAT_OFFSETS`, second-hand from the Cheat Engine tables), the stored soul
level as u16 at `+0x18`. The name is the character list's (`ds2-sl2.py --slots`).

NOT DECODED: what the level and infusion bytes mean on a ring or an armour piece. Rings carry
their +N in the item id, yet some equipped rings read level 10; one armour piece reads an infusion
byte of 4. Both are printed raw in `--json` and left out of the table.

Item names are `crates/ds2-build-import-core/data/items.tsv`'s, a community table's.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ITEMS = HERE.parent / "crates/ds2-build-import-core/data/items.tsv"


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


SECTION_HEADER = 0x20
SECTION_PLAYER = 0
SECTION_INVENTORY = 10
RECORD = 16
RECORDS = 3840
EQUIPPED_AT = 0x10004
EQUIPPED_COUNT = 50
HANDLE_IN_BAG_MAX = 0xEFF
EMPTY = 0xFFFF
GAME_STAT_ORDER = ["vigor", "endurance", "vitality", "attunement", "strength", "dexterity",
                   "intelligence", "faith", "adaptability"]
PLAYER_LEVEL_AT = 0x18
ARMOR = ("head", "chest", "hands", "legs")
INFUSIONS = ["No_Infusion", "Fire", "Magic", "Lightning", "Dark", "Poison", "Bleed", "Raw",
             "Enchanted", "Mundane"]
SLOT_NAMES = (["rh1", "lh1", "rh2", "lh2", "rh3", "lh3", "head", "chest", "hands", "legs",
               "ring1", "ring2", "ring3", "ring4", "ammo1", "ammo2", "ammo3", "ammo4"]
              + [f"item{i + 1}" for i in range(10)] + [f"spell{i + 1}" for i in range(14)]
              + [f"gesture{i + 1}" for i in range(8)])


def item_names() -> dict[int, str]:
    out = {}
    for line in ITEMS.read_text().splitlines():
        if line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) >= 2 and parts[0].isdigit():
            out[int(parts[0])] = parts[1]
    return out


def sections(payload: bytes) -> dict[int, bytes]:
    length = struct.unpack_from("<I", payload, 0)[0]
    stream = payload[4:4 + length]
    out, at = {}, 0
    while at + SECTION_HEADER <= len(stream):
        kind, _version, size = struct.unpack_from("<III", stream, at)
        out.setdefault(kind, stream[at + SECTION_HEADER:at + SECTION_HEADER + size])
        if kind == 0xD:
            break
        at += SECTION_HEADER + size
    return out


def character(payload: bytes, names: dict[int, str]) -> dict | None:
    sec = sections(payload)
    player, inv = sec.get(SECTION_PLAYER), sec.get(SECTION_INVENTORY)
    if not player or not inv or len(inv) < EQUIPPED_AT + EQUIPPED_COUNT * 2:
        return None
    stats = dict(zip(GAME_STAT_ORDER, struct.unpack_from("<9H", player, 0)))
    out = {"name": "", "stats": stats,
           "level": struct.unpack_from("<H", player, PLAYER_LEVEL_AT)[0],
           "level_from_stats": max(1, sum(stats.values()) - 53), "equipped": {}}
    for slot, handle in enumerate(struct.unpack_from(f"<{EQUIPPED_COUNT}H", inv, EQUIPPED_AT)):
        if handle == EMPTY:
            continue
        if handle > HANDLE_IN_BAG_MAX:
            out["equipped"][SLOT_NAMES[slot]] = {"handle": handle, "outside_bag": True}
            continue
        item, _u, dur, level, inf, _x = struct.unpack_from("<IIfBBH", inv, handle * RECORD)
        out["equipped"][SLOT_NAMES[slot]] = {
            "handle": handle, "item": item, "name": names.get(item, f"?{item}"),
            "level": level & 0x0F, "infusion": INFUSIONS[inf] if inf < len(INFUSIONS) else inf,
            "raw_level_byte": level, "raw_infusion_byte": inf,
            "durability": round(dur, 2)}
    return out


def read_save(path: str, key_hex: str) -> list[tuple[int, dict]]:
    sl2 = _load("ds2_sl2", "ds2-sl2.py")
    b = Path(path).read_bytes()
    found = {n: (off, size) for _i, n, off, size in sl2.entries(b)}
    names = item_names()
    rows = []
    for slot, state, _attrs, list_name in sl2.slot_records(b, key_hex):
        if state == "empty":
            continue
        off, size = found[f"USER_DATA{slot + 1:03d}"]
        payload = sl2.decrypt(b[off + 32:off + size], b[off + 16:off + 32], key_hex)
        c = character(payload, names)
        if c:
            c["name"] = list_name
            rows.append((slot, c))
    return rows


#: ArmorReinforceParam: twelve f32 at +0x00 (the +0 piece) and twelve at +0x30 (the max level), the
#: max level as i32 at +0x60 (`ds2-reinforce-max.py`). Which float is which type is INFERRED: with
#: this order the max block equals SoulsPlanner's defense for all but a few dozen of 427 pieces x 8
#: types, and swapping slash and thrust breaks 233 of them. The elemental ones are a tenth of the
#: displayed value.
ARP_TYPES = {"slash": (0, 1), "thrust": (1, 1), "strike": (2, 1), "physical": (3, 1),
             "magic": (4, 10), "lightning": (5, 10), "fire": (6, 10), "dark": (7, 10)}
ARP_BASE, ARP_MAX_BLOCK, ARP_MAX_LEVEL = 0x00, 0x30, 0x60
ARMOR_PARAM_ID_FROM_ITEM_ID = 10_000_000  # ds2_rva::ARMOR_PARAM_ID_FROM_ITEM_ID


def level_shortfall(arp: dict, equipped: dict) -> dict:
    """How much defense each type loses because the worn armour is below its max level.

    The game's own rule (`ds2_rva::CHR_ASM_EQUIP_ARMOR_LEVEL_OFFSET`, 0x14034dbb0 / 0x14034dda0):
    `base + (max - base) * clamp(level / max_level, 0, 1)`. build_defense assumes the max."""
    out = {k: 0.0 for k in ARP_TYPES}
    for s in ARMOR:
        e = equipped.get(s)
        row = arp.get(e["item"] - ARMOR_PARAM_ID_FROM_ITEM_ID) if e and "item" in e else None
        if row is None:
            continue
        top = struct.unpack_from("<i", row, ARP_MAX_LEVEL)[0]
        f = min(1.0, max(0.0, e["level"] / top)) if top > 0 else 1.0
        for k, (i, scale) in ARP_TYPES.items():
            lo = struct.unpack_from("<f", row, ARP_BASE + 4 * i)[0]
            hi = struct.unpack_from("<f", row, ARP_MAX_BLOCK + 4 * i)[0]
            out[k] += (hi - lo) * (1 - f) * scale
    return out


def defense(rows):
    """build_defense per character and its SL bracket's corpus distribution."""
    import numpy as np
    arp = _load("ds2_reinforce_max", "ds2-reinforce-max.py").Tables(None).arp
    rec = _load("ds2_builds_recommend", "ds2-builds-recommend.py")
    sp, mm = rec.dump_site_tables(rec.CACHE / "site-tables")
    data = rec.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    rec.apply_regulation(data)
    corpus, _ = rec.load_corpus(data)
    by = {}
    for b in corpus:
        by.setdefault(rec.sl_bracket(rec.soul_level(data, b)), []).append(b)
    dist = {}
    keys = rec.DMG + rec.PHYS_TYPES

    def key(name):
        # items.tsv names a second id of some weapons "X (Recolor)"; the planners know only X.
        name = (name or "").replace(" (Recolor)", "")
        return data.sp_key.get(rec.norm(name), "") if name else ""

    for _slot, c in rows:
        eq = c["equipped"]
        armor = [key(eq.get(s, {}).get("name")) or "Naked" for s in rec.ARMOR_SLOTS]
        rings = [key(eq.get(f"ring{i}", {}).get("name")) or "No_Ring" for i in range(1, 5)]
        hands = [(key(eq.get(h, {}).get("name")) or "Bare_Fists", eq.get(h, {}).get("infusion", "No_Infusion"))
                 for h in rec.HAND_SLOTS]
        b = rec.Build("", {s: c["stats"][s] for s in rec.STATS}, armor, hands, 0, rings, [], c["level"])
        c["sp_keys"] = {"armor": armor, "rings": rings, "hands": hands}
        c["defense"] = {k: round(float(v), 1) for k, v in rec.build_defense(data, b).items()}
        short = level_shortfall(arp, eq)
        c["defense_at_level"] = {k: round(v - short.get(k, 0.0), 1) for k, v in c["defense"].items()}
        br = rec.sl_bracket(c["level"])
        if br not in dist:
            ds = [rec.build_defense(data, x) for x in by.get(br, [])]
            dist[br] = {"n": len(ds), "range": rec.SL_BRACKETS[br]}
            for k in keys:
                v = np.array([d[k] for d in ds]) if ds else np.zeros(1)
                dist[br][k] = {"mean": round(float(v.mean()), 1), "median": round(float(np.median(v)), 1),
                               "p90": round(float(np.percentile(v, 90)), 1),
                               "p99": round(float(np.percentile(v, 99)), 1)}
        c["bracket"] = br
    return dist


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("save")
    ap.add_argument("--vanilla", action="store_true", help="vanilla DARK SOULS II key, not SOTFS")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--defense", action="store_true",
                    help="add ds2-builds-recommend.py's build_defense (armour at max upgrade, as it "
                         "assumes) and the corpus defense distribution of each SL bracket")
    a = ap.parse_args()
    sl2 = _load("ds2_sl2", "ds2-sl2.py")
    rows = read_save(a.save, sl2.VANILLA_KEY_HEX if a.vanilla else sl2.KEY_HEX)
    dist = defense(rows) if a.defense else None
    if a.json:
        print(json.dumps({"characters": [dict(slot=s, **c) for s, c in rows],
                          "brackets": dist}, indent=1, default=str))
        return 0
    for slot, c in rows:
        st = " ".join(f"{k[:3].upper()} {v}" for k, v in c["stats"].items())
        lv = c["level"] if c["level"] == c["level_from_stats"] else f"{c['level']} (stats say {c['level_from_stats']})"
        print(f"slot {slot}  {c['name']}  SL {lv}\n  {st}")
        for s, e in c["equipped"].items():
            if s.startswith(("item", "gesture", "ammo")):
                continue
            if e.get("outside_bag"):
                print(f"  {s:<7} handle {e['handle']:#x} (outside the bag)")
                continue
            if s.startswith(("ring", "spell")):
                print(f"  {s:<7} {e['name']}")
                continue
            inf = "" if e["infusion"] == "No_Infusion" or s in ARMOR else f" [{e['infusion']}]"
            print(f"  {s:<7} {e['name']} +{e['level']}{inf}")
        if "defense" in c:
            d = c["defense"]
            print("  defense, armour at max  " + " ".join(f"{k} {d[k]:.0f}" for k in d))
            print("  defense, armour as worn " + " ".join(f"{k} {c['defense_at_level'][k]:.0f}" for k in d))
            m = dist[c["bracket"]]
            print(f"  bracket SL {m['range'][0]}-{m['range'][1]} (n={m['n']}) mean " +
                  " ".join(f"{k} {m[k]['mean']:.0f}" for k in d))
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
