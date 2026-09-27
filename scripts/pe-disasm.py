#!/usr/bin/env python3
"""Disassemble a PE (x64) from a virtual address, annotating RIP-relative operands that land on
printable strings.

Pairs with pe-string-xrefs.py for binaries no Ghidra project holds yet.

usage: pe-disasm.py <pe> <va-hex> [count=60]
needs: pefile, capstone (e.g. `uv pip install pefile capstone` into a scratch venv)
"""
import re
import sys

import capstone
import pefile


def string_at(pe: pefile.PE, rva: int) -> str | None:
    try:
        data = pe.get_data(rva, 96)
    except pefile.PEFormatError:
        return None
    m = re.match(rb"[\x20-\x7e]{3,}", data)
    if m:
        return m.group().decode()
    m = re.match(rb"(?:[\x20-\x7e]\x00){3,}", data)
    return m.group().decode("utf-16-le") if m else None


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__.strip().splitlines()[-2], file=sys.stderr)
        return 2
    pe = pefile.PE(sys.argv[1], fast_load=True)
    base = pe.OPTIONAL_HEADER.ImageBase
    va = int(sys.argv[2], 16)
    count = int(sys.argv[3]) if len(sys.argv) > 3 else 60
    code = pe.get_data(va - base, count * 15)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    for n, ins in enumerate(md.disasm(code, va)):
        if n >= count:
            break
        note = ""
        for op in ins.operands:
            if op.type == capstone.x86.X86_OP_MEM and op.mem.base == capstone.x86.X86_REG_RIP:
                target = ins.address + ins.size + op.mem.disp
                s = string_at(pe, target - base)
                note = f"  ; {target:#x}" + (f" {s!r}" if s else "")
        print(f"{ins.address:#x}  {ins.mnemonic:8} {ins.op_str}{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
