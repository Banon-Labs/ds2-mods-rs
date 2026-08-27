#!/usr/bin/env python3
"""Disassemble a function out of `darksoulsii-deobf.bin`, with call/jump targets named.

    python3 scripts/ds2-disasm.py 0x1400fd930            # follow to the first ret
    python3 scripts/ds2-disasm.py 0x1400fd930 --bytes 512  # fixed window instead

Why this exists rather than a Ghidra query: Ghidra decompiles better, but headless costs minutes
per call and holds a project lock. Reading one 200-byte substate method is a job for objdump, and
this wraps the two annoying parts of that -- the flat image has no symbols and objdump on a raw
binary prints every branch as a bare hex address.

WHAT IT ADDS over `objdump -D`:

* `--adjust-vma` is set from the flat image's `file offset == RVA` property, so printed addresses
  are real VAs.
* Every `call`/`jmp` target that lands on a known RTTI vtable slot, or on an address this script
  was given a name for, is annotated inline. Names come from `scripts/ds2-rtti.py`'s vtable walk,
  so they are read out of the binary rather than typed in.
* `--follow` stops at the first `ret` at depth 0 instead of printing a fixed window that either
  truncates the function or runs past it into the next one.

THE CAVEAT THAT GOVERNS EVERY ADDRESS HERE, same as its sibling scripts: the deobfuscated image
is not the byte stream that runs. At the 286 Arxan-redirected entries this image shows recovered
code where the live process holds a five-byte stub. Data (vtables, globals) is trustworthy; a
function body must be checked with `scripts/ds2-arxan-chain.py` before anything is detoured onto
it. See `docs/ARXAN-FOOTPRINT.md`.
"""

from __future__ import annotations

import argparse
import os
import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

IMAGE_BASE = 0x1_4000_0000
SIZE_OF_IMAGE = 0x1D76000
REPO_ROOT = Path(__file__).resolve().parents[1]
IMAGE_PATH = REPO_ROOT / "darksoulsii-deobf.bin"

#: An x64 RTTICompleteObjectLocator starts with signature == 1 and holds the type descriptor RVA
#: at +12; a type descriptor holds its name 16 bytes in. Same three constants ds2-rtti.py uses.
COL_SIGNATURE = 1
COL_TYPE_DESCRIPTOR_OFFSET = 12
TYPE_DESCRIPTOR_NAME_OFFSET = 16
#: Stop naming vtable slots at the first entry that is not a plausible in-image pointer.
MAX_VIRTUALS = 64
#: How far `--follow` will walk before giving up and saying so. Substate methods are tens of
#: instructions; anything past this is either not a function or not one worth reading linearly.
MAX_FOLLOW_BYTES = 0x2000

#: Instructions that end a linear run. `jmp` is included because a tail call ends the body just as
#: much as a `ret` does, and printing past it prints the *next* function.
TERMINATORS = ("ret", "jmp")


def load_image() -> bytes:
    data = IMAGE_PATH.read_bytes()
    if len(data) != SIZE_OF_IMAGE:
        raise SystemExit(
            f"{IMAGE_PATH} is {len(data)} bytes, expected SizeOfImage {SIZE_OF_IMAGE}; "
            "offsets would not be RVAs"
        )
    return data


def find_all(buf: bytes, needle: bytes, aligned: int = 1) -> list[int]:
    out, i = [], 0
    while (i := buf.find(needle, i)) >= 0:
        if i % aligned == 0:
            out.append(i)
        i += 1
    return out


def vtable_symbols(data: bytes) -> dict[int, str]:
    """`{function VA: "Class::vN"}` for every virtual in every RTTI-described vtable.

    This is the whole reason the output is readable. A substate's `enter` is nothing but a call
    through slot 1, and an unnamed `call qword ptr [rax+8]` says nothing; `Class::v1` says what
    the game is about to run.

    Over-naming is possible and harmless: a slot that is really padding gets a name nothing calls.
    Under-naming is the failure that matters, and it shows up as a bare address -- visibly absent,
    never as a wrong name.

    THE SHAPE OF THIS IS LOAD-BEARING and the obvious version does not finish. `ds2-rtti.py`
    answers "where is class X" by searching the image once per name, which is right for one name
    and quadratic for all 5269 of them -- the first draft of this function did exactly that and
    ran for over thirty seconds without producing a line. So the direction is inverted: build the
    u32 and u64 views of the image ONCE, then resolve every class in three vectorised membership
    tests. Same three pointer hops, same result, about a second.
    """
    # Hop 0: every type descriptor name, as {file offset of the descriptor: class name}.
    by_td: dict[int, str] = {}
    for m in re.finditer(rb"\.\?AV([A-Za-z0-9_]+)@@", data):
        td_off = m.start() - TYPE_DESCRIPTOR_NAME_OFFSET
        if td_off >= 0:
            by_td[td_off] = m.group(1).decode()
    if not by_td:
        return {}

    u32 = np.frombuffer(data[: len(data) // 4 * 4], dtype="<u4")
    u64 = np.frombuffer(data[: len(data) // 8 * 8], dtype="<u8")

    # Hop 1: descriptor -> COL. The descriptor is referenced by 32-bit RVA at COL+12, and a COL
    # is 4-aligned, so a hit at u32 index `i` means a candidate COL at (i * 4) - 12.
    wanted = np.fromiter(by_td.keys(), dtype="<u4", count=len(by_td))
    hits = np.flatnonzero(np.isin(u32, wanted))
    col_offs = hits.astype(np.int64) * 4 - COL_TYPE_DESCRIPTOR_OFFSET
    col_offs = col_offs[col_offs >= 0]
    # The signature field separates a real COL from a coincidental 4-byte match.
    col_offs = col_offs[u32[col_offs // 4] == COL_SIGNATURE]
    by_col = {
        int(IMAGE_BASE + off): by_td[int(u32[(off + COL_TYPE_DESCRIPTOR_OFFSET) // 4])]
        for off in col_offs.tolist()
        if int(u32[(off + COL_TYPE_DESCRIPTOR_OFFSET) // 4]) in by_td
    }
    if not by_col:
        return {}

    # Hop 2: COL -> vtable. `vtable[-1]` holds the COL pointer, so the slots begin 8 bytes on.
    col_vas = np.fromiter(by_col.keys(), dtype="<u8", count=len(by_col))
    slots = np.flatnonzero(np.isin(u64, col_vas))

    symbols: dict[int, str] = {}
    limit = IMAGE_BASE + len(data)
    for slot in slots.tolist():
        cls = by_col[int(u64[slot])]
        for n in range(MAX_VIRTUALS):
            index = slot + 1 + n
            if index >= len(u64):
                break
            fn = int(u64[index])
            if not IMAGE_BASE <= fn < limit:
                break
            # First writer wins: a shared base implementation should read as the base's slot, not
            # as whichever derived class happened to be scanned last.
            symbols.setdefault(fn, f"{cls}::v{n}")
    return symbols


def objdump(data: bytes, va: int, length: int) -> list[tuple[int, str, str]]:
    """`(address, bytes, text)` per instruction, straight out of objdump."""
    off = va - IMAGE_BASE
    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as fh:
        fh.write(data[off : off + length])
        tmp = fh.name
    try:
        proc = subprocess.run(
            [
                "objdump", "-D",
                "-b", "binary",
                "-m", "i386:x86-64",
                "-M", "intel",
                f"--adjust-vma={va:#x}",
                tmp,
            ],
            capture_output=True,
            text=True,
            check=True,
        )
    finally:
        os.unlink(tmp)
    out = []
    for line in proc.stdout.splitlines():
        m = re.match(r"^\s+([0-9a-f]+):\t([0-9a-f ]+)\t(.*)$", line)
        if m:
            out.append((int(m.group(1), 16), m.group(2).strip(), m.group(3).strip()))
    return out


def annotate(text: str, symbols: dict[int, str]) -> str:
    """Append `; Class::vN` when a branch target has a name."""
    m = re.search(r"\b(0x[0-9a-f]+)\b", text)
    if not m:
        return text
    name = symbols.get(int(m.group(1), 16))
    return f"{text}    ; {name}" if name else text


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("address", help="virtual address, e.g. 0x1400fd930")
    ap.add_argument("--bytes", type=lambda s: int(s, 0), default=None,
                    help="disassemble a fixed window instead of following to the first ret/jmp")
    ap.add_argument("--no-symbols", action="store_true",
                    help="skip the RTTI vtable scan (about a second) when names are not needed")
    args = ap.parse_args(argv[1:])

    data = load_image()
    va = int(args.address, 0)
    if not IMAGE_BASE <= va < IMAGE_BASE + len(data):
        raise SystemExit(f"0x{va:x} is outside the image")

    symbols = {} if args.no_symbols else vtable_symbols(data)
    if (name := symbols.get(va)) is not None:
        print(f"=== 0x{va:x}  {name} ===")
    else:
        print(f"=== 0x{va:x} ===")

    length = args.bytes if args.bytes is not None else MAX_FOLLOW_BYTES
    for addr, raw, text in objdump(data, va, length):
        print(f"  0x{addr:x}:  {raw:<24} {annotate(text, symbols)}")
        # Only a fixed window is allowed to run past a terminator; --follow stops, because the
        # bytes after a function's last ret belong to the next function and reading them as if
        # they were part of this one is how a trace picks up code that never executes.
        if args.bytes is None and text.split()[0] in TERMINATORS:
            break
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
