#!/usr/bin/env python3
"""Draw a frontend atlas whole, with a pixel ruler, so art no `.flo` references is still visible.

    uv run --with pillow scripts/ds2-atlas-sheet.py /tmp/fe --out /tmp/sheets/atlas --scale 2

`ds2-flo-sheet.py` can only draw what a document claims: it crops the rects the shape table names
and nothing else. An atlas holds more than that -- `l02_02_Inventory.flo` claims 88 rects out of
`In-game_01`'s whole megapixel, and the art in the gaps is the art nothing in this repo has ever
seen. This draws every pixel, with a 32-unit grid and coordinates down the margins, so a glyph
found by eye can be named as a rect and handed straight to `FE_ITEM_WARN_*`.

Input is a directory of `.dds` files, which is what `scripts/ds2-tpf.py extract` writes.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

FONT = "/usr/share/fonts/TTF/DejaVuSansMono.ttf"
MARGIN = 44


def font(size: int) -> ImageFont.FreeTypeFont:
    try:
        return ImageFont.truetype(FONT, size)
    except OSError:
        return ImageFont.load_default(size=size)


def checkerboard(width: int, height: int, cell: int = 8) -> Image.Image:
    board = Image.new("RGBA", (width, height), (48, 48, 52, 255))
    draw = ImageDraw.Draw(board)
    for y in range(0, height, cell):
        for x in range(0, width, cell):
            if (x // cell + y // cell) % 2:
                draw.rectangle([x, y, x + cell - 1, y + cell - 1], fill=(72, 72, 78, 255))
    return board


def sheet(dds: Path, out_dir: Path, scale: int, grid: int) -> Path:
    art = Image.open(dds).convert("RGBA")
    board = checkerboard(art.width, art.height)
    board.alpha_composite(art)
    if scale != 1:
        board = board.resize((board.width * scale, board.height * scale), Image.NEAREST)

    page = Image.new("RGBA", (board.width + MARGIN, board.height + MARGIN), (18, 18, 22, 255))
    page.alpha_composite(board, (MARGIN, MARGIN))
    draw = ImageDraw.Draw(page)
    small = font(12)
    for x in range(0, art.width + 1, grid):
        at = MARGIN + x * scale
        heavy = x % (grid * 4) == 0
        draw.line([(at, MARGIN), (at, page.height)], fill=(255, 255, 255, 60 if heavy else 22))
        if heavy:
            draw.text((at + 2, 4), str(x), font=small, fill=(210, 210, 215, 255))
    for y in range(0, art.height + 1, grid):
        at = MARGIN + y * scale
        heavy = y % (grid * 4) == 0
        draw.line([(MARGIN, at), (page.width, at)], fill=(255, 255, 255, 60 if heavy else 22))
        if heavy:
            draw.text((2, at + 2), str(y), font=small, fill=(210, 210, 215, 255))
    draw.text((2, 22), f"{dds.stem} {art.width}x{art.height}", font=small, fill=(255, 220, 64, 255))

    target = out_dir / f"atlas-{dds.stem}.png"
    page.save(target)
    return target


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("tex", type=Path, help="directory of .dds files")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--scale", type=int, default=1)
    parser.add_argument("--grid", type=int, default=32, help="ruler spacing in atlas pixels")
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    for dds in sorted(args.tex.glob("*.dds")):
        print(f"  {sheet(dds, args.out, args.scale, args.grid)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
