#!/usr/bin/env python3
"""Count how the rest of the workspace uses `ds2-rva`, grouped by game area.

Input for docs/DS2-BINDINGS-CRATE.md: which hand-maintained constants a typed bindings crate
would retire first. Every `pub const` / `pub static` in crates/ds2-rva/src/lib.rs is classified
by name prefix into an area and by name suffix / type into a kind, then counted as whole-word
references in every other crate's `src/`.

The classification is by naming convention, not by reading each constant's doc comment, so a
constant named against the convention lands in the wrong row. Kinds:

  layout       *_OFFSET, *_OFFSETS, *_STRIDE, *_BYTES, *_SIZE, or typed `usize`
  vtable       name contains VTABLE (and is not a *_SLOT index)
  patch bytes  *PROLOGUE*, *_EXPECTED, *_STUB, *_BACKUP
  address      typed `u32` (an RVA)
  value        everything else (ids, enum values, masks)

Usage:
  python3 scripts/ds2-rva-census.py              # area table + per-crate totals
  python3 scripts/ds2-rva-census.py --area SFX   # also list that area's constants by use
"""

import argparse
import collections
import glob
import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

AREAS = [
    ("GameManagerImp", ("GAME", "SCREEN", "CONTENT")),
    ("Fe frontend", ("FE", "FEX", "FRONTEND", "FLO", "TITLE")),
    ("Player (PlayerParam/PlayerCtrl)", ("PLAYER", "CHARACTER", "CHR", "EQUIP", "ESTUS", "HP")),
    ("Navigation (Nv*)", ("NV", "NAVI")),
    ("SFX", ("SFX", "KATANA", "PRISM", "RESOURCE")),
    ("Save/load", ("SAVE", "SL", "USER")),
    ("Items/params/map", ("ITEM", "PARAM", "MAP")),
    ("Input/camera", ("PAD", "MOUSE", "KEYBOARD", "CAMERA", "SOFTWARE", "WINDOWS", "EX")),
    ("Network", ("NET", "STEAM")),
    ("DL strings", ("WSTRING", "DL")),
]


def area_of(name):
    prefix = name.split("_")[0]
    for area, prefixes in AREAS:
        if prefix in prefixes:
            return area
    return "Other"


def kind_of(name, ty):
    ty = ty.strip()
    if "VTABLE" in name and not name.endswith("_SLOT"):
        return "vtable"
    if name.endswith(("_OFFSET", "_OFFSETS", "_STRIDE", "_BYTES", "_SIZE")) or ty == "usize":
        return "layout"
    if "PROLOGUE" in name or name.endswith(("_EXPECTED", "_STUB", "_BACKUP")):
        return "patch bytes"
    if ty == "u32":
        return "address"
    return "value"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--area", help="also list this area's constants, most-used first")
    args = ap.parse_args()

    with open(os.path.join(ROOT, "crates/ds2-rva/src/lib.rs")) as f:
        src = f.read()
    consts = re.findall(r"^\s*pub (?:const|static) ([A-Z0-9_]+)\s*:\s*([^=]+)=", src, re.M)

    texts = {}
    for path in glob.glob(os.path.join(ROOT, "crates/*/src/**/*.rs"), recursive=True):
        if "/crates/ds2-rva/" in path:
            continue
        with open(path) as f:
            texts[path] = f.read()

    def crate_of(path):
        return path.split("/crates/")[1].split("/")[0]

    rows = []
    per_crate = collections.Counter()
    for name, ty in consts:
        pat = re.compile(r"\b" + name + r"\b")
        hits, crates = 0, set()
        for path, text in texts.items():
            n = len(pat.findall(text))
            if n:
                hits += n
                crates.add(crate_of(path))
                per_crate[crate_of(path)] += n
        rows.append((name, area_of(name), kind_of(name, ty), hits, crates))

    agg = collections.defaultdict(collections.Counter)
    users = collections.defaultdict(set)
    for name, area, kind, hits, crates in rows:
        a = agg[area]
        a["consts"] += 1
        a[kind] += 1
        a["refs"] += hits
        if kind in ("layout", "vtable"):
            a["layout_refs"] += hits
        a["unused"] += hits == 0
        users[area] |= crates

    print("area | consts | layout | vtable | address | value | patch | refs | layout+vtable refs | unused | consumer crates")
    for area, a in sorted(agg.items(), key=lambda kv: -kv[1]["refs"]):
        names = " ".join(sorted(c.removeprefix("ds2-") for c in users[area]))
        print(
            f"{area} | {a['consts']} | {a['layout']} | {a['vtable']} | {a['address']} | "
            f"{a['value']} | {a['patch bytes']} | {a['refs']} | {a['layout_refs']} | "
            f"{a['unused']} | {len(users[area])}: {names}"
        )
    totals = collections.Counter()
    for _, _, kind, hits, _ in rows:
        totals[kind] += 1
        totals["refs"] += hits
    print(f"total constants {len(rows)}: {dict(totals)}")
    print("references per consuming crate:")
    for crate, n in per_crate.most_common():
        print(f"  {n:5d}  {crate}")

    if args.area:
        print(f"\n{args.area}, most-used first:")
        for name, area, kind, hits, crates in sorted(rows, key=lambda r: -r[3]):
            if area == args.area:
                print(f"  {hits:4d}  {kind:11s}  {name}  {' '.join(sorted(crates))}")


if __name__ == "__main__":
    main()
