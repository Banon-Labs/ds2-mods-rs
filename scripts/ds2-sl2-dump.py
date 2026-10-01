#!/usr/bin/env python3
"""Decode everything this repo knows how to read out of a DARK SOULS II: SOTFS `.sl2` save.

    python3 scripts/ds2-sl2-dump.py SAVE.sl2                 # readable summary, every character
    python3 scripts/ds2-sl2-dump.py SAVE.sl2 --slot 3        # one character
    python3 scripts/ds2-sl2-dump.py SAVE.sl2 --json          # everything, as JSON on stdout
    python3 scripts/ds2-sl2-dump.py SAVE.sl2 --json --out F  # the same, written to F
    python3 scripts/ds2-sl2-dump.py --selftest               # offsets and bit order, no save

Read-only. It never writes a save. Decryption and the BND4 walk are `scripts/ds2-sl2.py`'s.

Every field below carries one of three tags, and the JSON keeps them (`"how"` keys):

* STATIC  -- read out of `darksoulsii-deobf.bin` (SOTFS 9527516); the address is given.
* FMG     -- a name from the game's own text files (`/menu/text/english/*.fmg`, pulled out of
             GameDataEbl by `scripts/ds2-ebl.py`). The id-to-text join is the game's.
* 3RD     -- a second-hand source: `crates/ds2-build-import-core/data/items.tsv` (a Cheat Engine
             table) or alfizari's Dark-Souls-2-Save-Editor. Used as a fallback or a cross-check.
* INFER   -- read off real saves without a static proof. Treat as a lead.

==================================================================================================
THE FILE
==================================================================================================

BND4 of 23 entries, each AES-128-CBC (see ds2-sl2.py). A decrypted entry is a u32 length, then a
chain of 0x20-byte headers `{u32 type, u32 version, u32 size, 20 bytes}` each followed by `size`
bytes, ending at type 0xD. Character slot n lives in USER_DATA(n+1) and USER_DATA(n+11).

SECTION TYPE -> CLASS (STATIC). `0x1402e4958` builds four 15-entry `SaveDataBase*` arrays, one per
kind of entry, and stores each object at array[i*8]. The loader `0x1402e47f0` maps a header's
type to i through the u32 table at `0x1410da1f0` = {0,1,...,12,14,13}, so i == type except that
type 14 is index 13 and type 13 is index 14. Each object's ctor writes the version at +8, and
each class's vtable slot 1 returns the section size as an immediate. All three agree with every
header in every save read here:

    type ver   size     class (vtable)                     entry       write / read
    0    0x6f  0x35c    SaveDataPlayer (0x1410da260)       slot+1      0x1402e5dc0 / 0x1402e5cd0
    1    0x6d  0x470    SaveDataGame (0x1410da288)         slot+1      0x1402e3fc0 / 0x1402e3c80
    2    0x67  0x1d0a0  SaveDataGenerator (0x1410da3c8)    slot+11     0x1402e4210 / 0x1402e4130
    3    0x66  0x32c    SaveDataOption (0x1410da2b0)       000         0x1402e5c90 / 0x1402e5c30
    4    0x6d  0x1370   SaveDataLoadInfo (0x1410da2d8)     000         0x1402e5690 / 0x1402e55c0
    4    0x6e  0x1370   SaveDataLoadInfo2 (0x1410da300)    022         0x1402e5620 / 0x1402e5570
    5    0x65  0xfa4    SaveDataShopLineup (0x1410da328)   slot+1      0x1402e5ed0 / 0x1402e5e80
    6    0x6f  0x61c    SaveDataNetwork (0x1410da350)      slot+1      0x1402e57e0 / 0x1402e5710
    7    0x65  0x9c30   SaveDataEnemy (0x1410da3a0)        slot+11     0x1402e3530 / 0x1402e3370
    8    0x6b  0x21c8   SaveDataEvent (0x1410da3f0)        slot+11     0x1402e3a70 / 0x1402e38f0
    9    0x69  0x31970  SaveDataObj (0x1410da378)          slot+11     0x1402e5a10 / 0x1402e5890
    10   0x6c  0x100bc  SaveDataItemInventory2 (0x1410da418) slot+1    0x1402e53f0 / 0x1402e53a0
    11   --    --       (array slot stored as NULL at 0x1402e4ab3; never written)
    12   0x66  0x1394   SaveDataItemPack (0x1410da440)     slot+1      0x1402e5500 / 0x1402e5450
    13   0x102 varies   _SaveDataEnd (0x1410da490)         all         padding: 0x79dc / 0x20000 /
                                                                       0x388 / 0x2000 by entry
    14   0x65  0x604    SaveDataGetObjItem (0x1410da468)   slot+1      0x1402e4320 / 0x1402e4250

Vtable layout (STATIC): slot 0 dtor, slot 1 size, slot 2 write(stream, enabled), slot 3
read(stream, ?, version, enabled). USER_DATA021 (2 MiB) carries no section chain at all and is
not read by anything this script found.

==================================================================================================
DECODED
==================================================================================================

CHARACTER LIST, type 4 (STATIC: reader `0x14019bb30`, writer `0x14019c190`). 16-byte prologue
`{u32 current slot (-> +0x1368), u32 10, 8 flag bytes}`, then ten 0x1F0 records copied verbatim to
`GameDataManager+0xD8`. NOTE: `ds2-sl2.py --slots` indexes records from four bytes BEFORE the
section data (it assumes a 12-byte header), so its +0x188/+0x19E are +0x174/+0x18A here. Record:

    +0x000 0xA4  appearance block; +0x92 u8 and +0x93 u8 are read by the title preview
                 (`0x1400ee240`), meaning not established
    +0x0A4 0xD0  visual-equipment stream, written by `0x141b4cfd3` and read back by
                 `0x140346c10` (`0x1400ee240`). INFER: u32 count, then that many u32 item ids
    +0x174 11 u16 stats, `assignAttributes` order (`0x1400ee240`)
    +0x18A wchar[32] name (`wcsncpy(.., 0x20)` at `0x1400ee3d9`)
    +0x1CC u32   play time, seconds: the writer adds a QueryPerformanceCounter delta and caps it
                 at 3599999 (999:59:59)
    +0x1D0 u32   location text id (mapname.fmg). The writer picks the last-rested bonfire's area,
                 or computes it from the map index, or the literal 0x98e4a0 (Things Betwixt) /
                 0x1346152 (Throne of Want)
    +0x1D4 u16 soul level; +0x1D6 u16 class (`ds2_rva::PLAYER_DATA_CLASS_OFFSET`)
    +0x1D8 u8  covenant (`SetCovenant` 0x14038bd80); +0x1D9 u8 flags (bit0 occupied, bit3
               visual stream valid -- the preview gates on it; bit2 and bit4 set by the writer
               from save kind and online state, meaning not established). It is NOT zero on
               disk: 0x0D or 0x1D in every saved record read here, 0x03 for a placeholder the
               writer marks (`0x14019c6d7`). ds2-sl2.py's "always zero" was measured 0x14 early
    +0x1DA u8  `PlayerParam+0x1D6` (`0x14038b960`), meaning not established
    +0x1DB u8, +0x1DC u32, +0x1E0 u32, +0x1E4 u32: carried over by the writer, not decoded

PLAYER, type 0 (STATIC: the record is copied to `GameDataPlayerTempData+8` by `0x14019ea20` and
applied by `PlayerParam::RestoreFromRecord` `0x14038ad20`):

    +0x00 11 u16  stats in game order (vig end vit att str dex int fth adp, then two)
    +0x18 u16     soul level (3RD agrees: editor "level" at payload 0x38)
    +0x1C u32     souls held -> PlayerParam+0xEC (3RD agrees: payload 60)
    +0x20 u32     soul memory -> +0xF4;  +0x24 u32 second soul-memory field -> +0xFC
    +0x28 u32     HP -> ChrParam+0x168/+0x170/+0x174 (3RD agrees: payload 72)
    +0x2C 7 f32   clamped into ChrParam +0x1C4, +0x1D0 ... +0x20C (names not established)
    +0x48 20 i32  -> PlayerParam+0x104 + 8i, each behind its own skip byte (not established)
    +0x98 i32     -> PlayerParam+0x1A4, capped at 1e8 (not established)
    +0x9C u8      -> PlayerParam+0x1AC (not established)
    +0x9D u8      current covenant -> +0x1AD (`ds2_rva::PLAYER_PARAM_COVENANT_OFFSET`)
    +0x9E 10 u8   covenant discovered flags, by covenant id -> +0x1AE
    +0xB2 10 u16  per-covenant u16 -> +0x1C2 (INFER: covenant progress; `ds2_rva` puts progress
                  at +0x1C4 for covenant 1, which is +0x1C2 + 2*1)
    +0xC6 u8 -> +0x1D6, +0xC7 u8 -> +0x1D7 (not established)
    +0xC8 0xA4    INFER: the same appearance block the character list keeps at +0x000 (byte
                  identical in the saves read); +0x16C 0xD0 the visual-equipment stream

GAME, type 1 (STATIC: writer `0x1402e3fc0`, reader `0x1402e3c80`, both on
`GameDataManager->player_data`):

    +0x00 f32[4]  position (x, y, z, 1) -- the reader feeds +0x00..+0x1B into the spawn request
    +0x10 f32[3]  orientation (INFER from values; read together with the position)
    +0x1C u8      player_data+0xB8 (the character list writer's "use the map, not the bonfire")
    +0x1D u8      bit0 -> +0xB9, bit1 -> +0xBA
    +0x20 u32     map id, bytes AA BB CC DD -> mAA_BB_CC_DD (`0x14019e1a0`)
    +0x24 wchar[32] name;  +0x64 u32 class
    +0x68 u32     NG cycle, clamped to 1..99 by the reader (3RD agrees: payload 1028)
    +0x6C 0x404   player_data+0xDC (`0x14019e270`): u32 count, then {u32 spell id, u32}
                  (INFER: the ids resolve to spell names in itemname.fmg)

INVENTORY, type 10 (STATIC: load loop `0x1401a5dda`..`0x1401a5f97` over the block at
inventory-manager +0x30):

    +0x00000 0xF00 x 16-byte records, handle = index (bag, `0x1401b87b0` answers 0)
    +0x0F000 u8 right-hand active slot, u8 left-hand active slot (-> `0x1401b3c60`/`0x1401b3c90`),
             u8 -> `0x1401b42f0`, u8
    +0x0F004 0x100 x 16-byte records, handles 0xF00..0xFFF: the second container
             (`0x1401b87b0` answers 1, manager +0x18). INFER from contents: key items and
             gestures. 3RD agrees: the editor's "key items" range is exactly this span
    +0x10004 50 u16 equipped handles: 0..41 into the bag's internal slots (`0x1401b3d50`),
             42..49 into the second container's eight slots (`0x1401b7ed0`, gestures)
    +0x10068 0x54 bytes, the first 0x30 handed to `0x1401bbb80` (not established)
    Record: {u32 item, u32 flags, 4 bytes, u8 level, u8 infusion, u16}. The 4 bytes are an f32
    durability when flags bit 8 is set and a u16 quantity otherwise (INFER; ds2_rva notes the live
    entry keeps quantity and durability in the same four bytes).

EVENT, type 8 -- the save flags (STATIC: writer `0x1402e3a70` calls the four managers below,
reader `0x1402e38f0` hands the same buffer back):

    +0x0000 0x4E2  event-flag group 10 (flags 100000..109999)  `0x1404742c0` copies flag-storage
    +0x04E4 0x4E2  event-flag group 20 (flags 200000..209999)  +0x008 and +0x4EA
    +0x09C8 42 x 0x19  per-map group G, by the map's index in the MapManager's tree
    +0x0DE4 0x400  event values, global (`0x14047a1c0`)
    +0x11E4 3 x 0x13C: maps loaded at save time: u32 map id; 0x19 bytes group G+1;
            0x19 bytes group G+2 at +0x20; 0x100 bytes event values at +0x3C
    +0x1598 256 u16 bonfire ids (sorted; bonfirename.fmg);  +0x1798 256 u8 bit0 of the
            bonfire record's byte 2;  +0x1898 256 u8 the rest of that byte;  +0x1998 256 u64
            (`0x14017e540` / `0x14017eb10`)
    +0x2198 u32 map id, +0x219C u32 last rested bonfire id, +0x21A0 u8 (EventManager +0x164,
            +0x16C, +0x168);  +0x21A8 8 u32 timers (`0x14047af60`)

FLAG ID -> BIT (STATIC, `readEventFlag` 0x140474230 / `writeEventFlag` 0x1404750b0): group =
id / 10000, byte = (id % 10000) >> 3, mask = 0x80 >> (id % 8) -- MSB first. Group ids come from
the registrations: 10 and 20 with 0x4E2 bytes each (Arxan fragments `0x1407249dd` and
`0x14070e63d` jumping back into `0x14047436a`, bank 0 = flag storage +0x8 / +0x4EA), and per map
`0x1404745c0`: G = ((map_id / 0x0A000000) * 100 + ((map_id >> 16) & 0xFF)) * 100, with G, G+1, G+2
holding 0x19 bytes each. m10_04 (Majula) is G 10400 -> flags 104000000..104000199.

==================================================================================================
NOT DECODED (raw summaries only): Generator (2), Option (3), ShopLineup (5), Network (6),
Enemy (7), Obj (9), ItemPack (12), GetObjItem (14), USER_DATA021. Which map each of the 42 per-map
event blocks belongs to: the index comes from a runtime tree (`0x1403bcec0`, Arxan-shattered), so
those flags are printed by index with their bits, not as flag ids. Gender, boss kills by name, and
the meaning of the bonfire upper bits are not established.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ITEMS_TSV = HERE.parent / "crates/ds2-build-import-core/data/items.tsv"
TEXT_CACHE = Path.home() / ".cache/ds2-mods-rs/text"
FMG_NAMES = ("itemname", "bonfirename", "mapname")


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


# --------------------------------------------------------------------------------------------
# Section chain
# --------------------------------------------------------------------------------------------

SECTION_HEADER = 0x20
SECTION_END = 0xD

#: type -> (class, how). The proof is the docstring's table.
SECTION_CLASS = {
    0: "SaveDataPlayer", 1: "SaveDataGame", 2: "SaveDataGenerator", 3: "SaveDataOption",
    4: "SaveDataLoadInfo", 5: "SaveDataShopLineup", 6: "SaveDataNetwork", 7: "SaveDataEnemy",
    8: "SaveDataEvent", 9: "SaveDataObj", 10: "SaveDataItemInventory2", 12: "SaveDataItemPack",
    13: "_SaveDataEnd", 14: "SaveDataGetObjItem",
}
#: The sizes each class's vtable slot 1 returns (STATIC), for checking a section before decoding.
SECTION_SIZE = {
    0: 0x35C, 1: 0x470, 2: 0x1D0A0, 3: 0x32C, 4: 0x1370, 5: 0xFA4, 6: 0x61C, 7: 0x9C30,
    8: 0x21C8, 9: 0x31970, 10: 0x100BC, 12: 0x1394, 14: 0x604,
}


def sections(payload: bytes) -> list[dict]:
    """Each header in the entry's chain: type, version, size, offset and the data bytes."""
    if len(payload) < 4:
        return []
    length = struct.unpack_from("<I", payload, 0)[0]
    stream = payload[4:4 + length]
    out, at = [], 0
    while at + SECTION_HEADER <= len(stream):
        kind, version, size = struct.unpack_from("<III", stream, at)
        if kind > 14:
            break
        out.append({"type": kind, "version": version, "size": size, "offset": at + 4,
                    "data": stream[at + SECTION_HEADER:at + SECTION_HEADER + size]})
        if kind == SECTION_END:
            break
        at += SECTION_HEADER + size
    return out


def raw_summary(sec: dict) -> dict:
    data = sec["data"]
    nz = sum(1 for b in data if b)
    return {"type": sec["type"], "class": SECTION_CLASS.get(sec["type"], "?"),
            "version": hex(sec["version"]), "size": hex(sec["size"]),
            "size_matches_class": SECTION_SIZE.get(sec["type"]) in (None, sec["size"]),
            "nonzero_bytes": nz, "sha1": hashlib.sha1(data).hexdigest()}


# --------------------------------------------------------------------------------------------
# Names
# --------------------------------------------------------------------------------------------


class Names:
    """itemname/bonfirename/mapname from the game's FMGs, items.tsv as the fallback for items."""

    def __init__(self, text_dir: Path | None, extract: bool):
        self.fmg: dict[str, dict[int, str]] = {n: {} for n in FMG_NAMES}
        self.source = "none"
        directory = text_dir or TEXT_CACHE
        if extract and text_dir is None:
            self._extract(directory)
        fmg = _load("ds2_fmg", "ds2-fmg.py")
        found = 0
        for n in FMG_NAMES:
            f = directory / f"{n}.fmg"
            if f.exists():
                self.fmg[n] = fmg.parse(f.read_bytes())
                found += 1
        if found:
            self.source = f"FMG ({directory})"
        self.tsv: dict[int, str] = {}
        if ITEMS_TSV.exists():
            for line in ITEMS_TSV.read_text().splitlines():
                parts = line.split("\t")
                if len(parts) >= 2 and parts[0].isdigit():
                    self.tsv[int(parts[0])] = parts[1]

    @staticmethod
    def _extract(directory: Path) -> None:
        missing = [n for n in FMG_NAMES if not (directory / f"{n}.fmg").exists()]
        for n in missing:
            try:
                subprocess.run([sys.executable, str(HERE / "ds2-ebl.py"), "extract",
                                f"/menu/text/english/{n}.fmg", "--out", str(directory)],
                               capture_output=True, timeout=120, check=False)
            except (OSError, subprocess.SubprocessError):
                return

    def item(self, item_id: int) -> tuple[str, str]:
        name = self.fmg["itemname"].get(item_id)
        if name:
            return name, "FMG"
        name = self.tsv.get(item_id)
        if name:
            return name, "3RD"
        return f"?{item_id}", "unknown"

    def bonfire(self, bonfire_id: int) -> str | None:
        return self.fmg["bonfirename"].get(bonfire_id)

    def place(self, text_id: int) -> str | None:
        return self.fmg["mapname"].get(text_id)


# --------------------------------------------------------------------------------------------
# Small readers
# --------------------------------------------------------------------------------------------

STAT_NAMES = ["vigor", "endurance", "vitality", "attunement", "strength", "dexterity",
              "intelligence", "faith", "adaptability"]
COVENANTS = ["No Covenant", "Heirs of the Sun", "Blue Sentinels", "Brotherhood of Blood",
             "Way of Blue", "Rat King", "Bell Keepers", "Dragon Remnants",
             "Company of Champions", "Pilgrims of Dark"]
#: `ds2_rva::PLAYER_DATA_CLASS_OFFSET`'s table; the gaps are the game's.
CLASSES = {1: "Warrior", 2: "Knight", 4: "Bandit", 6: "Cleric", 7: "Sorcerer", 8: "Explorer",
           9: "Swordsman", 10: "Deprived"}
INFUSIONS = ["None", "Fire", "Magic", "Lightning", "Dark", "Poison", "Bleed", "Raw",
             "Enchanted", "Mundane"]
EQUIP_SLOTS = (["rh1", "lh1", "rh2", "lh2", "rh3", "lh3", "head", "chest", "hands", "legs",
                "ring1", "ring2", "ring3", "ring4", "ammo1", "ammo2", "ammo3", "ammo4"]
               + [f"item{i + 1}" for i in range(10)] + [f"spell{i + 1}" for i in range(14)]
               + [f"gesture{i + 1}" for i in range(8)])


def wide(data: bytes, offset: int, chars: int = 32) -> str:
    chunk = data[offset:offset + chars * 2]
    for i in range(0, len(chunk) - 1, 2):
        if chunk[i] == 0 and chunk[i + 1] == 0:
            chunk = chunk[:i]
            break
    return chunk.decode("utf-16le", "replace")


def u8(d, o):
    return d[o]


def u16(d, o):
    return struct.unpack_from("<H", d, o)[0]


def u32(d, o):
    return struct.unpack_from("<I", d, o)[0]


def i32(d, o):
    return struct.unpack_from("<i", d, o)[0]


def f32(d, o):
    return round(struct.unpack_from("<f", d, o)[0], 4)


def map_name(map_id: int) -> str:
    """`mAA_BB_CC_DD` from a u32 map id, as `convertMapStringToId` builds it."""
    return "m{:02d}_{:02d}_{:02d}_{:02d}".format(map_id >> 24, (map_id >> 16) & 0xFF,
                                               (map_id >> 8) & 0xFF, map_id & 0xFF)


def playtime(seconds: int) -> str:
    return f"{seconds // 3600}:{seconds // 60 % 60:02d}:{seconds % 60:02d}"


# --------------------------------------------------------------------------------------------
# Character list (type 4)
# --------------------------------------------------------------------------------------------

LIST_PROLOGUE = 16
LIST_STRIDE = 0x1F0
LIST_COUNT = 10
#: The writer's last loop (`or al,0x3` at `0x14019c6d7`) ORs 3 into the flags of every record below the current
#: slot index whose bit 0 is clear: a record no save has filled in. Its name and stats are blank
#: even when the slot's own entries hold a character (seen in a real save: list record empty,
#: USER_DATA001 a SL 220 character).
LIST_PLACEHOLDER_FLAGS = 0x3


def visual_stream(block: bytes) -> dict:
    count = u32(block, 0)
    ids = list(struct.unpack_from(f"<{min(count, 0x33)}I", block, 4)) if count <= 0x33 else []
    return {"count": count, "item_ids": ids, "how": "INFER: u32 count then u32 item ids"}


def list_record(r: bytes, names: Names) -> dict:
    stats = struct.unpack_from("<11H", r, 0x174)
    flags = r[0x1D9]
    location = u32(r, 0x1D0)
    return {
        "name": wide(r, 0x18A),
        "stats": dict(zip(STAT_NAMES, stats[:9])), "stats_extra": list(stats[9:]),
        "soul_level": u16(r, 0x1D4),
        "class": CLASSES.get(u16(r, 0x1D6), u16(r, 0x1D6)),
        "covenant": COVENANTS[r[0x1D8]] if r[0x1D8] < len(COVENANTS) else r[0x1D8],
        "play_time_s": u32(r, 0x1CC), "play_time": playtime(u32(r, 0x1CC)),
        "location_text_id": location, "location": names.place(location),
        "flags": {"raw": hex(flags), "occupied": bool(flags & 1),
                  "visual_stream_valid": bool(flags & 8), "bit1": bool(flags & 2),
                  "bit2": bool(flags & 4), "bit4": bool(flags & 0x10)},
        "byte_1da": r[0x1DA], "byte_1db": r[0x1DB],
        "u32_1dc": hex(u32(r, 0x1DC)), "u32_1e0": hex(u32(r, 0x1E0)),
        "u32_1e4": hex(u32(r, 0x1E4)),
        "appearance_92": r[0x92], "appearance_93": r[0x93],
        "visual_equipment": visual_stream(r[0xA4:0x174]),
        "how": "STATIC 0x14019c190 writer / 0x1400ee240 preview; location FMG",
    }


def character_list(data: bytes, names: Names) -> dict:
    out = {"current_slot": i32(data, 0), "slot_count": u32(data, 4),
           "prologue_flags": data[8:16].hex(), "slots": []}
    for slot in range(LIST_COUNT):
        r = data[LIST_PROLOGUE + slot * LIST_STRIDE:][:LIST_STRIDE]
        if len(r) < LIST_STRIDE:
            break
        if not any(r):
            out["slots"].append({"slot": slot, "state": "empty"})
            continue
        rec = list_record(r, names)
        rec["slot"] = slot
        if r[0x1D9] == LIST_PLACEHOLDER_FLAGS:
            rec["state"] = "placeholder"
        elif all(v == 1 for v in rec["stats"].values()):
            rec["state"] = "blank"
        else:
            rec["state"] = "occupied"
        out["slots"].append(rec)
    return out


# --------------------------------------------------------------------------------------------
# Player (type 0) and Game (type 1)
# --------------------------------------------------------------------------------------------


def player(d: bytes) -> dict:
    stats = struct.unpack_from("<11H", d, 0)
    discovered = [COVENANTS[i] for i in range(10) if d[0x9E + i]]
    return {
        "stats": dict(zip(STAT_NAMES, stats[:9])), "stats_extra": list(stats[9:]),
        "u16_16": u16(d, 0x16),
        "soul_level": u16(d, 0x18),
        "souls": u32(d, 0x1C), "soul_memory": u32(d, 0x20), "soul_memory_2": u32(d, 0x24),
        "hp": u32(d, 0x28),
        "chr_param_floats_2c": [f32(d, 0x2C + 4 * i) for i in range(7)],
        "player_param_104_ints": list(struct.unpack_from("<20i", d, 0x48)),
        "i32_98": i32(d, 0x98), "u8_9c": d[0x9C],
        "covenant": COVENANTS[d[0x9D]] if d[0x9D] < 10 else d[0x9D],
        "covenants_discovered": discovered,
        "covenant_u16_b2": dict(zip(COVENANTS, struct.unpack_from("<10H", d, 0xB2))),
        "u8_c6": d[0xC6], "u8_c7": d[0xC7],
        "visual_equipment": visual_stream(d[0x16C:0x23C]),
        "how": "STATIC PlayerParam::RestoreFromRecord 0x14038ad20; 3RD agrees on SL/souls/HP",
    }


def game(d: bytes, names: Names) -> dict:
    count = u32(d, 0x6C)
    spells = []
    for i in range(min(count, (0x404 - 4) // 8)):
        sid, packed = struct.unpack_from("<II", d, 0x70 + 8 * i)
        spells.append({"id": sid, "name": names.item(sid)[0], "packed": hex(packed)})
    map_id = u32(d, 0x20)
    return {
        "position": [f32(d, 4 * i) for i in range(4)],
        "orientation": [f32(d, 0x10 + 4 * i) for i in range(3)],
        "u8_1c": d[0x1C], "flags_1d": hex(d[0x1D]),
        "map_id": hex(map_id), "map": map_name(map_id),
        "name": wide(d, 0x24), "class": CLASSES.get(u32(d, 0x64), u32(d, 0x64)),
        "ng_cycle": u32(d, 0x68),
        "spell_table": {"count": count, "entries": spells,
                        "how": "STATIC location 0x14019e270; INFER layout from contents"},
        "how": "STATIC 0x1402e3fc0 / 0x1402e3c80; 3RD agrees on name and NG offsets",
    }


# --------------------------------------------------------------------------------------------
# Inventory (type 10)
# --------------------------------------------------------------------------------------------

INV_RECORDS = 0xF00
INV_SECOND_AT = 0xF004
INV_SECOND_COUNT = 0x100
INV_EQUIPPED_AT = 0x10004
INV_EQUIPPED_COUNT = 50
INV_RECORD = 16
FLAG_HAS_DURABILITY = 0x100


def inv_record(raw: bytes, handle: int, names: Names) -> dict | None:
    item, flags = struct.unpack_from("<II", raw, 0)
    if item == 0:
        return None
    name, how = names.item(item)
    rec = {"handle": hex(handle), "item": item, "name": name, "name_from": how,
           "flags": hex(flags), "level": raw[12], "infusion_byte": raw[13],
           "u16_e": u16(raw, 14)}
    if flags & FLAG_HAS_DURABILITY:
        rec["durability"] = f32(raw, 8)
        rec["infusion"] = INFUSIONS[raw[13]] if raw[13] < len(INFUSIONS) else raw[13]
    else:
        rec["quantity"] = u16(raw, 8)
        rec["bytes_a_b"] = [raw[10], raw[11]]
    return rec


def inventory(d: bytes, names: Names) -> dict:
    bag, second = {}, {}
    for h in range(INV_RECORDS):
        rec = inv_record(d[h * INV_RECORD:(h + 1) * INV_RECORD], h, names)
        if rec:
            bag[h] = rec
    for k in range(INV_SECOND_COUNT):
        at = INV_SECOND_AT + k * INV_RECORD
        rec = inv_record(d[at:at + INV_RECORD], INV_RECORDS + k, names)
        if rec:
            second[INV_RECORDS + k] = rec
    equipped = {}
    for slot, handle in enumerate(struct.unpack_from(f"<{INV_EQUIPPED_COUNT}H", d,
                                                      INV_EQUIPPED_AT)):
        if handle == 0xFFFF:
            continue
        rec = bag.get(handle) or second.get(handle)
        equipped[EQUIP_SLOTS[slot]] = rec or {"handle": hex(handle), "missing": True}
    return {
        "active_right_slot": d[0xF000], "active_left_slot": d[0xF001],
        "byte_f002": d[0xF002], "byte_f003": d[0xF003],
        "equipped": equipped,
        "bag": list(bag.values()),
        "key_items_and_gestures": list(second.values()),
        "tail_10068": d[0x10068:0x100BC].hex(),
        "how": "STATIC load loop 0x1401a5dda..0x1401a5f97; record split INFER; names FMG/3RD",
    }


# --------------------------------------------------------------------------------------------
# Event (type 8): flags, values, bonfires
# --------------------------------------------------------------------------------------------

EV_GROUP10_AT = 0x000
EV_GROUP20_AT = 0x4E4
EV_GROUP_BYTES = 0x4E2
EV_PER_MAP_AT = 0x9C8
EV_PER_MAP_COUNT = 42
EV_MAP_BLOCK = 0x19
EV_VALUES_AT = 0xDE4
EV_VALUES_BYTES = 0x400
EV_LOADED_AT = 0x11E4
EV_LOADED_STRIDE = 0x13C
EV_LOADED_COUNT = 3
EV_BONFIRE_IDS = 0x1598
EV_BONFIRE_LIT = 0x1798
EV_BONFIRE_UPPER = 0x1898
EV_BONFIRE_U64 = 0x1998
EV_BONFIRE_COUNT = 0x100
EV_TAIL = 0x2198
EV_TIMERS = 0x21A8


def set_bits(block: bytes) -> list[int]:
    """Bit numbers set in a flag block, in the game's order: byte n>>3, mask 0x80 >> (n & 7)."""
    return [i * 8 + b for i, byte in enumerate(block) if byte for b in range(8)
            if byte & (0x80 >> b)]


def map_group(map_id: int) -> int:
    """`0x1404745c0`'s group for a map: its G; G+1 and G+2 follow."""
    return ((map_id // 0x0A000000) * 100 + ((map_id >> 16) & 0xFF)) * 100


def flag_ids(group: int, block: bytes) -> list[int]:
    return [group * 10000 + n for n in set_bits(block)]


def nonzero_words(block: bytes) -> dict[str, int]:
    return {hex(4 * i): v for i, v in enumerate(struct.unpack_from(f"<{len(block) // 4}I", block))
            if v}


def events(d: bytes, names: Names) -> dict:
    loaded = []
    for k in range(EV_LOADED_COUNT):
        base = EV_LOADED_AT + k * EV_LOADED_STRIDE
        map_id = u32(d, base)
        if map_id in (0, 0xFFFFFFFF):
            continue
        g = map_group(map_id)
        loaded.append({
            "map_id": hex(map_id), "map": map_name(map_id), "group": g,
            "flags_g1": flag_ids(g + 1, d[base + 4:base + 4 + EV_MAP_BLOCK]),
            "flags_g2": flag_ids(g + 2, d[base + 0x20:base + 0x20 + EV_MAP_BLOCK]),
            "values": nonzero_words(d[base + 0x3C:base + 0x3C + 0x100]),
        })
    per_map = []
    for i in range(EV_PER_MAP_COUNT):
        bits = set_bits(d[EV_PER_MAP_AT + i * EV_MAP_BLOCK:][:EV_MAP_BLOCK])
        if bits:
            per_map.append({"map_index": i, "bits": bits})
    bonfires = []
    for i in range(EV_BONFIRE_COUNT):
        bid = u16(d, EV_BONFIRE_IDS + 2 * i)
        if bid == 0:
            continue
        bonfires.append({"id": bid, "name": names.bonfire(bid),
                         "bit0": d[EV_BONFIRE_LIT + i], "upper": d[EV_BONFIRE_UPPER + i],
                         "u64": hex(struct.unpack_from("<Q", d, EV_BONFIRE_U64 + 8 * i)[0])})
    last = u32(d, EV_TAIL + 4)
    return {
        "flags_group10": flag_ids(10, d[EV_GROUP10_AT:EV_GROUP10_AT + EV_GROUP_BYTES]),
        "flags_group20": flag_ids(20, d[EV_GROUP20_AT:EV_GROUP20_AT + EV_GROUP_BYTES]),
        "per_map_blocks_by_index": per_map,
        "loaded_maps": loaded,
        "values_global": nonzero_words(d[EV_VALUES_AT:EV_VALUES_AT + EV_VALUES_BYTES]),
        "bonfires": bonfires,
        "event_map_id": hex(u32(d, EV_TAIL)), "event_map": map_name(u32(d, EV_TAIL)),
        "last_bonfire_id": last, "last_bonfire": names.bonfire(last),
        "byte_21a0": d[EV_TAIL + 8],
        "timers": list(struct.unpack_from("<8I", d, EV_TIMERS)),
        "how": "STATIC 0x1402e3a70 / 0x1404742c0 / 0x14047a1c0 / 0x14017e540 / 0x14047af60; "
               "flag bit order 0x140474230; bonfire names FMG",
    }


# --------------------------------------------------------------------------------------------
# The save
# --------------------------------------------------------------------------------------------


def decode(path: str, key_hex: str, names: Names, only_slot: int | None) -> dict:
    sl2 = _load("ds2_sl2", "ds2-sl2.py")
    raw = Path(path).read_bytes()
    if raw[:4] != b"BND4":
        raise SystemExit(f"{path}: not a BND4 container")
    payloads = {}
    for _i, name, off, size in sl2.entries(raw):
        payloads[name] = sl2.decrypt(raw[off + 32:off + size], raw[off + 16:off + 32], key_hex)
    out = {"file": path, "names_from": names.source, "entries": {}, "global": {},
           "characters": []}
    for name, p in payloads.items():
        secs = sections(p)
        out["entries"][name] = {"bytes": len(p),
                                "sections": [raw_summary(s) for s in secs] if secs and
                                secs[-1]["type"] == SECTION_END else "no section chain"}
    for entry in ("USER_DATA000", "USER_DATA022"):
        for s in sections(payloads.get(entry, b"")):
            if s["type"] == 4:
                key = "character_list" if entry == "USER_DATA000" else "character_list_2"
                out["global"][key] = character_list(s["data"], names)
                out["global"][key]["version"] = hex(s["version"])
            elif s["type"] == 3:
                out["global"]["options"] = raw_summary(s)
    listing = out["global"].get("character_list", {}).get("slots", [])
    for slot in range(LIST_COUNT):
        if only_slot is not None and slot != only_slot:
            continue
        state = listing[slot]["state"] if slot < len(listing) else "?"
        if state == "empty":
            continue
        char = {"slot": slot, "list_state": state, "list": listing[slot] if slot < len(listing)
                else None, "undecoded": []}
        for delta in (1, 11):
            entry = f"USER_DATA{slot + delta:03d}"
            for s in sections(payloads.get(entry, b"")):
                t = s["type"]
                if t == SECTION_END:
                    continue
                if SECTION_SIZE.get(t) != s["size"]:
                    char["undecoded"].append(raw_summary(s) | {"why": "size differs from class"})
                    continue
                if t == 0 and s["version"] == 0x6F:
                    char["player"] = player(s["data"])
                elif t == 1:
                    char["game"] = game(s["data"], names)
                elif t == 10 and s["version"] == 0x6C:
                    char["inventory"] = inventory(s["data"], names)
                elif t == 8 and s["version"] == 0x6B:
                    char["events"] = events(s["data"], names)
                else:
                    char["undecoded"].append(raw_summary(s))
        out["characters"].append(char)
    return out


def summary(doc: dict) -> str:
    lines = [f"{doc['file']}", f"names: {doc['names_from']}"]
    cl = doc["global"].get("character_list", {})
    lines.append(f"character list: current slot {cl.get('current_slot')}  "
                 f"prologue {cl.get('prologue_flags')}")
    for c in doc["characters"]:
        lst = c.get("list") or {}
        p, g = c.get("player", {}), c.get("game", {})
        lines.append("")
        lines.append(f"== slot {c['slot']} ({c['list_state']})  {lst.get('name', '')}  "
                     f"{lst.get('class', '')}  SL {p.get('soul_level', lst.get('soul_level'))}  "
                     f"NG+{(g.get('ng_cycle') or 1) - 1}  play {lst.get('play_time')}")
        if p:
            lines.append("  " + " ".join(f"{k[:3].upper()} {v}" for k, v in p["stats"].items()))
            lines.append(f"  souls {p['souls']}  soul memory {p['soul_memory']} / "
                         f"{p['soul_memory_2']}  HP {p['hp']}  covenant {p['covenant']}  "
                         f"discovered {', '.join(p['covenants_discovered']) or '-'}")
        if g:
            pos = ", ".join(f"{v:.1f}" for v in g["position"][:3])
            lines.append(f"  at {g['map']} ({pos})  location {lst.get('location')}  "
                         f"spells known {g['spell_table']['count']}")
        inv = c.get("inventory")
        if inv:
            lines.append(f"  inventory: {len(inv['bag'])} bag records, "
                         f"{len(inv['key_items_and_gestures'])} key items/gestures; active "
                         f"R{inv['active_right_slot']} L{inv['active_left_slot']}")
            for slot, e in inv["equipped"].items():
                if slot.startswith(("item", "gesture", "ammo")) or e.get("missing"):
                    continue
                lvl = f" +{e['level']}" if e.get("level") else ""
                inf = e.get("infusion", "None")
                inf = f" [{inf}]" if inf not in ("None", None) else ""
                lines.append(f"    {slot:<8} {e['name']}{lvl}{inf}")
            keys = [e["name"] for e in inv["key_items_and_gestures"]
                    if not 63000000 <= e["item"] < 64000000]
            if keys:
                lines.append(f"  key items: {', '.join(keys)}")
        ev = c.get("events")
        if ev:
            lit = [b for b in ev["bonfires"] if b["bit0"]]
            lines.append(f"  event flags: group10 {len(ev['flags_group10'])} set, group20 "
                         f"{len(ev['flags_group20'])} set, per-map blocks "
                         f"{sum(len(m['bits']) for m in ev['per_map_blocks_by_index'])} bits in "
                         f"{len(ev['per_map_blocks_by_index'])} maps, loaded "
                         f"{', '.join(m['map'] for m in ev['loaded_maps']) or '-'}")
            lines.append(f"  bonfires known {len(ev['bonfires'])}, bit0 set {len(lit)}; last rested "
                         f"{ev['last_bonfire']} ({ev['last_bonfire_id']})")
            lines.append("    " + "; ".join(b["name"] or str(b["id"]) for b in lit))
        if c["undecoded"]:
            lines.append("  undecoded: " + ", ".join(
                f"{u['class']} ({u['nonzero_bytes']} nonzero)" for u in c["undecoded"]))
    return "\n".join(lines)


def selftest() -> int:
    failed = 0

    def check(what, got, want):
        nonlocal failed
        ok = got == want
        failed += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {what}: {got!r}" + ("" if ok else f" want {want!r}"))

    # 0x140474230: byte (id % 10000) >> 3, mask 0x80 >> (id & 7).
    block = bytearray(EV_MAP_BLOCK)
    block[0] = 0x80
    block[1] = 0x01
    check("bit order is MSB first", set_bits(bytes(block)), [0, 15])
    check("Majula's group", map_group(0x0A040000), 10400)
    check("Drangleic Castle's group", map_group(0x14150000), 22100)
    check("flag ids", flag_ids(10400, bytes(block)), [104000000, 104000015])
    check("map name", map_name(0x0A040000), "m10_04_00_00")
    check("play time", playtime(4595), "1:16:35")
    # Every offset in the event block stays inside the 0x21C8 section.
    check("event tail inside section", EV_TIMERS + 32 <= SECTION_SIZE[8], True)
    check("loaded-map blocks end before bonfires",
          EV_LOADED_AT + EV_LOADED_COUNT * EV_LOADED_STRIDE <= EV_BONFIRE_IDS, True)
    check("inventory equipped array inside section",
          INV_EQUIPPED_AT + 2 * INV_EQUIPPED_COUNT <= SECTION_SIZE[10], True)
    hdr = struct.pack("<III", 0, 0x6F, 4).ljust(SECTION_HEADER, b"\0") + b"abcd"
    end = struct.pack("<III", SECTION_END, 0x102, 0).ljust(SECTION_HEADER, b"\0")
    stream = hdr + end
    got = [(s["type"], s["data"]) for s in sections(struct.pack("<I", len(stream)) + stream)]
    check("section chain", got, [(0, b"abcd"), (13, b"")])
    print("ds2-sl2-dump.py selftest:", "OK" if not failed else f"{failed} FAILED")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("save", nargs="?")
    ap.add_argument("--slot", type=int, help="only this character slot (0-9)")
    ap.add_argument("--json", action="store_true", help="print everything as JSON")
    ap.add_argument("--out", help="write the JSON here instead of stdout")
    ap.add_argument("--vanilla", action="store_true", help="vanilla DARK SOULS II key")
    ap.add_argument("--text-dir", type=Path,
                    help=f"directory holding itemname/bonfirename/mapname .fmg (default "
                         f"{TEXT_CACHE}, extracted from the game install on first use)")
    ap.add_argument("--no-extract", action="store_true",
                    help="do not run ds2-ebl.py to pull the FMGs; fall back to items.tsv")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if not a.save:
        ap.error("a save file is required unless --selftest is given")
    sl2 = _load("ds2_sl2", "ds2-sl2.py")
    names = Names(a.text_dir, extract=not a.no_extract)
    doc = decode(a.save, sl2.VANILLA_KEY_HEX if a.vanilla else sl2.KEY_HEX, names, a.slot)
    if a.json or a.out:
        text = json.dumps(doc, indent=1, default=str)
        if a.out:
            Path(a.out).write_text(text)
            print(f"wrote {a.out}")
        else:
            print(text)
        return 0
    print(summary(doc))
    return 0


if __name__ == "__main__":
    sys.exit(main())
