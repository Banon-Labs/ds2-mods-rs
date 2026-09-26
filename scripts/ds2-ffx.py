#!/usr/bin/env python3
"""Read DARK SOULS II `.ffxbnd(.dcx)` effect bundles: list members, pull one `.ffx` out.

The loose bundles live under `Game/sfx/`. `sfx9999.ffxbnd.dcx` is a DS2 `DCX`/`DFLT` around a
`BND4` whose entries are 36 bytes (format byte `0x74`, UTF-16 names):

    u8 flags; u8[3] 0; i32 -1; u64 size; u64 size_again; u32 data_offset; i32 id; u32 name_offset

`sfxcommon.ffxbnd` is a bare `BND4` with format byte `0x70` and single-byte names, so its entries
drop `size_again` and are 28 bytes. The entry layout is therefore built from the header's format
byte (SoulsFormats `Binder.Format`: ids, names, long offsets, compression), and the entry size the
header declares must equal the size those flags imply. The stored byte is bit-reversed unless its
low bit is set and its high bit clear, or the header's bit-big-endian byte is set.

Measured on this install 2026-09-26: `sfx9999.ffxbnd.dcx` 1778 members, entry 36;
`sfxcommon.ffxbnd` one member, entry 28, a zero-length `dummy.dmy` and no `.ffx` at all.

    python3 scripts/ds2-ffx.py list  [--bundle PATH] [--id 833 --id 834]
    python3 scripts/ds2-ffx.py dump  --id 833 [--bundle PATH] [--out FILE]
    python3 scripts/ds2-ffx.py --selftest
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

# SoulsFormats `Binder.Format` bits, after `bnd4_format` has undone the stored bit order.
FMT_BIG_ENDIAN = 0x01
FMT_IDS = 0x02
FMT_NAMES1 = 0x04
FMT_NAMES2 = 0x08
FMT_LONG_OFFSETS = 0x10
FMT_COMPRESSION = 0x20


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


def bytez(data: bytes, at: int) -> str:
    end = data.index(b"\0", at)
    return data[at:end].decode("shift_jis")


def bnd4_format(raw: int, bit_big_endian: bool) -> int:
    """SoulsFormats `Binder.ReadFormat`: the stored byte is bit-reversed unless told otherwise."""
    if bit_big_endian or (raw & 0x01 and not raw & 0x80):
        return raw
    return int(f"{raw:08b}"[::-1], 2)


def bnd4_entry_fields(fmt: int) -> list[tuple[str, str]]:
    """`[(field, struct code)]` of one file entry, in order, as the format flags select them."""
    fields = [("flags", "B"), ("pad", "3s"), ("sentinel", "i"), ("size", "Q")]
    if fmt & FMT_COMPRESSION:
        fields.append(("size2", "Q"))
    fields.append(("offset", "Q" if fmt & FMT_LONG_OFFSETS else "I"))
    if fmt & FMT_IDS:
        fields.append(("id", "i"))
    if fmt & (FMT_NAMES1 | FMT_NAMES2):
        fields.append(("name_at", "I"))
    if fmt == FMT_NAMES1:
        # SoulsFormats reads a second id and an asserted zero for this exact format only.
        fields += [("id", "i"), ("zero", "i")]
    return fields


def members(data: bytes) -> list[tuple[int, str, bytes]]:
    """`[(id, name, bytes)]` for every entry."""
    if data[:4] != b"BND4":
        raise Fail(f"expected BND4, got {data[:4]!r}")
    if data[0x08]:
        raise Fail("big-endian BND4, this parses little-endian only")
    count = struct.unpack_from("<I", data, 0x0C)[0]
    header = struct.unpack_from("<Q", data, 0x10)[0]
    entry = struct.unpack_from("<Q", data, 0x20)[0]
    unicode = data[0x30] != 0
    fmt = bnd4_format(data[0x31], data[0x09] != 0)
    if fmt & FMT_BIG_ENDIAN:
        raise Fail(f"format {fmt:#04x} marks big-endian entries, this parses little-endian only")
    fields = bnd4_entry_fields(fmt)
    layout = "<" + "".join(code for _, code in fields)
    if header != BND4_HEADER_SIZE:
        raise Fail(f"BND4 header {header:#x}, this parses {BND4_HEADER_SIZE:#x}")
    if entry != struct.calcsize(layout):
        raise Fail(f"BND4 entry size {entry}, format {fmt:#04x} implies {struct.calcsize(layout)}")
    out = []
    for index in range(count):
        values = dict(zip([name for name, _ in fields], struct.unpack_from(layout, data, header + index * entry)))
        if values["sentinel"] != -1 or values["pad"] != b"\0\0\0" or values.get("zero", 0) != 0:
            raise Fail(f"entry {index}: sentinel {values['sentinel']} pad {values['pad'].hex()}")
        size, offset = values["size"], values["offset"]
        if offset + size > len(data):
            raise Fail(f"entry {index} points outside the file")
        blob = data[offset:offset + size]
        size2 = values.get("size2", size)
        if size2 != size:
            # Stored as a bare zlib stream (`78 9c`), `size2` being the inflated length.
            blob = zlib.decompress(blob)
            if len(blob) != size2:
                raise Fail(f"entry {index}: inflated to {len(blob)}, header says {size2}")
        name = ""
        if "name_at" in values:
            name = (utf16z if unicode else bytez)(data, values["name_at"])
        out.append((values.get("id", index), name, blob))
    return out


def bnd4_fixture(raw_format: int, unicode: bool, files: list[tuple[int, str, bytes]]) -> bytes:
    """A little-endian BND4 in the given stored format byte, for the selftest."""
    fmt = bnd4_format(raw_format, False)
    fields = bnd4_entry_fields(fmt)
    layout = "<" + "".join(code for _, code in fields)
    entry = struct.calcsize(layout)
    names_at = BND4_HEADER_SIZE + entry * len(files)
    names = b""
    name_offsets = []
    for _, name, _ in files:
        name_offsets.append(names_at + len(names))
        names += name.encode("utf-16le") + b"\0\0" if unicode else name.encode("shift_jis") + b"\0"
    data_at = names_at + len(names)
    entries = b""
    payload = b""
    for (ident, _, blob), name_at in zip(files, name_offsets):
        values = {"flags": 0x40, "pad": b"\0\0\0", "sentinel": -1, "size": len(blob), "size2": len(blob),
                  "offset": data_at + len(payload), "id": ident, "name_at": name_at, "zero": 0}
        entries += struct.pack(layout, *(values[name] for name, _ in fields))
        payload += blob
    head = bytearray(BND4_HEADER_SIZE)
    head[0:4] = b"BND4"
    head[0x0A] = 1
    struct.pack_into("<IQ8sQQ", head, 0x0C, len(files), BND4_HEADER_SIZE, b"00000000", entry, data_at)
    head[0x30] = int(unicode)
    head[0x31] = raw_format
    return bytes(head) + entries + names + payload


def selftest() -> int:
    # Stored `0x74` is `0x2e` (ids, both names, compression): the sfx9999 layout.
    assert bnd4_format(0x74, False) == FMT_IDS | FMT_NAMES1 | FMT_NAMES2 | FMT_COMPRESSION
    # Stored `0x70` is `0x0e`: no compression flag, so no `size2`, as in sfxcommon.ffxbnd.
    assert bnd4_format(0x70, False) == FMT_IDS | FMT_NAMES1 | FMT_NAMES2
    # Low bit set and high bit clear means already in order.
    assert bnd4_format(0x2F, False) == 0x2F
    assert bnd4_format(0x74, True) == 0x74
    for raw, unicode, size in ((0x74, True, 36), (0x70, False, 28)):
        files = [(833, "f0000833.ffx", b"DLsE-a"), (7, "dummy.dmy", b"")]
        data = bnd4_fixture(raw, unicode, files)
        assert struct.unpack_from("<Q", data, 0x20)[0] == size, (raw, size)
        assert members(data) == files, (raw, members(data))
        # A header whose declared entry size disagrees with its flags is refused, not parsed.
        bad = bytearray(data)
        struct.pack_into("<Q", bad, 0x20, 64 - size)
        try:
            members(bytes(bad))
        except Fail:
            pass
        else:
            raise AssertionError(f"format {raw:#x}: mismatched entry size parsed")
    print("selftest ok")
    return 0


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


def drop_child(out: bytearray, tree: "Tree", where: str) -> None:
    """Zero the effect id of a `Param type 37` (child effect) so its parent spawns nothing there.

    Shipped files already use id 0 for an unused slot (effect 833's action 14 has seven of its
    eight slots at 0), so this is a value the engine is known to accept, not a new shape.
    """
    at = int(where, 0)
    found = tree.header(at, len(out))
    if found is None or found[0] != "FXSerializableParam":
        raise Fail(f"{where}: not a Param object")
    body = at + 10
    kind = struct.unpack_from("<i", out, body)[0]
    if kind != 37:
        raise Fail(f"{where}: Param type {kind}, not 37 (child effect)")
    struct.pack_into("<i", out, body + 4, 0)


def patch(blob: bytes, sets: list[str], new_id: int | None, drops: list[str] | None = None) -> bytes:
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
    for where in drops or []:
        drop_child(out, tree, where)
    if new_id is not None:
        struct.pack_into("<i", out, tree.root + 10 + 4, new_id)
    Tree(bytes(out)).dump()
    return bytes(out)


def load(bundle: Path) -> list[tuple[int, str, bytes]]:
    return members(inflate(bundle.read_bytes()))


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--selftest", action="store_true", help="parse synthetic BND4s in both entry layouts")
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
    p.add_argument("--drop-child", dest="drops", action="append", default=[], metavar="OFFSET",
                   help="offset of a `Param type 37` from `tree`: that child effect is not spawned")
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
        args.out.write_bytes(patch(blob, args.sets, args.new_id, args.drops))
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
