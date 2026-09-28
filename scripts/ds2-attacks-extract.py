#!/usr/bin/env python3
"""Build `~/.cache/ds2-builds/attacks.json`: every weapon attack, its hitboxes and its hit count.

    python3 scripts/ds2-attacks-extract.py --unpack-tae     # once: decrypt + unpack c000100_pl.tae
    python3 scripts/ds2-attacks-extract.py                  # regulation + TAE XML -> attacks.json
    python3 scripts/ds2-attacks-extract.py --out - | jq '.[] | select(.w == 2890000)'

`scripts/ds2-builds-recommend.py` reads the file this writes (bleed/poison ranking by build-up per
hit times hits per attack, and the R1 timeline). It was first produced by a scratchpad one-liner
in session 7d4beb64 (2026-09-28, the research behind docs/DS2-DPS-MECHANICS.md); this is that
program, committed, and it regenerates the same bytes (checked with `cmp` against the cached file
when it was committed).

THE CHAIN, one row per weapon x attack slot:

  WeaponParam.weaponActionCategoryId -> WeaponActionCategoryParam.atkId<slot>
    -> WeaponAttackMotionParam row: attackAnim, start/endPlaySpeed, damageId01..03, bulletId01..03
    -> the TAE animation `attackAnim` in /timeact/chr/c000100_pl.tae (the player's)
       -> every event of type 2200 (hitbox window), its `ID` param picks damageId01/02/03
          -> PlayerDamageParam row: damageRate (the motion value), hitDistance, hit flags

HIT COUNT per hitbox is `n = floor(window_seconds / hitDistance) + 1` when hitDistance > 0, else 1.
That hitDistance is a RE-HIT INTERVAL IN SECONDS is INFERRED (docs/DS2-DPS-MECHANICS.md), not read
out of the executable. `max_hits` sums `n` over the live hitboxes (hitFlagEnemy or hitFlagFriend).
Times are 30 fps frames, rounded to two places; param floats are rounded to four, as the original
did, so the output is byte-stable.

INPUTS, all read from the installed game except two local tools' data:

  * enc_regulation.bnd.dcx, decrypted by scripts/ds2-regulation.py.
  * Field layouts from Smithbox's DS2S paramdefs (`--defs`), the same XML the rest of the build
    research decoded with. Each layout's size is checked against the param's measured stride and
    a mismatch refuses the param rather than decoding it at the wrong offsets.
  * itemname.fmg (GameDataEbl, /menu/text/english/itemname.fmg) for names; the WeaponParam id is
    the ItemParam id is the text id.
  * The TAE as WitchyBND's XML (`--tae-dir`, one `anim-<id>.xml` per animation). `--unpack-tae`
    produces it: the entry is one of GameDataEbl's AES-range-encrypted ones, decrypted here with
    the per-entry key record (openssl, AES-128-ECB over each range), then unpacked by WitchyBND,
    which needs a terminal, so it runs under `script`. No TAE parser is vendored here; the event
    param templates are WitchyBND's.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import shutil
import struct
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
CACHE = Path.home() / ".cache/ds2-builds"
DEFAULT_OUT = CACHE / "attacks.json"
DEFAULT_TAE_DIR = CACHE / "tae"
DEFAULT_DEFS = Path.home() / ".local/share/smithbox/app/Assets/PARAM/DS2S/Defs"
DEFAULT_WITCHY = Path.home() / "projects/WitchyBND/bin/Debug/net10.0/linux-x64/WitchyBND"
TAE_PATH = "/timeact/chr/c000100_pl.tae"
FMG_PATH = "/menu/text/english/itemname.fmg"
HITBOX_EVENT = 2200  # TAE event type: attack hitbox window
FPS = 30

#: regulation member -> Smithbox paramdef
PARAMS = {
    "WeaponParam": "WEAPON_PARAM",
    "WeaponActionCategoryParam": "WEAPON_ACTION_CATEGORY_PARAM",
    "WeaponAttackMotionParam": "WEAPON_ATTACK_MOTION_PARAM",
    "PlayerDamageParam": "DAMAGE_PARAM",
}
SIZES = {"u8": 1, "s8": 1, "u16": 2, "s16": 2, "u32": 4, "s32": 4, "f32": 4, "angle32": 4, "dummy8": 1, "fixstr": 1}
FORMATS = {"u8": "B", "s8": "b", "u16": "H", "s16": "h", "u32": "I", "s32": "i", "f32": "f", "angle32": "f"}


def load_module(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def layout(defs: Path, name: str) -> tuple[int, list]:
    """`(size, [[field, type, offset, bit_position, bits]])` from a Smithbox paramdef XML."""
    fields, off, bit_pos, bit_base = [], 0, None, 0
    for f in ET.parse(defs / f"{name}.xml").getroot().iter("Field"):
        ty, nm, bits, n = re.match(r"(\w+)\s+(\w+)(?::(\d+))?(?:\[(\d+)\])?", f.get("Def")).groups()
        size = SIZES[ty]
        if bits:
            bits = int(bits)
            if bit_pos is None or bit_pos + bits > size * 8:
                bit_base, bit_pos, off = off, 0, off + size
            fields.append([nm, ty, bit_base, bit_pos, bits])
            bit_pos += bits
            continue
        bit_pos = None
        n = int(n) if n else 1
        if ty in ("dummy8", "fixstr"):
            off += size * n
            continue
        for i in range(n):
            fields.append([nm + (f"[{i}]" if n > 1 else ""), ty, off, None, None])
            off += size
    return off, fields


def decode_params(regulation: Path, defs: Path) -> dict[str, dict[str, dict]]:
    reg = load_module("ds2regulation", "ds2-regulation.py")
    members = reg.load(regulation, reg.REGULATION_KEY_HEX)
    out = {}
    for member, def_name in PARAMS.items():
        size, fields = layout(defs, def_name)
        p = reg.Param(member, members[f"{member}.param"])
        if p.stride != size:
            raise SystemExit(f"{member}: stride {p.stride} != paramdef {def_name} size {size}")
        rows = {}
        for k, rid in enumerate(p.ids):
            b, r = p.row(k), {}
            for nm, ty, off, bit_pos, bits in fields:
                v = struct.unpack_from("<" + FORMATS[ty], b, off)[0]
                if bits:
                    v = (v >> bit_pos) & ((1 << bits) - 1)
                if isinstance(v, float):
                    v = round(v, 4)
                r[nm] = v
            rows[str(rid)] = r
        out[member] = rows
    return out


def ebl_read(game_dir: Path, virtual_path: str) -> bytes:
    """One GameDataEbl entry, decrypting its AES ranges when it has a key record."""
    ebl = load_module("ds2ebl", "ds2-ebl.py")
    bhd, bdt, pem = ebl.archive_paths(game_dir, ebl.DEFAULT_ARCHIVE)
    header = ebl.Bhd5(ebl.decrypt_bhd(bhd, pem))
    size, offset, aes_key, _ = header.lookup(virtual_path)
    with bdt.open("rb") as handle:
        handle.seek(offset)
        data = bytearray(handle.read(size))
    if aes_key:
        key = header.blob[aes_key : aes_key + 16]
        count = struct.unpack_from("<i", header.blob, aes_key + 16)[0]
        for i in range(count):
            start, end = struct.unpack_from("<qq", header.blob, aes_key + 20 + 16 * i)
            if start == -1 or end <= start:
                continue
            data[start:end] = subprocess.run(
                ["openssl", "enc", "-d", "-aes-128-ecb", "-nopad", "-K", key.hex()],
                input=bytes(data[start:end]), capture_output=True, check=True).stdout
    return ebl.dcx_decompress(bytes(data))


def item_names(game_dir: Path, regulation: Path) -> dict[str, str]:
    """ItemParam id -> itemname.fmg text, whitespace-collapsed; ids with no text get a marker."""
    reg = load_module("ds2regulation", "ds2-regulation.py")
    fmg = load_module("ds2fmg", "ds2-fmg.py")
    texts = fmg.parse(ebl_read(game_dir, FMG_PATH))
    items = reg.Param("ItemParam.param", reg.load(regulation, reg.REGULATION_KEY_HEX)["ItemParam.param"])
    names = {}
    for rid in items.ids:
        label = texts.get(rid)
        names[str(rid)] = " ".join((label if label is not None else "<no name in fmg>").split())
    return names


def unpack_tae(game_dir: Path, tae_dir: Path, witchy: Path) -> None:
    """Decrypt c000100_pl.tae and unpack it with WitchyBND into `tae_dir` (anim-*.xml)."""
    if not witchy.is_file():
        raise SystemExit(f"WitchyBND not found at {witchy} (--witchy)")
    work = tae_dir.parent / (tae_dir.name + ".work")
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    tae = work / "c000100_pl.tae"
    tae.write_bytes(ebl_read(game_dir, TAE_PATH))
    (work / "project.json").write_text('{\n  "GameType": DarkSoulsIISOTFS,\n}\n')
    subprocess.run(["script", "-q", "-e", "-c", f"{witchy} -p -u {tae}", "/dev/null"],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    unpacked = work / "c000100_pl-tae"
    if not any(unpacked.glob("anim-*.xml")):
        raise SystemExit(f"WitchyBND produced no anim-*.xml under {unpacked}")
    shutil.rmtree(tae_dir, ignore_errors=True)
    unpacked.rename(tae_dir)
    shutil.rmtree(work, ignore_errors=True)
    print(f"unpacked {sum(1 for _ in tae_dir.glob('anim-*.xml'))} animations to {tae_dir}", file=sys.stderr)


def tae_events(tae_dir: Path, anim: int, cache: dict) -> list | None:
    """`[(type, start_frame, end_frame, [param values])]` sorted by start, or None if absent."""
    if anim in cache:
        return cache[anim]
    f = tae_dir / f"anim-{anim:09d}.xml"
    out = None
    if f.exists():
        out = []
        for e in ET.parse(f).getroot().find("events"):
            t = int(e.findtext("type"))
            s = float(e.findtext("startTime")) * FPS
            en = float(e.findtext("endTime")) * FPS
            ps = e.find("params")
            pv = [p.get("value") for p in ps] if ps is not None else []
            out.append((t, round(s, 2), round(en, 2), pv))
        out = sorted(out, key=lambda x: x[1])
    cache[anim] = out
    return out


def attacks(d: dict, names: dict, tae_dir: Path) -> list[dict]:
    W, A, M, P = d["WeaponParam"], d["WeaponActionCategoryParam"], d["WeaponAttackMotionParam"], d["PlayerDamageParam"]
    slots = [k[5:] for k in A["1000"] if k.startswith("atkId")]
    cache: dict = {}
    rows = []
    for wid, w in sorted(W.items(), key=lambda x: int(x[0])):
        a = A.get(str(w["weaponActionCategoryId"]))
        if not a:
            continue
        for s in slots:
            mid = a["atkId" + s]
            m = M.get(str(mid))
            if not m or mid < 10:
                continue
            evs = tae_events(tae_dir, m["attackAnim"], cache)
            if evs is None:
                rows.append(dict(w=int(wid), name=names.get(wid, "?"), slot=s, motion=mid, anim=m["attackAnim"],
                                 missing_tae=True))
                continue
            hits = []
            for t, st, en, pv in evs:
                if t != HITBOX_EVENT:
                    continue
                idx = int(pv[0]) if pv else 0
                did = [m["damageId01"], m["damageId02"], m["damageId03"]][idx] if idx < 3 else 0
                p = P.get(str(did)) if did else None
                hd = p["hitDistance"] if p else 0
                live = bool(p) and (p["hitFlagEnemy"] or p["hitFlagFriend"])
                hits.append(dict(id=idx, start=st, end=en, dmg=did, rate=p and p["damageRate"], interval=hd, live=live,
                                 n=(int((en - st) / FPS / hd) + 1 if hd > 0 else 1)))
            live = [h for h in hits if h["live"]]
            rows.append(dict(w=int(wid), name=names.get(wid, "?"), slot=s, motion=mid, anim=m["attackAnim"],
                             spd=[m["startPlaySpeed"], m["endPlaySpeed"]],
                             bullets=[m["bulletId01"], m["bulletId02"], m["bulletId03"]],
                             hits=hits, max_hits=sum(h["n"] for h in live)))
    return rows


def main() -> int:
    reg_default = load_module("ds2regulation", "ds2-regulation.py")
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default=str(DEFAULT_OUT), help="output path, or - for stdout")
    ap.add_argument("--tae-dir", type=Path, default=DEFAULT_TAE_DIR, help="WitchyBND anim-*.xml directory")
    ap.add_argument("--defs", type=Path, default=DEFAULT_DEFS, help="Smithbox DS2S paramdef directory")
    ap.add_argument("--regulation", type=Path, default=reg_default.DEFAULT_REGULATION)
    ap.add_argument("--game-dir", type=Path, default=reg_default.GAME_DIR)
    ap.add_argument("--unpack-tae", action="store_true", help="decrypt + unpack the TAE into --tae-dir, then exit")
    ap.add_argument("--witchy", type=Path, default=DEFAULT_WITCHY, help="WitchyBND executable (for --unpack-tae)")
    args = ap.parse_args()

    if args.unpack_tae:
        unpack_tae(args.game_dir, args.tae_dir, args.witchy)
        return 0
    if not any(args.tae_dir.glob("anim-*.xml")):
        raise SystemExit(f"no anim-*.xml under {args.tae_dir}; run with --unpack-tae first")
    rows = attacks(decode_params(args.regulation, args.defs), item_names(args.game_dir, args.regulation), args.tae_dir)
    text = json.dumps(rows) + "\n"
    if args.out == "-":
        sys.stdout.write(text)
    else:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(text)
        missing = sum(1 for r in rows if r.get("missing_tae"))
        print(f"wrote {len(rows)} attack rows ({missing} with no TAE animation) to {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
