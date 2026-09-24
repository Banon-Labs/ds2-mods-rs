#!/usr/bin/env python3
"""Which definitions instantiate a given definition, and with what transform.

`ds2-flo.py tree` walks DOWN from a definition. This walks UP: it answers "where is this thing
placed, and by whom", which is the question a layout offset actually depends on. The record fields
are the ones `ds2-flo.py` documents -- `rec+0x00` definition index, `rec+0x08` transform block,
`rec+0x12` kind, `rec+0x1c` element id -- read here rather than imported so this stays a single
file that can be run against any `.flo`.

    scripts/ds2-flo-parents.py /tmp/menu02/l02_03_equipment.flo 0x75
"""
import struct
import sys
from pathlib import Path

DEF_TABLE_FIELD = 0x18
DEF_COUNT_FIELD = 0x4C
DEF_STRIDE = 0x48
DEF_CHILD_COUNT = 0x02
DEF_CHILDREN = 0x08
RECORD_STRIDE = 0x28
RECORD_TRANSFORM = 0x08
RECORD_KIND = 0x12
RECORD_ID = 0x1C
KIND_NESTED = 0x4


def parents(blob: bytes, want: int) -> list[str]:
    table = struct.unpack_from("<Q", blob, DEF_TABLE_FIELD)[0]
    count = struct.unpack_from("<I", blob, DEF_COUNT_FIELD)[0]
    found = []
    for i in range(count):
        at = table + i * DEF_STRIDE
        if at + DEF_STRIDE > len(blob):
            continue
        index, children = struct.unpack_from("<HH", blob, at)
        array = struct.unpack_from("<Q", blob, at + DEF_CHILDREN)[0]
        if array + children * RECORD_STRIDE > len(blob):
            continue
        for slot in range(children):
            record = array + slot * RECORD_STRIDE
            names = struct.unpack_from("<H", blob, record)[0]
            kind = struct.unpack_from("<H", blob, record + RECORD_KIND)[0]
            if names != want or not kind & KIND_NESTED:
                continue
            transform = struct.unpack_from("<Q", blob, record + RECORD_TRANSFORM)[0]
            if transform + 0x10 > len(blob):
                continue
            x, y, scale_x, scale_y = struct.unpack_from("<4f", blob, transform)
            element = struct.unpack_from("<I", blob, record + RECORD_ID)[0]
            found.append(
                f"def {index:#06x} child[{slot}] -> {want:#06x} "
                f"xy=({x:g},{y:g}) scale=({scale_x:g},{scale_y:g}) id={element:#010x}"
            )
    return found


def main() -> int:
    if len(sys.argv) != 3:
        sys.exit(f"usage: {sys.argv[0]} <file.flo> <definition index>")
    blob = Path(sys.argv[1]).read_bytes()
    lines = parents(blob, int(sys.argv[2], 0))
    if not lines:
        print(f"nothing instantiates {int(sys.argv[2], 0):#06x} -- it is a root, or the index is wrong")
    for line in lines:
        print(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
