#!/usr/bin/env python3
"""List the functions whose entry is an Arxan redirect, from `.pdata` and from Ghidra, and diff them.

A redirected function starts with an unconditional `jmp` whose target lies in a `.text` section
after the first one -- the executable block Arxan appends to the image. This script finds those
entries two ways and explains every entry that one way finds and the other does not:

* **`.pdata`**: every `RUNTIME_FUNCTION` in the exception directory. The directory's own size is
  used, not the `.pdata` section's virtual size: the section is padded past the table, and on DS2
  the padding holds bytes that parse as six bogus entries.
* **Ghidra**: every function entry point in the DS2 program served by the Ghidra MCP daemon on
  127.0.0.1:8766 (`scripts/ghidra/mcp_query.py`, read-only `getAllFunctions`), or a saved list of
  entry points (`--ghidra-list`). The redirect test is applied to the image bytes at each entry, the
  same test `scripts/ghidra/rt/Ds2ArxanStubs.java` applies to Ghidra's decoded instruction.

The image is either the flat mapped image `darksoulsii-deobf.bin` (file offset == RVA) or the
shipped `DarkSoulsII.exe` in file layout; the layout is detected from the file size against
`SizeOfImage`. `--same-image A B` maps both and compares them byte for byte.

    python3 scripts/ds2-arxan-redirects.py                       # .pdata census
    python3 scripts/ds2-arxan-redirects.py --list                # every .pdata redirect
    python3 scripts/ds2-arxan-redirects.py --ghidra              # diff against the Ghidra daemon
    python3 scripts/ds2-arxan-redirects.py --ghidra --save-ghidra /tmp/ghidra-entries.txt
    python3 scripts/ds2-arxan-redirects.py --ghidra-list /tmp/ghidra-entries.txt
    python3 scripts/ds2-arxan-redirects.py --same-image darksoulsii-deobf.bin "$EXE"
    python3 scripts/ds2-arxan-redirects.py --selftest

Byte-level, no disassembler: the redirect is the first instruction, so the opcode at the entry is
the whole test. `e9 rel32` and `eb rel8` are the unconditional direct jumps Ghidra's test would
count. `ff 25` (an indirect jump through a pointer) is reported separately and is not a redirect
under either method, because Ghidra's instruction has no static flow for it.
"""

from __future__ import annotations

import argparse
import importlib.util
import struct
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
#: The image is gitignored, so a linked worktree has none of its own; fall back to the main checkout's.
DEFAULT_IMAGE = next(
    (p / "darksoulsii-deobf.bin" for p in (REPO_ROOT, *REPO_ROOT.parents)
     if (p / "darksoulsii-deobf.bin").exists()),
    REPO_ROOT / "darksoulsii-deobf.bin",
)
MCP_QUERY = Path(__file__).resolve().parent / "ghidra" / "mcp_query.py"
UNW_FLAG_CHAININFO = 0x4


class Image:
    """A PE image mapped to its in-memory layout, so every index below is an RVA."""

    def __init__(self, raw: bytes) -> None:
        pe = struct.unpack_from("<I", raw, 0x3C)[0]
        if raw[pe : pe + 4] != b"PE\0\0":
            raise SystemExit("not a PE image")
        count = struct.unpack_from("<H", raw, pe + 6)[0]
        opt = pe + 24
        opt_size = struct.unpack_from("<H", raw, pe + 20)[0]
        if struct.unpack_from("<H", raw, opt)[0] != 0x20B:
            raise SystemExit("not a PE32+ image")
        self.base = struct.unpack_from("<Q", raw, opt + 24)[0]
        size_of_image = struct.unpack_from("<I", raw, opt + 56)[0]
        size_of_headers = struct.unpack_from("<I", raw, opt + 60)[0]
        self.sections: list[tuple[str, int, int]] = []
        table = opt + opt_size
        raw_sections = []
        for i in range(count):
            entry = raw[table + i * 40 : table + (i + 1) * 40]
            name = entry[:8].rstrip(b"\0").decode(errors="replace")
            vsize, vaddr, rawsz, rawptr = struct.unpack_from("<IIII", entry, 8)
            self.sections.append((name, vaddr, vsize))
            raw_sections.append((vaddr, vsize, rawptr, rawsz))
        self.flat = len(raw) == size_of_image
        if self.flat:
            self.mem = raw
        else:
            mem = bytearray(size_of_image)
            mem[:size_of_headers] = raw[:size_of_headers]
            for vaddr, vsize, rawptr, rawsz in raw_sections:
                n = min(rawsz, vsize) if vsize else rawsz
                mem[vaddr : vaddr + n] = raw[rawptr : rawptr + n]
            self.mem = bytes(mem)
        exc_rva, exc_size = struct.unpack_from("<II", raw, opt + 112 + 3 * 8)
        self.exception_dir = (exc_rva, exc_size)

    def text_blocks(self) -> list[tuple[int, int]]:
        return [(va, va + vs) for name, va, vs in self.sections if name == ".text"]

    def redirect_blocks(self) -> list[tuple[int, int]]:
        """Every `.text` after the first: Arxan's appended code, as Ds2ArxanStubs.java reads it."""
        return self.text_blocks()[1:]

    def section_of(self, rva: int) -> str:
        texts = self.text_blocks()
        for name, va, vs in self.sections:
            if va <= rva < va + vs:
                if name == ".text" and len(texts) > 1:
                    return f".text#{texts.index((va, va + vs)) + 1}"
                return name
        return "?"

    def runtime_functions(self) -> list[tuple[int, int, bool]]:
        """[(begin_rva, end_rva, chained)] from the exception directory, in table order."""
        if hasattr(self, "_rfs"):
            return self._rfs
        rva, size = self.exception_dir
        out = []
        for off in range(rva, rva + size - size % 12, 12):
            begin, end, unwind = struct.unpack_from("<III", self.mem, off)
            chained = unwind < len(self.mem) and bool((self.mem[unwind] >> 3) & UNW_FLAG_CHAININFO)
            out.append((begin, end, chained))
        self._rfs = out
        return out

    def jump_at(self, rva: int) -> tuple[str, int | None]:
        """(kind, target_rva) of an unconditional jump at `rva`, or ("", None)."""
        m = self.mem
        if rva + 6 > len(m):
            return "", None
        if m[rva] == 0xE9:
            return "e9", rva + 5 + struct.unpack_from("<i", m, rva + 1)[0]
        if m[rva] == 0xEB:
            return "eb", rva + 2 + struct.unpack_from("<b", m, rva + 1)[0]
        if m[rva] == 0xFF and m[rva + 1] == 0x25:
            return "ff25", None
        return "", None

    def redirect_target(self, rva: int) -> int | None:
        kind, target = self.jump_at(rva)
        if kind not in ("e9", "eb") or target is None:
            return None
        for lo, hi in self.redirect_blocks():
            if lo <= target < hi:
                return target
        return None


def load(path: Path) -> Image:
    if not path.exists():
        raise SystemExit(f"{path} missing -- see README.md for how to produce it")
    return Image(path.read_bytes())


def pdata_redirects(img: Image) -> dict[int, int]:
    return {
        begin: t
        for begin, end, _chained in img.runtime_functions()
        if begin and end > begin and (t := img.redirect_target(begin)) is not None
    }


def ghidra_entries(port: int, save: Path | None) -> list[int]:
    spec = importlib.util.spec_from_file_location("mcp_query", MCP_QUERY)
    mcp = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mcp)
    info = mcp.call("getContext", {}, port=port)
    name = (info.get("result") or {}).get("program_name") if isinstance(info, dict) else None
    if name != "DarkSoulsII.exe":
        raise SystemExit(f"port {port} serves {name!r}, not DarkSoulsII.exe")
    entries: list[int] = []
    offset, total = 0, None
    while total is None or offset < total:
        r = mcp.call("getAllFunctions", {"offset": offset, "limit": 10000}, timeout=120, port=port)
        res = r.get("result") if isinstance(r, dict) else None
        if not isinstance(res, dict):
            raise SystemExit(f"getAllFunctions failed at offset {offset}: {r}")
        total = res["totalCount"]
        items = res["items"]
        if not items:
            break
        entries += [int(i["entry_point"], 16) for i in items if not i.get("is_external")]
        offset += len(items)
    if save is not None:
        save.write_text("".join(f"{va:#x}\n" for va in entries))
    return entries


def read_entry_list(path: Path) -> list[int]:
    out = []
    for line in path.read_text().split("\n"):
        tok = line.strip().split()
        if tok:
            out.append(int(tok[0], 16))
    return out


def census(img: Image) -> dict[int, int]:
    rfs = img.runtime_functions()
    live = [r for r in rfs if r[0] and r[1] > r[0]]
    red = pdata_redirects(img)
    chained = {b for b, _e, c in live if c}
    stray = sum(1 for b, _e, _c in live if img.jump_at(b)[0] == "ff25")
    print(f"image                      {'flat mapped' if img.flat else 'file layout'}")
    for lo, hi in img.redirect_blocks():
        print(f"redirect territory         {img.base + lo:#x}-{img.base + hi - 1:#x}")
    print(f".pdata entries             {len(live)}")
    print(f"  chained (not a start)    {len(chained)}")
    print(f"  arxan-redirected         {len(red)}")
    print(f"    of which chained       {len(set(red) & chained)}")
    print(f"  distinct stub targets    {len(set(red.values()))}")
    print(f"  ff 25 entries            {stray}")
    return red


def covering_record(img: Image, rva: int) -> tuple[int, int, bool] | None:
    """The `.pdata` record whose [begin, end) holds `rva`, if any (the table is sorted)."""
    import bisect

    rfs = [r for r in img.runtime_functions() if r[0] and r[1] > r[0]]
    i = bisect.bisect_right([r[0] for r in rfs], rva) - 1
    if i >= 0 and rfs[i][0] <= rva < rfs[i][1]:
        return rfs[i]
    return None


def explain_ghidra_only(img: Image, va: int) -> str:
    rva = va - img.base
    sec = img.section_of(rva)
    if sec != ".text#1":
        return f"entry is in {sec}: a function inside Arxan's own block"
    rec = covering_record(img, rva)
    if rec is None:
        return "no .pdata record covers it: a function in a gap between unwind records"
    if rec[0] == rva:
        return "entry is a .pdata start -- the two methods disagree about the bytes"
    kind = "chained" if rec[2] else "primary"
    return f"inside the {kind} .pdata record at {img.base + rec[0]:#x}, not at its start"


def explain_pdata_only(img: Image, va: int, ghidra: list[int]) -> str:
    import bisect

    rec = covering_record(img, va - img.base)
    kind = "chained record: a fragment of a function, not a start" if rec and rec[2] else "primary record"
    i = bisect.bisect_right(ghidra, va) - 1
    owner = f"nearest Ghidra entry below is {ghidra[i]:#x}" if i >= 0 else "below every Ghidra entry"
    return f"{kind}; Ghidra has no function here ({owner})"


def diff(img: Image, ghidra: list[int]) -> tuple[set[int], set[int]]:
    import collections

    red = pdata_redirects(img)
    pdata_set = {img.base + r for r in red}
    ghidra_red = {va for va in ghidra if img.redirect_target(va - img.base) is not None}
    in_text1 = {va for va in ghidra_red if img.section_of(va - img.base) == ".text#1"}
    g_only = ghidra_red - pdata_set
    p_only = pdata_set - ghidra_red
    print(f"ghidra functions           {len(ghidra)}")
    print(f"  arxan-redirected         {len(ghidra_red)}")
    print(f"    entry in .text#1       {len(in_text1)}")
    print(f"    entry in .text#2+      {len(ghidra_red) - len(in_text1)}")
    print(f"in both                    {len(ghidra_red & pdata_set)}")
    union = ghidra_red | pdata_set
    targets = {img.redirect_target(va - img.base) for va in union}
    print(f"union                      {len(union)}")
    print(f"  distinct stub targets    {len(targets)}")
    srt = sorted(ghidra)
    g_why = {va: explain_ghidra_only(img, va) for va in g_only}
    p_why = {va: explain_pdata_only(img, va, srt) for va in p_only}
    print(f"ghidra only                {len(g_only)}")
    for why, n in collections.Counter(w.split(" at 0x")[0] for w in g_why.values()).most_common():
        print(f"  {n:5d}  {why}")
    print(f".pdata only                {len(p_only)}")
    for why, n in collections.Counter(w.split(" (")[0] for w in p_why.values()).most_common():
        print(f"  {n:5d}  {why}")
    print("ghidra only, each:")
    for va in sorted(g_only):
        print(f"  {va:#x} -> {img.base + img.redirect_target(va - img.base):#x}  {g_why[va]}")
    print(".pdata only, each:")
    for va in sorted(p_only):
        print(f"  {va:#x} -> {img.base + red[va - img.base]:#x}  {p_why[va]}")
    return g_only, p_only


def same_image(a: Path, b: Path) -> bool:
    ia, ib = load(a), load(b)
    if len(ia.mem) != len(ib.mem):
        print(f"mapped sizes differ: {len(ia.mem):#x} vs {len(ib.mem):#x}")
        return False
    pages = [p for p in range(0, len(ia.mem), 0x1000) if ia.mem[p : p + 0x1000] != ib.mem[p : p + 0x1000]]
    print(f"{a.name} ({'flat' if ia.flat else 'file'}) vs {b.name} ({'flat' if ib.flat else 'file'}): "
          f"{len(pages)} of {len(ia.mem) // 0x1000} mapped pages differ")
    for p in pages[:20]:
        print(f"  rva {p:#x} in {ia.section_of(p)}")
    return not pages


def build_test_pe(flat: bool) -> bytes:
    """A tiny PE32+ with .text, .pdata, .text: three functions, one redirect, one chained record."""
    base = 0x140000000
    secs = [(".text", 0x1000), (".pdata", 0x2000), (".text", 0x3000)]
    hdr = bytearray(0x400)
    hdr[0:2] = b"MZ"
    struct.pack_into("<I", hdr, 0x3C, 0x80)
    hdr[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HH", hdr, 0x84, 0x8664, len(secs))
    struct.pack_into("<H", hdr, 0x94, 240)
    opt = 0x98
    struct.pack_into("<H", hdr, opt, 0x20B)
    struct.pack_into("<Q", hdr, opt + 24, base)
    struct.pack_into("<II", hdr, opt + 56, 0x4000, 0x400)
    struct.pack_into("<II", hdr, opt + 112 + 24, 0x2000, 4 * 12)
    for i, (name, va) in enumerate(secs):
        struct.pack_into("<8sIIII", hdr, opt + 240 + i * 40, name.encode(), 0x1000, va, 0x200, 0x400 + i * 0x200)
    text = bytearray(0x200)
    text[0x00:0x05] = b"\xe9" + struct.pack("<i", 0x3010 - (0x1000 + 5))   # redirect into .text#2
    text[0x20:0x25] = b"\xe9" + struct.pack("<i", 0x1100 - (0x1020 + 5))   # jmp within .text#1
    text[0x40:0x44] = b"\x48\x83\xec\x28"                                    # ordinary prologue
    text[0x60:0x65] = b"\xe9" + struct.pack("<i", 0x3020 - (0x1060 + 5))   # redirect, chained record
    text[0x100] = 0x01                                                       # unwind info, version 1
    text[0x110] = 0x01 | (UNW_FLAG_CHAININFO << 3)
    pdata = bytearray(0x200)
    for i, (b, e, u) in enumerate([(0x1000, 0x1010, 0x1100), (0x1020, 0x1030, 0x1100),
                                   (0x1040, 0x1050, 0x1100), (0x1060, 0x1070, 0x1110)]):
        struct.pack_into("<III", pdata, i * 12, b, e, u)
    struct.pack_into("<III", pdata, 4 * 12, 0x1080, 0x1090, 0x1100)  # past the directory: padding
    pdata[4 * 12] = 0xE9
    text2 = bytearray(0x200)
    text2[0x10:0x15] = b"\xe9" + struct.pack("<i", 0x3020 - (0x3010 + 5))  # a thunk inside .text#2
    bodies = [text, pdata, text2]
    if flat:
        img = bytearray(0x4000)
        img[:0x400] = hdr
        for (_n, va), body in zip(secs, bodies):
            img[va : va + len(body)] = body
        return bytes(img)
    return bytes(hdr) + b"".join(bytes(b) for b in bodies)


def selftest() -> int:
    flat, filed = Image(build_test_pe(True)), Image(build_test_pe(False))
    assert flat.flat and not filed.flat
    assert flat.mem[0x1000:0x4000] == filed.mem[0x1000:0x4000], "file layout maps to the flat image"
    assert len(flat.runtime_functions()) == 4, "the directory size bounds the table, not the section"
    red = pdata_redirects(flat)
    assert red == {0x1000: 0x3010, 0x1060: 0x3020}, red
    assert [c for _b, _e, c in flat.runtime_functions()] == [False, False, False, True]
    assert flat.redirect_target(0x1020) is None, "a jmp inside .text#1 is not a redirect"
    assert flat.redirect_target(0x3010) == 0x3020
    base = flat.base
    import contextlib
    import io

    with contextlib.redirect_stdout(io.StringIO()):
        g_only, p_only = diff(flat, [base + 0x1000, base + 0x1020, base + 0x1040, base + 0x1068, base + 0x3010])
    assert g_only == {base + 0x3010}, g_only
    assert p_only == {base + 0x1060}, p_only
    assert "Arxan's own block" in explain_ghidra_only(flat, base + 0x3010)
    assert "no .pdata record covers it" in explain_ghidra_only(flat, base + 0x1058)
    assert "inside the chained .pdata record at 0x140001060" in explain_ghidra_only(flat, base + 0x1068)
    why = explain_pdata_only(flat, base + 0x1060, [base + 0x1000, base + 0x1040])
    assert why.startswith("chained record") and "0x140001040" in why, why
    print("selftest ok")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--image", type=Path, default=DEFAULT_IMAGE)
    ap.add_argument("--list", action="store_true", help="print every .pdata redirect")
    ap.add_argument("--ghidra", action="store_true", help="diff against the Ghidra daemon's functions")
    ap.add_argument("--port", type=int, default=8766, help="Ghidra MCP daemon port (8766 is DS2)")
    ap.add_argument("--save-ghidra", type=Path, help="write the fetched Ghidra entry points here")
    ap.add_argument("--ghidra-list", type=Path, help="diff against saved Ghidra entry points")
    ap.add_argument("--same-image", nargs=2, type=Path, metavar=("A", "B"))
    ap.add_argument("--records", nargs=2, metavar=("LO", "HI"),
                    help="dump the .pdata records whose start VA is in [LO, HI)")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args(argv)
    if args.selftest:
        return selftest()
    if args.records:
        img = load(args.image)
        lo, hi = (int(x, 16) - img.base for x in args.records)
        for b, e, c in img.runtime_functions():
            if lo <= b < hi:
                t = img.redirect_target(b)
                print(f"{img.base + b:#x}-{img.base + e:#x}  {'chained' if c else 'primary'}  "
                      f"{img.mem[b:b + 5].hex(' ')}"
                      + (f"  -> {img.base + t:#x}" if t is not None else ""))
        return 0
    if args.same_image:
        return 0 if same_image(*args.same_image) else 1
    img = load(args.image)
    red = census(img)
    if args.list:
        for rva in sorted(red):
            print(f"  {img.base + rva:#x} -> {img.base + red[rva]:#x}")
    if args.ghidra or args.ghidra_list:
        entries = (read_entry_list(args.ghidra_list) if args.ghidra_list
                   else ghidra_entries(args.port, args.save_ghidra))
        diff(img, entries)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
