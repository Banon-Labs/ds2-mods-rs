#!/usr/bin/env python3
"""Every weapon's and armor piece's maximum reinforcement level, read out of the regulation.

    python3 scripts/ds2-reinforce-max.py                      # TSV on stdout
    python3 scripts/ds2-reinforce-max.py --out max.tsv        # TSV to a file, tallies on stderr
    python3 scripts/ds2-reinforce-max.py --survey             # the evidence for the field choice
    python3 scripts/ds2-reinforce-max.py --selftest

Output: `item_id <TAB> max_reinforce <TAB> kind` (kind is `weapon` or `armor`) for every id in
`crates/ds2-build-import-core/data/items.tsv` that the regulation resolves to a weapon or an armor
piece. The item id is the one the game's item spawn takes (the ItemParam row id).

Nothing here comes from a planner site or an item name. Names are read from items.tsv only to
label rows in the report. The params are decrypted and parsed by `scripts/ds2-regulation.py`,
which checks their structure on every load.

Provenance of every offset
--------------------------

REGULATION means the value is a field in the shipped `enc_regulation.bnd.dcx`. INFERRED means the
field's meaning was not read from a paramdef, because there is no DS2 paramdef in this repo. It was
worked out from the evidence given here and has not been checked against the executable.

ItemParam (stride 84), keyed by item id
  +0x14 i32  WeaponParam row id, -1 if the item is not a weapon.  REGULATION, meaning INFERRED:
             it equals the item id on weapon rows (Falchion 1600000 -> 1600000) and is -1 on
             armor rows. `docs/DS2-BUILD-EMBEDDINGS.md` "Infusability" (the i32[5] link) already
             relies on it.
  +0x18 i32  ArmorParam row id, -1 if the item is not armor.  REGULATION, meaning INFERRED:
             it is item id - 10000000 on armor rows (Wanderer Manchettes 21030102 -> 11030102)
             and -1 on weapon rows.

WeaponParam (stride 188)
  +0x08 i32  WeaponReinforceParam row id (`weaponReinforceId`).  REGULATION, meaning INFERRED
             (docs/DS2-BUILD-EMBEDDINGS.md). All 473 values resolve to a WeaponReinforceParam row.

WeaponReinforceParam (stride 244)
  +0x48 i32  maximum reinforce level.  REGULATION, meaning INFERRED. The evidence:
             * over all 374 rows it takes only the values 10 (251 rows), 5 (118) and 0 (5);
             * Falchion (row 1600) 10, Old Knight Hammer (2730) 10, Moonlight Greatsword (1871) 5;
             * on 327 rows it equals the number of levels the row's cost row (+0xF0) prices, and
               every +5 row's cost row is Twinkling Titanite (61030000). The only rows where it
               disagrees with a cost row that prices at least one level are rows 2, 3 and 4
               (0 here, 10 in the cost row), and no WeaponParam row points at them.
  +0xF0 i32  ReinforceCostParam row id.  REGULATION, meaning INFERRED: the one field in the row
             whose values are ReinforceCostParam ids, with the count agreement above.

ArmorParam (stride 84), keyed by armor id
  +0x10 i32  ArmorReinforceParam row id.  REGULATION, meaning INFERRED. It equals the row's own
             id on every armor piece items.tsv names, and so does +0x18. The script refuses a
             piece where either differs.
  +0x34 f32  weight.  REGULATION, meaning INFERRED from the two "... of Aurous" sets (`--survey`):
             it matches the planner's published weights exactly for all eight pieces. It is not
             used for the max.

ArmorReinforceParam (stride 116), keyed by armor id
  +0x60 i32  maximum reinforce level.  REGULATION, meaning INFERRED. Over 502 rows it is 10 (312),
             5 (165) or 0 (25; every one an unnamed row with no items.tsv item). It is the field
             `~/.cache/ds2-builds/armor-phys.json` "max_level" was built from: all 435 entries of
             that file equal +0x60 of their armor_id's row. Every row whose cost row prices at
             least one level has +0x60 equal to that count.
  +0x70 i32  ReinforceCostParam row id.  REGULATION, meaning INFERRED, same evidence as weapons.

ReinforceCostParam (stride 100)
  +0x00 i32[10]  souls for each level +1..+10.       REGULATION, meaning INFERRED
  +0x28 i32[10]  material item id for each level.    REGULATION, meaning INFERRED
             (60970000 Titanite Shard, 60975000 Large Shard, 60980000 Chunk, 60990000 Slab,
             61030000 Twinkling Titanite, 61000000 Petrified Dragon Bone.) "Levels priced" is the
             count of levels with a nonzero souls cost or a nonzero material id.

Why there is no per-level array to count
----------------------------------------

Neither reinforce param stores a per-level stat table. WeaponReinforceParam holds a base block
(+0x00) and a max block (+0x24) of attack values; ArmorReinforceParam holds a base block (+0x00)
and a max block (+0x30) of defense values. The one 11-wide float run in WeaponReinforceParam
(+0x74..+0x9C) is fully populated on +5 rows too, so it says nothing about the cap. The only other
source of a cap is the cost row, and `--survey` compares the two.

Where the max field and the cost row disagree
---------------------------------------------

Some rows carry a nonzero max while their cost row prices no level at all (every souls cost and
every material is 0). Among items.tsv ids that is the four Black Dragon armor pieces (+0x60 = 5,
cost rows 40000000..40000300 all zero) and weapons whose cost id is 0, such as Binoculars and Key
to the Embedded. The TSV reports the max field. Whether the blacksmith offers an upgrade for these
items depends on the executable, which has not been checked. `--cost-gated` writes 0 for them
instead. Every run lists them on stderr.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import struct
import sys
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
ITEMS_TSV = REPO / "crates/ds2-build-import-core/data/items.tsv"
ARMOR_PHYS = Path.home() / ".cache/ds2-builds/armor-phys.json"

# ItemParam
ITEM_WEAPON_ID = 0x14
ITEM_ARMOR_ID = 0x18
# WeaponParam
WEAPON_REINFORCE_ID = 0x08
# WeaponReinforceParam
WRP_MAX = 0x48
WRP_COST_ID = 0xF0
# ArmorParam
ARMOR_REINFORCE_ID = 0x10
ARMOR_REINFORCE_ID_ALT = 0x18
ARMOR_WEIGHT = 0x34
# ArmorReinforceParam
ARP_MAX = 0x60
ARP_MAX_DEFENSE = 0x30
ARP_COST_ID = 0x70
# ReinforceCostParam
COST_SOULS = 0x00
COST_MATERIAL = 0x28
COST_LEVELS = 10

EXPECTED_STRIDES = {
    "ItemParam.param": 84,
    "WeaponParam.param": 188,
    "WeaponReinforceParam.param": 244,
    "ArmorParam.param": 84,
    "ArmorReinforceParam.param": 116,
    "ReinforceCostParam.param": 100,
}

# (item id, kind, max field, levels the cost row prices, what it is). Read with this script and
# cross-checked between the two sources, as the docstring describes.
SELFTEST = [
    (1600000, "weapon", 10, 10, "Falchion, ordinary weapon"),
    (2730000, "weapon", 10, 10, "Old Knight Hammer, ordinary weapon (titanite, not twinkling)"),
    (1871000, "weapon", 5, 5, "Moonlight Greatsword, twinkling-titanite weapon"),
    (21030102, "armor", 10, 10, "Wanderer Manchettes, titanite armor"),
    (25060101, "armor", 5, 5, "Agdayne's Black Robe, +5 armor (armor-phys.json says 5)"),
    # the only non-upgradable signal the data holds for a named armor piece: a max field of 5
    # with a cost row that prices nothing. No items.tsv armor has +0x60 = 0.
    (21330100, "armor", 5, 0, "Black Dragon Helm, cost row prices no level"),
]


def load_regulation_module():
    spec = importlib.util.spec_from_file_location("ds2_regulation", REPO / "scripts/ds2-regulation.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def i32(row: bytes, off: int) -> int:
    return struct.unpack_from("<i", row, off)[0]


def f32(row: bytes, off: int) -> float:
    return struct.unpack_from("<f", row, off)[0]


def read_items() -> dict[int, str]:
    out = {}
    for line in ITEMS_TSV.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        a, b = line.split("\t", 1)
        out[int(a)] = b
    return out


class Tables:
    def __init__(self, reg_path: Path | None):
        reg = load_regulation_module()
        members = reg.load(reg_path or reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
        p = {}
        for name, stride in EXPECTED_STRIDES.items():
            param = reg.Param(name, members[name])
            if param.stride != stride:
                raise SystemExit(f"{name}: stride {param.stride}, this script was written for {stride}")
            p[name] = param.by_id()
        self.item = p["ItemParam.param"]
        self.weapon = p["WeaponParam.param"]
        self.wrp = p["WeaponReinforceParam.param"]
        self.armor = p["ArmorParam.param"]
        self.arp = p["ArmorReinforceParam.param"]
        self.cost = p["ReinforceCostParam.param"]

    def levels_priced(self, cost_id: int) -> int | None:
        row = self.cost.get(cost_id)
        if row is None:
            return None
        return sum(
            1
            for k in range(COST_LEVELS)
            if i32(row, COST_SOULS + 4 * k) or i32(row, COST_MATERIAL + 4 * k)
        )

    def resolve(self, item_id: int) -> tuple[str, int, int | None] | str:
        """`(kind, max field, levels priced)`, "not-equipment", or why it could not be resolved."""
        row = self.item.get(item_id)
        if row is None:
            return "no ItemParam row"
        wid, aid = i32(row, ITEM_WEAPON_ID), i32(row, ITEM_ARMOR_ID)
        if wid != -1 and aid != -1:
            return f"ItemParam names both weapon {wid} and armor {aid}"
        if wid != -1:
            w = self.weapon.get(wid)
            if w is None:
                return f"weapon id {wid} has no WeaponParam row"
            rid = i32(w, WEAPON_REINFORCE_ID)
            r = self.wrp.get(rid)
            if r is None:
                return f"weaponReinforceId {rid} has no WeaponReinforceParam row"
            return "weapon", i32(r, WRP_MAX), self.levels_priced(i32(r, WRP_COST_ID))
        if aid != -1:
            a = self.armor.get(aid)
            if a is None:
                return f"armor id {aid} has no ArmorParam row"
            rid, alt = i32(a, ARMOR_REINFORCE_ID), i32(a, ARMOR_REINFORCE_ID_ALT)
            if rid != aid or alt != aid:
                return f"ArmorParam {aid}: +0x10={rid} +0x18={alt}, not both the row id"
            r = self.arp.get(rid)
            if r is None:
                return f"armor reinforce id {rid} has no ArmorReinforceParam row"
            return "armor", i32(r, ARP_MAX), self.levels_priced(i32(r, ARP_COST_ID))
        return "not-equipment"


def run(tables: Tables, cost_gated: bool):
    names = read_items()
    rows, unresolved, disagree, skipped = [], [], [], 0
    for item_id in names:
        res = tables.resolve(item_id)
        if res == "not-equipment":
            skipped += 1
            continue
        if isinstance(res, str):
            unresolved.append((item_id, names[item_id], res))
            continue
        kind, mx, priced = res
        if priced != mx:
            disagree.append((item_id, names[item_id], kind, mx, priced))
            if cost_gated and not priced:
                mx = 0
        rows.append((item_id, mx, kind))
    return rows, unresolved, disagree, skipped


def cmd_survey(t: Tables) -> None:
    print("WeaponReinforceParam +0x48:", dict(Counter(i32(r, WRP_MAX) for r in t.wrp.values())))
    c = Counter((i32(r, WRP_MAX), t.levels_priced(i32(r, WRP_COST_ID))) for r in t.wrp.values())
    print("  (max field, levels priced by cost row at +0xF0):", dict(c))
    used = {i32(w, WEAPON_REINFORCE_ID) for w in t.weapon.values()}
    for rid in sorted(t.wrp):
        r = t.wrp[rid]
        if i32(r, WRP_MAX) == 0:
            print(f"  row {rid}: max 0, cost row prices {t.levels_priced(i32(r, WRP_COST_ID))}, "
                  f"a WeaponParam row points at it: {rid in used}")
    print("ArmorReinforceParam +0x60:", dict(Counter(i32(r, ARP_MAX) for r in t.arp.values())))
    c = Counter((i32(r, ARP_MAX), t.levels_priced(i32(r, ARP_COST_ID))) for r in t.arp.values())
    print("  (max field, levels priced by cost row at +0x70):", dict(c))
    if ARMOR_PHYS.exists():
        j = json.loads(ARMOR_PHYS.read_text())
        same = sum(
            1 for v in j.values()
            if v["armor_id"] in t.arp and i32(t.arp[v["armor_id"]], ARP_MAX) == v["max_level"]
        )
        print(f"  {ARMOR_PHYS} max_level == +0x60 for {same} of {len(j)} entries")
    print("Aurous sets: ArmorParam +0x34 weight, ArmorReinforceParam +0x30..+0x3C max defense")
    for base in (11360100, 11361100):
        for s in range(4):
            a, r = t.armor[base + s], t.arp[base + s]
            print(f"  item {base + s + 10000000}: weight {f32(a, ARMOR_WEIGHT):.1f}  max defense "
                  f"{[f32(r, ARP_MAX_DEFENSE + 4 * k) for k in range(4)]}")


def cmd_selftest(t: Tables) -> int:
    bad = 0
    for item_id, kind, mx, priced, what in SELFTEST:
        got = t.resolve(item_id)
        ok = got == (kind, mx, priced)
        bad += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {item_id} {what}: want {(kind, mx, priced)}, got {got}")
    rows, unresolved, disagree, _ = run(t, cost_gated=True)
    gated = dict((i, m) for i, m, _ in rows)
    ok = gated.get(21330100) == 0 and gated.get(1600000) == 10
    bad += not ok
    print(f"{'ok  ' if ok else 'FAIL'} --cost-gated gives Black Dragon Helm 0 and Falchion 10")
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--regulation", type=Path, default=None, help="enc_regulation.bnd.dcx")
    ap.add_argument("--out", type=Path, default=None, help="write the TSV here instead of stdout")
    ap.add_argument("--cost-gated", action="store_true",
                    help="write 0 where the max field is nonzero but the cost row prices no level")
    ap.add_argument("--survey", action="store_true", help="print the evidence for the fields")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    t = Tables(args.regulation)

    if args.survey:
        cmd_survey(t)
        return 0
    if args.selftest:
        return cmd_selftest(t)

    rows, unresolved, disagree, skipped = run(t, args.cost_gated)
    text = "item_id\tmax_reinforce\tkind\n" + "".join(f"{i}\t{m}\t{k}\n" for i, m, k in rows)
    if args.out:
        args.out.write_text(text)
    else:
        sys.stdout.write(text)
    err = sys.stderr
    print(f"resolved {len(rows)} ({sum(k == 'weapon' for *_, k in rows)} weapon, "
          f"{sum(k == 'armor' for *_, k in rows)} armor); {skipped} items.tsv ids are neither; "
          f"{len(unresolved)} unresolved", file=err)
    for i, n, why in unresolved:
        print(f"  unresolved {i} {n}: {why}", file=err)
    for i, n, k, mx, pr in disagree:
        tail = " -> 0 (--cost-gated)" if args.cost_gated and not pr else ""
        print(f"  max field {mx}, cost row prices {pr}: {i} {n} ({k}){tail}", file=err)
    return 0


if __name__ == "__main__":
    sys.exit(main())
