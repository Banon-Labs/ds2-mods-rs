#!/usr/bin/env python3
"""Decode a DARK SOULS II `.flo` frontend layout, from the game's own loader.

    scripts/ds2-flo.py tree /tmp/menu02/l02_01_In-Game.flo --def 0x263
    scripts/ds2-flo.py find /tmp/menu02/l02_01_In-Game.flo --id 0x1eacc9
    scripts/ds2-flo.py defs /tmp/menu02/l02_01_In-Game.flo

Get the file first -- `scripts/ds2-ebl.py extract /menu/02.febnd.dcx --out /tmp/menu02` -- which
also names the trap: the PAUSE MENU is `/menu/02.febnd.dcx` (`l02_01_In-Game.flo`), not
`/menu/42.febnd.dcx`, which is the OPTIONS screen and shares none of its ids.

# Where the format comes from

Not pattern-matching. Every field below is read off the code that reads it, and the function is
named beside it so the claim can be checked:

  `FUN_140b54740(doc, index)`   the definition lookup: linear scan of `[doc+0x18]`, `[doc+0x4c]`
                                entries, STRIDE 0x48, key = the `u16` at `+0x00`.
  `FUN_140b50f20(.., DEF, ..)`  reads `DEF+0x02` as the child count and `DEF+0x08` as the child
                                record array, and walks it at STRIDE 0x28.
  `FUN_140b50bc0(.., rec, ..)`  reads a record: `rec+0x00` definition index, `rec+0x08` transform
                                block, `rec+0x12` kind flags, `rec+0x14`/`+0x16` frame range,
                                `rec+0x1c` the ELEMENT ID.
  `FUN_140b6bd80(parent, ..)`   attaches the built component, and bounds the display list by
                                `[parent+0x48]+0x02` -- the parent DEFINITION's child count. So the
                                child count is also the CAPACITY: a fourth row needs it raised.
  `FUN_140b6a130`               `FeComponentObject::findByIdPath`, which matches `[this+0x48]+0x1c`
                                against one path component -- i.e. the id below IS what a scene
                                path resolves against.

The header is the document object itself, loaded in place: `doc+0x18` and the file's `+0x18` hold
the same `0xb620`, and `doc+0x4c` and the file's `+0x4c` the same count. The `u64` fields that are
file offsets on disk are absolute pointers once loaded -- which is why the runtime patch in
`ds2-menu-row` copies them rather than recomputing them.

# What this does NOT decode

The leaf payloads. `rec+0x12` selects one of four tables (`FUN_140b54700/54740/54780/547c0`) and
only the 0x4 case -- a nested definition -- is walked here. Shapes, textures and text fields are
printed as leaves with their definition index and nothing more. Nothing in this repo has needed
them: a new ROW reuses an existing row's definition, so its contents are the game's own.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

#: `[doc+0x18]`, the definition table's file offset. Read from the header rather than assumed --
#: `FUN_140b54740` dereferences this exact field.
DEF_TABLE_OFFSET_FIELD = 0x18
#: `[doc+0x4c]`, `u16`, how many definitions the table holds.
DEF_COUNT_FIELD = 0x4C
#: Stride of one definition. `FUN_140b54740`: `add rcx, 0x48`.
DEF_STRIDE = 0x48
#: Stride of one child record. `FUN_140b50f20`: `lVar8 = lVar8 + 0x28`.
RECORD_STRIDE = 0x28
#: Size of one transform block, from the spacing of the blocks the records point at.
TRANSFORM_SIZE = 0x30

#: `rec+0x12`, the kind flags `FUN_140b50bc0` switches on -- in this order, first match wins.
#: It is a FLAG WORD, not an enum: `FUN_140b50bc0` masks it with `0xd` and records carrying `0x1004`
#: exist, so bits above the low nibble mean something this has not decoded. Only `NESTED` is walked.
KIND_SHAPE = 0x1
KIND_TEXT = 0x2
KIND_NESTED = 0x4
KIND_TEXTURE = 0x8


class Flo:
    """A parsed `.flo`, addressed the way the loader addresses it."""

    def __init__(self, blob: bytes) -> None:
        self.blob = blob
        self.def_table = struct.unpack_from("<Q", blob, DEF_TABLE_OFFSET_FIELD)[0]
        self.def_count = struct.unpack_from("<H", blob, DEF_COUNT_FIELD)[0]
        if not 0 < self.def_table < len(blob):
            raise SystemExit(f"definition table offset {self.def_table:#x} is outside the file")
        self.defs: dict[int, int] = {}
        for i in range(self.def_count):
            off = self.def_table + i * DEF_STRIDE
            if off + DEF_STRIDE > len(blob):
                break
            self.defs.setdefault(struct.unpack_from("<H", blob, off)[0], off)

    def definition(self, index: int) -> tuple[int, int, int] | None:
        """`(table offset, child count, child array offset)` for a definition index."""
        off = self.defs.get(index)
        if off is None:
            return None
        count = struct.unpack_from("<H", self.blob, off + 0x02)[0]
        array = struct.unpack_from("<Q", self.blob, off + 0x08)[0]
        return off, count, array

    def record(self, offset: int) -> dict[str, int | float]:
        """One 0x28-byte child record, plus the transform block it points at."""
        b = self.blob
        definition, capacity = struct.unpack_from("<HH", b, offset)
        transform = struct.unpack_from("<Q", b, offset + 0x08)[0]
        depth, kind = struct.unpack_from("<HH", b, offset + 0x10)
        last_frame, first_frame = struct.unpack_from("<HH", b, offset + 0x14)
        element_id = struct.unpack_from("<I", b, offset + 0x1C)[0]
        x, y, scale_x, scale_y = struct.unpack_from("<4f", b, transform)
        colour = struct.unpack_from("<I", b, transform + 0x18)[0]
        return {
            "offset": offset,
            "definition": definition,
            "capacity": capacity,
            "transform": transform,
            "depth": depth,
            "kind": kind,
            "first_frame": first_frame,
            "last_frame": last_frame,
            "id": element_id,
            "x": x,
            "y": y,
            "scale_x": scale_x,
            "scale_y": scale_y,
            "colour": colour,
        }

    def children(self, index: int) -> list[dict[str, int | float]]:
        found = self.definition(index)
        if found is None:
            return []
        _, count, array = found
        return [self.record(array + i * RECORD_STRIDE) for i in range(count)]


def render(rec: dict[str, int | float]) -> str:
    return (
        f"rec@{rec['offset']:#08x} def={rec['definition']:#06x} id={rec['id']:#08x} "
        f"xy=({rec['x']:g},{rec['y']:g}) depth={rec['depth']} kind={rec['kind']:#x} "
        f"frames={rec['first_frame']}..{rec['last_frame']} colour={rec['colour']:08x} "
        f"xform={rec['transform']:#08x}"
    )


def tree(flo: Flo, index: int, indent: int = 0, seen: frozenset[int] = frozenset()) -> None:
    if index in seen:
        print(" " * indent + f"def {index:#06x} (already shown)")
        return
    found = flo.definition(index)
    if found is None:
        print(" " * indent + f"def {index:#06x} MISSING")
        return
    off, count, array = found
    print(" " * indent + f"def {index:#06x} @{off:#08x} children={count} array={array:#08x}")
    for i, rec in enumerate(flo.children(index)):
        print(" " * (indent + 2) + f"[{i}] {render(rec)}")
        if rec["kind"] & KIND_NESTED:
            tree(flo, int(rec["definition"]), indent + 6, seen | {index})


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("action", choices=("tree", "find", "defs", "header"))
    ap.add_argument("file", type=Path)
    ap.add_argument("--def", dest="definition", type=lambda s: int(s, 0), help="definition index")
    ap.add_argument("--id", dest="element", type=lambda s: int(s, 0), help="element id to locate")
    args = ap.parse_args()

    try:
        flo = Flo(args.file.read_bytes())
    except OSError as exc:
        sys.exit(f"cannot read {args.file}: {exc}")

    if args.action == "header":
        for off in range(0x08, 0x48, 8):
            print(f"  hdr+{off:#04x} = {struct.unpack_from('<Q', flo.blob, off)[0]:#08x}")
        print(f"  definitions: {flo.def_count} at {flo.def_table:#08x}, stride {DEF_STRIDE:#x}")
        return 0

    if args.action == "defs":
        for index in sorted(flo.defs):
            off, count, array = flo.definition(index)  # type: ignore[misc]
            print(f"def {index:#06x} @{off:#08x} children={count} array={array:#08x}")
        return 0

    if args.action == "find":
        if args.element is None:
            sys.exit("find needs --id")
        # Every definition's child array, so the answer names the PARENT -- which is the thing a
        # new sibling has to be added to.
        for index in sorted(flo.defs):
            for i, rec in enumerate(flo.children(index)):
                if rec["id"] == args.element:
                    print(f"def {index:#06x} child[{i}] {render(rec)}")
        return 0

    if args.definition is None:
        sys.exit("tree needs --def")
    tree(flo, args.definition)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
