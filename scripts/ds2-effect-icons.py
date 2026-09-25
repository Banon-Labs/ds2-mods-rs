#!/usr/bin/env python3
"""Find and dump the HUD's status-effect glyphs (`EI_%04d.tpf`) so a person can pick one.

    python3 scripts/ds2-effect-icons.py locate
    uv run --with pillow python3 scripts/ds2-effect-icons.py dump --out <dir> [--family vow]

`dump` walks every id 0..9999 of the family, writes each glyph as `<NAME>.png`, and a
`sheet.html` showing them all labelled by id -- the id is what SpEffectParam's icon field and
the flo placeholder use.

The executable builds the path as `menu:/tex/Icon/` + `effect/EI_%04d.tpf` (both strings are in
`darksoulsii-deobf.bin`), and `l01_20_effect_icon.flo` -- the HUD document that draws the icon row
under the stamina bar -- names `EI_0010` as its placeholder texture. `locate` tries spellings of
that path against every `*Ebl` archive's hash table and prints the ones that exist.
"""

from __future__ import annotations

import argparse
import importlib.util
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
ARCHIVES = ["GameDataEbl", "LqChrEbl", "LqMapEbl", "LqObjEbl", "LqPartsEbl"]
ROOTS = ["/menu/tex/icon/", "/menu/tex/", "/menu/", "/icon/", "/menu/icon/", "/tex/icon/"]
LEAVES = [
    "effect/ei_{id:04d}.tpf",
    "effect/ei_{id:04d}.tpf.dcx",
    "effect.tpfbhd",
    "effect.tpfbdt",
    "effect_low.tpfbhd",
    "effect.fetexbnd.dcx",
    "icon.tpfbhd",
    "effect/effect.tpfbhd",
]


def load_ebl():
    spec = importlib.util.spec_from_file_location("ds2_ebl", REPO_ROOT / "scripts" / "ds2-ebl.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def headers(ebl):
    for archive in ARCHIVES:
        bhd, bdt, pem = ebl.archive_paths(ebl.GAME_DIR, archive)
        yield archive, bdt, ebl.Bhd5(ebl.decrypt_bhd(bhd, pem))


def locate(ebl) -> int:
    hits = 0
    for archive, _bdt, header in headers(ebl):
        for root in ROOTS:
            for leaf in LEAVES:
                path = root + leaf.format(id=10)
                entry = header.entries.get(ebl.path_hash(path))
                if entry:
                    hits += 1
                    size, offset, aes, _bucket = entry
                    print(f"{archive}  {path}  size={size} offset={offset:#x} aes={aes:#x}")
    print(f"{hits} hit(s)")
    return 0 if hits else 1


FAMILIES = {
    "effect": "/menu/tex/icon/effect/ei_{id:04d}.tpf",
    "vow": "/menu/tex/icon/vow/vi_{id:04d}.tpf",
}


def load_tpf():
    spec = importlib.util.spec_from_file_location("ds2_tpf", REPO_ROOT / "scripts" / "ds2-tpf.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def dump(ebl, family: str, out: Path) -> int:
    import base64
    import io

    from PIL import Image

    tpf = load_tpf()
    out.mkdir(parents=True, exist_ok=True)
    archive, bdt, header = next(headers(ebl))
    pattern = FAMILIES[family]
    cells = []
    for icon_id in range(10000):
        path = pattern.format(id=icon_id)
        entry = header.entries.get(ebl.path_hash(path))
        if entry is None:
            continue
        size, offset, aes, _bucket = entry
        blob = ebl.dcx_decompress(ebl.read_entry(bdt, size, offset, aes, path))
        for texture in tpf.textures(blob):
            image = Image.open(io.BytesIO(texture["payload"])).convert("RGBA")
            name = Path(path).stem.upper()
            image.save(out / f"{name}.png")
            buffer = io.BytesIO()
            image.save(buffer, "PNG")
            cells.append((icon_id, name, image.size, base64.b64encode(buffer.getvalue()).decode()))
    rows = "\n".join(
        f'<figure><img src="data:image/png;base64,{b64}" width="{w}" height="{h}" alt="{name}">'
        f"<figcaption>{icon_id}<br><small>{name}</small></figcaption></figure>"
        for icon_id, name, (w, h), b64 in cells
    )
    (out / "sheet.html").write_text(
        f"<!doctype html><meta charset=utf-8><title>{family} glyphs</title>"
        "<style>body{background:#222;color:#ddd;font:12px sans-serif;display:flex;flex-wrap:wrap;gap:6px}"
        "figure{margin:0;padding:4px;background:#333;text-align:center}</style>"
        f"<p style='width:100%'>{len(cells)} {family} glyphs from {archive}</p>{rows}"
    )
    print(f"{len(cells)} glyphs -> {out}")
    return 0 if cells else 1


#: The game's own voice-chat HUD icon: `FeScenePlayerVoiceChatIcon` (update `0x14050a5f0`) toggles
#: element 0x5f5c3e0 and its children 3e0/3e1/3e2, which is def 0x3a in `l01_01_hp.flo`. Its four
#: shapes all sample `waku_03`; these are their source rects, read with `ds2-flo.py shape`.
VOICE_CHAT_SHAPES = {
    "vc_shape_2f": (902.5, 169.25, 936.1, 203.15),
    "vc_shape_30": (916.05, 210.1, 935.25, 234.7),
    "vc_shape_35_under_3e2": (884.45, 179.9, 900.6, 197.4),
    "vc_shape_37_under_3e0_3e1": (838.85, 176.85, 860.75, 199.3),
}


def crop(atlas: Path, out: Path) -> int:
    from PIL import Image

    out.mkdir(parents=True, exist_ok=True)
    sheet = Image.open(atlas).convert("RGBA")
    for name, (x0, y0, x1, y1) in VOICE_CHAT_SHAPES.items():
        piece = sheet.crop((int(x0), int(y0), round(x1), round(y1)))
        piece.save(out / f"{name}.png")
        print(f"{name}  {piece.size}  -> {out / (name + '.png')}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("action", choices=["locate", "dump", "voice"])
    parser.add_argument("--out", type=Path)
    parser.add_argument("--family", choices=sorted(FAMILIES), default="effect")
    parser.add_argument("--atlas", type=Path, help="waku_03.dds, from `ds2-tpf.py extract waku_03.tpf`")
    args = parser.parse_args()
    if args.action == "voice":
        if not (args.out and args.atlas):
            parser.error("voice needs --atlas and --out")
        return crop(args.atlas, args.out)
    if args.action == "dump":
        if not args.out:
            parser.error("dump needs --out")
        return dump(load_ebl(), args.family, args.out)
    ebl = load_ebl()
    if args.action == "locate":
        return locate(ebl)
    return 2


if __name__ == "__main__":
    sys.exit(main())
