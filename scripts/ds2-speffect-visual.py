#!/usr/bin/env python3
"""List DARK SOULS II SpEffects whose event script only draws something.

    python3 scripts/ds2-regulation.py extract --out <dir>
    python3 scripts/ds2-speffect-visual.py <dir>
    python3 scripts/ds2-speffect-visual.py <dir> --allow weapon-sfx --allow icon
    python3 scripts/ds2-speffect-visual.py <dir> --id 110000010

DS2 has no SpEffectParam. A SpEffect id is an event in one of the eleven `SpEffect*.emevd`
members of the regulation (see docs/DS2-SPEFFECT.md), and everything the effect does is an
instruction in that event. So "purely visual" is a property of the instruction list: an event
qualifies when every instruction in it is one of the visual or bookkeeping instructions below and
at least one of them draws something.

Instruction names come from DarkScript3's `ds2scholar-common.emedf.json`
(github.com/AinTunez/DarkScript3, `DarkScript3/Resources`). They are community names, not From's,
and several instructions there are still "Unknown Command". This script copies only the handful
of names and argument layouts it needs; the rest are classed as gameplay or unknown and so
disqualify an event. Arguments are decoded with the emedf type codes (0 u8, 1 u16, 2 u32, 3 s8,
4 s16, 5 s32, 6 f32), packed with natural alignment and the block padded to 4 bytes; the decode
asserts that the packed length equals the instruction's stored arg length, so a wrong layout
fails instead of printing shifted numbers.
"""

from __future__ import annotations

import argparse
import importlib.util
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("ds2_emevd", HERE / "ds2-emevd.py")
emevd = importlib.util.module_from_spec(_spec)
sys.modules["ds2_emevd"] = emevd
_spec.loader.exec_module(emevd)

ITEMS_TSV = HERE.parent / "crates/ds2-build-import-core/data/items.tsv"

# (bank, index) -> (class, name, [(arg name, emedf type code)])
# class: "draw" puts something on screen on the character, "meta" is SpEffect bookkeeping,
# the optional classes are only accepted when --allow names them.
KNOWN = {
    (100040, 1): ("draw", "Spawn SFX", [("sfx", 5), ("u1", 0), ("u2", 0), ("u3", 0), ("u4", 0), ("u5", 6)]),
    (100040, 8): ("draw", "Apply ChrFullBodySFX", [("fullbody_sfx_param", 5), ("u", 0)]),
    (100700, 4): ("meta", "Set SpEffect Length", [("seconds", 6)]),
    (100130, 2): ("icon", "Display Icon", [("u1", 5), ("u2", 5), ("u3", 6)]),
    (100130, 4): (
        "weapon-sfx",
        "Apply Weapon SFX",
        [("u1", 5), ("u2", 6), ("u3", 5), ("u4", 6), ("u5", 5), ("u6", 5)],
    ),
    (100130, 5): ("weapon-sfx", "Draw Disable Full Body SFX", [("u", 5)]),
}

TYPE = {0: ("B", 1), 1: ("H", 2), 2: ("I", 4), 3: ("b", 1), 4: ("h", 2), 5: ("i", 4), 6: ("f", 4)}

SPEFFECT_FILES = [
    "SpEffectAbnormalState",
    "SpEffectWeapon",
    "SpEffectArmor",
    "SpEffectSpell",
    "SpEffectRing",
    "SpEffectPassiveItem",
    "SpEffectActiveItem",
    "SpEffectEnemy",
    "SpEffectWideUse",
    "SpEffectCondition",
    "SpEffectMustSub",
]


def decode(ins, layout) -> dict:
    off = 0
    out = {}
    for name, code in layout:
        fmt, size = TYPE[code]
        off = (off + size - 1) // size * size
        (out[name],) = struct.unpack_from("<" + fmt, ins.args, off)
        off += size
    off = (off + 3) // 4 * 4
    if off != len(ins.args):
        raise SystemExit(
            f"{ins.bank}[{ins.index}]: layout packs to {off} bytes, instruction has {len(ins.args)}"
        )
    return out


def load_names() -> dict[int, str]:
    names = {}
    if not ITEMS_TSV.exists():
        return names
    for line in ITEMS_TSV.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) >= 2 and parts[0].isdigit():
            names[int(parts[0])] = parts[1]
    return names


def guess_name(eid: int, names: dict[int, str]) -> str:
    # Inferred, second-hand: effects share the id of the thing that grants them, with a small
    # suffix for variants. Try the exact id, then the id with its last two and four digits zeroed.
    for cand in (eid, eid // 100 * 100, eid // 10000 * 10000):
        if cand in names:
            return names[cand] + ("" if cand == eid else f" (item {cand})")
    return ""


def fmt_args(d: dict) -> str:
    return " ".join(f"{k}={v:g}" if isinstance(v, float) else f"{k}={v}" for k, v in d.items())


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dir", help="directory holding the extracted SpEffect*.emevd members")
    ap.add_argument(
        "--allow",
        action="append",
        default=[],
        choices=["icon", "weapon-sfx"],
        help="also accept these instruction classes (not accepted by default)",
    )
    ap.add_argument("--id", dest="ids", type=int, action="append", help="print these events whatever they hold")
    args = ap.parse_args()

    ok_classes = {"draw", "meta", *args.allow}
    names = load_names()
    total = 0
    hits = []
    for stem in SPEFFECT_FILES:
        ev = emevd.Emevd(Path(args.dir) / f"{stem}.emevd")
        bad = ev.check()
        if bad:
            raise SystemExit(f"{ev.path}: {bad[0]}")
        for e in ev.events:
            total += 1
            lines = []
            classes = set()
            for ins in e.instructions:
                known = KNOWN.get((ins.bank, ins.index))
                if known is None:
                    classes.add("gameplay-or-unknown")
                    lines.append(f"{ins.bank}[{ins.index}] {ins.render_args()}")
                    continue
                cls, name, layout = known
                classes.add(cls)
                lines.append(f"{ins.bank}[{ins.index}] {name}: {fmt_args(decode(ins, layout))}")
            if args.ids:
                if e.id in args.ids:
                    hits.append((stem, e, lines, classes))
                continue
            if classes <= ok_classes and "draw" in classes:
                hits.append((stem, e, lines, classes))

    for stem, e, lines, classes in hits:
        name = guess_name(e.id, names)
        print(
            f"{e.id}  {stem}  rest={e.rest} params={e.param_count}"
            + (f"  [{name}]" if name else "")
        )
        for line in lines:
            print("    " + line)
    if not args.ids:
        print(f"# {len(hits)} of {total} SpEffect events qualify (classes accepted: {sorted(ok_classes)})")


if __name__ == "__main__":
    main()
