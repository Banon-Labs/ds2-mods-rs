#!/usr/bin/env python3
"""List every shape quad in a set of `.flo` files that samples a named texture, and who draws it.

    python3 scripts/ds2-flo-texuse.py waku_03 /tmp/menu*/*.flo
    python3 scripts/ds2-flo-texuse.py waku_03 /tmp/menu02/l02_01_In-Game.flo --raw

Layouts are the ones `scripts/ds2-flo.py` and `scripts/ds2-flo-sheet.py` read off the loader:
shape table `[doc+0x08]` x `[doc+0x48]` at stride 0x18; quad stride 0x40 with `+0x00` offset xy,
`+0x08` scale xy, `+0x18` colour, `+0x20` texture-table entry, `+0x30` source-rect pointer;
texture table `[doc+0x30]..[doc+0x40]` at stride 0x18. `--raw` prints the other quad words too, so
fields nothing has named yet (`+0x10`, `+0x28`, `+0x38`) can be looked at rather than guessed.

For each matching shape it also prints every child record (any definition) whose `kind & 1` names
that shape, with the record's transform -- the placement the shape is drawn at.
"""

from __future__ import annotations

import argparse
import struct
from pathlib import Path


def u16(b: bytes, o: int) -> int:
    return struct.unpack_from("<H", b, o)[0]


def u64(b: bytes, o: int) -> int:
    return struct.unpack_from("<Q", b, o)[0]


def textures(b: bytes) -> dict[int, tuple[str, int, int, int]]:
    start, end = u64(b, 0x30), u64(b, 0x40)
    out = {}
    for at in range(start, end, 0x18):
        name_at, tid, _u, w, h = struct.unpack_from("<QIHHH", b, at)
        name = b[name_at : b.index(b"\0", name_at)].decode("ascii", "replace")
        out[at] = (name, tid, w, h)
    return out


def shape_users(b: bytes) -> dict[int, list[str]]:
    """shape index -> records naming it (kind & 1, not nested)."""
    table, count = u64(b, 0x18), u16(b, 0x4C)
    users: dict[int, list[str]] = {}
    for i in range(count):
        d = table + i * 0x48
        key, n = u16(b, d), u16(b, d + 2)
        arr = u64(b, d + 8)
        for c in range(n):
            r = arr + c * 0x28
            if r + 0x28 > len(b):
                break
            definition = u16(b, r)
            kind = u16(b, r + 0x12)
            if kind & 0x4 or not kind & 0x1:
                continue
            xf = u64(b, r + 8)
            x, y, sx, sy = struct.unpack_from("<4f", b, xf) if 0 < xf < len(b) - 0x30 else (0, 0, 0, 0)
            rest = struct.unpack_from("<4f", b, xf + 0x10) if 0 < xf < len(b) - 0x30 else ()
            colour = struct.unpack_from("<I", b, xf + 0x18)[0] if 0 < xf < len(b) - 0x30 else 0
            eid = struct.unpack_from("<I", b, r + 0x1C)[0]
            depth = u16(b, r + 0x10)
            users.setdefault(definition, []).append(
                f"def {key:#06x} child[{c}] id={eid:#08x} kind={kind:#x} depth={depth} "
                f"xy=({x:g},{y:g}) scale=({sx:g},{sy:g}) xf+0x10={tuple(round(v, 3) for v in rest)} "
                f"colour={colour:08x}"
            )
    return users


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("texture", help="texture name as the .flo texture table spells it, e.g. waku_03")
    ap.add_argument("files", nargs="+", type=Path)
    ap.add_argument("--raw", action="store_true", help="print all 16 dwords of each quad")
    ap.add_argument("--no-users", action="store_true")
    args = ap.parse_args()

    for path in args.files:
        b = path.read_bytes()
        tex = textures(b)
        if args.texture == "?":
            # Texture table only, with how many quads sample each entry.
            table, count = u64(b, 0x08), u16(b, 0x48)
            uses: dict[int, int] = {}
            for i in range(count):
                at = table + i * 0x18
                for q in range(u16(b, at + 2)):
                    t = u64(b, u64(b, at + 8) + q * 0x40 + 0x20)
                    uses[t] = uses.get(t, 0) + 1
            print(path.name + ": " + ", ".join(
                f"{n} {w}x{h} id={tid} quads={uses.get(at, 0)}" for at, (n, tid, w, h) in tex.items()
            ))
            continue
        # `*` takes every texture, and each quad line then names the one it samples.
        wanted = {
            at for at, t in tex.items() if args.texture == "*" or t[0].lower() == args.texture.lower()
        }
        if not wanted:
            continue
        users = {} if args.no_users else shape_users(b)
        table, count = u64(b, 0x08), u16(b, 0x48)
        for i in range(count):
            at = table + i * 0x18
            key, nq = u16(b, at), u16(b, at + 2)
            quads = u64(b, at + 8)
            hits = []
            for q in range(nq):
                qa = quads + q * 0x40
                if u64(b, qa + 0x20) not in wanted:
                    continue
                x, y, sx, sy, kx, ky = struct.unpack_from("<6f", b, qa)
                colour = struct.unpack_from("<I", b, qa + 0x18)[0]
                src = u64(b, qa + 0x30)
                rect = struct.unpack_from("<4f", b, src) if 0 < src < len(b) - 0x10 else None
                # Affine read: dest.x = sx*u + ky*v + x, dest.y = kx*u + sy*v + y. Checked on
                # l02_01_In-Game.flo shape 0x0049, whose scale-0 quads with (kx,ky)=(-1,1) land
                # exactly on the ends of its scale-1 horizontal rules -- a 90-degree rotation.
                dest = ""
                if rect:
                    xs = [sx * u + ky * v + x for u in (rect[0], rect[2]) for v in (rect[1], rect[3])]
                    ys = [kx * u + sy * v + y for u in (rect[0], rect[2]) for v in (rect[1], rect[3])]
                    dest = (
                        f" dest=({min(xs):.1f},{min(ys):.1f})-({max(xs):.1f},{max(ys):.1f})"
                        f" {max(xs) - min(xs):.0f}x{max(ys) - min(ys):.0f}"
                    )
                rot = f" rot=({kx:g},{ky:g})" if kx or ky else ""
                line = (
                    f"    q{q} [{tex.get(u64(b, qa + 0x20), ('?',))[0]}] "
                    f"off=({x:g},{y:g}) scale=({sx:g},{sy:g}){rot} colour={colour:08x} "
                    f"rect={tuple(round(v, 2) for v in rect) if rect else None}{dest}"
                )
                if args.raw:
                    words = struct.unpack_from("<16I", b, qa)
                    line += "\n      raw " + " ".join(f"{w:08x}" for w in words)
                hits.append(line)
            if hits:
                print(f"{path.name} shape {key:#06x} quads={nq} ({len(hits)} on {args.texture})")
                print("\n".join(hits))
                for u in users.get(key, []):
                    print(f"      used by {u}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
