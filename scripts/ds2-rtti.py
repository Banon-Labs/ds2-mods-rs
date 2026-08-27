#!/usr/bin/env python3
"""Locate MSVC RTTI type descriptors and vtables for named C++ classes, without Ghidra.

    python3 scripts/ds2-rtti.py FeSubStateTitleLogo FeSubStateBase

Why this exists alongside `scripts/ghidra/query.sh`: Ghidra headless takes an exclusive lock on
the project directory, so only one query can run at a time, and a full analysis pass costs
minutes. DS2 carries 5271 MSVC RTTI type descriptors (measured), which means class identification
is a *string search followed by three pointer hops* -- no disassembly required. For "where is
class X's vtable", this answers in under a second and can run while Ghidra is busy.

It reads `darksoulsii-deobf.bin`, the dearxan-deobfuscated flat image. Flat means file offset ==
RVA and VA == 0x140000000 + offset; the script asserts the file size matches the PE SizeOfImage
rather than trusting that.

CAVEAT, the same one that governs every static read in this repo: the deobfuscated image is not
the bytes that run. At the 286 Arxan-redirected functions the deobf image shows recovered code
where the live process has a stub. A vtable slot is data, not code, so the slot addresses here
are trustworthy; whether the FUNCTION at a slot is Arxan-redirected must be checked separately
before hooking it.
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

IMAGE_BASE = 0x1_4000_0000
SIZE_OF_IMAGE = 0x1D76000
REPO_ROOT = Path(__file__).resolve().parents[1]
IMAGE_PATH = REPO_ROOT / "darksoulsii-deobf.bin"

#: An x64 RTTICompleteObjectLocator starts with signature == 1 and holds the type descriptor RVA
#: at +12. Both facts are what separate a real COL from a coincidental 4-byte match.
COL_SIGNATURE = 1
COL_TYPE_DESCRIPTOR_OFFSET = 12
#: A type descriptor is { void* vftable; void* spare; char name[] }, so the name is 16 bytes in.
TYPE_DESCRIPTOR_NAME_OFFSET = 16
#: Stop counting a vtable at the first slot that is not a plausible in-image pointer.
MAX_VIRTUALS = 256


def find_all(buf: bytes, needle: bytes, aligned: int = 1) -> list[int]:
    out, i = [], 0
    while (i := buf.find(needle, i)) >= 0:
        if i % aligned == 0:
            out.append(i)
        i += 1
    return out


def vtable_length(data: bytes, vt_off: int) -> int:
    """Slots until one stops looking like a pointer into the image.

    This over-counts when unrelated pointer data follows a vtable, so treat it as an upper
    bound worth eyeballing, not a measurement.
    """
    n = 0
    while n < MAX_VIRTUALS:
        end = vt_off + n * 8
        if end + 8 > len(data):
            break
        ptr = struct.unpack_from("<Q", data, end)[0]
        if not IMAGE_BASE <= ptr < IMAGE_BASE + len(data):
            break
        n += 1
    return n


def locate(data: bytes, name: str) -> list[dict]:
    """Every (type descriptor, COL, vtable) triple for a class name."""
    mangled = f".?AV{name}@@".encode()
    results = []
    for name_off in find_all(data, mangled):
        td_off = name_off - TYPE_DESCRIPTOR_NAME_OFFSET
        if td_off < 0:
            continue
        # Type descriptors are referenced by 32-bit RVA from the COL.
        for col_ref in find_all(data, struct.pack("<I", td_off), aligned=4):
            col_off = col_ref - COL_TYPE_DESCRIPTOR_OFFSET
            if col_off < 0 or col_off + 4 > len(data):
                continue
            if struct.unpack_from("<I", data, col_off)[0] != COL_SIGNATURE:
                continue
            col_va = IMAGE_BASE + col_off
            # vtable[-1] is the COL pointer, so the vtable proper begins 8 bytes after it.
            for slot in find_all(data, struct.pack("<Q", col_va), aligned=8):
                vt_off = slot + 8
                results.append(
                    {
                        "type_descriptor": IMAGE_BASE + td_off,
                        "col": col_va,
                        "vtable": IMAGE_BASE + vt_off,
                        "virtuals": vtable_length(data, vt_off),
                    }
                )
    return results


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    data = IMAGE_PATH.read_bytes()
    if len(data) != SIZE_OF_IMAGE:
        # A mismatch means this is not the flat image the offset arithmetic assumes.
        print(
            f"{IMAGE_PATH} is {len(data)} bytes, expected SizeOfImage {SIZE_OF_IMAGE}; "
            "offsets would not be RVAs",
            file=sys.stderr,
        )
        return 1

    for name in argv[1:]:
        hits = locate(data, name)
        if not hits:
            print(f"{name}: no RTTI type descriptor")
            continue
        for hit in hits:
            print(
                f"{name}: vtable=0x{hit['vtable']:x} virtuals={hit['virtuals']} "
                f"col=0x{hit['col']:x} td=0x{hit['type_descriptor']:x}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
