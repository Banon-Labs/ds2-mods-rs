#!/usr/bin/env python3
"""Read the player's Morpheme animation binder: where each attack plays at which swing speed.

    python3 scripts/ds2-anibnd.py windows                 # -> ~/.cache/ds2-builds/attack-speed-windows.json
    python3 scripts/ds2-anibnd.py windows --anim 21030011  # print one animation's windows
    python3 scripts/ds2-anibnd.py list                    # the binder's members
    python3 scripts/ds2-anibnd.py dump c0001.nmb --out DIR
    python3 scripts/ds2-anibnd.py sets DIR/c0001.nmb      # every event-track set: TAE id + speed events
    python3 scripts/ds2-anibnd.py strtab DIR/c0001.nmb "ControlParameters|Attack_SwingSpeed"
    python3 scripts/ds2-anibnd.py nodes DIR/c0001.nmb 111  # node defs fed by control parameter 111

WHAT THE EXE DOES (docs/DS2-DPS-MECHANICS.md "Attack play speed"): `ChrEventTrackActionAttack`
`0x140368e70` counts `+0xa0` up on the start and down on the end of every animation event whose
user data is 150, `+0xa4` for 151; `0x14035d580` then sets the network control parameter
`Attack_SwingSpeed` to flags+0x200 x startPlaySpeed while a 150 is open, endPlaySpeed while a 151
is, else 1.0. Those events are in this binder, not in the TAE.

THE FILE, every step read here rather than taken from a tool:

  GameDataEbl /morpheme4/chr/c0001.anibnd.dcx   no AES record; DCX -> BND4 (UTF-16 names, read by
                                                 ds2-ffx.py's BND4 reader), 898 members
    c0001.nmb      the Morpheme network: a run of chunks `{u32 0x18, u32 0xa, u32 type, u8 id[20],
                   u64 size, u32 align, u32 ?}` + payload, MSVC 0xcd fill between them.
                   Chunk type 4 (5762 of them) is a DURATION EVENT TRACK: `u32 n, u32 2, u32 name_at,
                   u32 0, u32 track user data, u32 channel, u32 events_at`, events `{f32 start,
                   f32 duration, u32 user data}`, offsets from the payload start. Times are FRACTIONS
                   of the clip (every one is in 0..1).
                   The type-10 chunk is the network definition. Its attrib data of type 0x17 (marker
                   `ff ff 17 00`) is a source event-track set: counts and id lists of the discrete,
                   curve and duration tracks one animation source plays; ids are the first u32 of
                   the track chunk's id.
    aXX_YY_ZZZZ_*.nsa   the animations; header +0x28 f32 is the clip length in seconds.

THE JOIN. In every set, the duration track named `TimeAct` has one event whose user data is the
TAE animation id the set's clip plays (21030011 for the Black Flamestone Dagger's 2H R1); the
`AttackSpeedControl` track holds the 150/151 events. The nsa's name encodes the same id (a21_03_0011
-> 21030011). The clip length used to turn fractions into seconds is that nsa's. The check that
this is the right length: 92% of the 150/151 boundaries land on the 1/30 s grid of their clip
(`windows` prints the count that do not; the rest miss by up to half a frame, fractions that fit a
clip of another length), and the dagger 2H R1's 150 ends on the frame its hitbox opens.

`Attack_SwingSpeed` (network node 111) feeds the speed input of 136 nodes of type 125, the type that
also takes `DodgeSpeed`, `StepSpeed`, `ItemUse_Speed` and the constants `PlaySpeed_2`/`PlaySpeed_5`:
a play-speed modifier over its one child. That type 125 is Morpheme's play-speed modifier is
INFERRED from those inputs (the EXE has `nodePlaySpeedModifierQueueUpdateTime` but the type number
was not tied to it).
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
ANIBND = "/morpheme4/chr/c0001.anibnd.dcx"
DEFAULT_OUT = Path.home() / ".cache/ds2-builds/attack-speed-windows.json"
CHUNK_HEADER = 0x30
DURATION_TRACK = 4
NSA_LENGTH = 0x28
NSA_NAME = re.compile(r"a(\d\d)_(\d\d)_(\d{4})_.*\.nsa$")
SET_MARK = b"\xff\xff\x17\x00\xcd\xcd\xcd\xcd"  # refcount 0xffff, attrib type 0x17, pad
#: the node-name IDMappedStringTable {n, length, ids, offsets, data} in this build's c0001.nmb
NAMES_HEADER = 0xCB7A8
ACTION_START, ACTION_END = 150, 151


def load_module(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def members(game_dir: Path | None = None) -> list[tuple[str, bytes]]:
    ebl = load_module("ds2ebl", "ds2-ebl.py")
    bhd, bdt, pem = ebl.archive_paths(game_dir or ebl.GAME_DIR, ebl.DEFAULT_ARCHIVE)
    header = ebl.Bhd5(ebl.decrypt_bhd(bhd, pem))
    size, offset, aes_key, _ = header.lookup(ANIBND)
    data = ebl.dcx_decompress(ebl.read_entry(bdt, size, offset, aes_key, ANIBND))
    # ds2-ebl.py's reader takes single-byte names; this binder's are UTF-16, which ds2-ffx.py reads.
    ffx = load_module("ds2ffx", "ds2-ffx.py")
    return [(name.replace("\\", "/").rsplit("/", 1)[-1], blob) for _id, name, blob in ffx.members(data)]


def chunks(nmb: bytes) -> list[tuple[int, int, bytes, bytes]]:
    """`[(offset, type, id20, payload)]` over the network file's top-level chunks."""
    out, at = [], 0
    while at + CHUNK_HEADER <= len(nmb):
        tag, ver, kind = struct.unpack_from("<III", nmb, at)
        if (tag, ver) != (0x18, 0x0A):
            raise SystemExit(f"chunk at {at:#x}: header {tag:#x} {ver:#x}, expected 0x18 0xa")
        ident = nmb[at + 0xC : at + 0x20]
        size = struct.unpack_from("<Q", nmb, at + 0x20)[0]
        start = at + CHUNK_HEADER
        out.append((at, kind, ident, nmb[start : start + size]))
        at = (start + size + 3) & ~3
        while nmb[at : at + 4] == b"\xcd\xcd\xcd\xcd":  # MSVC fill between chunks
            at += 4
    if at != len(nmb):
        raise SystemExit(f"chunk walk stopped at {at:#x} of {len(nmb):#x}")
    return out


def duration_track(payload: bytes) -> dict:
    """A type-4 chunk: a Morpheme duration event track."""
    count, kind, name_at, _z, user, channel, events_at = struct.unpack_from("<IIIIIII", payload, 0)
    end = payload.index(b"\0", name_at)
    events = [struct.unpack_from("<ffI", payload, events_at + 12 * i) for i in range(count)]
    return {"name": payload[name_at:end].decode("latin-1"), "kind": kind, "user": user,
            "channel": channel, "events": events}


def duration_tracks(nmb: bytes) -> dict[int, dict]:
    return {struct.unpack_from("<I", ident)[0]: duration_track(payload)
            for _at, kind, ident, payload in chunks(nmb) if kind == DURATION_TRACK}


def track_sets(nmb: bytes) -> list[tuple[int, list[int], list[int], list[int]]]:
    """`[(offset, discrete ids, curve ids, duration ids)]` for every source event-track-set attrib.

    Layout, offsets relative to the attrib start (8 bytes before the marker): +0x10 u32 count,
    +0x18 u64 ids offset, +0x20 u64 sizes offset, repeated at +0x28 and +0x40 for curve and
    duration tracks. Ids are 8-byte slots holding the first u32 of the track chunk's id; the sizes
    list holds each track chunk's payload size (checked by `sets`: an id that names no chunk prints
    as `?`).
    """
    out, at = [], 0
    while (at := nmb.find(SET_MARK, at)) >= 0:
        base = at - 8
        lists = []
        for head in (0x10, 0x28, 0x40):
            count = struct.unpack_from("<I", nmb, base + head)[0]
            ids_at = struct.unpack_from("<Q", nmb, base + head + 8)[0]
            lists.append([struct.unpack_from("<I", nmb, base + ids_at + 8 * i)[0] for i in range(count)])
        out.append((base, *lists))
        at += len(SET_MARK)
    return out


def node_name(nmb: bytes, node: int) -> str:
    n, _length, ids_at, offs_at, data_at = struct.unpack_from("<IIQQQ", nmb, NAMES_HEADER)
    for i in range(n):
        if struct.unpack_from("<I", nmb, NAMES_HEADER + ids_at + 4 * i)[0] == node:
            off = struct.unpack_from("<I", nmb, NAMES_HEADER + offs_at + 4 * i)[0]
            at = NAMES_HEADER + data_at + off
            return nmb[at : nmb.index(b"\0", at)].decode("latin-1")
    return "?"


def nodes(nmb: bytes) -> dict[int, dict]:
    """Node definitions found by shape: `u32 type, u16 0x0100, u16 parent, u16 id, u16 children,
    u16 input CPs`, the children list (u16 each) at header + `u64 [+0x20]` and the input CP list
    (u16 node, u16 pin each) at `[+0x28]`. A heuristic scan: it finds the node types whose lists sit
    at those offsets (852 headers, not the network's ~10k nodes), which is enough to say what a
    control parameter feeds, not to walk the tree.
    """
    count = struct.unpack_from("<I", nmb, NAMES_HEADER)[0]
    out: dict[int, dict] = {}
    for p in range(0, len(nmb) - 0x30, 4):
        if nmb[p + 4 : p + 6] != b"\x00\x01" or nmb[p + 2 : p + 4] != b"\0\0":
            continue
        kind, _f, parent, nid, n_child, n_cp = struct.unpack_from("<IHHHHH", nmb, p)
        if not (0 < kind < 0x400 and nid < count and parent < count and n_child < 64 and n_cp < 64):
            continue
        child_at, cp_at = struct.unpack_from("<QQ", nmb, p + 0x20)
        if not (0x40 <= child_at < 0x1000 and 0x40 <= cp_at < 0x1000):
            continue
        children = list(struct.unpack_from(f"<{n_child}H", nmb, p + child_at))
        cps = [struct.unpack_from("<HH", nmb, p + cp_at + 4 * i) for i in range(n_cp)]
        out.setdefault(nid, {"at": p, "type": kind, "parent": parent, "children": children, "cps": cps})
    return out


def nsa_lengths(files: list[tuple[str, bytes]]) -> dict[int, float]:
    """TAE animation id -> clip seconds, from each nsa's name and header."""
    out = {}
    for name, blob in files:
        m = NSA_NAME.match(name)
        if m:
            out[int(m[1]) * 1_000_000 + int(m[2]) * 10_000 + int(m[3])] = struct.unpack_from("<f", blob, NSA_LENGTH)[0]
    return out


def speed_windows(files: list[tuple[str, bytes]]) -> tuple[dict, list[str]]:
    """`{anim: {"length": s, "windows": [[action, start s, end s], ...]}}` for every animation whose
    event-track set has an AttackSpeedControl track, and a list of problems (no clip length,
    sets of one animation that disagree, boundaries off the 1/30 s grid)."""
    nmb = next(blob for name, blob in files if name.endswith(".nmb"))
    lengths = nsa_lengths(files)
    tracks = duration_tracks(nmb)
    out: dict[int, dict] = {}
    problems: list[str] = []
    worst, off_grid, boundaries = 0.0, [], 0
    for base, _disc, _curve, dur in track_sets(nmb):
        named = [tracks[i] for i in dur if i in tracks]
        anims = {e[2] for t in named if t["name"] == "TimeAct" for e in t["events"]}
        speed = sorted((u, s, s + d) for t in named if t["name"] == "AttackSpeedControl" for s, d, u in t["events"])
        if not speed:
            continue
        if len(anims) != 1:
            problems.append(f"set {base:#x}: TimeAct ids {sorted(anims)}, skipped")
            continue
        anim = anims.pop()
        length = lengths.get(anim)
        if length is None:
            problems.append(f"set {base:#x}: anim {anim} has no nsa")
            continue
        windows = [[u, round(s * length, 4), round(e * length, 4)] for u, s, e in speed]
        for _u, s, e in windows:
            for x in (s, e):
                miss = abs(x * 30 - round(x * 30))
                worst, boundaries = max(worst, miss), boundaries + 1
                if miss > 0.05:
                    off_grid.append(anim)
        prior = out.get(anim)
        if prior and prior["windows"] != windows:
            problems.append(f"anim {anim}: sets disagree, {prior['windows']} vs {windows}")
            continue
        out[anim] = {"length": round(length, 4), "windows": windows}
    problems.append(f"{boundaries} 150/151 boundaries; {len(off_grid)} more than 0.05 frames off the 1/30 s grid "
                    f"(worst {worst:.3f} frames), in {sorted(set(off_grid))[:12]}")
    return out, problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p_win = sub.add_parser("windows", help="the 150/151 windows of every attack animation, in seconds")
    p_win.add_argument("--out", type=Path, default=DEFAULT_OUT)
    p_win.add_argument("--anim", type=int, action="append", help="print these animations instead of writing")
    sub.add_parser("list")
    p_dump = sub.add_parser("dump")
    p_dump.add_argument("match")
    p_dump.add_argument("--out", type=Path, required=True)
    for name in ("chunks", "sets"):
        sub.add_parser(name).add_argument("nmb", type=Path)
    p_str = sub.add_parser("strtab", help="node name -> node id, or ids (comma-separated) -> names")
    p_str.add_argument("nmb", type=Path)
    p_str.add_argument("name")
    p_nodes = sub.add_parser("nodes", help="node defs whose input CPs include the given node ids")
    p_nodes.add_argument("nmb", type=Path)
    p_nodes.add_argument("cp_ids", help="comma-separated node ids, or \"all\"")
    args = parser.parse_args()

    if args.command == "strtab":
        nmb = args.nmb.read_bytes()
        if re.fullmatch(r"(0x[0-9a-fA-F]+|\d+)(,(0x[0-9a-fA-F]+|\d+))*", args.name):
            for nid in (int(x, 0) for x in args.name.split(",")):
                print(f"{nid} ({nid:#x}): {node_name(nmb, nid)!r}")
            return 0
        n, _length, ids_at, offs_at, data_at = struct.unpack_from("<IIQQQ", nmb, NAMES_HEADER)
        for i in range(n):
            off = struct.unpack_from("<I", nmb, NAMES_HEADER + offs_at + 4 * i)[0]
            at = NAMES_HEADER + data_at + off
            if nmb[at : nmb.index(b"\0", at)].decode("latin-1") == args.name:
                nid = struct.unpack_from("<I", nmb, NAMES_HEADER + ids_at + 4 * i)[0]
                print(f"{args.name}: node {nid} ({nid:#x})")
        return 0
    if args.command == "nodes":
        nmb = args.nmb.read_bytes()
        found = nodes(nmb)
        want = None if args.cp_ids == "all" else {int(x, 0) for x in args.cp_ids.split(",")}
        for nid, d in sorted(found.items()):
            if want is None or want & {c for c, _ in d["cps"]}:
                cps = [(hex(c), pin, node_name(nmb, c)) for c, pin in d["cps"]]
                print(f"node {nid:#06x} type {d['type']:3d} parent {d['parent']:#06x} at {d['at']:#x} "
                      f"children {[hex(c) for c in d['children']]} cps {cps}")
        print(f"{len(found)} node headers", file=sys.stderr)
        return 0
    if args.command == "sets":
        nmb = args.nmb.read_bytes()
        tracks = duration_tracks(nmb)
        for base, disc, curve, dur in track_sets(nmb):
            named = [tracks.get(i, {"name": f"?{i:08x}", "events": []}) for i in dur]
            ta = [e[2] for t in named if t["name"] == "TimeAct" for e in t["events"]]
            asc = [f"{s:.4f}+{d:.4f}:{u}" for t in named if t["name"] == "AttackSpeedControl" for s, d, u in t["events"]]
            print(f"{base:#09x} d{len(disc)} c{len(curve)} u{len(dur)} tae {ta} speed {asc}")
        return 0
    if args.command == "chunks":
        for at, kind, ident, payload in chunks(args.nmb.read_bytes()):
            print(f"{at:#09x} type {kind:3d} size {len(payload):7d} id {ident.hex()} {payload[:16].hex()}")
            if kind == DURATION_TRACK:
                t = duration_track(payload)
                ev = " ".join(f"[{s:.4f}+{d:.4f} u{u}]" for s, d, u in t["events"])
                print(f"    track {ident[:4].hex()} {t['name']!r} kind {t['kind']} user {t['user']} ch {t['channel']} {ev}")
        return 0

    files = members()
    if args.command == "windows":
        table, problems = speed_windows(files)
        if args.anim:
            for anim in args.anim:
                print(anim, json.dumps(table.get(anim)))
        else:
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps({str(k): v for k, v in sorted(table.items())}, indent=1) + "\n")
            print(f"wrote {args.out}: {len(table)} animations", file=sys.stderr)
        for p in problems:
            print(p, file=sys.stderr)
        return 0
    if args.command == "list":
        for name, blob in files:
            print(f"{len(blob):9d} {blob[:4]!r:12} {name}")
        return 0
    args.out.mkdir(parents=True, exist_ok=True)
    for name, blob in files:
        if args.match in name:
            target = args.out / name
            target.write_bytes(blob)
            print(f"wrote {target} ({len(blob)} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
