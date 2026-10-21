#!/usr/bin/env python3
"""List every patched spot in a live DarkSoulsII.exe image against the clean binary.

    python3 scripts/ds2-image-diff.py <live image .bin> [--clean darksoulsii-deobf.bin]

The live image comes from scripts/frida/ds2sc-image-dump.js with MODULE = 'DarkSoulsII.exe': the
module from its base, so offset == RVA. The clean side is the PE on disk, mapped section by section.
Each differing run in an executable section is printed with its VA, length and, when the first
patched bytes are a jump, where it jumps. That is the inventory of hooks Seamless, our loader and
Arxan's own repairs left in the game; the targets name whose each one is (match them against the
module list the dump sent).
"""

from __future__ import annotations

import argparse
import pathlib
import struct
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
IMAGE_BASE = 0x140000000
SCN_EXECUTE = 0x20000000


def sections(pe: bytes) -> list[tuple[str, int, int, int, int]]:
    """(name, rva, virtual size, raw offset, characteristics) of each section."""
    e_lfanew = struct.unpack_from("<I", pe, 0x3C)[0]
    count = struct.unpack_from("<H", pe, e_lfanew + 6)[0]
    optional = struct.unpack_from("<H", pe, e_lfanew + 20)[0]
    table = e_lfanew + 24 + optional
    out = []
    for index in range(count):
        at = table + index * 40
        name = pe[at:at + 8].rstrip(b"\0").decode("latin-1")
        vsize, rva, raw_size, raw = struct.unpack_from("<IIII", pe, at + 8)
        flags = struct.unpack_from("<I", pe, at + 36)[0]
        out.append((name, rva, min(vsize, raw_size), raw, flags))
    return out


def jump_target(va: int, code: bytes) -> str:
    if code[:1] == b"\xe9":
        return f"jmp {va + 5 + struct.unpack_from('<i', code, 1)[0]:#x}"
    if code[:2] == b"\xff\x25":
        disp = struct.unpack_from("<i", code, 2)[0]
        return "jmp [rip]" + (f" -> {struct.unpack_from('<Q', code, 6)[0]:#x}" if disp == 0 else f"+{disp:#x}")
    if code[:2] == b"\x48\xb8" and code[10:12] == b"\xff\xe0":
        return f"movabs rax; jmp rax -> {struct.unpack_from('<Q', code, 2)[0]:#x}"
    return ""


def diff(live: bytes, clean: bytes) -> list[str]:
    lines = []
    for name, rva, size, raw, flags in sections(clean):
        if not flags & SCN_EXECUTE:
            continue
        a, b = live[rva:rva + size], clean[raw:raw + size]
        offset = 0
        while offset < size:
            if a[offset] == b[offset]:
                offset += 1
                continue
            start = offset
            # A run ends after 8 equal bytes in a row, so a patch with an untouched byte inside it
            # is still one run.
            same = 0
            while offset < size and same < 8:
                same = same + 1 if a[offset] == b[offset] else 0
                offset += 1
            length = offset - same - start
            va = IMAGE_BASE + rva + start
            lines.append(f"{va:#x} {name} len={length} live={a[start:start + 14].hex()} "
                         f"clean={b[start:start + 14].hex()} {jump_target(va, a[start:start + 14])}")
    return lines


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("live", type=pathlib.Path)
    ap.add_argument("--clean", type=pathlib.Path, default=REPO / "darksoulsii-deobf.bin")
    args = ap.parse_args()
    lines = diff(args.live.read_bytes(), args.clean.read_bytes())
    print("\n".join(lines))
    print(f"{len(lines)} patched run(s)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
