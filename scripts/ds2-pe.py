#!/usr/bin/env python3
"""Shared PE geometry for `darksoulsii-deobf.bin`: sections, `.pdata` function ranges, xrefs.

Imported by the other `scripts/ds2-*.py` helpers. Nothing here disassembles; it answers the two
questions a raw-image search cannot answer on its own:

* **Which function does this address live in?** `.pdata` is the PE exception table, a sorted array
  of `RUNTIME_FUNCTION { begin_rva, end_rva, unwind_rva }`. Every non-leaf x64 function in an MSVC
  binary has an entry, so a binary search over it turns "file offset 0x4fe7a3" into "inside the
  function at 0x1404fe760". That is what makes a raw byte scan into an xref list with owners.

* **Who references this address?** Two forms, kept separate because they mean different things:
  `riprefs` finds RIP-relative operands (`lea rax,[rip+d]` -- how a vtable or global is
  materialised), `callrefs` finds `e8`/`e9 rel32` (a direct call or tail-jump).

FALSE POSITIVES ARE POSSIBLE AND ARE NOT FILTERED. Nothing here decodes instructions, so a 4-byte
window that satisfies the displacement equation without being a displacement field will be
reported. Every hit is a candidate to disassemble, not a fact -- the same caveat that governs
`scripts/ds2-xrefs.py`. Attributing a hit to a `.pdata` owner is a strong filter in practice: a
coincidental match in the middle of a real function's body is rarer than one in data.
"""

from __future__ import annotations

import bisect
import struct
from pathlib import Path

import numpy as np

IMAGE_BASE = 0x1_4000_0000
REPO_ROOT = Path(__file__).resolve().parents[1]
IMAGE_PATH = REPO_ROOT / "darksoulsii-deobf.bin"


def load_image() -> bytes:
    if not IMAGE_PATH.exists():
        raise SystemExit(f"{IMAGE_PATH} missing -- see README.md for how to produce it")
    return IMAGE_PATH.read_bytes()


def sections(data: bytes) -> list[tuple[str, int, int, int, int]]:
    """[(name, virtual_address, virtual_size, raw_pointer, raw_size)]."""
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe : pe + 4] != b"PE\0\0":
        raise SystemExit("not a PE image")
    count = struct.unpack_from("<H", data, pe + 6)[0]
    opt_size = struct.unpack_from("<H", data, pe + 20)[0]
    table = pe + 24 + opt_size
    out = []
    for i in range(count):
        entry = data[table + i * 40 : table + (i + 1) * 40]
        name = entry[:8].rstrip(b"\0").decode(errors="replace")
        vsize, vaddr, rawsz, rawptr = struct.unpack_from("<IIII", entry, 8)
        out.append((name, vaddr, vsize, rawptr, rawsz))
    return out


def pdata_functions(data: bytes) -> list[tuple[int, int, int]]:
    """Sorted [(begin_va, end_va, unwind_rva)] from the exception table."""
    for name, vaddr, vsize, _rawptr, _rawsz in sections(data):
        if name == ".pdata":
            blob = data[vaddr : vaddr + vsize]
            count = len(blob) // 12
            arr = np.frombuffer(blob[: count * 12], dtype=np.uint32).reshape(count, 3)
            fns = [
                (int(b) + IMAGE_BASE, int(e) + IMAGE_BASE, int(u))
                for b, e, u in arr
                if b and e > b
            ]
            fns.sort()
            return fns
    raise SystemExit("no .pdata section")


class Owners:
    """Address -> owning function start, by binary search over `.pdata`."""

    def __init__(self, data: bytes) -> None:
        self.functions = pdata_functions(data)
        self._starts = [f[0] for f in self.functions]

    def of(self, va: int) -> int | None:
        i = bisect.bisect_right(self._starts, va) - 1
        if i < 0:
            return None
        begin, end, _ = self.functions[i]
        return begin if begin <= va < end else None


def _signed_disp32(data: bytes, first: int, count: int) -> np.ndarray:
    """Signed little-endian 32-bit reads at every offset in [first, first+count)."""
    buf = np.frombuffer(data, dtype=np.uint8)
    u32 = (
        buf[first : first + count].astype(np.int64)
        | (buf[first + 1 : first + 1 + count].astype(np.int64) << 8)
        | (buf[first + 2 : first + 2 + count].astype(np.int64) << 16)
        | (buf[first + 3 : first + 3 + count].astype(np.int64) << 24)
    )
    return np.where(u32 >= 0x8000_0000, u32 - 0x1_0000_0000, u32)


def riprefs(data: bytes, target: int) -> list[int]:
    """VAs of 4-byte windows that would be a RIP-relative operand naming `target`.

    Returned as the VA of the displacement field itself, not of the instruction that owns it.
    """
    count = len(data) - 4
    disp = _signed_disp32(data, 0, count)
    position = np.arange(count, dtype=np.int64)
    want = target - IMAGE_BASE - 4
    return [int(x) + IMAGE_BASE for x in np.nonzero(disp + position == want)[0]]


def callrefs(data: bytes, target: int) -> dict[str, list[int]]:
    """VAs of `e8 rel32` (call) and `e9 rel32` (jmp) instructions whose target is `target`."""
    count = len(data) - 5
    disp = _signed_disp32(data, 1, count)
    position = np.arange(count, dtype=np.int64)
    want = target - IMAGE_BASE - 5
    reaches = disp + position == want
    opcode = np.frombuffer(data, dtype=np.uint8)[:count]
    return {
        label: [int(x) + IMAGE_BASE for x in np.nonzero(reaches & (opcode == byte))[0]]
        for byte, label in ((0xE8, "call"), (0xE9, "jmp"))
    }
