#!/usr/bin/env python3
"""Pull a frontend texture out of DARK SOULS II's archive, by the name a `.flo` calls it.

    scripts/ds2-tpf.py find In-game_01.tpf
    scripts/ds2-tpf.py extract In-game_01.tpf --out /tmp/fe

# Why finding one is not a path lookup

`GameDataEbl.bhd` keys entries by a hash of the archive path and stores no names, so a file can
only be opened if its path is already known. The frontend textures have no known path: every
spelling the executable suggests -- `menu:/tex/09/`, `menu:/tex/Icon/`, `menu:/tex/language/`,
`/menu/tex/<name>.tpf` -- misses. `scripts/ds2-ebl.py` can still open an entry by hash, so this
walks EVERY entry, unwraps the `BND4`s (directly and through `DCX`), and matches on the member
name inside. That is slow -- it reads the 13 GB archive's directory the hard way -- and it is the
only thing that works, so the result is cached under the scratchpad.

A `.flo` names its textures in a table the document header points at (`+0x30`, entries of `0x18`
bytes: name offset, id, width, height). `scripts/ds2-flo.py textures` prints it. Those names are
the member names here: `In-game_01` in the table is `In-game_01.tpf` in the archive.

# The TPF container

Read off `BONFIRE_LIT.tpf`, whose single entry made the stride unambiguous, and checked against
every multi-texture container this finds:

    +0x00  'TPF\\0'
    +0x04  u32  total size of the texture data
    +0x08  u32  texture count
    +0x0c  u8   version, u8 flag, u8 encoding, u8 pad
    +0x10  entries, STRIDE 0x14:
           +0x00 u32 offset   +0x04 u32 size   +0x08 u8 format  +0x09 u8 type
           +0x0a u8 mipmaps   +0x0b u8 flags   +0x0c u32 name offset   +0x10 u32 unknown
    then   NUL-terminated names, then the DDS payloads.

Each payload is a plain `DDS ` file and is written out as one.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import struct
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
#: Where the member->entry index is kept, because building it reads the whole archive.
CACHE = Path("/tmp/claude-1000/ds2-tpf-index.json")

TPF_MAGIC = b"TPF\x00"
TPF_COUNT_FIELD = 0x08
TPF_ENTRY_TABLE = 0x10
TPF_ENTRY_STRIDE = 0x14


def load_ebl():
    """`scripts/ds2-ebl.py` as a module -- its name is not an identifier, so import it by path."""
    spec = importlib.util.spec_from_file_location("ds2_ebl", REPO_ROOT / "scripts" / "ds2-ebl.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def bnd4_members(ebl, data: bytes) -> list[tuple[str, bytes]]:
    """The members of a `BND4`, or nothing if this one is not readable.

    `ds2-ebl.py` raises `SystemExit` on a compressed member rather than returning a partial list.
    A sweep over the whole archive meets those, and one of them is not a reason to stop.
    """
    try:
        return ebl.bnd4_files(data)
    except BaseException:
        return []


def build_index(ebl) -> dict[str, str]:
    """`member name -> archive entry hash`, for every `.tpf` inside a `BND4` in the archive."""
    bhd, bdt, pem = ebl.archive_paths(ebl.GAME_DIR, ebl.DEFAULT_ARCHIVE)
    header = ebl.Bhd5(ebl.decrypt_bhd(bhd, pem))
    index: dict[str, str] = {}
    with open(bdt, "rb") as archive:
        for path_hash, (size, offset, aes_key, _bucket) in header.entries.items():
            if aes_key:
                continue
            archive.seek(offset)
            magic = archive.read(4)
            if magic not in (b"BND4", b"DCX\x00"):
                continue
            archive.seek(offset)
            blob = archive.read(size)
            if magic == b"DCX\x00":
                try:
                    blob = ebl.dcx_decompress(blob)
                except BaseException:
                    continue
            if blob[:4] != b"BND4":
                continue
            for name, _payload in bnd4_members(ebl, blob):
                if name.lower().endswith(".tpf"):
                    index.setdefault(name, f"{path_hash:#010x}")
    return index


def index_for(ebl, refresh: bool = False) -> dict[str, str]:
    if not refresh and CACHE.exists():
        return json.loads(CACHE.read_text())
    index = build_index(ebl)
    CACHE.parent.mkdir(parents=True, exist_ok=True)
    CACHE.write_text(json.dumps(index, indent=0, sort_keys=True))
    return index


def member(ebl, path_hash: int, wanted: str) -> bytes | None:
    """One `BND4` member's bytes, out of the archive entry that holds it."""
    bhd, bdt, pem = ebl.archive_paths(ebl.GAME_DIR, ebl.DEFAULT_ARCHIVE)
    header = ebl.Bhd5(ebl.decrypt_bhd(bhd, pem))
    size, offset, _aes, _bucket = header.entries[path_hash]
    with open(bdt, "rb") as archive:
        archive.seek(offset)
        blob = archive.read(size)
    if blob[:4] == b"DCX\x00":
        blob = ebl.dcx_decompress(blob)
    for name, payload in bnd4_members(ebl, blob):
        if name.lower() == wanted.lower():
            return payload
    return None


def textures(tpf: bytes) -> list[dict[str, object]]:
    """Every texture in a TPF: its name, its format byte, and its `DDS ` payload."""
    if tpf[:4] != TPF_MAGIC:
        raise SystemExit(f"not a TPF (magic {tpf[:4]!r})")
    count = struct.unpack_from("<I", tpf, TPF_COUNT_FIELD)[0]
    out = []
    for i in range(count):
        at = TPF_ENTRY_TABLE + i * TPF_ENTRY_STRIDE
        offset, size, fmt, kind, mips, flags, name_at, _unk = struct.unpack_from("<IIBBBBII", tpf, at)
        name = tpf[name_at : tpf.index(b"\0", name_at)].decode("ascii", "replace")
        payload = tpf[offset : offset + size]
        width = height = 0
        four_cc = b""
        if payload[:4] == b"DDS ":
            height, width = struct.unpack_from("<2I", payload, 0x0C)
            four_cc = payload[0x54:0x58]
        out.append(
            {
                "name": name,
                "format": fmt,
                "kind": kind,
                "mipmaps": mips,
                "flags": flags,
                "width": width,
                "height": height,
                "four_cc": four_cc.decode("ascii", "replace"),
                "payload": payload,
            }
        )
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("action", choices=("find", "extract", "index"))
    parser.add_argument("name", nargs="?", help="member name, e.g. In-game_01.tpf")
    parser.add_argument("--out", type=Path, help="directory to write into, for `extract`")
    parser.add_argument("--refresh", action="store_true", help="rebuild the index instead of using the cache")
    args = parser.parse_args()

    ebl = load_ebl()
    index = index_for(ebl, refresh=args.refresh)

    if args.action == "index":
        print(f"{len(index)} .tpf members indexed -> {CACHE}")
        return 0

    if not args.name:
        parser.error(f"{args.action} needs a member name")
    hits = {k: v for k, v in index.items() if args.name.lower() in k.lower()}
    if not hits:
        print(f"no .tpf member matching {args.name!r} in {len(index)} indexed")
        return 1

    if args.action == "find":
        for name, where in sorted(hits.items()):
            print(f"{name}  entry={where}")
        return 0

    if not args.out:
        parser.error("extract needs --out")
    args.out.mkdir(parents=True, exist_ok=True)
    for name, where in sorted(hits.items()):
        blob = member(ebl, int(where, 16), name)
        if blob is None:
            print(f"{name}: gone from entry {where} -- the index is stale, rerun with --refresh")
            continue
        (args.out / name).write_bytes(blob)
        for texture in textures(blob):
            dds = args.out / f"{texture['name']}.dds"
            dds.write_bytes(texture["payload"])
            print(
                f"{name} -> {dds}  {texture['width']}x{texture['height']} "
                f"{texture['four_cc']} format={texture['format']} mips={texture['mipmaps']}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
