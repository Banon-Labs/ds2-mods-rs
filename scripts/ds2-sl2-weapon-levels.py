#!/usr/bin/env python3
"""Compare the inventory's saved levels between two decrypted character payloads.

    python3 scripts/ds2-sl2.py -x before/ DS2SOFS0000.co2     # before a session
    python3 scripts/ds2-sl2.py -x after/  DS2SOFS0000.co2     # after it saved
    python3 scripts/ds2-sl2-weapon-levels.py before/USER_DATA003.bin after/USER_DATA003.bin

Written to prove that a save made while `ds2-weapon-sync` has lowered the inventory keeps the real
levels. The save's inventory is one block (`ds2_rva::ITEM_INVENTORY_SAVE_BLOCK_OFFSET`): a 16-byte
record `{u32 item, u32, f32 durability, u8 level, u8 infusion, u16}` per bag entry index, 3840 of
them. Its place in the payload is found with `--anchor`, the hex of one record known to be in it
at `--anchor-index` (read from the running game, e.g. with scripts/frida/weapon-sync-save-block.js),
or given directly with `--block-offset`.

Prints every record whose item stayed the same and whose level byte changed, and exits 1 if there
is one. Read-only.
"""
from __future__ import annotations

import argparse
import struct
import sys

RECORDS = 3840
STRIDE = 16


def locate(data: bytes, args: argparse.Namespace) -> int:
    if args.block_offset is not None:
        return args.block_offset
    anchor = bytes.fromhex(args.anchor)
    at = data.find(anchor)
    if at < 0:
        raise SystemExit(f"anchor {args.anchor} not found")
    return at - args.anchor_index * STRIDE


def records(path: str, args: argparse.Namespace) -> tuple[int, dict[int, tuple[int, int]]]:
    data = open(path, "rb").read()
    base = locate(data, args)
    out = {}
    for k in range(RECORDS):
        if base + (k + 1) * STRIDE > len(data):
            break
        item = struct.unpack_from("<I", data, base + k * STRIDE)[0]
        level = data[base + k * STRIDE + 0x0C]
        if item not in (0, 0xFFFFFFFF):
            out[k] = (item, level)
    return base, out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("before")
    ap.add_argument("after")
    ap.add_argument("--anchor", default="50973100000100000000fa4305",
                    help="hex of one save record known to be in the block (default: entry 3 of the "
                         "character this was written against)")
    ap.add_argument("--anchor-index", type=int, default=3, help="that record's entry index")
    ap.add_argument("--block-offset", type=lambda s: int(s, 0), default=None,
                    help="the block's offset in the payload, instead of an anchor")
    args = ap.parse_args()
    b_base, before = records(args.before, args)
    a_base, after = records(args.after, args)
    print(f"block at +0x{b_base:x} before, +0x{a_base:x} after; records before={len(before)} after={len(after)}")
    print(f"records above +0 before: {sum(1 for v in before.values() if v[1] & 0x0F)}")
    changed = [(k, before[k], after[k]) for k in before
               if k in after and before[k][0] == after[k][0] and before[k][1] != after[k][1]]
    print(f"same item, level changed: {len(changed)}")
    for k, (item, lb), (_, la) in changed[:40]:
        print(f"  #{k} item={item} level 0x{lb:02x} -> 0x{la:02x}")
    moved = sum(1 for k in before if k in after and before[k][0] != after[k][0])
    gone = sum(1 for k in before if k not in after)
    new = sum(1 for k in after if k not in before)
    print(f"item changed: {moved} gone: {gone} new: {new}")
    return 1 if changed else 0


if __name__ == "__main__":
    sys.exit(main())
