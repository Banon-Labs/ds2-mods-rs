#!/usr/bin/env python3
"""Print every direct `call`/`jmp rel32` in a PE's .text that lands on one of the given VAs, with
the function (from .pdata) each site sits in. With `--func`, print only the containing function
of each given VA instead.

Pairs with pe-disasm.py and pe-string-xrefs.py for binaries no Ghidra project holds yet (third-party
DLLs such as the DS2 Lighting Engine's dxgi.dll). Indirect calls (vtables, function pointers) are
not found.

usage: pe-call-xrefs.py <pe> [--func] <va-hex> [<va-hex> ...]
needs: pefile (e.g. `uv pip install pefile` into a scratch venv)
"""
import bisect
import struct
import sys

import pefile


def functions(pe: pefile.PE) -> list[tuple[int, int]]:
    d = pe.OPTIONAL_HEADER.DATA_DIRECTORY[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_EXCEPTION"]]
    data = pe.get_data(d.VirtualAddress, d.Size)
    return sorted(struct.unpack_from("<II", data, i) for i in range(0, len(data) - 11, 12))


def containing(fns: list[tuple[int, int]], rva: int) -> tuple[int, int] | None:
    i = bisect.bisect_right(fns, (rva, 0xFFFFFFFF)) - 1
    return fns[i] if i >= 0 and fns[i][0] <= rva < fns[i][1] else None


def main() -> int:
    args = sys.argv[1:]
    only_func = "--func" in args
    args = [a for a in args if a != "--func"]
    if len(args) < 2:
        print(__doc__.strip().splitlines()[-2], file=sys.stderr)
        return 2
    pe = pefile.PE(args[0], fast_load=True)
    base = pe.OPTIONAL_HEADER.ImageBase
    fns = functions(pe)
    targets = {int(a, 16) for a in args[1:]}

    def where(rva: int) -> str:
        f = containing(fns, rva)
        return "in func %#x..%#x" % (f[0] + base, f[1] + base) if f else "outside .pdata"

    if only_func:
        for va in sorted(targets):
            print("%#x %s" % (va, where(va - base)))
        return 0

    text = next(sec for sec in pe.sections if sec.Name.startswith(b".text"))
    code, text_rva = text.get_data(), text.VirtualAddress
    for i in range(len(code) - 5):
        op = code[i]
        if op not in (0xE8, 0xE9):
            continue
        va = base + text_rva + i + 5 + struct.unpack_from("<i", code, i + 1)[0]
        if va in targets:
            kind = "call" if op == 0xE8 else "jmp "
            print("%#x %s %#x  %s" % (base + text_rva + i, kind, va, where(text_rva + i)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
