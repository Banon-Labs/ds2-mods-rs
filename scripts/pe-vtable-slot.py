#!/usr/bin/env python3
"""Find the vtable slot of exported virtual methods in a PE, so a caller holding only an interface
pointer can call them.

usage: uv run --with pefile python3 scripts/pe-vtable-slot.py <dll> <export-substring> [...]

For every export whose decorated name contains one of the substrings, it finds each 8-byte slot in
the image's read-only data holding that export's address, walks back to the start of that vtable
(the slot after the RTTI complete-object-locator pointer, which points into data rather than code),
and prints the vtable's VA and the method's slot index. It also lists the whole vtable with every
slot's export name, where one exists, so the index can be checked against its neighbours.
Read-only.
"""
import sys

import pefile

path, wanted = sys.argv[1], sys.argv[2:]
pe = pefile.PE(path)
base = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()
exports = {base + e.address: (e.name or b"").decode() for e in pe.DIRECTORY_ENTRY_EXPORT.symbols}


def section_of(rva):
    for s in pe.sections:
        if s.VirtualAddress <= rva < s.VirtualAddress + max(s.Misc_VirtualSize, s.SizeOfRawData):
            return s
    return None


def is_code(va):
    s = section_of(va - base)
    return s is not None and bool(s.Characteristics & 0x20000000)


def qword(rva):
    return int.from_bytes(image[rva:rva + 8], "little")


targets = {va: n for va, n in exports.items() if any(w in n for w in wanted)}
seen = set()
for va, name in sorted(targets.items(), key=lambda x: x[1]):
    needle = va.to_bytes(8, "little")
    at = image.find(needle)
    while at >= 0:
        if at % 8 == 0 and not is_code(base + at):
            start = at
            while is_code(qword(start - 8)):
                start -= 8
            slot = (at - start) // 8
            print(f"{name}\n    vtable=0x{base + start:x} slot={slot} (byte offset 0x{slot * 8:x})")
            if start not in seen:
                seen.add(start)
                i = start
                while is_code(qword(i)):
                    print(f"      [{(i - start) // 8:3}] 0x{qword(i):x} {exports.get(qword(i), '')}")
                    i += 8
        at = image.find(needle, at + 1)
