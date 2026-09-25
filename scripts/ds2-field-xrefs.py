#!/usr/bin/env python3
"""Find instructions that touch a STRUCT FIELD at `[reg+disp]`, without Ghidra.

    python3 scripts/ds2-field-xrefs.py 0x198            # every width
    python3 scripts/ds2-field-xrefs.py 0x198 --width 2  # 16-bit only
    python3 scripts/ds2-field-xrefs.py 0x148 --width 4 --limit 40

WHY THIS EXISTS AND WHY IT IS NOT `ds2-xrefs.py`. That script answers "who reads this GLOBAL"
by solving the RIP-relative displacement equation, and a struct field has no absolute address to
solve for -- `mov ax,[rdi+0x198]` names a register the scan cannot know. So the question this
repo actually asks during input and frontend work -- "the poll writes `PadDevice+0x198`, who
READS it?" -- had no tool at all, and the fallback was Ghidra, which `ds2-mods-rs-bc2` records as
not yet ported. This is the cheap 80% of that query: about a second, no project lock.

HOW, AND THEREFORE WHAT IT GETS WRONG. An x86-64 `mod=10` ModRM encodes a signed 32-bit
displacement immediately after the ModRM (and after a SIB byte when `rm == 100`). So a field at
`+0x198` appears as the four little-endian bytes `98 01 00 00` sitting at a displacement
position. This scans for those bytes and then walks BACKWARDS over an optional SIB, the ModRM,
an optional REX prefix and an optional operand-size prefix to see whether a plausible
`mov`-family opcode is sitting where one would have to be.

NOTHING HERE DECODES FORWARD FROM A KNOWN INSTRUCTION BOUNDARY, which is the same caveat
`ds2-xrefs.py` carries and it bites harder here: four bytes that merely look like a displacement
are common. EVERY HIT IS A CANDIDATE TO DISASSEMBLE, NOT A FACT. Feed them to
`scripts/ds2-disasm.py` before believing any of them. The filters below exist to make that list
short enough to actually read:

* `--width` keeps only the operand size you care about. A 16-bit field read is `0f b7` (movzx)
  or `66 8b` (mov r16), and demanding that alone throws out the flood of 64-bit `48 8b`
  stack-frame traffic that dominates an unfiltered scan.
* `rm == 100` (a SIB byte) is dropped by default, because a SIB at `mod=10` is overwhelmingly
  `[rsp+disp32]` or an indexed array -- a stack slot, not an object field. `--allow-sib` keeps
  them for the cases where the field really is reached through an index.

THE CAVEAT THAT GOVERNS EVERY ADDRESS HERE, same as its sibling scripts: the deobfuscated image
is not the byte stream that runs. At the 286 Arxan-redirected entries this image shows recovered
code where the live process holds a five-byte stub. See `docs/ARXAN-FOOTPRINT.md`.
"""

from __future__ import annotations

import argparse
import re
import struct
import sys
from pathlib import Path

IMAGE_BASE = 0x1_4000_0000
REPO_ROOT = Path(__file__).resolve().parents[1]
IMAGE_PATH = REPO_ROOT / "darksoulsii-deobf.bin"

#: Opcodes that move a value to or from `[reg+disp32]`, keyed by operand size in bytes.
#:
#: Each entry is (opcode bytes, needs the 0x66 operand-size prefix, human name). The 16-bit forms
#: are the ones that matter for a button bitmask; the others are here because the same question
#: gets asked about pointers and floats and it would be a worse tool without them.
OPCODES: dict[int, list[tuple[bytes, bool, str]]] = {
    2: [
        (b"\x0f\xb7", False, "movzx r32,[m16]"),
        (b"\x0f\xbf", False, "movsx r32,[m16]"),
        (b"\x8b", True, "mov r16,[m16]"),
        (b"\x89", True, "mov [m16],r16"),
    ],
    4: [
        (b"\x8b", False, "mov r32,[m32]"),
        (b"\x89", False, "mov [m32],r32"),
        (b"\x63", False, "movsxd r64,[m32]"),
    ],
    8: [
        (b"\x8b", False, "mov r64,[m64]"),
        (b"\x89", False, "mov [m64],r64"),
        (b"\x8d", False, "lea r64,[m]"),
    ],
}

#: A REX prefix is any byte in this range, and it sits immediately before the opcode.
REX_RANGE = range(0x40, 0x50)

#: `mod=10` ModRM bytes: the displacement that follows is 32-bit. `mod=00`/`01`/`11` cannot carry
#: a 4-byte displacement, so they can never encode the field being searched for.
MODRM_DISP32 = range(0x80, 0xC0)

#: Bytes of context printed per hit, so the candidate can be disassembled backwards by eye.
CONTEXT = 8


def register_name(modrm: int, rex: int | None) -> str:
    """Name the base register of a `mod=10` ModRM, honouring REX.B."""
    names = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi"]
    high = ["r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"]
    index = modrm & 0x07
    if rex is not None and rex & 0x01:
        return high[index]
    return names[index]


def scan(image: bytes, field: int, width: int | None, allow_sib: bool) -> list[dict]:
    """Every candidate instruction whose `mod=10` displacement equals `field`."""
    needle = struct.pack("<i", field)
    widths = [width] if width else sorted(OPCODES)
    found: dict[int, dict] = {}

    for match in re.finditer(re.escape(needle), image):
        disp = match.start()
        # The ModRM is one byte before the displacement, or two when a SIB sits between them.
        for sib_present in (False, True):
            modrm_at = disp - (2 if sib_present else 1)
            if modrm_at < 4:
                continue
            modrm = image[modrm_at]
            if modrm not in MODRM_DISP32:
                continue
            has_sib = (modrm & 0x07) == 0x04
            if has_sib != sib_present:
                continue
            if has_sib and not allow_sib:
                continue

            for size in widths:
                for opcode, needs_66, name in OPCODES[size]:
                    start = modrm_at - len(opcode)
                    if start < 2:
                        continue
                    if image[start : start + len(opcode)] != opcode:
                        continue
                    rex = None
                    if image[start - 1] in REX_RANGE:
                        rex = image[start - 1]
                        start -= 1
                    # A 16-bit `mov` is only 16-bit because of the 0x66 prefix; a 64-bit one is
                    # only 64-bit because of REX.W. Demanding the right one is the whole reason
                    # `--width 2` produces a readable list instead of a dump.
                    if needs_66:
                        if start < 1 or image[start - 1] != 0x66:
                            continue
                        start -= 1
                    elif size == 8 and not (rex is not None and rex & 0x08):
                        continue
                    elif size == 4 and rex is not None and rex & 0x08:
                        continue

                    found[start] = {
                        "va": IMAGE_BASE + start,
                        "name": name,
                        "base": register_name(modrm, rex),
                        "size": size,
                        "sib": has_sib,
                        "bytes": image[start : disp + 4],
                        "context": image[max(0, start - CONTEXT) : disp + 4 + CONTEXT],
                    }
    return [found[key] for key in sorted(found)]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("field", help="field displacement, e.g. 0x198")
    parser.add_argument(
        "--width",
        type=int,
        choices=sorted(OPCODES),
        help="operand size in bytes. Omit for every size, which is much noisier.",
    )
    parser.add_argument(
        "--allow-sib",
        action="store_true",
        help="keep `[base+index*scale+disp32]` forms. Dropped by default because at mod=10 a "
        "SIB is nearly always a stack slot rather than an object field.",
    )
    parser.add_argument("--limit", type=int, default=60, help="most hits to print. DEFAULT: 60")
    arguments = parser.parse_args(argv[1:])

    try:
        field = int(arguments.field, 0)
    except ValueError:
        print(f"not a number: {arguments.field}", file=sys.stderr)
        return 2
    if not IMAGE_PATH.is_file():
        print(f"no image at {IMAGE_PATH}", file=sys.stderr)
        return 2

    image = IMAGE_PATH.read_bytes()
    hits = scan(image, field, arguments.width, arguments.allow_sib)

    width_note = f"{arguments.width}-byte" if arguments.width else "any-width"
    print(
        f"+{field:#x}: {len(hits)} {width_note} candidate(s)"
        f"{'' if arguments.allow_sib else ', SIB forms dropped'}"
    )
    print("EVERY LINE IS A CANDIDATE, NOT A FACT -- confirm with scripts/ds2-disasm.py.")
    for hit in hits[: arguments.limit]:
        print(
            f"  0x{hit['va']:x}  {hit['name']:<18} base={hit['base']:<3}"
            f"  {hit['bytes'].hex(' ')}"
        )
    if len(hits) > arguments.limit:
        print(f"  ... {len(hits) - arguments.limit} more (raise --limit)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
