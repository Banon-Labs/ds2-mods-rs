#!/usr/bin/env python3
"""Print every RIP-relative `lea` in a PE's .text that points at one of the given strings.

A quick static cross-reference for binaries no Ghidra project holds yet (third-party DLLs such as
the DS2 Lighting Engine's dxgi.dll). Only `lea reg, [rip+disp32]` is matched, which is how MSVC
materialises string addresses; a string reached some other way will not show up.

usage: pe-string-xrefs.py <pe> <string> [<string> ...]
needs: pefile (e.g. `uv pip install pefile` into a scratch venv)
"""
import re
import struct
import sys

import pefile


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__.strip().splitlines()[-2], file=sys.stderr)
        return 2
    pe = pefile.PE(sys.argv[1], fast_load=True)
    base = pe.OPTIONAL_HEADER.ImageBase
    raw = pe.__data__

    targets = {}
    for s in sys.argv[2:]:
        needle = s.encode() + b"\0"
        start = 0
        while (off := raw.find(needle, start)) >= 0:
            start = off + 1
            # Only whole strings: the byte before must end another string or be padding.
            if off and raw[off - 1] not in (0,):
                continue
            targets[pe.get_rva_from_offset(off)] = s

    text = next(sec for sec in pe.sections if sec.Name.startswith(b".text"))
    code, text_rva = text.get_data(), text.VirtualAddress
    for m in re.finditer(rb"[\x48\x4c]\x8d[\x05\x0d\x15\x1d\x25\x2d\x35\x3d]", code):
        i = m.start()
        rva = text_rva + i + 7 + struct.unpack_from("<i", code, i + 3)[0]
        if rva in targets:
            print(f"{base + text_rva + i:#x} {targets[rva]!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
