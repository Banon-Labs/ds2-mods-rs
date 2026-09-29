#!/usr/bin/env python3
"""Read DARK SOULS II's own bitmap fonts and check the `.ccm` layout against the real files.

    scripts/ds2-fefont.py check            # every layout claim below, against both faces
    scripts/ds2-fefont.py glyph Big A      # one glyph's cell: '#' fill, 'o' outline, ' ' clear

`Game/font/English/FeFont_{Big,Small}.fontbnd.dcx` is a `DCX`/`DFLT` zlib stream (`DFLT` at 0x28, payload at
0x4c, inflated size big-endian at 0x1c) around a `BND4` whose names are single-byte (header
+0x30 = 0). It holds `FeFont_X.ccm` and one `.tpf` per texture page:

    page 0   FeFont_X_0000_win64.tpf   512x512 DXT5   circled numbers, squared CJK symbols only
    page n   FeFont_X_000n.tpf         256x256        DXT5 in Big (1..13), DXT3 in Small (1..7)

# The `.ccm`, version 0x20000

    +0x00 u32 version 0x20000          +0x04 u32 file size
    +0x08 u16 line height (41 / 28)    +0x0a u16 tex width 256   +0x0c u16 tex height 256
    +0x0e u16 region count             +0x10 u16 glyph count     +0x12 u16 0
    +0x14 u32 region table offset      +0x18 u32 glyph table offset
    +0x1c u8 4, u8 0, u8 page count (14 / 8), u8 0
    regions, 8 bytes:  u16 x1, y1, x2, y2              x2/y2 exclusive
    glyphs, 24 bytes:  u32 code, u32 region OFFSET (bytes from file start, not an index),
                       i16 page, i16 pre-space, i16 width, i16 advance, u32 0, u32 0

The ink is drawn `pre-space` right of the pen, and the pen then moves `pre-space + advance`:
`advance` is measured from the ink's left edge, not from the pen. The digits are what show it --
their `pre + advance` is one tabular step in both faces (Big 23: '0' 2+21, '1' 5+18, '4' 0+23;
Small 15), while `advance` alone is not -- and the CJK brackets agree (Big '「' pre 23, advance
11: a full-width 34, where `advance` alone would put the next glyph on top of this one's ink).
`width` equals the region's width for nearly every glyph and is not needed to draw. A page-0
region is in that page's own 512-pixel space, so a region is read as pixels of the page it names
rather than scaled by the declared 256.

The ink is a light grey fill (~197..205) inside a one-pixel near-black (~16) outline, alpha
covering both -- not a white mask.

`check` exits non-zero on the first claim the files contradict.
"""

from __future__ import annotations

import argparse
import struct
import sys
import zlib
from pathlib import Path

GAME_DIR = (
    Path.home()
    / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
)
DCX_PAYLOAD = 0x4C
EXPECTED = {
    # face: (line height, page count, page-1+ format)
    "Big": (41, 14, b"DXT5"),
    "Small": (28, 8, b"DXT3"),
}


def bnd4(path: Path) -> dict[str, bytes]:
    dcx = path.read_bytes()
    assert dcx[:4] == b"DCX\0" and dcx[0x24:0x28] == b"DCP\0" and dcx[0x28:0x2C] == b"DFLT", "not a DCX/DFLT"
    data = zlib.decompress(dcx[DCX_PAYLOAD:])
    assert len(data) == struct.unpack_from(">I", dcx, 0x1C)[0], "DCX size mismatch"
    assert data[:4] == b"BND4" and data[0x30] == 0, "not a single-byte-name BND4"
    count = struct.unpack_from("<I", data, 0x0C)[0]
    header, entry = struct.unpack_from("<Q", data, 0x10)[0], struct.unpack_from("<Q", data, 0x20)[0]
    members = {}
    for index in range(count):
        size, unpacked, offset, _id, name_at = struct.unpack_from(
            "<QQIiI", data, header + index * entry + 8
        )
        assert size == unpacked, "compressed member"
        name = data[name_at : data.index(b"\0", name_at)].decode()
        members[name] = data[offset : offset + size]
    return members


def parse_ccm(ccm: bytes):
    version, size, line, tw, th, nreg, nglyph, zero, reg_at, glyph_at = struct.unpack_from(
        "<IIHHHHHHII", ccm, 0
    )
    unk, zero2, pages, zero3 = ccm[0x1C:0x20]
    glyphs = []
    for index in range(nglyph):
        code, region, page, pre, width, advance, pad1, pad2 = struct.unpack_from(
            "<IIhhhhII", ccm, glyph_at + index * 24
        )
        glyphs.append(
            dict(
                code=code,
                region=region,
                rect=struct.unpack_from("<4H", ccm, region),
                page=page,
                pre=pre,
                width=width,
                advance=advance,
                pad=(pad1, pad2),
            )
        )
    header = dict(
        version=version, size=size, line=line, tex=(tw, th), regions=nreg, glyphs=nglyph,
        zero=zero, region_at=reg_at, glyph_at=glyph_at, unk=unk, zero2=zero2, pages=pages,
        zero3=zero3,
    )
    return header, glyphs


def tpf_dds(tpf: bytes) -> tuple[int, int, bytes, bytes]:
    assert tpf[:4] == b"TPF\0"
    offset, size = struct.unpack_from("<II", tpf, 0x10)
    dds = tpf[offset : offset + size]
    height, width = struct.unpack_from("<II", dds, 12)
    return width, height, dds[84:88], dds[128:]


def rgb565(v: int) -> tuple[int, int, int]:
    return ((v >> 11) & 31) * 255 // 31, ((v >> 5) & 63) * 255 // 63, (v & 31) * 255 // 31


def decode(width: int, height: int, fourcc: bytes, blocks: bytes) -> list[list[tuple]]:
    """DXT3/DXT5 to rows of (r, g, b, a)."""
    px = [[(0, 0, 0, 0)] * width for _ in range(height)]
    at = 0
    for by in range(0, height, 4):
        for bx in range(0, width, 4):
            block = blocks[at : at + 16]
            at += 16
            if fourcc == b"DXT5":
                a0, a1 = block[0], block[1]
                bits = int.from_bytes(block[2:8], "little")
                if a0 > a1:
                    table = [a0, a1] + [((6 - i) * a0 + (i + 1) * a1) // 7 for i in range(6)]
                else:
                    table = [a0, a1] + [((4 - i) * a0 + (i + 1) * a1) // 5 for i in range(4)]
                    table += [0, 255]
                alpha = [table[(bits >> (3 * i)) & 7] for i in range(16)]
            else:
                bits = int.from_bytes(block[:8], "little")
                alpha = [((bits >> (4 * i)) & 15) * 17 for i in range(16)]
            c0, c1, idx = struct.unpack_from("<HHI", block, 8)
            p0, p1 = rgb565(c0), rgb565(c1)
            palette = [
                p0, p1,
                tuple((2 * a + b) // 3 for a, b in zip(p0, p1)),
                tuple((a + 2 * b) // 3 for a, b in zip(p0, p1)),
            ]
            for i in range(16):
                px[by + i // 4][bx + i % 4] = palette[(idx >> (2 * i)) & 3] + (alpha[i],)
    return px


def page_of(members: dict[str, bytes], face: str, page: int) -> bytes:
    suffix = "_win64" if page == 0 else ""
    return members[f"FeFont_{face}_{page:04d}{suffix}.tpf"]


def check(game: Path) -> int:
    for face, (line, page_count, fourcc) in EXPECTED.items():
        members = bnd4(game / "font/English" / f"FeFont_{face}.fontbnd.dcx")
        ccm = members[f"FeFont_{face}.ccm"]
        header, glyphs = parse_ccm(ccm)
        claims = [
            ("version is 0x20000", header["version"] == 0x20000),
            ("+0x04 is the file size", header["size"] == len(ccm)),
            (f"line height is {line}", header["line"] == line),
            ("declared texture is 256x256", header["tex"] == (256, 256)),
            ("+0x12 and the pad bytes are zero", header["zero"] == header["zero2"] == header["zero3"] == 0),
            ("regions start at 0x20", header["region_at"] == 0x20),
            ("glyphs follow the regions", header["glyph_at"] == 0x20 + 8 * header["regions"]),
            ("glyph table ends at the file end", header["glyph_at"] + 24 * header["glyphs"] == len(ccm)),
            (f"page count is {page_count}", header["pages"] == page_count),
            ("one .tpf per page", sum(n.endswith(".tpf") for n in members) == page_count),
            ("codes are sorted and unique",
             [g["code"] for g in glyphs] == sorted({g["code"] for g in glyphs})),
            ("every region offset is on the region table",
             all((g["region"] - 0x20) % 8 == 0 and g["region"] < header["glyph_at"] for g in glyphs)),
            ("every region is used", len({g["region"] for g in glyphs}) == header["regions"]),
            ("glyph pad words are zero", all(g["pad"] == (0, 0) for g in glyphs)),
            ("printable ASCII is all there",
             {g["code"] for g in glyphs} >= set(range(0x20, 0x7F))),
            ("every page >= 1 glyph cell is the full line tall",
             all(g["rect"][3] - g["rect"][1] == line for g in glyphs if g["page"] >= 1)),
        ]
        sizes = {}
        for page in range(page_count):
            w, h, cc, _ = tpf_dds(page_of(members, face, page))
            sizes[page] = (w, h, cc)
        claims += [
            ("page 0 is 512x512 DXT5", sizes[0] == (512, 512, b"DXT5")),
            (f"pages 1.. are 256x256 {fourcc.decode()}",
             all(sizes[p] == (256, 256, fourcc) for p in range(1, page_count))),
            ("page 0 holds nothing below U+2000",
             all(g["code"] >= 0x2000 for g in glyphs if g["page"] == 0)),
        ]
        well_formed = [g for g in glyphs if g["rect"][2] > g["rect"][0] and g["rect"][3] > g["rect"][1]]
        claims.append((
            "every well-formed region is inside its page",
            all(g["rect"][2] <= sizes[g["page"]][0] and g["rect"][3] <= sizes[g["page"]][1]
                for g in well_formed),
        ))
        by_code = {g["code"]: g for g in glyphs}
        steps = {by_code[ord(d)]["pre"] + by_code[ord(d)]["advance"] for d in "0123456789"}
        bare = {by_code[ord(d)]["advance"] for d in "0123456789"}
        claims.append((f"digits share one pen step pre+advance ({sorted(steps)})",
                       len(steps) == 1 and len(bare) > 1))
        w, h, cc, blocks = tpf_dds(page_of(members, face, by_code[ord("A")]["page"]))
        px = decode(w, h, cc, blocks)
        x1, y1, x2, y2 = by_code[ord("A")]["rect"]
        inked = [px[y][x] for y in range(y1, y2) for x in range(x1, x2) if px[y][x][3] > 128]
        light = sum(1 for p in inked if p[0] > 150)
        dark = sum(1 for p in inked if p[0] < 40)
        claims.append(("'A' ink is light fill plus dark outline", light > 20 and dark > 20))
        print(f"FeFont_{face}: {header['glyphs']} glyphs, {header['regions']} regions, "
              f"line {header['line']}, {len(glyphs) - len(well_formed)} malformed region(s), "
              f"'A' = {by_code[ord('A')]}")
        for claim, ok in claims:
            print(f"  {'ok ' if ok else 'BAD'} {claim}")
            if not ok:
                return 1
    return 0


def glyph(game: Path, face: str, char: str) -> int:
    members = bnd4(game / "font/English" / f"FeFont_{face}.fontbnd.dcx")
    _, glyphs = parse_ccm(members[f"FeFont_{face}.ccm"])
    found = next((g for g in glyphs if g["code"] == ord(char)), None)
    if found is None:
        print(f"{char!r} is not in FeFont_{face}", file=sys.stderr)
        return 1
    print(found)
    w, h, cc, blocks = tpf_dds(page_of(members, face, found["page"]))
    px = decode(w, h, cc, blocks)
    x1, y1, x2, y2 = found["rect"]
    for y in range(y1, y2):
        print("".join(
            " " if px[y][x][3] < 64 else ("#" if px[y][x][0] > 100 else "o") for x in range(x1, x2)
        ))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--game-dir", type=Path, default=GAME_DIR)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("check")
    one = sub.add_parser("glyph")
    one.add_argument("face", choices=sorted(EXPECTED))
    one.add_argument("char")
    args = parser.parse_args()
    if args.command == "check":
        return check(args.game_dir)
    return glyph(args.game_dir, args.face, args.char)


if __name__ == "__main__":
    sys.exit(main())
