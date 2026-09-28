#!/usr/bin/env python3
"""Disassemble the .pdata function(s) containing the given VAs in darksoulsii-deobf.bin, with calls
into FMOD import thunks named (thunk map built from the PE import table on first run).

usage: uv run --with pefile --with numpy python3 scripts/ds2-fmod-disasm.py 0x1409f5178 [...]
Read-only. Same Arxan caveat as scripts/ds2-disasm.py: the file is not necessarily the running bytes.
"""
import importlib.util
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
EXE = Path.home() / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game/DarkSoulsII.exe"
CACHE = Path("/tmp/ds2-fmod-thunks.json")
spec = importlib.util.spec_from_file_location("pe", REPO / "scripts/ds2-pe.py")
pe_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pe_mod)
BASE = 0x140000000


def thunk_map(img):
    if CACHE.exists():
        return {int(k): v for k, v in json.loads(CACHE.read_text()).items()}
    import pefile
    pe = pefile.PE(str(EXE), fast_load=True)
    pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"]])
    iat = {}
    for e in pe.DIRECTORY_ENTRY_IMPORT:
        if b"fmod" not in e.dll.lower():
            continue
        for imp in e.imports:
            if imp.name:
                n = imp.name.decode()
                m = re.match(r"\?(\w+)@(\w+)@FMOD@@", n)
                iat[imp.address - BASE] = f"{m.group(2)}::{m.group(1)}" if m else n
    out = {}
    i = 0
    while (i := img.find(b"\xff\x25", i)) >= 0:
        t = i + 6 + int.from_bytes(img[i + 2:i + 6], "little", signed=True)
        if t in iat:
            out[BASE + i] = iat[t]
        i += 1
    CACHE.write_text(json.dumps(out))
    return out


img = pe_mod.load_image()
names = thunk_map(img)
funcs = pe_mod.pdata_functions(img)
for a in sys.argv[1:]:
    va = int(a, 16)
    cands = [(b - BASE, e - BASE) for b, e, _ in funcs if b <= va < e and e - b < 0x10000]
    f = max(cands) if cands else None
    if not f:
        print(f"{a}: no pdata owner")
        continue
    b, e = f
    print(f"==== {a} in function {BASE + b:#x}..{BASE + e:#x}")
    r = subprocess.run(["objdump", "-D", "-b", "binary", "-mi386:x86-64", "-M", "intel",
                        f"--start-address={BASE + b}", f"--stop-address={BASE + e}", f"--adjust-vma={BASE}",
                        str(REPO / "darksoulsii-deobf.bin")],
                       capture_output=True, text=True).stdout
    for line in r.splitlines():
        if not re.match(r"\s+[0-9a-f]+:", line):
            continue
        parts = line.split("\t")
        text = f"{parts[0].strip()} {parts[-1].strip()}"
        m = re.search(r"(call|jmp)\s+0x([0-9a-f]+)", text)
        if m and int(m.group(2), 16) in names:
            text += f"   ; FMOD {names[int(m.group(2), 16)]}"
        print(text)
