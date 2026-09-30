#!/usr/bin/env python3
"""Find an item's inventory icon in the game's archives, and write which items borrow another's.

    python3 scripts/ds2-item-icons.py locate [--id 1220000 ...]
    python3 scripts/ds2-item-icons.py table [--out crates/ds2-overlay/data/item-icons.tsv]

The executable builds an item icon's path as `icon:/tex/Icon/` + `IC_%010d.tpf`
(docs/DS2-ITEM-REQUIREMENTS.md). `GameDataEbl` keys its entries by a hash of the archive path, not
the mount-prefixed one, and the executable does not spell out where `icon:` is mounted. `locate`
tries spellings of that path against every `*Ebl` archive's hash table and prints the ones that
exist, as `scripts/ds2-effect-icons.py locate` does for the status-effect glyphs. It found them at
`/menu/tex/icon/ic_<10 digits>.tpf` in `GameDataEbl`, and nowhere else.

THE ID IN THE NAME IS NOT ALWAYS THE ITEM'S. It is `ItemParam` field 0, which the regulation reader
calls the row's own id because it is that for most rows. The rest name another item -- the Old
Mirrah Greatsword (1911000) names the Mirrah Greatsword (1910000), and most keys name 64000000 --
or carry 0, which names nothing and leaves the item's own id. `table` checks every named pair
against the archive -- the named id has an icon file, the row's own id has none -- and writes them
for `ds2-overlay` to look an icon up by.
"""

from __future__ import annotations

import argparse
import importlib.util
import struct
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
ARCHIVES = ["GameDataEbl", "LqChrEbl", "LqMapEbl", "LqObjEbl", "LqPartsEbl"]
#: Where `icon:` might be mounted: the menu's own root first, as `menu:` is for the effect glyphs.
ROOTS = [
    "/menu/tex/icon/",
    "/menu/tex/icon/item/",
    "/menu/tex/icon/tex/icon/",
    "/menu/icon/",
    "/menu/",
    "/icon/tex/icon/",
    "/icon/",
    "/tex/icon/",
]
LEAVES = ["ic_{id:010d}.tpf", "ic_{id:010d}.tpf.dcx"]
#: Longsword, Dagger and Uchigatana, by their `items.tsv` ids.
DEFAULT_IDS = [1220000, 1000000, 1450000]
#: Where an item's icon is, once `locate` found it.
ICON_PATH = "/menu/tex/icon/ic_{id:010d}.tpf"
#: What `table` writes, for `crates/ds2-overlay/src/item_icon.rs` to compile in.
DEFAULT_TABLE = REPO_ROOT / "crates" / "ds2-overlay" / "data" / "item-icons.tsv"


def load_module(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def headers(ebl):
    for archive in ARCHIVES:
        bhd, bdt, pem = ebl.archive_paths(ebl.GAME_DIR, archive)
        yield archive, bdt, ebl.Bhd5(ebl.decrypt_bhd(bhd, pem))


def locate(ebl, ids: list[int]) -> int:
    hits = 0
    for archive, _bdt, header in headers(ebl):
        print(f"{archive}: {len(header.entries)} entries", flush=True)
        for root in ROOTS:
            for leaf in LEAVES:
                for item in ids:
                    path = root + leaf.format(id=item)
                    entry = header.entries.get(ebl.path_hash(path))
                    if entry:
                        hits += 1
                        size, offset, aes, _bucket = entry
                        print(f"{archive}  {path}  size={size} offset={offset:#x} aes={aes:#x}")
    print(f"{hits} hit(s)")
    return 0 if hits else 1


def table(ebl, out: Path) -> int:
    reg = load_module("ds2_regulation", "ds2-regulation.py")
    members = reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
    items = reg.Param("ItemParam.param", members["ItemParam.param"])
    _archive, _bdt, header = next(headers(ebl))

    def has_icon(icon: int) -> bool:
        return ebl.path_hash(ICON_PATH.format(id=icon)) in header.entries

    pairs = []
    unset = []
    for index, item in enumerate(items.ids):
        (icon,) = struct.unpack_from("<i", items.row(index), 0)
        # `0` is no id at all: four rows carry it, and the one of them with an icon has it under
        # its own id.
        if icon == 0:
            unset.append(item)
        elif icon != item:
            pairs.append((item, icon))
    print(f"  {len(unset)} rows leave the field 0, and keep their own id: {unset}")
    # The claim the table rests on, checked for every pair rather than for the four it was read
    # off: the named id has an icon, the row's own id has none.
    named = sum(1 for _, icon in pairs if has_icon(icon))
    own = [item for item, _ in pairs if has_icon(item)]
    print(f"ItemParam: {len(items.ids)} rows, {len(pairs)} name another item's icon")
    print(f"  {named} of {len(pairs)} named ids have an icon file")
    for item, icon in pairs:
        if not has_icon(icon):
            print(f"    {item} names {icon}, which has none (its own: {has_icon(item)})")
    print(f"  {len(own)} of those rows have one of their own too: {own[:10]}")
    if named != len(pairs) or own:
        print("  ^ the field is not the icon id for every row; nothing written")
        return 1
    out.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Items whose inventory icon is another item's: the item id, then the id its icon is",
        "# filed under (`/menu/tex/icon/ic_<id>.tpf` in GameDataEbl). ItemParam field 0 out of",
        "# enc_regulation.bnd.dcx, written by `scripts/ds2-item-icons.py table`, which checked that",
        "# every named id has an icon and no item here has one of its own. Every other item's icon",
        "# is filed under its own id.",
    ]
    lines += [f"{item}\t{icon}" for item, icon in pairs]
    out.write_text("\n".join(lines) + "\n")
    print(f"wrote {len(pairs)} pairs -> {out}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("action", choices=["locate", "table"])
    parser.add_argument("--id", type=int, action="append", dest="ids")
    parser.add_argument("--out", type=Path, default=DEFAULT_TABLE)
    args = parser.parse_args()
    ebl = load_module("ds2_ebl", "ds2-ebl.py")
    if args.action == "table":
        return table(ebl, args.out)
    return locate(ebl, args.ids or DEFAULT_IDS)


if __name__ == "__main__":
    sys.exit(main())
