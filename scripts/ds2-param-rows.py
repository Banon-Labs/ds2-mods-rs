#!/usr/bin/env python3
"""Print decoded rows of any regulation param, by field name.

    python3 scripts/ds2-param-rows.py EnemyParam --id 744000
    python3 scripts/ds2-param-rows.py EnemyDamageParam --range 74400000:74409999
    python3 scripts/ds2-param-rows.py PlayerDamageParam --where damage01!=0 --fields damage01,damageType01
    python3 scripts/ds2-param-rows.py EnemyParam --json > enemy.json

The member is decoded with the Smithbox DS2S paramdef named by the param's own header type name
(`CHR_PARAM` for EnemyParam), through scripts/ds2-attacks-extract.py's `layout`, and refused when the
def size is not the measured stride. Read-only: prints to stdout, one `id field=value ...` line per
row, or one JSON object `{id: {field: value}}` with --json.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def extractor():
    spec = importlib.util.spec_from_file_location("ds2attacks", HERE / "ds2-attacks-extract.py")
    ex = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ex)
    return ex


def decode(ex, reg, members: dict, member: str, defs: Path) -> dict[int, dict]:
    p = reg.Param(member, members[f"{member}.param"])
    size, fields = ex.layout(defs, p.type_name)
    if p.stride != size:
        raise SystemExit(f"{member}: stride {p.stride} != paramdef {p.type_name} size {size}")
    rows = {}
    for k, rid in enumerate(p.ids):
        b, r = p.row(k), {}
        for nm, ty, off, bit_pos, bits in fields:
            v = struct.unpack_from("<" + ex.FORMATS[ty], b, off)[0]
            if bits:
                v = (v >> bit_pos) & ((1 << bits) - 1)
            if isinstance(v, float):
                v = round(v, 4)
            r[nm] = v
        rows[rid] = r
    return rows


def matches(row: dict, cond: str) -> bool:
    m = re.fullmatch(r"(\w+(?:\[\d+\])?)(==|!=|>=|<=|>|<)(-?[\d.]+)", cond)
    if not m:
        raise SystemExit(f"--where {cond!r}: expected FIELD(==|!=|>=|<=|>|<)NUMBER")
    f, op, v = m.group(1), m.group(2), float(m.group(3))
    x = row[f]
    return {"==": x == v, "!=": x != v, ">=": x >= v, "<=": x <= v, ">": x > v, "<": x < v}[op]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    ap.add_argument("param", help="regulation member without .param, e.g. EnemyDamageParam")
    ap.add_argument("--id", type=int, action="append", default=[], help="row id (repeatable)")
    ap.add_argument("--range", help="LO:HI inclusive row id range")
    ap.add_argument("--where", action="append", default=[], help="FIELD<op>NUMBER (repeatable, all must hold)")
    ap.add_argument("--fields", help="comma-separated fields to print (default all)")
    ap.add_argument("--nonzero", action="store_true", help="print only fields that are not 0")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--offsets", action="store_true", help="print each field's byte offset and type, no rows")
    ap.add_argument("--defs", type=Path, default=None, help="Smithbox DS2S paramdef directory")
    a = ap.parse_args()

    ex = extractor()
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    members = reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
    if a.offsets:
        p = reg.Param(a.param, members[f"{a.param}.param"])
        size, fields = ex.layout(a.defs or ex.DEFAULT_DEFS, p.type_name)
        print(f"# {a.param} {p.type_name} size {size:#x} stride {p.stride:#x}")
        for nm, ty, off, bit_pos, bits in fields:
            print(f"{off:#06x}{'' if bits is None else f'.{bit_pos}:{bits}'} {ty} {nm}")
        return 0
    rows = decode(ex, reg, members, a.param, a.defs or ex.DEFAULT_DEFS)
    lo, hi = (int(x) for x in a.range.split(":")) if a.range else (None, None)
    keep = a.fields.split(",") if a.fields else None
    out = {}
    for rid, r in rows.items():
        if a.id and rid not in a.id:
            continue
        if lo is not None and not lo <= rid <= hi:
            continue
        if not all(matches(r, c) for c in a.where):
            continue
        r = {k: r[k] for k in keep} if keep else r
        if a.nonzero:
            r = {k: v for k, v in r.items() if v != 0}
        out[rid] = r
    if a.json:
        json.dump({str(k): v for k, v in out.items()}, sys.stdout)
        print()
    else:
        for rid, r in out.items():
            print(rid, " ".join(f"{k}={v}" for k, v in r.items()))
        print(f"# {len(out)} of {len(rows)} rows", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
