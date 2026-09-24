#!/usr/bin/env python3
"""Find art in an atlas by COLOUR, and report it as rects a `.flo` can be pointed at.

    uv run --with pillow --with numpy scripts/ds2-atlas-find.py /tmp/fe/waku_03.dds --red
    uv run --with pillow --with numpy scripts/ds2-atlas-find.py /tmp/fe/waku_03.dds \\
        --red --claimed /tmp/menu02/l02_01_In-Game.flo

# Why this exists

An atlas is a megapixel of unlabelled art and the thing being looked for -- the game's own red
"you cannot use this" mark -- is a few hundred red pixels somewhere in it. Rendering the sheet and
asking a person to find it works but costs a round trip; this finds it arithmetically instead:
threshold on colour, label the connected blobs, print their bounding boxes. The boxes ARE the
answer, because a shape's source rect is exactly a box in this space.

`--claimed <flo>` marks each blob that overlaps a rect some shape in that document already
samples. An unclaimed blob is art nothing in that document draws, which is the interesting kind.

The default predicate is `--red`: opaque, and red clearly dominant over green and blue. `--opaque`
takes everything with alpha instead, which is how to inventory a sheet that is not colour-coded.
"""

from __future__ import annotations

import argparse
import struct
import sys
from collections import deque
from pathlib import Path

import numpy as np
from PIL import Image


def claimed_rects(flo: Path, texture: str) -> list[tuple[float, float, float, float]]:
    """Every source rect the document samples out of the named texture."""
    blob = flo.read_bytes()
    table_start = struct.unpack_from("<Q", blob, 0x30)[0]
    table_end = struct.unpack_from("<Q", blob, 0x40)[0]
    textures = {}
    for at in range(table_start, table_end, 0x18):
        name_at, _tid, _u1, _w, _h = struct.unpack_from("<QIHHH", blob, at)
        textures[at] = blob[name_at : blob.index(b"\0", name_at)].decode("ascii", "replace")
    shapes = struct.unpack_from("<Q", blob, 0x08)[0]
    count = struct.unpack_from("<H", blob, 0x48)[0]
    out = []
    for i in range(count):
        at = shapes + i * 0x18
        quad_count = struct.unpack_from("<H", blob, at + 0x02)[0]
        quads = struct.unpack_from("<Q", blob, at + 0x08)[0]
        for q in range(quad_count):
            quad = quads + q * 0x40
            if quad + 0x40 > len(blob):
                break
            texture_at = struct.unpack_from("<Q", blob, quad + 0x20)[0]
            source = struct.unpack_from("<Q", blob, quad + 0x30)[0]
            if textures.get(texture_at) != texture or not 0 < source < len(blob) - 0x10:
                continue
            x0, y0, x1, y1 = struct.unpack_from("<4f", blob, source)
            out.append((min(x0, x1), min(y0, y1), max(x0, x1), max(y0, y1)))
    return out


def blobs(mask: np.ndarray, min_pixels: int) -> list[tuple[int, int, int, int, int]]:
    """Connected components of a boolean mask as `(x0, y0, x1, y1, pixels)`, 8-connected."""
    height, width = mask.shape
    seen = np.zeros_like(mask, dtype=bool)
    found = []
    for start_y in range(height):
        row = np.nonzero(mask[start_y] & ~seen[start_y])[0]
        for start_x in row:
            if seen[start_y, start_x]:
                continue
            queue = deque([(start_y, int(start_x))])
            seen[start_y, start_x] = True
            x0 = x1 = int(start_x)
            y0 = y1 = start_y
            pixels = 0
            while queue:
                y, x = queue.popleft()
                pixels += 1
                x0, x1 = min(x0, x), max(x1, x)
                y0, y1 = min(y0, y), max(y1, y)
                for dy in (-1, 0, 1):
                    for dx in (-1, 0, 1):
                        ny, nx = y + dy, x + dx
                        if 0 <= ny < height and 0 <= nx < width and mask[ny, nx] and not seen[ny, nx]:
                            seen[ny, nx] = True
                            queue.append((ny, nx))
            if pixels >= min_pixels:
                found.append((x0, y0, x1 + 1, y1 + 1, pixels))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("dds", type=Path)
    parser.add_argument("--red", action="store_true", help="red/orange pixels only (the default)")
    parser.add_argument("--opaque", action="store_true", help="every pixel with alpha instead")
    parser.add_argument("--alpha", type=int, default=64, help="alpha floor")
    parser.add_argument("--min-pixels", type=int, default=24)
    parser.add_argument("--gap", type=int, default=0, help="dilate by this before labelling, to join strokes")
    parser.add_argument("--claimed", type=Path, help="a .flo whose claimed rects to cross off")
    parser.add_argument("--limit", type=int, default=40)
    args = parser.parse_args()

    art = np.asarray(Image.open(args.dds).convert("RGBA")).astype(np.int16)
    red, green, blue, alpha = art[..., 0], art[..., 1], art[..., 2], art[..., 3]
    if args.opaque and not args.red:
        mask = alpha > args.alpha
        what = "opaque"
    else:
        mask = (alpha > args.alpha) & (red > 90) & (red - green > 40) & (red - blue > 40)
        what = "red/orange"

    if args.gap:
        grown = mask.copy()
        for dy in range(-args.gap, args.gap + 1):
            for dx in range(-args.gap, args.gap + 1):
                grown |= np.roll(np.roll(mask, dy, axis=0), dx, axis=1)
        mask = grown

    rects = claimed_rects(args.claimed, args.dds.stem) if args.claimed else []
    found = sorted(blobs(mask, args.min_pixels), key=lambda b: -b[4])
    print(f"{args.dds.name} {art.shape[1]}x{art.shape[0]}: {mask.sum()} {what} px, {len(found)} blobs")
    for x0, y0, x1, y1, pixels in found[:args.limit]:
        patch = art[y0:y1, x0:x1]
        hit = patch[..., 3] > args.alpha
        mean = patch[hit][..., :3].mean(axis=0) if hit.any() else np.zeros(3)
        overlaps = [
            r for r in rects if not (r[2] <= x0 or r[0] >= x1 or r[3] <= y0 or r[1] >= y1)
        ]
        tag = "CLAIMED" if overlaps else "unclaimed"
        print(
            f"  ({x0},{y0})-({x1},{y1})  {x1-x0}x{y1-y0}  {pixels}px  "
            f"rgb=({mean[0]:.0f},{mean[1]:.0f},{mean[2]:.0f})  {tag}"
            + (f" by {overlaps[0][0]:.1f},{overlaps[0][1]:.1f}" if overlaps else "")
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
