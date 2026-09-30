#!/usr/bin/env python3
"""Describe atlas rects by numbers, so a piece's role can be read without looking at it.

    uv run --with pillow --with numpy scripts/ds2-atlas-rects.py /tmp/fe/waku_02.dds \
        2.95,81.15,229.05,84.8  952.55,96.3,1024,283.75

For each rect `x0,y0,x1,y1` (texture pixels, the numbers a `.flo` quad's source pointer names):
size, alpha coverage, premultiplied mean colour, and the alpha + luma PROFILE along each axis --
the mean over the other axis, resampled to `--bins` buckets. A border line shows as a narrow
bright band in the cross-axis profile; a soft panel as a ramp; a fade-out at a line's end as a
falling tail in the long-axis profile. `--rows` prints every pixel row's mean alpha/luma instead
of buckets for the cross axis, which is how a 3-pixel-thick line is measured exactly.
"""

from __future__ import annotations

import argparse

import numpy as np
from PIL import Image


def profile(values: np.ndarray, bins: int) -> str:
    if len(values) == 0:
        return "-"
    edges = np.linspace(0, len(values), min(bins, len(values)) + 1).astype(int)
    return " ".join(f"{values[a:b].mean():3.0f}" for a, b in zip(edges[:-1], edges[1:]) if b > a)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dds")
    ap.add_argument("rects", nargs="+", help="x0,y0,x1,y1")
    ap.add_argument("--bins", type=int, default=16)
    ap.add_argument("--rows", action="store_true", help="per-pixel cross-axis profile")
    args = ap.parse_args()

    img = np.asarray(Image.open(args.dds).convert("RGBA")).astype(np.float32)
    h, w = img.shape[:2]
    for spec in args.rects:
        x0, y0, x1, y1 = (float(v) for v in spec.split(","))
        xa, xb = sorted((x0, x1))
        ya, yb = sorted((y0, y1))
        ix0, iy0 = max(0, int(np.floor(xa))), max(0, int(np.floor(ya)))
        ix1, iy1 = min(w, int(np.ceil(xb))), min(h, int(np.ceil(yb)))
        crop = img[iy0:iy1, ix0:ix1]
        if crop.size == 0:
            print(f"{spec}: empty")
            continue
        a = crop[..., 3]
        luma = 0.299 * crop[..., 0] + 0.587 * crop[..., 1] + 0.114 * crop[..., 2]
        cov = (a > 16).mean() * 100
        wsum = a.sum()
        mean_rgb = (
            tuple(int((crop[..., c] * a).sum() / wsum) for c in range(3)) if wsum else (0, 0, 0)
        )
        print(
            f"rect ({xa:g},{ya:g})-({xb:g},{yb:g})  {xb - xa:.2f}x{yb - ya:.2f}  "
            f"alpha>16 {cov:5.1f}%  alpha mean {a.mean():5.1f} max {a.max():3.0f}  "
            f"rgb(alpha-weighted) #{mean_rgb[0]:02x}{mean_rgb[1]:02x}{mean_rgb[2]:02x}"
        )
        print(f"  alpha by column (x->): {profile(a.mean(axis=0), args.bins)}")
        print(f"  alpha by row    (y v): {profile(a.mean(axis=1), args.bins)}")
        print(f"  luma  by column (x->): {profile(luma.mean(axis=0), args.bins)}")
        print(f"  luma  by row    (y v): {profile(luma.mean(axis=1), args.bins)}")
        if args.rows:
            for r in range(crop.shape[0]):
                row = crop[r]
                ra = row[..., 3]
                rw = ra.sum()
                rgb = tuple(int((row[..., c] * ra).sum() / rw) for c in range(3)) if rw else (0, 0, 0)
                print(
                    f"    y={iy0 + r:4d} alpha {ra.mean():5.1f}  "
                    f"#{rgb[0]:02x}{rgb[1]:02x}{rgb[2]:02x}"
                )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
