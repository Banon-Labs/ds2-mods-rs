#!/usr/bin/env python3
"""Write generated builds into a DARK SOULS II: SOTFS `.sl2`, one character slot per build.

    python3 scripts/ds2-sl2-builds.py --save MULE.zip --base-slot 1 --builds BUILDS.json -o OUT.sl2 \\
        --steam-id-from ~/.../DarkSoulsII/<id>/DS2SOFS0000.co2
    python3 scripts/ds2-sl2-builds.py --selftest      # offsets, soul memory, record choice; no save

Offline: nothing here talks to the game. Every target slot starts as a byte copy of `--base-slot`
(its two per-character entries and its character-list record), then gets the build written over
it. The base is a mule: a character holding most of the game's items at full upgrade, so a build
is EQUIPPED by pointing the equip array at records the bag already holds, never by inventing one.

WHAT IS WRITTEN, and where (layout: `scripts/ds2-sl2-dump.py`'s docstring, each field STATIC there):

    player  (type 0, USER_DATA(slot+1))   +0x00 9 u16 stats, +0x18 u16 soul level,
                                          +0x20 u32 soul memory, +0x24 u32 second soul memory,
                                          +0x28 u32 HP
    game    (type 1, USER_DATA(slot+1))   +0x24 wchar[32] name, +0x64 u32 class
    bag     (type 10, USER_DATA(slot+1))  +0x10004 50 u16 equipped handles (index = internal
                                          slot), +0xF000 / +0xF001 the active right/left slot;
                                          a weapon record's infusion byte (+0x0D) when the bag
                                          holds the weapon only under another infusion -- the
                                          edit a blacksmith makes, on a copy nothing else wears
    list    (type 4, USER_DATA000 and USER_DATA022, record slot)
                                          +0x174 9 u16 stats, +0x18A wchar[32] name,
                                          +0x1D4 u16 soul level, +0x1D6 u16 class

SOUL MEMORY BEFORE LEVEL (user rule, `ds2-soul-memory-before-level`). The soul memory written is
the souls the levels cost: PlayerLevelUpSoulsParam rows `class level .. level-1`, read out of the
installed `enc_regulation.bnd.dcx` by `scripts/ds2-regulation.py`, never typed in. `write_slot`
computes it before it touches the level, and refuses a build whose level is not the game's own
`sum(stats) - 53` (measured 2026-10-21: the game recomputes the level from the stats that way).

THE CRYPTO is `ds2-sl2-rebind.py`'s: each changed entry is re-encrypted under its OWN IV and its
MD5 recomputed over `iv || ciphertext`. `--steam-id`/`--steam-id-from` also rewrites the owner's
Steam ID, which a foreign save needs before this account's game will load it.

VERIFY, DO NOT TRUST: the finished file is decrypted again and every written field read back.
A mismatch writes nothing.

NOT WRITTEN: the title screen's visual-equipment stream (list +0x0A4 / player +0x16C). The game
rewrites it from the equipped gear the first time the character saves; until then the title
preview shows the base slot's look.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import struct
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


sl2 = _load("ds2_sl2", "ds2-sl2.py")
rebind = _load("ds2_sl2_rebind", "ds2-sl2-rebind.py")
dump = _load("ds2_sl2_dump", "ds2-sl2-dump.py")

KEY = sl2.KEY_HEX
SECTION_HEADER = 0x20
T_PLAYER, T_GAME, T_LIST, T_BAG = 0, 1, 4, 10

# player
P_STATS, P_LEVEL, P_SOUL_MEMORY, P_SOUL_MEMORY_2, P_HP = 0x00, 0x18, 0x20, 0x24, 0x28
# game
G_NAME, G_CLASS = 0x24, 0x64
NAME_UNITS = 32
# character list
L_PROLOGUE, L_STRIDE = 16, 0x1F0
L_STATS, L_NAME, L_LEVEL, L_CLASS = 0x174, 0x18A, 0x1D4, 0x1D6
# bag
B_RECORD, B_RECORDS = 16, 0xF00
B_ACTIVE_RIGHT, B_ACTIVE_LEFT = 0xF000, 0xF001
B_EQUIPPED, B_EQUIPPED_COUNT, EMPTY = 0x10004, 50, 0xFFFF
R_ITEM, R_LEVEL, R_INFUSION = 0x0, 0xC, 0xD

#: Internal equip slot by name: the equip array's index (`ds2-sl2-equipped.py`, STATIC).
SLOTS = {name: i for i, name in enumerate(dump.EQUIP_SLOTS)}
STATS = dump.STAT_NAMES  # the game's order
CLASS_IDS = {v.lower(): k for k, v in dump.CLASSES.items()}
INFUSIONS = [i.lower() for i in dump.INFUSIONS]  # index = the record's infusion byte
#: The recommender's infusion spelling for the uninfused weapon.
INFUSION_ALIASES = {"no_infusion": "none", "no infusion": "none", "": "none"}
#: Item id ranges by kind (the first two decimal digits of the id; `items.tsv` agrees).
WEAPON_IDS = range(1_000_000, 12_000_000)
ARMOR_IDS = range(11_000_000, 30_000_000)
SPELL_IDS = range(31_000_000, 36_000_000)
RING_IDS = range(40_000_000, 50_000_000)
ANY_ITEM = range(0, 10**9)


def norm(name: str) -> str:
    return re.sub(r"[^a-z0-9+]", "", name.lower())


# --------------------------------------------------------------------------------------------
# Container
# --------------------------------------------------------------------------------------------


def read_container(path: Path) -> bytes:
    """The `.sl2` bytes, out of a zip when given one (the mule ships zipped, one member)."""
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as z:
            names = [n for n in z.namelist() if n.lower().endswith((".sl2", ".co2"))]
            if len(names) != 1:
                raise SystemExit(f"{path}: want exactly one .sl2/.co2 member, found {names}")
            return z.read(names[0])
    return path.read_bytes()


class Save:
    """A decrypted container: entry name -> mutable plaintext, resealed by `seal`."""

    def __init__(self, raw: bytes):
        if raw[:4] != b"BND4":
            raise SystemExit("not a BND4 container")
        self.raw = raw
        self.where = {n: (off, size) for _i, n, off, size in sl2.entries(raw)}
        self.plain: dict[str, bytearray] = {}
        self.dirty: set[str] = set()

    def entry(self, name: str) -> bytearray:
        if name not in self.plain:
            off, size = self.where[name]
            blob = self.raw[off:off + size]
            if hashlib.md5(blob[16:]).digest() != blob[:16]:
                raise SystemExit(f"{name}: MD5 does not check; refusing a damaged save")
            self.plain[name] = bytearray(rebind.crypt(blob[32:], blob[16:32], KEY, encrypt=False))
        return self.plain[name]

    def put(self, name: str, data: bytes) -> None:
        cur = self.entry(name)
        if len(data) != len(cur):
            raise SystemExit(f"{name}: {len(data)} bytes would replace {len(cur)}")
        cur[:] = data
        self.dirty.add(name)

    def touch(self, name: str) -> bytearray:
        self.dirty.add(name)
        return self.entry(name)

    def seal(self) -> bytes:
        out = bytearray(self.raw)
        for name in sorted(self.dirty):
            off, size = self.where[name]
            iv = bytes(self.raw[off + 16:off + 32])
            ct = rebind.crypt(bytes(self.plain[name]), iv, KEY, encrypt=True)
            if len(ct) != size - 32:
                raise SystemExit(f"{name}: re-encrypt changed length")
            out[off:off + 16] = hashlib.md5(iv + ct).digest()
            out[off + 32:off + size] = ct
        return bytes(out)


def section_offsets(payload: bytes) -> dict[int, int]:
    """Section type -> offset of its data within the payload (first of each type)."""
    length = struct.unpack_from("<I", payload, 0)[0]
    out, at = {}, 4
    while at + SECTION_HEADER <= 4 + length:
        kind, _ver, size = struct.unpack_from("<III", payload, at)
        if kind > 14:
            break
        out.setdefault(kind, at + SECTION_HEADER)
        if kind == 0xD:
            break
        at += SECTION_HEADER + size
    return out


def char_entry(slot: int) -> str:
    return f"USER_DATA{slot + 1:03d}"


def world_entry(slot: int) -> str:
    return f"USER_DATA{slot + 11:03d}"


LIST_ENTRIES = ("USER_DATA000", "USER_DATA022")


def list_record(save: Save, entry: str, slot: int) -> int:
    p = save.entry(entry)
    return section_offsets(p)[T_LIST] + L_PROLOGUE + slot * L_STRIDE


def put_name(buf: bytearray, at: int, name: str) -> None:
    units = name.encode("utf-16le")
    if len(units) // 2 >= NAME_UNITS:
        raise SystemExit(f"name {name!r} is longer than {NAME_UNITS - 1} characters")
    buf[at:at + NAME_UNITS * 2] = units + b"\0" * (NAME_UNITS * 2 - len(units))


# --------------------------------------------------------------------------------------------
# Soul memory
# --------------------------------------------------------------------------------------------


def soul_costs() -> dict[int, int]:
    """PlayerLevelUpSoulsParam: level -> souls to go from it to the next, from the installed game."""
    reg = _load("ds2_regulation", "ds2-regulation.py")
    members = reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
    return reg.souls_table(members)[1]


def soul_memory_for(costs: dict[int, int], start_level: int, level: int) -> int:
    """The souls a character of a class starting at `start_level` spent to reach `level`."""
    missing = [lv for lv in range(start_level, level) if lv not in costs]
    if missing:
        raise SystemExit(f"soul cost table has no row for level {missing[0]}")
    return sum(costs[lv] for lv in range(start_level, level))


# --------------------------------------------------------------------------------------------
# Choosing bag records
# --------------------------------------------------------------------------------------------


class Bag:
    """The base slot's bag records, by item id, with names from the game's own FMG."""

    def __init__(self, inv: bytes, names):
        self.records: dict[int, tuple[int, int, int]] = {}  # handle -> (item, level, infusion)
        for h in range(B_RECORDS):
            item = struct.unpack_from("<I", inv, h * B_RECORD + R_ITEM)[0]
            if item:
                self.records[h] = (item, inv[h * B_RECORD + R_LEVEL] & 0x0F, inv[h * B_RECORD + R_INFUSION])
        self.by_name: dict[str, set[int]] = {}
        for item, _, _ in self.records.values():
            self.by_name.setdefault(norm(names.item(item)[0]), set()).add(item)
        self.used: set[int] = set()

    def ids(self, name: str, kind: range) -> set[int]:
        return {i for i in self.by_name.get(norm(name), ()) if i in kind}

    def pick(self, name: str, kind: range, infusion: str | None = None) -> tuple[int, int | None]:
        """(handle, infusion byte to write or None) for the best unused record of `name`: the wanted
        infusion at the highest level, else the highest-level copy of the item to re-infuse."""
        ids = self.ids(name, kind)
        if not ids:
            raise LookupError(f"the base slot's bag holds no {name!r}")
        free = [(h, r) for h, r in self.records.items() if r[0] in ids and h not in self.used]
        if not free:
            raise LookupError(f"every {name!r} in the bag is already equipped")
        want = None if infusion is None else INFUSIONS.index(INFUSION_ALIASES.get(infusion.lower(), infusion.lower()))
        exact = [(h, r) for h, r in free if want is None or r[2] == want]
        pool = exact or free
        h, r = max(pool, key=lambda hr: (hr[1][1], -hr[0]))
        self.used.add(h)
        return h, (None if want is None or r[2] == want else want)


# --------------------------------------------------------------------------------------------
# Writing one slot
# --------------------------------------------------------------------------------------------


def copy_slot(save: Save, base: int, slot: int) -> None:
    if slot == base:
        return
    save.put(char_entry(slot), bytes(save.entry(char_entry(base))))
    save.put(world_entry(slot), bytes(save.entry(world_entry(base))))
    for e in LIST_ENTRIES:
        src, dst = list_record(save, e, base), list_record(save, e, slot)
        buf = save.touch(e)
        buf[dst:dst + L_STRIDE] = buf[src:src + L_STRIDE]


def slot_kind(slot_name: str) -> range:
    if slot_name[:2] in ("rh", "lh"):
        return WEAPON_IDS
    if slot_name.startswith("ring"):
        return RING_IDS
    if slot_name.startswith("spell"):
        return SPELL_IDS
    if slot_name.startswith(("item", "ammo")):
        return ANY_ITEM
    return ARMOR_IDS


def write_slot(save: Save, build: dict, costs: dict[int, int], names) -> dict:
    """Write `build` into its slot (already a copy of the base). Returns what was equipped."""
    slot = build["slot"]
    stats = [int(build["stats"][s]) for s in STATS]
    level = sum(stats) - 53
    if level != build["soul_level"]:
        raise SystemExit(f"slot {slot}: stats sum to level {level}, build says {build['soul_level']}")
    cls = CLASS_IDS[build["class"].lower()]
    # Soul memory is computed here, before anything below writes a level.
    memory = soul_memory_for(costs, int(build["class_level"]), level)

    char = save.touch(char_entry(slot))
    sec = section_offsets(char)
    p, g, inv = sec[T_PLAYER], sec[T_GAME], sec[T_BAG]
    struct.pack_into("<II", char, p + P_SOUL_MEMORY, memory, memory)
    struct.pack_into("<9H", char, p + P_STATS, *stats)
    struct.pack_into("<H", char, p + P_LEVEL, level)
    struct.pack_into("<I", char, p + P_HP, int(build["hp"]))
    put_name(char, g + G_NAME, build["name"])
    struct.pack_into("<I", char, g + G_CLASS, cls)
    for e in LIST_ENTRIES:
        r = list_record(save, e, slot)
        buf = save.touch(e)
        struct.pack_into("<9H", buf, r + L_STATS, *stats)
        put_name(buf, r + L_NAME, build["name"])
        struct.pack_into("<HH", buf, r + L_LEVEL, level, cls)

    bag = Bag(bytes(char[inv:inv + B_RECORDS * B_RECORD]), names)
    handles = list(struct.unpack_from(f"<{B_EQUIPPED_COUNT}H", char, inv + B_EQUIPPED))
    for i in range(SLOTS["gesture1"]):
        handles[i] = EMPTY  # the gear is the build's, gestures stay the base's
    equipped, problems = {}, []
    for slot_name, want in build["equip"].items():
        kind = slot_kind(slot_name)
        try:
            h, reinfuse = bag.pick(want["name"], kind, want.get("infusion") if kind is WEAPON_IDS else None)
        except LookupError as e:
            problems.append(f"{slot_name}: {e}")  # one missing piece does not cancel the rest
            continue
        if reinfuse is not None:
            char[inv + h * B_RECORD + R_INFUSION] = reinfuse
        handles[SLOTS[slot_name]] = h
        item, lv, inf = bag.records[h]
        equipped[slot_name] = {"handle": h, "item": item, "name": names.item(item)[0], "level": lv,
                               "infusion": INFUSIONS[reinfuse if reinfuse is not None else inf],
                               "reinfused": reinfuse is not None}
    struct.pack_into(f"<{B_EQUIPPED_COUNT}H", char, inv + B_EQUIPPED, *handles)
    char[inv + B_ACTIVE_RIGHT] = 0 if handles[SLOTS["rh1"]] != EMPTY else 0xFF
    char[inv + B_ACTIVE_LEFT] = 0 if handles[SLOTS["lh1"]] != EMPTY else 0xFF
    return {"slot": slot, "name": build["name"], "level": level, "soul_memory": memory,
            "equipped": equipped, "problems": problems}


# --------------------------------------------------------------------------------------------
# Verify
# --------------------------------------------------------------------------------------------


def verify(raw: bytes, builds: list[dict], reports: list[dict], steam_id: str | None) -> list[str]:
    """Re-read the sealed file with fresh decryption and compare every written field."""
    out, save = [], Save(raw)
    for name in save.where:
        off, size = save.where[name]
        blob = raw[off:off + size]
        if hashlib.md5(blob[16:]).digest() != blob[:16]:
            out.append(f"{name}: MD5 mismatch")
    for b, rep in zip(builds, reports):
        slot = b["slot"]
        char = save.entry(char_entry(slot))
        sec = section_offsets(char)
        p, g, inv = sec[T_PLAYER], sec[T_GAME], sec[T_BAG]
        stats = list(struct.unpack_from("<9H", char, p + P_STATS))
        if stats != [int(b["stats"][s]) for s in STATS]:
            out.append(f"slot {slot}: player stats read back {stats}")
        if struct.unpack_from("<H", char, p + P_LEVEL)[0] != rep["level"]:
            out.append(f"slot {slot}: level")
        if list(struct.unpack_from("<II", char, p + P_SOUL_MEMORY)) != [rep["soul_memory"]] * 2:
            out.append(f"slot {slot}: soul memory")
        if dump.wide(char, g + G_NAME) != b["name"]:
            out.append(f"slot {slot}: game name")
        for e in LIST_ENTRIES:
            buf = save.entry(e)
            r = list_record(save, e, slot)
            if dump.wide(buf, r + L_NAME) != b["name"] or list(struct.unpack_from("<9H", buf, r + L_STATS)) != stats:
                out.append(f"slot {slot}: {e} character-list record")
        handles = struct.unpack_from(f"<{B_EQUIPPED_COUNT}H", char, inv + B_EQUIPPED)
        for slot_name, e in rep["equipped"].items():
            h = handles[SLOTS[slot_name]]
            item = struct.unpack_from("<I", char, inv + h * B_RECORD)[0]
            if h != e["handle"] or item != e["item"]:
                out.append(f"slot {slot}: {slot_name} reads handle {h} item {item}")
    if steam_id:
        found = {h[3].lower() for h in rebind.find_ids(rebind.decrypt_all(raw, KEY))}
        if found != {steam_id.lower()}:
            out.append(f"Steam IDs after rebind: {sorted(found)}")
    return out


# --------------------------------------------------------------------------------------------


def selftest() -> int:
    costs = {1: 500, 2: 528, 3: 600}
    assert soul_memory_for(costs, 1, 1) == 0
    assert soul_memory_for(costs, 1, 3) == 1028  # level.rs's anchor: SM at level 3 is 1,028
    assert soul_memory_for(costs, 2, 4) == 1128
    assert SLOTS["rh1"] == 0 and SLOTS["lh1"] == 1 and SLOTS["head"] == 6 and SLOTS["ring1"] == 10
    assert SLOTS["spell1"] == 28 and SLOTS["gesture1"] == 42 and len(SLOTS) == B_EQUIPPED_COUNT
    assert INFUSIONS.index("raw") == 7 and INFUSIONS.index("none") == 0
    assert slot_kind("lh2") is WEAPON_IDS and slot_kind("ring4") is RING_IDS and slot_kind("legs") is ARMOR_IDS

    class N:
        def item(self, i):
            return ({1: "Uchigatana", 2: "Uchigatana", 3: "Life Ring+3"}[i // 1000], "x")

    inv = bytearray(B_RECORDS * B_RECORD)
    for h, (item, lv, inf) in {5: (1000, 10, 0), 6: (2000, 10, 7), 7: (1000, 5, 7), 9: (3000, 0, 0)}.items():
        struct.pack_into("<I", inv, h * B_RECORD, item)
        inv[h * B_RECORD + R_LEVEL], inv[h * B_RECORD + R_INFUSION] = lv, inf
    kinds = range(0, 10_000)
    bag = Bag(bytes(inv), N())
    assert bag.pick("Uchigatana", kinds, "Raw") == (6, None)       # exact infusion at +10 wins
    assert bag.pick("Uchigatana", kinds, "Raw") == (7, None)       # then the next exact one
    assert bag.pick("Uchigatana", kinds, "Fire") == (5, 1)         # then re-infuse the last copy
    bag = Bag(bytes(inv), N())
    assert bag.pick("life ring +3", kinds) == (9, None)            # names compare normalised
    try:
        bag.pick("Nothing", kinds)
        raise AssertionError("picked a missing item")
    except LookupError:
        pass
    print("selftest ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--save", type=Path, help="the base container: .sl2/.co2, or a zip holding one")
    ap.add_argument("--base-slot", type=int, help="the character every target slot starts from")
    ap.add_argument("--builds", type=Path, help="JSON: {\"builds\": [...]} (scripts/ds2-bracket-builds.py)")
    ap.add_argument("-o", "--out", type=Path)
    ap.add_argument("--steam-id", help="rebind the save to this SteamID64, 16 hex characters")
    ap.add_argument("--steam-id-from", type=Path, help="rebind to the Steam ID inside this save")
    ap.add_argument("--report", type=Path, help="also write what was equipped, per slot, as JSON")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if not (a.save and a.base_slot is not None and a.builds and a.out):
        ap.error("--save, --base-slot, --builds and -o are required")
    builds = json.loads(a.builds.read_text())["builds"]
    if len({b["slot"] for b in builds}) != len(builds):
        ap.error("two builds name the same slot")

    raw = read_container(a.save)
    save = Save(raw)
    states = {slot: state for slot, state, _attrs, _name in sl2.slot_records(raw, KEY)}
    if states.get(a.base_slot) == "empty":
        ap.error(f"base slot {a.base_slot} holds no character")
    names = dump.Names(None, extract=True)
    costs = soul_costs()

    reports = []
    for b in builds:
        copy_slot(save, a.base_slot, b["slot"])
        reports.append(write_slot(save, b, costs, names))

    steam_id = a.steam_id
    if a.steam_id_from:
        ids = {h[3] for h in rebind.find_ids(rebind.decrypt_all(a.steam_id_from.read_bytes(), KEY))}
        if len(ids) != 1:
            ap.error(f"{a.steam_id_from}: want one Steam ID, found {sorted(ids)}")
        steam_id = ids.pop()
    sealed = save.seal()
    if steam_id:
        sealed, _ = rebind.rebind(sealed, steam_id, KEY, rebind.decrypt_all(sealed, KEY))

    problems = verify(sealed, builds, reports, steam_id)
    for r in reports:
        print(f"slot {r['slot']}  {r['name']:16}  SL {r['level']:3}  soul memory {r['soul_memory']:,}")
        for s, e in r["equipped"].items():
            print(f"    {s:7} {e['name']} +{e['level']} {e['infusion']}{'  (re-infused)' if e['reinfused'] else ''}")
        for p in r["problems"]:
            print(f"    NOT EQUIPPED {p}")
    if problems:
        for p in problems:
            print(f"FAILED {p}", file=sys.stderr)
        return 1
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_bytes(sealed)
    if a.report:
        a.report.write_text(json.dumps(reports, indent=1))
    print(f"wrote {a.out}  (verified: MD5s check, every written field reads back"
          + (f", Steam ID {steam_id})" if steam_id else ")"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
