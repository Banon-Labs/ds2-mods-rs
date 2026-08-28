#!/usr/bin/env python3
"""Map MSVC RTTI class names to their vtable addresses in the flat DS2 image.

DS2 SOTFS ships 5258 RTTI type descriptors, so class identification here is READING NAMES rather
than inferring vtable shape -- but Ghidra only surfaces that when it has already applied the RTTI
analyzer to the address you happen to be looking at. This does the reverse lookup the whole way
round: name -> COMPLETE_OBJECT_LOCATOR -> vtable VA, for every class at once, offline, in a second.

    scripts/ds2-rtti-vtables.py 'FeLayoutScene|FexScene'     # regex over class names
    scripts/ds2-rtti-vtables.py --slot 0x190 'FeLayoutScene' # also print that vtable slot
    scripts/ds2-rtti-vtables.py --owner 0x141957660          # which vtable holds this address?

The image is a FLAT dump (`darksoulsii-deobf.bin`), so file offset == RVA and VA == 0x140000000 +
offset. That is the only reason a 4-byte RVA can be compared against a file offset directly.

A class can have SEVERAL vtables -- one per base under multiple inheritance -- and they are printed
in address order. The first is the primary (offset-0) one unless the COL's `offset` field says
otherwise, which is printed alongside.
"""

from __future__ import annotations

import argparse
import re
import struct
import sys
from pathlib import Path

BASE = 0x1_4000_0000
TEXT_LO = BASE + 0x1000
TEXT_HI = BASE + 0x1D0_0000
IMAGE = Path(__file__).resolve().parent.parent / "darksoulsii-deobf.bin"

# `.?AV` = class, `.?AU` = struct. The two junk pointers before the name are the type_info vftable
# and a cached-name slot, hence the -0x10 to reach the descriptor's own start.
TYPE_DESCRIPTOR = re.compile(rb"\.\?A[VU][\w@?$]{2,300}\x00")
TYPE_DESCRIPTOR_HEADER = 0x10


def load(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as exc:
        sys.exit(f"cannot read {path}: {exc}")


def type_descriptors(image: bytes) -> dict[int, str]:
    """offset of each RTTI type descriptor -> its demangle-able name."""
    found: dict[int, str] = {}
    for match in TYPE_DESCRIPTOR.finditer(image):
        start = match.start() - TYPE_DESCRIPTOR_HEADER
        if start >= 0:
            found[start] = match.group(0)[:-1].decode("ascii", "replace")
    return found


def locators(image: bytes, descriptors: dict[int, str]) -> dict[int, tuple[str, int]]:
    """offset of each RTTICompleteObjectLocator -> (class name, this-pointer offset).

    The COL's `pTypeDescriptor` is a 4-byte RVA at +0x0c, and `signature` at +0x00 is 0 (x86) or
    1 (x64, where the trailing `pSelf` RVA exists). Requiring the signature is what keeps this from
    matching any old dword that happens to equal a descriptor's RVA.
    """
    out: dict[int, tuple[str, int]] = {}
    for off in range(0, len(image) - 24, 4):
        target = struct.unpack_from("<I", image, off + 12)[0]
        name = descriptors.get(target)
        if name is None:
            continue
        signature = struct.unpack_from("<I", image, off)[0]
        if signature in (0, 1):
            out[off] = (name, struct.unpack_from("<I", image, off + 4)[0])
    return out


def vtables(image: bytes, cols: dict[int, tuple[str, int]]) -> dict[str, list[tuple[int, int]]]:
    """class name -> [(vtable VA, this-offset)], sorted by address.

    A vtable is identified by its `[-8]` slot pointing at a COL and its `[0]` slot pointing into
    .text. Without the second test, any COL-shaped pointer in a data structure reads as a vtable.
    """
    out: dict[str, list[tuple[int, int]]] = {}
    for off in range(8, len(image) - 8, 8):
        pointer = struct.unpack_from("<Q", image, off - 8)[0]
        if not BASE <= pointer < BASE + len(image):
            continue
        entry = cols.get(pointer - BASE)
        if entry is None:
            continue
        first = struct.unpack_from("<Q", image, off)[0]
        if TEXT_LO <= first < TEXT_HI:
            out.setdefault(entry[0], []).append((BASE + off, entry[1]))
    for name in out:
        out[name].sort()
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("pattern", nargs="?", default=".", help="regex matched against class names")
    ap.add_argument("--slot", type=lambda s: int(s, 0), help="also print this byte offset into each vtable")
    ap.add_argument("--owner", type=lambda s: int(s, 0), help="print the vtable containing this VA and stop")
    ap.add_argument("--image", type=Path, default=IMAGE)
    args = ap.parse_args()

    image = load(args.image)
    table = vtables(image, locators(image, type_descriptors(image)))

    if args.owner is not None:
        # Walk down from the queried address to the nearest vtable start that covers it.
        starts = {va: (name, this) for name, entries in table.items() for va, this in entries}
        for va in sorted(starts, reverse=True):
            if va <= args.owner:
                name, this = starts[va]
                print(f"{name} vtable=0x{va:x} this+0x{this:x} slot=0x{args.owner - va:x}")
                return 0
        print("no vtable at or below that address")
        return 1

    pattern = re.compile(args.pattern)
    for name in sorted(table):
        if not pattern.search(name):
            continue
        for va, this in table[name]:
            line = f"{name}  vtable=0x{va:x}  this+0x{this:x}"
            if args.slot is not None:
                off = va - BASE + args.slot
                line += f"  [+0x{args.slot:x}]=0x{struct.unpack_from('<Q', image, off)[0]:x}"
            print(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
