#!/usr/bin/env python3
"""Read DARK SOULS II `.ffxbnd(.dcx)` effect bundles: list members, pull one `.ffx` out.

The loose bundles live under `Game/sfx/`. `sfx9999.ffxbnd.dcx` is a DS2 `DCX`/`DFLT` around a
`BND4` whose entries are 36 bytes (format byte `0x74`, UTF-16 names):

    u32 flags; i32 -1; u64 size; u64 size_again; u32 data_offset; i32 id; u32 name_offset

Measured on this install 2026-09-25: 1778 members, header 0x40, entry 36. The layout is asserted
per entry rather than trusted, same as `scripts/ds2-regulation.py`.

    python3 scripts/ds2-ffx.py list  [--bundle PATH] [--id 833 --id 834]
    python3 scripts/ds2-ffx.py dump  --id 833 [--bundle PATH] [--out FILE]
"""

from __future__ import annotations

import argparse
import struct
import sys
import zlib
from pathlib import Path

GAME_SFX = Path.home() / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game/sfx"
DEFAULT_BUNDLE = GAME_SFX / "sfx9999.ffxbnd.dcx"

DCX_PAYLOAD_OFFSET = 0x4C
BND4_HEADER_SIZE = 0x40
BND4_ENTRY_SIZE = 36


class Fail(SystemExit):
    """Every consistency failure raises this; nothing here returns a best-effort parse."""


def inflate(data: bytes) -> bytes:
    if data[:4] != b"DCX\x00":
        return data
    uncompressed, compressed = struct.unpack_from(">II", data, 0x1C)
    if compressed != len(data) - DCX_PAYLOAD_OFFSET:
        raise Fail(f"DCX declares {compressed} compressed bytes, {len(data) - DCX_PAYLOAD_OFFSET} follow")
    out = zlib.decompress(data[DCX_PAYLOAD_OFFSET:])
    if len(out) != uncompressed:
        raise Fail(f"DCX declares {uncompressed} inflated bytes, got {len(out)}")
    return out


def utf16z(data: bytes, at: int) -> str:
    end = at
    while data[end:end + 2] != b"\0\0":
        end += 2
    return data[at:end].decode("utf-16le")


def members(data: bytes) -> list[tuple[int, str, bytes]]:
    """`[(id, name, bytes)]` for every entry."""
    if data[:4] != b"BND4":
        raise Fail(f"expected BND4, got {data[:4]!r}")
    count = struct.unpack_from("<I", data, 0x0C)[0]
    header = struct.unpack_from("<Q", data, 0x10)[0]
    entry = struct.unpack_from("<Q", data, 0x20)[0]
    if header != BND4_HEADER_SIZE or entry != BND4_ENTRY_SIZE:
        raise Fail(f"BND4 header/entry {header:#x}/{entry}, this parses {BND4_HEADER_SIZE:#x}/{BND4_ENTRY_SIZE}")
    out = []
    for index in range(count):
        at = header + index * entry
        _flags, sentinel, size, size2, offset, ident, name_at = struct.unpack_from("<IiQQIiI", data, at)
        if sentinel != -1:
            raise Fail(f"entry {index}: sentinel {sentinel}")
        if offset + size > len(data):
            raise Fail(f"entry {index} points outside the file")
        blob = data[offset:offset + size]
        if size2 != size:
            # Stored as a bare zlib stream (`78 9c`), `size2` being the inflated length.
            blob = zlib.decompress(blob)
            if len(blob) != size2:
                raise Fail(f"entry {index}: inflated to {len(blob)}, header says {size2}")
        out.append((ident, utf16z(data, name_at), blob))
    return out


class Tree:
    """A `DLsE` walk that needs no per-param schema.

    Every serialized object is `{u16 class_index + 1; i32 version; i32 length_including_header}`
    (SoulsFormats `FFXDLSE.FXSerializable`), so objects nest by their own lengths. Inside a body,
    raw `i32`s and child objects are interleaved; a child is recognised by a class index in range,
    a version 1..5 and a length that fits the parent. Leaves are printed by class: the typed
    primitives decode, everything else is a raw `i32` (shown with its float reading too).
    """

    def __init__(self, blob: bytes) -> None:
        if blob[:4] != b"DLsE" or blob[4:6] != b"\x01\x03":
            raise Fail(f"not a DS2 DLsE ffx: {blob[:8].hex(' ')}")
        self.blob = blob
        at = 4 + 4 + 8 + 1 + 4
        (count,) = struct.unpack_from("<h", blob, at)
        at += 2
        self.classes = []
        for _ in range(count):
            (length,) = struct.unpack_from("<i", blob, at)
            self.classes.append(blob[at + 4:at + 4 + length].decode("ascii"))
            at += 4 + length
        self.root = at
        self.lines: list[str] = []

    def header(self, at: int, end: int) -> tuple[str, int, int] | None:
        if at + 10 > end:
            return None
        cls, version, length = struct.unpack_from("<hii", self.blob, at)
        if not 1 <= cls <= len(self.classes) or not 1 <= version <= 5 or not 10 <= length <= end - at:
            return None
        return self.classes[cls - 1], version, length

    def walk(self, at: int, end: int, depth: int) -> int:
        pad = "  " * depth
        if self._vector(at, end):
            name, body, stop = "DLVector", at, end
        else:
            found = self.header(at, end)
            if found is None:
                raise Fail(f"no object at {at:#x}")
            name, _version, length = found
            body, stop = at + 10, at + length
        short = name.replace("FXSerializable", "")
        if name == "FXSerializableEffect":
            # SoulsFormats FXEffect: i32 0, i32 id, i32 0, i32 0, i32 2, i16 0, then an empty
            # DLVector (its `i16 2; i32 0` is what SoulsFormats asserts as two loose fields).
            ident = struct.unpack_from("<i", self.blob, body + 4)[0]
            self.lines.append(f"{pad}{at:#06x} Effect id {ident}")
            body += 0x14 + 2
            body = self.walk(body, stop, depth + 1)
            while body < stop - 1:
                body = self.walk(body, stop, depth + 1)
            return stop
        if name == "DLVector":
            # DLVector is {u16 class; i32 count; i32 * count}: no version or length field.
            (count,) = struct.unpack_from("<i", self.blob, at + 2)
            values = struct.unpack_from(f"<{count}i", self.blob, at + 6)
            self.lines.append(f"{pad}{at:#06x} DLVector {list(values)}")
            return at + 6 + 4 * count
        if name.startswith("FXSerializablePrimitive<"):
            kind = name[len("FXSerializablePrimitive<"):-1]
            if kind == "dl_int32":
                value = str(struct.unpack_from("<i", self.blob, body)[0])
            elif kind == "FXColorRGBA":
                value = "rgba " + " ".join(f"{v:.3f}" for v in struct.unpack_from("<4f", self.blob, body))
            else:
                value = f"{struct.unpack_from('<f', self.blob, body)[0]:.4f}"
            self.lines.append(f"{pad}{at:#06x} {kind} = {value}")
            return stop
        label = short
        if name == "FXSerializableParam":
            label = f"Param type {struct.unpack_from('<i', self.blob, body)[0]}"
            body += 4
        elif name == "FXSerializableAction":
            label = f"Action id {struct.unpack_from('<i', self.blob, body)[0]}"
            body += 4
        self.lines.append(f"{pad}{at:#06x} {label}")
        raws: list[str] = []
        while body < stop:
            if self.header(body, stop) is not None or self._vector(body, stop):
                if raws:
                    self.lines.append(f"{pad}  raw {' '.join(raws)}")
                    raws = []
                body = self.walk(body, stop, depth + 1)
                continue
            if stop - body >= 4:
                (i,) = struct.unpack_from("<i", self.blob, body)
                (f,) = struct.unpack_from("<f", self.blob, body)
                raws.append(str(i) if -100000 < i < 100000 else f"{f:.4g}f")
                body += 4
            else:
                raws.append(self.blob[body:stop].hex())
                body = stop
        if raws:
            self.lines.append(f"{pad}  raw {' '.join(raws)}")
        return stop

    def _vector(self, at: int, end: int) -> bool:
        if "DLVector" not in self.classes or at + 6 > end:
            return False
        cls, count = struct.unpack_from("<hi", self.blob, at)
        return cls == self.classes.index("DLVector") + 1 and 0 <= count and at + 6 + 4 * count <= end

    def dump(self) -> str:
        self.walk(self.root, len(self.blob), 0)
        return "\n".join(self.lines)


def patch(blob: bytes, sets: list[str], new_id: int | None) -> bytes:
    """Rewrite leaf primitives in place, by the offsets `tree` prints. Same size, so no object
    length anywhere changes and the tree stays valid by construction.

    `OFFSET=VALUE`: OFFSET is a primitive's object offset from `tree`; VALUE is a float, an int,
    or `r,g,b,a` for a colour. The primitive's class decides how it is written, so a float cannot
    land in an int slot by accident.
    """
    tree = Tree(blob)
    out = bytearray(blob)
    for item in sets:
        where, _, value = item.partition("=")
        at = int(where, 0)
        found = tree.header(at, len(blob))
        if found is None or not found[0].startswith("FXSerializablePrimitive<"):
            raise Fail(f"{where}: not a primitive object (use an offset `tree` prints for a leaf)")
        kind = found[0][len("FXSerializablePrimitive<"):-1]
        body = at + 10
        if kind == "dl_int32":
            struct.pack_into("<i", out, body, int(value, 0))
        elif kind == "FXColorRGBA":
            parts = [float(v) for v in value.split(",")]
            if len(parts) != 4:
                raise Fail(f"{where}: a colour takes r,g,b,a")
            struct.pack_into("<4f", out, body, *parts)
        else:
            struct.pack_into("<f", out, body, float(value))
    if new_id is not None:
        struct.pack_into("<i", out, tree.root + 10 + 4, new_id)
    Tree(bytes(out)).dump()
    return bytes(out)


def load(bundle: Path) -> list[tuple[int, str, bytes]]:
    return members(inflate(bundle.read_bytes()))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("list")
    p.add_argument("--bundle", type=Path, default=DEFAULT_BUNDLE)
    p.add_argument("--id", dest="ids", type=int, action="append")
    p = sub.add_parser("dump")
    p.add_argument("--bundle", type=Path, default=DEFAULT_BUNDLE)
    p.add_argument("--id", type=int, required=True)
    p.add_argument("--out", type=Path)
    p = sub.add_parser("tree", help="print the DLsE object tree of one member")
    p.add_argument("--bundle", type=Path, default=DEFAULT_BUNDLE)
    p.add_argument("--id", type=int, required=True)
    p = sub.add_parser("patch", help="rewrite leaf values of one member and write a new .ffx")
    p.add_argument("--bundle", type=Path, default=DEFAULT_BUNDLE)
    p.add_argument("--id", type=int, required=True)
    p.add_argument("--new-id", type=int)
    p.add_argument("--set", dest="sets", action="append", default=[], metavar="OFFSET=VALUE")
    p.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    entries = load(args.bundle)
    if args.cmd == "list":
        for ident, name, blob in entries:
            if args.ids and ident not in args.ids:
                continue
            print(f"{ident:8d}  {len(blob):8d}  {blob[:8].hex(' ')}  {name}")
        return 0
    found = [e for e in entries if e[0] == args.id]
    if not found:
        raise Fail(f"no member with id {args.id}")
    _, name, blob = found[0]
    if args.cmd == "tree":
        print(name)
        print(Tree(blob).dump())
        return 0
    if args.cmd == "patch":
        args.out.write_bytes(patch(blob, args.sets, args.new_id))
        print(f"{name} -> {args.out} ({len(args.sets)} value(s), id {args.new_id or args.id})")
        return 0
    if args.out:
        args.out.write_bytes(blob)
        print(f"{name}: {len(blob)} bytes -> {args.out}")
    else:
        sys.stdout.buffer.write(blob)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
