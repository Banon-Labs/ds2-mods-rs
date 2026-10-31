#!/usr/bin/env python3
"""Every usable item's SpEffect event, and what each consumable is worth to one generated build.

    python3 scripts/ds2-consumables.py dump > consumables.json
    python3 scripts/ds2-consumables.py values Fume_Ultra_Greatsword:Dark --sl 100

`dump`: every event of SpEffectActiveItem/AbnormalState/PassiveItem.emevd as JSON: the event's file
and id, its itemname.fmg name, the ItemParam row (the event id, else the id with its last digit
zeroed, as variant event 62170010 belongs to item 62170000), that row's ItemUsageParam and
ItemTypeParam rows, and every instruction as `[bank, index, words]`, each word scripts/ds2-emevd.py's
`(u32, f32, i32)` reading. Nothing is interpreted.

`values`: the recommender's generate_build for WEAPON:INFUSION at --sl, then each quick item an
invader may use, valued in one unit (kit_edge in scripts/ds2-builds-recommend.py says what it is and
why), with the bracket's status threat it rests on.
"""
import argparse
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
FILES = ("SpEffectActiveItem.emevd", "SpEffectAbnormalState.emevd", "SpEffectPassiveItem.emevd")


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def dump(files) -> int:
    ex = _load("ds2attacks", "ds2-attacks-extract.py")
    reg = ex.load_module("ds2regulation", "ds2-regulation.py")
    d = ex.decode_params(reg.DEFAULT_REGULATION, ex.DEFAULT_DEFS, {
        "ItemParam": "ITEM_PARAM", "ItemUsageParam": "ITEM_USAGE_PARAM", "ItemTypeParam": "ITEM_TYPE_PARAM"})
    names = ex.item_names(reg.GAME_DIR, reg.DEFAULT_REGULATION)
    members = reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
    emevd = ex.load_module("ds2emevd", "ds2-emevd.py")
    out = []
    for file in files:
        for e in emevd.Emevd(Path(file), members[file]).events:
            item = d["ItemParam"].get(str(e.id)) or d["ItemParam"].get(str(e.id // 10 * 10)) or {}
            out.append({"file": file, "id": e.id,
                        "name": names.get(str(e.id)) or names.get(str(e.id // 10 * 10), ""), "item": item,
                        "usage": d["ItemUsageParam"].get(str(item.get("itemUsageParamId"))) or {},
                        "type": d["ItemTypeParam"].get(str(item.get("itemTypeParamId"))) or {},
                        "ins": [[i.bank, i.index, [list(w) for w in i.words()]] for i in e.instructions]})
    json.dump(out, sys.stdout, default=str)
    return 0


def values(target: str, sl: int) -> int:
    rec = _load("ds2_builds_recommend", "ds2-builds-recommend.py")
    sp, mm = rec.dump_site_tables(rec.CACHE / "site-tables")
    data = rec.Data(json.loads(sp.read_text()), json.loads(mm.read_text()))
    print(rec.apply_regulation(data)[:100] + " ...", file=sys.stderr)
    corpus, _ = rec.load_corpus(data)
    weapon, inf = target.split(":")
    g = rec.generate_build(data, corpus, weapon, inf, sl)
    if g is None:
        raise SystemExit(f"no build for {target} at SL {sl}")
    edge = rec.kit_edge(data, corpus, g, weapon, inf, sl)
    print(f"{g['primary'][0]} ({inf}) SL {sl}: max HP {edge['hp']}, mean hit taken {edge['taken']:.1f}, "
          f"hit dealt {edge['dealt']:.1f}, bracket mean HP {edge['their_hp']:.0f}; "
          f"an even fight is {edge['hits']:.1f} hits taken")
    for s, t in edge["status"].items():
        print(f"  {s}: {t['share']:.1%} of {t['n']} bracket builds build it up; my cut {t['cut']:.2f}; "
              f"{t['procs']:.3f} procs a fight, {t['damage']:.0f} HP")
    print(f"{'item':28} {'kind':8} {'edge %':>8} {'s':>5}  why")
    for v, n, kind, sec, why in edge["items"]:
        print(f"{n:28} {kind:8} {100 * v:8.2f} {sec:5g}  {why}")
    for n, why in edge["refused"]:
        print(f"{n:28} refused  {why}")
    print("choose_kit's quick items:")
    for n, why in rec.choose_kit(data, corpus, g, weapon, inf, sl)["items"]:
        print(f"  {n:28} {why}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("dump", help="every item event as JSON")
    d.add_argument("--file", action="append", choices=FILES, help="event files to dump (default: all three)")
    v = sub.add_parser("values", help="each consumable's worth to one generated build")
    v.add_argument("target", help="WEAPON:INFUSION, the recommender's keys")
    v.add_argument("--sl", type=int, required=True)
    a = ap.parse_args()
    if a.cmd == "dump":
        return dump(a.file or FILES)
    return values(a.target, a.sl)


if __name__ == "__main__":
    sys.exit(main())
