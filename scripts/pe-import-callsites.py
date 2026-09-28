#!/usr/bin/env python3
"""List imports from named DLLs in darksoulsii-deobf.bin with their IAT slot, MSVC `jmp [rip]` thunk,
and the direct `call`/`jmp rel32` sites of that thunk (plus direct `call [rip+IAT]` sites).

usage: uv run --with pefile python3 scripts/pe-import-callsites.py <exe> <dll-substring> [name-regex]
Read-only. False positives possible for rel32 scans (no decoding); treat hits as candidates.
"""
import re
import sys
from pathlib import Path

import pefile

exe, dll = sys.argv[1], sys.argv[2].lower()
rx = re.compile(sys.argv[3]) if len(sys.argv) > 3 else None
pe = pefile.PE(exe, fast_load=True)
pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"]])
img = Path("/home/banon/projects/ds2-mods-rs/darksoulsii-deobf.bin").read_bytes()  # flat: off == rva
base = pe.OPTIONAL_HEADER.ImageBase
text = [s for s in pe.sections if s.Name.startswith(b".text")]
ranges = [(s.VirtualAddress, s.VirtualAddress + s.Misc_VirtualSize) for s in text]


def scan(pat_first, target_rva, instr_len):
    hits = []
    for lo, hi in ranges:
        i = lo
        while True:
            i = img.find(pat_first, i, hi)
            if i < 0:
                break
            disp = int.from_bytes(img[i + len(pat_first): i + len(pat_first) + 4], "little", signed=True)
            if i + instr_len + disp == target_rva:
                hits.append(i)
            i += 1
    return hits


for entry in pe.DIRECTORY_ENTRY_IMPORT:
    if dll not in entry.dll.decode().lower():
        continue
    for imp in entry.imports:
        name = imp.name.decode() if imp.name else f"ord{imp.ordinal}"
        if rx and not rx.search(name):
            continue
        iat = imp.address - base
        thunks = scan(b"\xff\x25", iat, 6)
        direct = scan(b"\xff\x15", iat, 6)
        calls = []
        for t in thunks:
            calls += scan(b"\xe8", t, 5)
        print(f"{name}\n    iat={base+iat:#x} thunks={[hex(base+t) for t in thunks]} "
              f"calls_via_thunk={len(calls)} {[hex(base+c) for c in calls][:12]} direct_calls={[hex(base+d) for d in direct][:8]}")
