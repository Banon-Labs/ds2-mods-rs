#!/usr/bin/env python3
"""Find RIP-relative references to an absolute address, without Ghidra.

    python3 scripts/ds2-xrefs.py 0x14160de1a [0x...]

Ghidra answers this better (it knows instruction boundaries; this does not), but Ghidra headless
holds an exclusive project lock and costs minutes per query. This costs about a second, which
makes it the right tool for "who reads this global" during an investigation, and the wrong tool
for anything that must be exhaustive and exact.

HOW, and therefore WHAT IT GETS WRONG: an x86-64 RIP-relative operand stores
`disp32 = target - address_of_next_instruction`. So for a displacement field at file offset `p`
whose instruction ends at `p + 4`, a reference to `target` means

    read_u32(p) + p + 4 + IMAGE_BASE == target

which rearranges to a single vectorised comparison over the whole image. The catch is that
nothing here decodes instructions: a 4-byte window that satisfies the equation but is not a
displacement field -- ordinary data, or a misaligned read straddling two instructions -- is a
FALSE POSITIVE. Every hit is a candidate to disassemble, not a fact. Hits are reported with the
16 bytes of context needed to do that.

It also finds only RIP-relative references. An address materialised into a register and used
indirectly, or reached through a vtable or jump table, is invisible here.
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

import numpy as np

IMAGE_BASE = 0x1_4000_0000
REPO_ROOT = Path(__file__).resolve().parents[1]
IMAGE_PATH = REPO_ROOT / "darksoulsii-deobf.bin"
#: Bytes of context printed per hit -- enough to disassemble backwards to the instruction start.
CONTEXT_BEFORE = 8
CONTEXT_AFTER = 8
#: Bytes that may follow a displacement field before the instruction ends: no immediate, imm8,
#: imm16, imm32. `mov BYTE PTR [rip+X], 1` is the imm8 case and is common for flag writes.
IMMEDIATE_TAILS = (0, 1, 2, 4)


def rip_refs(data: bytes, target: int) -> list[tuple[int, int]]:
    """(file offset of the displacement field, immediate-tail length) for every candidate."""
    raw = np.frombuffer(data, dtype=np.uint8)
    n = len(raw) - 4
    # The displacement field read as little-endian u32 at every byte offset.
    disp = (
        raw[0:n].astype(np.uint32)
        | (raw[1 : n + 1].astype(np.uint32) << np.uint32(8))
        | (raw[2 : n + 2].astype(np.uint32) << np.uint32(16))
        | (raw[3 : n + 3].astype(np.uint32) << np.uint32(24))
    )
    offsets = np.arange(n, dtype=np.uint32)
    reach = (disp + offsets).astype(np.uint32)
    out = []
    for tail in IMMEDIATE_TAILS:
        # disp + (p + 4 + tail) + IMAGE_BASE == target, modulo 2**32 as the CPU computes it.
        want = np.uint32((target - IMAGE_BASE - 4 - tail) & 0xFFFF_FFFF)
        out.extend((int(p), tail) for p in np.flatnonzero(reach == want))
    return sorted(out)


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    data = IMAGE_PATH.read_bytes()
    for arg in argv[1:]:
        target = int(arg, 0)
        hits = rip_refs(data, target)
        print(f"=== 0x{target:x}: {len(hits)} candidate RIP-relative reference(s) ===")
        for off, tail in hits:
            lo = max(0, off - CONTEXT_BEFORE)
            ctx = data[lo : off + 4 + tail + CONTEXT_AFTER].hex(" ")
            # The instruction starts somewhere in the bytes before the displacement field.
            print(f"  disp@0x{IMAGE_BASE + off:x} imm{tail * 8:<2}  ctx[{lo - off:+d}]: {ctx}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
