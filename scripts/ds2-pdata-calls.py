#!/usr/bin/env python3
"""Direct-call xrefs and containing functions for the memory-layout image, using its own .pdata.

    python3 scripts/ds2-pdata-calls.py func <va> [<va> ...]     containing .pdata function
    python3 scripts/ds2-pdata-calls.py callers <va> [<va> ...]  E8/E9 rel32 sites landing on VA
    python3 scripts/ds2-pdata-calls.py ptrs <va> [<va> ...]     8-byte absolute pointers to VA

WHY: `ds2-xrefs.py` finds every 4-byte window satisfying the RIP-relative equation and so reports
data and misaligned windows as candidates; `pe-call-xrefs.py` needs a file-layout PE and is wrong
on `darksoulsii-deobf.bin` (see its docstring). This reads the exception directory straight out of
the memory-layout image (file offset == RVA), keeps only windows whose preceding byte is an E8/E9
opcode, and names the .pdata function each site sits in. A hit is still a candidate: an E8 byte
inside another instruction can satisfy the equation. Disassemble before relying on one.
Vtable slots and function-pointer tables are not calls; `ptrs` finds those.
"""

from __future__ import annotations

import bisect
import struct
import sys
from pathlib import Path

import numpy as np

IMAGE_BASE = 0x1_4000_0000
IMAGE_PATH = Path(__file__).resolve().parents[1] / "darksoulsii-deobf.bin"

DATA = IMAGE_PATH.read_bytes()
_PE = struct.unpack_from("<I", DATA, 0x3C)[0]
#: PE32+ optional header starts 24 bytes into the NT headers; data directories at +112.
_EXC_RVA, _EXC_SIZE = struct.unpack_from("<II", DATA, _PE + 24 + 112 + 3 * 8)
FUNCS = sorted(
    struct.unpack_from("<II", DATA, _EXC_RVA + i) for i in range(0, _EXC_SIZE - 11, 12)
)
STARTS = [f[0] for f in FUNCS]


def func_of(va: int) -> tuple[int, int] | None:
    rva = va - IMAGE_BASE
    i = bisect.bisect_right(STARTS, rva) - 1
    if i >= 0 and FUNCS[i][0] <= rva < FUNCS[i][1]:
        return IMAGE_BASE + FUNCS[i][0], IMAGE_BASE + FUNCS[i][1]
    return None


def callers(target: int) -> list[tuple[int, str]]:
    rva = target - IMAGE_BASE
    out = []
    for shift in range(4):
        buf = DATA[1 + shift :]
        disp = np.frombuffer(buf[: len(buf) // 4 * 4], dtype="<i4").astype(np.int64)
        pos = np.arange(len(disp), dtype=np.int64) * 4 + 1 + shift
        for h in np.nonzero(disp + pos + 4 == rva)[0]:
            p = int(pos[h])
            op = DATA[p - 1]
            if op in (0xE8, 0xE9):
                out.append((IMAGE_BASE + p - 1, "call" if op == 0xE8 else "jmp"))
    return sorted(out)


def ptrs(target: int) -> list[int]:
    out = []
    for shift in range(8):
        buf = DATA[shift:]
        q = np.frombuffer(buf[: len(buf) // 8 * 8], dtype="<u8")
        out += [IMAGE_BASE + shift + int(h) * 8 for h in np.nonzero(q == np.uint64(target))[0]]
    return sorted(out)


def main() -> int:
    if len(sys.argv) < 3 or sys.argv[1] not in ("func", "callers", "ptrs"):
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        return 2
    cmd = sys.argv[1]
    for v in (int(a, 16) for a in sys.argv[2:]):
        if cmd == "func":
            f = func_of(v)
            print(hex(v), "->", f and f"{f[0]:#x}..{f[1]:#x}")
        elif cmd == "callers":
            print(f"== callers of {v:#x}")
            for site, kind in callers(v):
                f = func_of(site)
                print(f"  {kind} @ {site:#x}  in {f and hex(f[0])}")
        else:
            print(f"== pointers to {v:#x}")
            for p in ptrs(v):
                print(f"  {p:#x}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
