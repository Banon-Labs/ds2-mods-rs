#!/usr/bin/env python3
"""Find an item's inventory icon in the game's archives, and write which items borrow another's.

    python3 scripts/ds2-item-icons.py locate [--id 1220000 ...]
    python3 scripts/ds2-item-icons.py table [--out crates/ds2-overlay/data/item-icons.tsv]
    uv run --with pillow python3 scripts/ds2-item-icons.py measure [--kind armor] [--id 1620000 ...]
    uv run --with pillow python3 scripts/ds2-item-icons.py png [--kind ring] --id 1220000 --out <dir>
    uv run --with pillow python3 scripts/ds2-item-icons.py categories --out <sheet.png>

`measure` reads every icon of one kind -- by default every weapon's (every `ItemParam` row with a
`WeaponParam` id: weapons, shields and catalysts), or `--kind armor`, `ring` or `item` -- and says
how big the texture is and where its ink is -- the box around the pixels at least an eighth
opaque -- so a layout can be sized to the art rather than to a guess. `png` writes icons at their
own size, for a design canvas to show the real thing. `categories` draws the inventory tabs'
category icons on one labelled sheet. The Equipment page does NOT draw those in an empty slot:
its silhouettes are frames of a layout sprite, `FE_EQUIP_EMPTY_ART` in `crates/ds2-rva`.

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


#: A pixel counts as ink from this alpha up: an eighth opaque, so a soft glow's fringe does not
#: stretch the box.
INK_ALPHA = 32


#: Which `ItemParam` rows each `--kind` reads. A weapon names a `WeaponParam` row at +0x14 and an
#: armour piece an `ArmorParam` row at +0x18 (`scripts/ds2-reinforce-max.py`). Rings and the rest
#: have no such link here, so they go by the id ranges `items.tsv` shows: every ring is a
#: 40000000-41999999 row (Third Dragon Ring 40040002, Flynn's Ring 41100000), and consumables,
#: ammunition and keys are 50000000 and up (Estus Flask 60155010, Wood Bolt 60910000).
KINDS = {
    "weapon": lambda item, fields: fields[5] != -1,
    "armor": lambda item, fields: fields[6] != -1,
    "ring": lambda item, fields: 40000000 <= item < 42000000 and fields[5] == fields[6] == -1,
    "item": lambda item, fields: item >= 50000000 and fields[5] == fields[6] == -1,
}


def item_icons(ebl, kind: str = "weapon"):
    """`(item, icon id, RGBA image)` for every `ItemParam` row of `kind` (see [`KINDS`])."""
    import io

    from PIL import Image

    reg = load_module("ds2_regulation", "ds2-regulation.py")
    tpf = load_module("ds2_tpf", "ds2-tpf.py")
    members = reg.load(reg.DEFAULT_REGULATION, reg.REGULATION_KEY_HEX)
    items = reg.Param("ItemParam.param", members["ItemParam.param"])
    _archive, bdt, header = next(headers(ebl))
    keep = KINDS[kind]
    for index, item in enumerate(items.ids):
        fields = struct.unpack_from("<21i", items.row(index), 0)
        if not keep(item, fields):
            continue
        icon = fields[0] or item
        path = ICON_PATH.format(id=icon)
        entry = header.entries.get(ebl.path_hash(path))
        if entry is None:
            yield item, icon, None
            continue
        size, offset, aes, _bucket = entry
        blob = ebl.dcx_decompress(ebl.read_entry(bdt, size, offset, aes, path))
        texture = tpf.textures(blob)[0]
        yield item, icon, Image.open(io.BytesIO(texture["payload"])).convert("RGBA")


def ink_box(image) -> tuple[int, int, int, int] | None:
    """The box around the pixels at least [`INK_ALPHA`] opaque, `(x0, y0, x1, y1)`."""
    return image.getchannel("A").point(lambda a: 255 if a >= INK_ALPHA else 0).getbbox()


def spread(values: list[int]) -> str:
    """min, tenth, median, ninetieth and max of `values`."""
    ordered = sorted(values)
    pick = lambda share: ordered[min(len(ordered) - 1, int(share * len(ordered)))]
    return (
        f"min {ordered[0]}  p10 {pick(0.1)}  median {pick(0.5)}  p90 {pick(0.9)}  "
        f"max {ordered[-1]}"
    )


def measure(ebl, ids: list[int], kind: str) -> int:
    from collections import Counter

    sizes: Counter = Counter()
    boxes: dict[tuple[int, int], list[tuple[int, int, int, int]]] = {}
    missing = []
    for item, icon, image in item_icons(ebl, kind):
        if image is None:
            missing.append(item)
            continue
        box = ink_box(image)
        sizes[image.size] += 1
        if box:
            boxes.setdefault(image.size, []).append(box)
        if item in ids:
            print(f"  {item} (icon {icon}): texture {image.size[0]}x{image.size[1]}, ink {box}"
                  + (f" = {box[2] - box[0]}x{box[3] - box[1]}" if box else ""))
    print(f"{kind} icons by texture size: {dict(sizes)}; no icon: {missing}")
    for size, found in boxes.items():
        print(f"{size[0]}x{size[1]}, {len(found)} icons with ink:")
        print(f"  ink width   {spread([b[2] - b[0] for b in found])}")
        print(f"  ink height  {spread([b[3] - b[1] for b in found])}")
        print(f"  ink left    {spread([b[0] for b in found])}")
        print(f"  ink top     {spread([b[1] for b in found])}")
        print(f"  ink right   {spread([b[2] for b in found])}")
        print(f"  ink bottom  {spread([b[3] for b in found])}")
        union = (min(b[0] for b in found), min(b[1] for b in found),
                 max(b[2] for b in found), max(b[3] for b in found))
        print(f"  every icon's ink inside {union}")
    return 0 if sizes else 1


def png(ebl, ids: list[int], out: Path, kind: str) -> int:
    out.mkdir(parents=True, exist_ok=True)
    written = 0
    for item, icon, image in item_icons(ebl, kind):
        if item in ids and image is not None:
            target = out / f"ic_{item:010d}.png"
            image.save(target)
            written += 1
            print(f"{item} (icon {icon}) {image.size[0]}x{image.size[1]} -> {target}")
    return 0 if written == len(set(ids)) else 1


#: Where an item category's icon is: `Item_Category/IC_CA_%05d.tpf` under `menu:/tex/Icon/`, the
#: kind-12 case of the icon path builder at 0x14048c3e0, found in `GameDataEbl` by hash.
CATEGORY_PATH = "/menu/tex/icon/item_category/ic_ca_{id:05d}.tpf"
#: The inventory's category tabs, from the 36-entry table at 0x14156b120 (`{icon id, 0, name,
#: 1, group}` in 0x18-byte rows). The names there are Japanese; these are translations of them.
CATEGORY_NAMES = {
    10: "dagger", 20: "straight sword", 30: "greatsword", 40: "curved sword",
    50: "ultra greatsword", 60: "twinblade", 70: "thrusting sword", 80: "axe", 90: "hammer",
    100: "spear", 110: "fist", 120: "whip", 130: "bow/crossbow", 140: "catalyst", 150: "shield",
    400: "head", 410: "chest", 420: "hands", 430: "legs", 500: "ring", 600: "arrows & bolts",
    700: "active item", 710: "passive item", 720: "material", 800: "spell", 900: "gesture",
    1000: "shop weapon", 1400: "shop armor", 1500: "shop ring", 1600: "shop ammo",
    1700: "shop item", 1800: "shop spell", 2000: "storage",
}


def categories(ebl, out: Path) -> int:
    """Every item category icon the archive holds, as one labelled sheet at `out`."""
    import io

    from PIL import Image, ImageDraw

    tpf = load_module("ds2_tpf", "ds2-tpf.py")
    _archive, bdt, header = next(headers(ebl))
    icons = []
    for cid in range(0, 10000, 10):
        path = CATEGORY_PATH.format(id=cid)
        entry = header.entries.get(ebl.path_hash(path))
        if entry is None:
            continue
        size, offset, aes, _bucket = entry
        blob = ebl.dcx_decompress(ebl.read_entry(bdt, size, offset, aes, path))
        image = Image.open(io.BytesIO(tpf.textures(blob)[0]["payload"])).convert("RGBA")
        icons.append((cid, image))
        print(f"  {cid:5d}  {image.size[0]}x{image.size[1]}  {CATEGORY_NAMES.get(cid, '(no tab)')}")
    print(f"{len(icons)} category icons at {CATEGORY_PATH}")
    cell, label, columns = 96, 30, 8
    rows = -(-len(icons) // columns)
    sheet = Image.new("RGBA", (columns * cell, rows * (cell + label)), (40, 36, 32, 255))
    draw = ImageDraw.Draw(sheet)
    for n, (cid, image) in enumerate(icons):
        x, y = (n % columns) * cell, (n // columns) * (cell + label)
        scale = min(cell / image.size[0], cell / image.size[1])
        shown = image.resize((int(image.size[0] * scale), int(image.size[1] * scale)))
        sheet.alpha_composite(shown, (x + (cell - shown.size[0]) // 2, y))
        draw.text((x + 3, y + cell + 2), str(cid), fill=(255, 255, 255, 255))
        draw.text((x + 3, y + cell + 14), CATEGORY_NAMES.get(cid, "")[:15], fill=(200, 190, 170, 255))
    out.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out)
    print(f"sheet -> {out}")
    return 0 if icons else 1


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("action", choices=["locate", "table", "measure", "png", "categories"])
    parser.add_argument("--id", type=int, action="append", dest="ids")
    parser.add_argument("--kind", choices=sorted(KINDS), default="weapon",
                        help="which items `measure` and `png` read (default: weapon)")
    parser.add_argument("--out", type=Path, default=DEFAULT_TABLE)
    args = parser.parse_args()
    ebl = load_module("ds2_ebl", "ds2-ebl.py")
    if args.action == "table":
        return table(ebl, args.out)
    if args.action == "measure":
        return measure(ebl, args.ids or [], args.kind)
    if args.action == "png":
        if not args.ids or args.out == DEFAULT_TABLE:
            parser.error("png needs --id and an --out directory")
        return png(ebl, args.ids, args.out, args.kind)
    if args.action == "categories":
        if args.out == DEFAULT_TABLE:
            parser.error("categories needs --out <sheet.png>")
        return categories(ebl, args.out)
    return locate(ebl, args.ids or DEFAULT_IDS)


if __name__ == "__main__":
    sys.exit(main())
