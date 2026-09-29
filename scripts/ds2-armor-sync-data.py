#!/usr/bin/env python3
"""The regulation facts `ds2-armor-sync` is built on, read out of the shipped params.

    python3 scripts/ds2-armor-sync-data.py

What it checks, each from the decrypted regulation (`scripts/ds2-regulation.py`, through
`scripts/ds2-reinforce-max.py`'s loader):

1. The armour equip request and record carry the ArmorParam id, not the ItemParam id: the
   inventory notifier `0x1401b66a0` fills the request's item from `ItemParam +0x18`. This prints
   how many armour ItemParam rows have `+0x18 == item id - 10000000`, which is the mapping the
   crate uses to compare an inventory entry with its equipment record.
2. The naked pieces `0x14034a... 0x140349a80` falls back to for an empty armour slot
   (`11001100..11001103`) are ArmorParam rows.
3. How defense moves with reinforcement. The physical defense read `0x14034dbb0(row, level, type)`
   (called per armour piece by `PlayerGameParamCalculator` `0x140380070`, with `level` and `row`
   from `ChrAsmEquip +0x2a8 / +0x2b0 + piece * 0x30`, via `0x1403486b0`) answers
   `base + (max - base) * clamp(level / maxLevel, 0, 1)`: type 0/1/2 read base `+0x00/+0x04/+0x08`
   and max `+0x30/+0x34/+0x38`, type 3 and -1 read `+0x0c/+0x3c`, and maxLevel is the `u8` at
   `+0x60` (disassembly `0x14034dbee..0x14034dd99`). The elemental cut (`0x140381350` ->
   `0x14034dda0`) lerps the same way from `+0x0c..+0x2c` to `+0x3c..+0x5c`. This prints the four
   physical values for a few pieces at +0, the midpoint and the max, and the spread of max levels
   (+10 and +5 pieces both exist, like weapons).

Exit status 1 if (1) or (2) does not hold, so it can be re-run as a check.
"""

from __future__ import annotations

import importlib.util
import struct
import sys
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

ARMOR_ITEM_ID_OFFSET = 10_000_000
NAKED = range(11001100, 11001104)
SAMPLES = [21030102, 25060101, 21330100]
#: (name, base offset, max offset), in `0x14034dbb0`'s type order 0, 1, 2, 3.
PHYSICAL = [
    ("slash", 0x00, 0x30),
    ("strike", 0x04, 0x34),
    ("thrust", 0x08, 0x38),
    ("standard", 0x0C, 0x3C),
]
ARP_MAX_LEVEL = 0x60


def load_tables():
    spec = importlib.util.spec_from_file_location(
        "ds2_reinforce_max", REPO / "scripts/ds2-reinforce-max.py"
    )
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ds2_reinforce_max"] = mod
    spec.loader.exec_module(mod)
    return mod, mod.Tables(None)


def i32(row: bytes, off: int) -> int:
    return struct.unpack_from("<i", row, off)[0]


def f32(row: bytes, off: int) -> float:
    return struct.unpack_from("<f", row, off)[0]


def defense(row: bytes, level: int) -> str:
    top = row[ARP_MAX_LEVEL]
    frac = min(max(level / top, 0.0), 1.0) if top else 0.0
    return " ".join(
        f"{name}={f32(row, base) + (f32(row, high) - f32(row, base)) * frac:.1f}"
        for name, base, high in PHYSICAL
    )


def main() -> int:
    mod, t = load_tables()
    ok = True
    armour = [(iid, i32(row, 0x18)) for iid, row in t.item.items() if i32(row, 0x18) != -1]
    mapped = Counter(aid == iid - ARMOR_ITEM_ID_OFFSET for iid, aid in armour)
    print(f"ItemParam rows naming an ArmorParam id: {len(armour)}; +0x18 == item - 10000000: {dict(mapped)}")
    ok &= mapped.get(False, 0) == 0
    naked = [aid in t.armor for aid in NAKED]
    print(f"naked pieces {NAKED.start}..{NAKED.stop - 1} are ArmorParam rows: {naked}")
    ok &= all(naked)
    print(f"ArmorReinforceParam max level (+0x60): {dict(Counter(r[ARP_MAX_LEVEL] for r in t.arp.values()))}")
    names = mod.read_items()
    for iid in SAMPLES:
        aid = i32(t.item[iid], 0x18)
        row = t.arp.get(aid)
        if row is None:
            print(f"{iid}: armor {aid} has no ArmorReinforceParam row")
            continue
        top = row[ARP_MAX_LEVEL]
        print(f"{iid} {names.get(iid, '?')} (armor {aid}, max +{top})")
        for level in sorted({0, top // 2, top}):
            print(f"  +{level}: {defense(row, level)}")
    print("verdict:", "holds" if ok else "DOES NOT HOLD")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
