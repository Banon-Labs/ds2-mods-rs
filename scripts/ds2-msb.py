#!/usr/bin/env python3
"""List a DARK SOULS II map file's (MSB2) parts, regions or events, with positions.

    python3 scripts/ds2-ebl.py extract /map/m20_11_00_00/m20_11_00_00.msb --out /tmp/msb
    python3 scripts/ds2-msb.py /tmp/msb/m20_11_00_00.msb parts
    python3 scripts/ds2-msb.py /tmp/msb/m20_11_00_00.msb POINT_PARAM_ST

The layout is SoulsFormats' `Formats/MSB/MSB2` (JKAnderson/SoulsFormats): a chain of params,
each `i32 version, i32 count, i64 name offset, i64 offsets[count]`, the last offset being the next
param. A part is `i64 name offset, u8 type, u8 0, i16 id, i16 model index, i16 0, f32 position[3],
f32 rotation[3], f32 scale[3]`, names relative to the entry. Only the fields printed are read.
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

PART_TYPES = {0: "MapPiece", 1: "Object", 3: "Collision", 4: "Navmesh", 5: "ConnectCollision"}


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    data = Path(sys.argv[1]).read_bytes()

    def utf16(offset: int) -> str:
        end = offset
        while data[end : end + 2] != b"\0\0":
            end += 2
        return data[offset:end].decode("utf-16le")

    params: dict[str, tuple[int, ...]] = {}
    pos = 0x10
    while pos:
        _version, count, name_offset = struct.unpack_from("<iiq", data, pos)
        offsets = struct.unpack_from(f"<{count}q", data, pos + 16)
        params[utf16(name_offset)] = offsets[:-1]
        pos = offsets[-1]
    for name, entries in params.items():
        print(f"# {name} {len(entries)}", file=sys.stderr)

    def name_of(entry: int) -> str:
        return utf16(entry + struct.unpack_from("<q", data, entry)[0])

    models = [name_of(o) for o in params["MODEL_PARAM_ST"]]
    mode = sys.argv[2]
    if mode == "parts":
        for o in params["PARTS_PARAM_ST"]:
            kind = PART_TYPES.get(data[o + 8], str(data[o + 8]))
            model_index = struct.unpack_from("<h", data, o + 12)[0]
            model = models[model_index] if 0 <= model_index < len(models) else model_index
            p = struct.unpack_from("<3f", data, o + 16)
            r = struct.unpack_from("<3f", data, o + 28)
            # An Object's type data begins with its MapObjectInstanceParam row id.
            extra = ""
            if data[o + 8] == 1:
                type_data = o + struct.unpack_from("<q", data, o + 0x70)[0]
                extra = f"\tinstance={struct.unpack_from('<i', data, type_data)[0]}"
            print(
                f"{kind}\t{name_of(o)}\t{model}\t"
                f"pos={p[0]:.2f},{p[1]:.2f},{p[2]:.2f}\trot={r[0]:.1f},{r[1]:.1f},{r[2]:.1f}{extra}"
            )
    else:
        for o in params[mode]:
            print(f"{name_of(o)}\t{data[o + 8 : o + 0x40].hex()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
