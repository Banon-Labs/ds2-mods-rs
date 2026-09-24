#!/usr/bin/env python3
"""Draw a `.flo`'s art: every shape cropped out of its atlas, labelled with the id that names it.

    uv run --with pillow scripts/ds2-flo-sheet.py \\
        /tmp/menu02/l02_02_Inventory.flo --tex /tmp/fe --out /tmp/sheets

# What this is for

`ds2-item-warn` draws its badge by cloning an infusion glyph and tinting it red, because nothing
in this repo could say what art the atlas holds -- `scripts/ds2-flo.py` reads rects and the rects
are just numbers. This turns the numbers into pictures a person can look at, so "is there an X in
there" stops being a question that needs a texture viewer and a guess.

Two sheets per document:

* `<doc>-tiles.png` -- one cell per shape, the shape's own atlas crop scaled up on a checkerboard,
  with its SHAPE INDEX (what `--shape` takes) and source rect under it.
* `<doc>-<texture>.png` -- the whole atlas at native size with every claimed rect outlined and
  numbered. Art no shape claims shows through unmarked, which is the only way to see it: a rect
  table cannot name what it does not reference.

# Where a quad says which texture it samples

The quad is `0x40` bytes (`scripts/ds2-flo.py`). `+0x20` is a file offset into the TEXTURE TABLE,
whose start the document header keeps at `+0x30` and whose entries are `0x18` bytes:

    +0x00 u64 name offset   +0x08 u32 id   +0x0e u16 width   +0x10 u16 height

`+0x30` of the quad is the source rect, four floats `(x0, y0, x1, y1)` in that texture's pixels.
Checked on `l02_02_Inventory.flo`: the quad of shape `0x1a` points at table entry 12, which is id
15, `In-game_01`, `1024x1024` -- and the nine infusion glyphs' rects all land inside it.

The `.dds` files come from `scripts/ds2-tpf.py extract <name>.tpf --out <dir>`; `--tex` is that
directory. A texture with no file is listed and skipped rather than silently dropped.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

#: The document header fields this needs, both file offsets.
TEXTURE_TABLE_FIELD = 0x30
TEXTURE_TABLE_END_FIELD = 0x40
TEXTURE_STRIDE = 0x18

SHAPE_TABLE_FIELD = 0x08
SHAPE_COUNT_FIELD = 0x48
SHAPE_STRIDE = 0x18
SHAPE_QUAD_COUNT_FIELD = 0x02
SHAPE_QUADS_FIELD = 0x08

QUAD_STRIDE = 0x40
QUAD_TEXTURE_FIELD = 0x20
QUAD_SOURCE_FIELD = 0x30

FONT = "/usr/share/fonts/TTF/DejaVuSansMono.ttf"


def font(size: int) -> ImageFont.FreeTypeFont:
    try:
        return ImageFont.truetype(FONT, size)
    except OSError:
        return ImageFont.load_default(size=size)


def checkerboard(width: int, height: int, cell: int = 8) -> Image.Image:
    """A background that makes transparent art readable instead of invisible."""
    board = Image.new("RGBA", (width, height), (48, 48, 52, 255))
    draw = ImageDraw.Draw(board)
    for y in range(0, height, cell):
        for x in range(0, width, cell):
            if (x // cell + y // cell) % 2:
                draw.rectangle([x, y, x + cell - 1, y + cell - 1], fill=(72, 72, 78, 255))
    return board


class Document:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.blob = path.read_bytes()

    def textures(self) -> dict[int, dict[str, object]]:
        """Texture table entries, keyed by their file offset so a quad's pointer resolves."""
        start = struct.unpack_from("<Q", self.blob, TEXTURE_TABLE_FIELD)[0]
        end = struct.unpack_from("<Q", self.blob, TEXTURE_TABLE_END_FIELD)[0]
        out = {}
        for at in range(start, end, TEXTURE_STRIDE):
            name_at, tid, _u1, width, height = struct.unpack_from("<QIHHH", self.blob, at)
            name = self.blob[name_at : self.blob.index(b"\0", name_at)].decode("ascii", "replace")
            out[at] = {"id": tid, "name": name, "width": width, "height": height}
        return out

    def shapes(self) -> list[dict[str, object]]:
        """Every shape, with each quad's texture entry and source rect."""
        table = struct.unpack_from("<Q", self.blob, SHAPE_TABLE_FIELD)[0]
        count = struct.unpack_from("<H", self.blob, SHAPE_COUNT_FIELD)[0]
        out = []
        for i in range(count):
            at = table + i * SHAPE_STRIDE
            key = struct.unpack_from("<H", self.blob, at)[0]
            quad_count = struct.unpack_from("<H", self.blob, at + SHAPE_QUAD_COUNT_FIELD)[0]
            quads_at = struct.unpack_from("<Q", self.blob, at + SHAPE_QUADS_FIELD)[0]
            quads = []
            for q in range(quad_count):
                quad = quads_at + q * QUAD_STRIDE
                texture = struct.unpack_from("<Q", self.blob, quad + QUAD_TEXTURE_FIELD)[0]
                source = struct.unpack_from("<Q", self.blob, quad + QUAD_SOURCE_FIELD)[0]
                rect = (
                    struct.unpack_from("<4f", self.blob, source)
                    if 0 < source < len(self.blob) - 0x10
                    else None
                )
                quads.append({"texture": texture, "rect": rect})
            out.append({"index": key, "quads": quads})
        return out


def atlas_for(name: str, tex_dir: Path) -> Image.Image | None:
    for candidate in (tex_dir / f"{name}.dds", tex_dir / f"{name}.DDS", tex_dir / f"{name}.png"):
        if candidate.exists():
            return Image.open(candidate).convert("RGBA")
    return None


def crop(atlas: Image.Image, rect: tuple[float, float, float, float]) -> Image.Image | None:
    x0, y0, x1, y1 = rect
    box = (
        max(0, int(round(min(x0, x1)))),
        max(0, int(round(min(y0, y1)))),
        min(atlas.width, int(round(max(x0, x1)))),
        min(atlas.height, int(round(max(y0, y1)))),
    )
    if box[2] - box[0] < 1 or box[3] - box[1] < 1:
        return None
    return atlas.crop(box)


def tile_sheet(
    document: Document,
    tex_dir: Path,
    out: Path,
    cell: int = 128,
    columns: int = 8,
    only: set[int] | None = None,
) -> Path:
    """One cell per shape: its art, its index, and the rect it samples.

    `only` narrows it to named shape indices, which is how a candidate handful gets looked at
    without hunting for them among eighty-eight.
    """
    textures = document.textures()
    atlases: dict[str, Image.Image | None] = {}
    cells: list[tuple[str, list[str], Image.Image | None]] = []
    for shape in document.shapes():
        if only is not None and int(shape["index"]) not in only:
            continue
        art = None
        lines = [f"shape {shape['index']:#06x}"]
        for quad in shape["quads"]:
            entry = textures.get(quad["texture"])
            if entry is None or quad["rect"] is None:
                continue
            name = str(entry["name"])
            if name not in atlases:
                atlases[name] = atlas_for(name, tex_dir)
            atlas = atlases[name]
            lines.append(name)
            x0, y0, x1, y1 = quad["rect"]
            lines.append(f"{min(x0,x1):.0f},{min(y0,y1):.0f} {abs(x1-x0):.0f}x{abs(y1-y0):.0f}")
            if atlas is not None and art is None:
                art = crop(atlas, quad["rect"])
            break
        cells.append((f"{shape['index']:#06x}", lines, art))

    point = max(11, cell // 14)
    label_height = point * 4
    rows = (len(cells) + columns - 1) // columns
    sheet = Image.new("RGBA", (columns * cell, rows * (cell + label_height)), (24, 24, 28, 255))
    draw = ImageDraw.Draw(sheet)
    small = font(point)
    for i, (_key, lines, art) in enumerate(cells):
        cx = (i % columns) * cell
        cy = (i // columns) * (cell + label_height)
        pane = checkerboard(cell - 8, cell - 8)
        if art is not None:
            scale = min((cell - 16) / art.width, (cell - 16) / art.height, cell / 16)
            resized = art.resize(
                (max(1, int(art.width * scale)), max(1, int(art.height * scale))), Image.NEAREST
            )
            pane.alpha_composite(
                resized, ((pane.width - resized.width) // 2, (pane.height - resized.height) // 2)
            )
        sheet.alpha_composite(pane, (cx + 4, cy + 4))
        for row, text in enumerate(lines[:3]):
            draw.text(
                (cx + 5, cy + cell + row * (point + 2) - 4), text, font=small, fill=(220, 220, 225, 255)
            )
    sheet.save(out)
    return out


def atlas_sheets(document: Document, tex_dir: Path, out_dir: Path, scale: int = 1) -> list[Path]:
    """Each atlas at native size, with every rect the document claims outlined and numbered."""
    textures = document.textures()
    claimed: dict[str, list[tuple[int, tuple[float, float, float, float]]]] = {}
    for shape in document.shapes():
        for quad in shape["quads"]:
            entry = textures.get(quad["texture"])
            if entry is None or quad["rect"] is None:
                continue
            claimed.setdefault(str(entry["name"]), []).append((int(shape["index"]), quad["rect"]))
    written = []
    for name, rects in sorted(claimed.items()):
        atlas = atlas_for(name, tex_dir)
        if atlas is None:
            print(f"  {name}: no .dds in {tex_dir} -- skipped")
            continue
        board = checkerboard(atlas.width, atlas.height)
        board.alpha_composite(atlas)
        if scale != 1:
            board = board.resize((board.width * scale, board.height * scale), Image.NEAREST)
        draw = ImageDraw.Draw(board)
        small = font(11 if scale == 1 else 11 * scale)
        for index, (x0, y0, x1, y1) in rects:
            box = [
                min(x0, x1) * scale,
                min(y0, y1) * scale,
                max(x0, x1) * scale,
                max(y0, y1) * scale,
            ]
            draw.rectangle(box, outline=(255, 64, 64, 255), width=max(1, scale))
            draw.text((box[0] + 2, box[1] + 1), f"{index:#x}", font=small, fill=(255, 220, 64, 255))
        target = out_dir / f"{document.path.stem}-{name}.png"
        board.save(target)
        written.append(target)
    return written


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("file", type=Path, help="the .flo to draw")
    parser.add_argument("--tex", type=Path, required=True, help="directory of extracted .dds atlases")
    parser.add_argument("--out", type=Path, required=True, help="directory to write the sheets into")
    parser.add_argument("--scale", type=int, default=1, help="magnify the atlas sheets")
    parser.add_argument("--columns", type=int, default=8)
    parser.add_argument("--cell", type=int, default=128, help="tile size in the tile sheet")
    parser.add_argument(
        "--shapes",
        help="comma-separated shape indices to draw ALONE, e.g. 0x57,0x59,0x5b -- skips the atlas sheets",
    )
    parser.add_argument("--name", default="tiles", help="what to call the tile sheet")
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    document = Document(args.file)
    print(f"{args.file.name}: {len(document.shapes())} shapes, {len(document.textures())} textures")
    only = {int(s, 0) for s in args.shapes.split(",")} if args.shapes else None
    tiles = tile_sheet(
        document,
        args.tex,
        args.out / f"{args.file.stem}-{args.name}.png",
        cell=args.cell,
        columns=args.columns,
        only=only,
    )
    print(f"  {tiles}")
    if only is None:
        for path in atlas_sheets(document, args.tex, args.out, scale=args.scale):
            print(f"  {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
