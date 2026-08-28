#!/usr/bin/env python3
"""Read a DARK SOULS II `.fmg` text file.

    python3 scripts/ds2-ebl.py extract /menu/text/english/ingamemenu.fmg --out /tmp/ds2text
    python3 scripts/ds2-fmg.py /tmp/ds2text/ingamemenu.fmg --id 0x200f26 --id 0x200f2a
    python3 scripts/ds2-fmg.py /tmp/ds2text/*.fmg --grep 'quit|exit|desktop'

The FMG names are not guessable and are not listed anywhere in the archive -- a BHD5 stores only
path HASHES. They were recovered by taking every identifier-shaped string in the executable
(43720 of them) and testing `/menu/text/english/<name>.fmg` against the index, which found 26:

    bloodmessageconjunction bloodmessagesentence bloodmessageword bloodmessagewordcategory
    bofire bonfirename charamaking common dconlymessage detailedexplanation iconhelp ingamemenu
    ingamesystem itemname keyguide mapevent mapname npcmenu pluralselect prologue shop
    simpleexplanation titleflow titlemenu weapontype win32onlymessage

# The format, and the one field that is easy to get wrong

Version 1, little-endian, 32-bit offsets:

    +0x00  u8 0, u8 bigEndian, u8 version, u8 0
    +0x04  u32 file size
    +0x08  u32 1
    +0x0c  u32 group count
    +0x10  u32 string count
    +0x14  u32 offset of the string-offset table
    +0x18  u32 0
    +0x1c  groups: { u32 offsetIndex, u32 firstID, u32 lastID } x groupCount

**The group table starts at 0x1c, not 0x20.** Starting it at 0x20 puts its last entry four bytes
past the offset table and yields ids in the billions -- which reads like a real answer right up
until a string offset lands outside the file. `0x1c + groupCount * 12 == stringOffsetTable` is the
check that settles it, and this script makes it rather than assuming it.

Ids are NOT unique across files: `0x9ca4` is "Quit" in `win32onlymessage.fmg` and an empty entry in
`shop.fmg`. Anything that binds a caption by id should confirm the id resolves in exactly one.
"""

from __future__ import annotations

import argparse
import re
import struct
import sys
from pathlib import Path

GROUP_TABLE_OFFSET = 0x1C
GROUP_SIZE = 12


def parse(blob: bytes) -> dict[int, str]:
    """`{text id: string}`. Raises `SystemExit` if the header does not check out."""
    if len(blob) < 0x1C:
        raise SystemExit("too short to be an FMG")
    size, _one, groups, strings, offset_table, _zero = struct.unpack_from("<IIIIII", blob, 4)
    if size != len(blob):
        raise SystemExit(f"header says {size} bytes, file is {len(blob)}")
    end = GROUP_TABLE_OFFSET + groups * GROUP_SIZE
    if end != offset_table:
        raise SystemExit(
            f"group table {GROUP_TABLE_OFFSET:#x}..{end:#x} does not meet the string-offset "
            f"table at {offset_table:#x} -- the header layout is not the one this expects"
        )

    def read(at: int) -> str | None:
        if not at or at >= len(blob):
            return None
        out: list[str] = []
        while at + 1 < len(blob):
            (ch,) = struct.unpack_from("<H", blob, at)
            if ch == 0:
                break
            out.append(chr(ch))
            at += 2
        return "".join(out)

    out: dict[int, str] = {}
    for i in range(groups):
        index, first, last = struct.unpack_from("<3i", blob, GROUP_TABLE_OFFSET + i * GROUP_SIZE)
        for k, text_id in enumerate(range(first, last + 1)):
            if 0 <= index + k < strings:
                (at,) = struct.unpack_from("<I", blob, offset_table + (index + k) * 4)
                value = read(at)
                if value is not None:
                    out[text_id] = value
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+", type=Path)
    ap.add_argument("--id", dest="ids", action="append", type=lambda s: int(s, 0), default=[])
    ap.add_argument("--grep", help="regex matched against the strings")
    ap.add_argument("--max-length", type=int, default=0, help="skip strings longer than this")
    args = ap.parse_args()

    pattern = re.compile(args.grep, re.I) if args.grep else None
    for path in args.files:
        try:
            table = parse(path.read_bytes())
        except OSError as exc:
            print(f"{path}: {exc}", file=sys.stderr)
            continue
        rows = sorted(table.items())
        if args.ids:
            rows = [(k, table[k]) for k in args.ids if k in table]
        if pattern:
            rows = [(k, v) for k, v in rows if pattern.search(v)]
        if args.max_length:
            rows = [(k, v) for k, v in rows if len(v) <= args.max_length]
        if not rows and (args.ids or pattern):
            continue
        print(f"== {path.name} ({len(table)} strings) ==")
        for text_id, value in rows:
            print(f"  {text_id:#010x} ({text_id:>10})  {value!r}")
        for missing in (i for i in args.ids if i not in table):
            print(f"  {missing:#010x} ({missing:>10})  <absent>")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
