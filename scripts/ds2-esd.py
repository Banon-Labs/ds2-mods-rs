#!/usr/bin/env python3
"""Disassemble a DARK SOULS II talk/event EzState script (`*.esd`, `fsSL` long format).

    python3 scripts/ds2-ebl.py extract /ezstate/talk_m10_04_00_00.esd --out DIR
    python3 scripts/ds2-esd.py DIR/talk_m10_04_00_00.esd                   # every state
    python3 scripts/ds2-esd.py DIR/talk_m10_04_00_00.esd --command 130455  # states calling it,
                                                                           # and what leads there

Layout, with every field width checked against the header's own declared record sizes (the
script refuses a file whose sizes differ, rather than misreading it):

  header   0x6C bytes; record sizes 0x20 group / 0x48 state / 0x38 condition / 0x18 command /
           0x10 argument; all offsets are 8-byte and relative to the data start at 0x6C
  group    id, states offset, state count, states offset again
  state    id, conditions (offset, count), entry / exit / while commands (offset, count each)
  cond.    target state offset, pass commands (offset, count), subconditions (offset, count),
           evaluator (offset, length)
  command  bank i32, id i32, arguments (offset, count); each argument is an expression

Expressions are EzState bytecode; `expr()` names the opcodes it knows and prints any other byte
as `op_XX` so an unknown one is visible instead of silently skipped.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

DATA_START = 0x6C
SIZES = {"group": 0x20, "state": 0x48, "condition": 0x38, "command": 0x18, "argument": 0x10}

BINARY = {
    0x8C: "+", 0x8E: "-", 0x8F: "*", 0x90: "/", 0x91: "<=", 0x92: ">=", 0x93: "<", 0x94: ">",
    0x95: "==", 0x96: "!=", 0x98: "&&", 0x99: "||",
}


def expr(code: bytes) -> str:
    """Bytecode to an infix string."""
    stack: list[str] = []
    i = 0
    while i < len(code):
        b = code[i]
        i += 1
        if b <= 0x7F:
            stack.append(str(b - 64))
        elif b == 0x80:
            stack.append(repr(struct.unpack_from("<f", code, i)[0]))
            i += 4
        elif b == 0x81:
            stack.append(repr(struct.unpack_from("<d", code, i)[0]))
            i += 8
        elif b == 0x82:
            stack.append(str(struct.unpack_from("<i", code, i)[0]))
            i += 4
        elif 0x84 <= b <= 0x8A:
            n = b - 0x84
            args = stack[len(stack) - n :] if n else []
            del stack[len(stack) - n :]
            fn = stack.pop()
            stack.append(f"f{fn}({', '.join(args)})")
        elif b in BINARY:
            r = stack.pop()
            left = stack.pop()
            stack.append(f"({left} {BINARY[b]} {r})")
        elif b == 0x8D:
            stack.append(f"-{stack.pop()}")
        elif b == 0x9A:
            stack.append(f"!{stack.pop()}")
        elif b == 0xA1:
            break
        elif b == 0xA5:
            end = i
            while code[end : end + 2] != b"\0\0":
                end += 2
            stack.append(repr(code[i:end].decode("utf-16-le")))
            i = end + 2
        elif 0xA7 <= b <= 0xAE:
            stack.append(f"(reg{b - 0xA7} := {stack.pop()})")
        elif 0xAF <= b <= 0xB6:
            stack.append(f"reg{b - 0xAF}")
        elif b == 0xB7:
            stack.append(f"abort_if_false({stack.pop()})")
        else:
            stack.append(f"op_{b:02X}")
    return " ; ".join(stack)


class Esd:
    def __init__(self, blob: bytes) -> None:
        if blob[:4] != b"fsSL":
            raise SystemExit(f"not an fsSL ESD: {blob[:4]!r}")
        h = struct.unpack_from("<23i", blob, 0)
        declared = {"group": h[9], "state": h[11], "condition": h[13], "command": h[15], "argument": h[17]}
        if declared != SIZES:
            raise SystemExit(f"record sizes {declared} differ from the long format {SIZES}")
        self.b = blob
        self.groups: list[tuple[int, int, int]] = []
        # The data-start block is 1, four unknown i32, a pad, then the group table offset and count.
        groups_off, group_count = struct.unpack_from("<qq", blob, DATA_START + 0x18)
        for g in range(group_count):
            gid, soff, scount, _ = struct.unpack_from("<4q", blob, DATA_START + groups_off + g * 0x20)
            self.groups.append((gid, soff, scount))

    def q(self, off: int, n: int = 1) -> tuple[int, ...]:
        return struct.unpack_from(f"<{n}q", self.b, DATA_START + off)

    def state(self, off: int) -> dict:
        sid, co, cc, eo, ec, xo, xc, wo, wc = self.q(off, 9)
        return {
            "id": sid,
            "conditions": [self.q(co + 8 * k)[0] for k in range(cc)],
            "entry": self.commands(eo, ec),
            "exit": self.commands(xo, xc),
            "while": self.commands(wo, wc),
        }

    def commands(self, off: int, count: int) -> list[str]:
        out = []
        for k in range(count):
            bank, cid = struct.unpack_from("<ii", self.b, DATA_START + off + k * 0x18)
            ao, ac = self.q(off + k * 0x18 + 8, 2)
            args = []
            for a in range(ac):
                eo, el = self.q(ao + a * 0x10, 2)
                args.append(expr(self.b[DATA_START + eo : DATA_START + eo + el]))
            out.append(f"c{bank}_{cid}({', '.join(args)})")
        return out

    def condition(self, off: int) -> dict:
        target, po, pc, so, sc, eo, el = self.q(off, 7)
        return {
            "target": target,
            "pass": self.commands(po, pc),
            "sub": [self.q(so + 8 * k)[0] for k in range(sc)],
            "eval": expr(self.b[DATA_START + eo : DATA_START + eo + el]),
        }

    def states(self):
        for gid, soff, scount in self.groups:
            for s in range(scount):
                off = soff + s * 0x48
                yield gid, off, self.state(off)


def show_condition(esd: Esd, off: int, names: dict[int, str], depth: int) -> None:
    c = esd.condition(off)
    pad = "    " * depth
    target = names.get(c["target"], "-") if c["target"] != -1 else "-"
    print(f"{pad}if {c['eval']}  -> {target}")
    for p in c["pass"]:
        print(f"{pad}    do {p}")
    for s in c["sub"]:
        show_condition(esd, s, names, depth + 1)


def show_state(esd: Esd, gid: int, st: dict, names: dict[int, str]) -> None:
    print(f"group {gid} state {st['id']}")
    for kind in ("entry", "while", "exit"):
        for c in st[kind]:
            print(f"    {kind}: {c}")
    for c in st["conditions"]:
        show_condition(esd, c, names, 1)


def scan_int(value: int) -> int:
    """Map ids are not listed in the archive (it stores hashes), so every m<AA>_<BB>_<CC>_00 in a
    generous range is hashed and the ones present are read."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("ds2ebl", Path(__file__).with_name("ds2-ebl.py"))
    ebl = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ebl)
    bhd, bdt, pem = ebl.archive_paths(ebl.GAME_DIR, "GameDataEbl")
    blob = ebl.decrypt_bhd(bhd, pem)
    header = ebl.Bhd5(blob)
    needle = struct.pack("<i", value)
    found = 0
    for kind in ("talk", "event"):
        for a in (10, 20, 30, 40, 50, 99):
            for b in range(60):
                for c in range(4):
                    found += scan_one(ebl, header, bdt, blob, needle,
                                      f"/ezstate/{kind}_m{a:02d}_{b:02d}_{c:02d}_00.esd")
    print(f"{found} talk/event scripts read")
    return 0


def scan_one(ebl, header, bdt, blob: bytes, needle: bytes, path: str) -> int:
    """1 if `path` exists in the archive (printing it when it contains `needle`), else 0."""
    entry = header.entries.get(ebl.path_hash(path))
    if entry is None:
        return 0
    size, offset, key, _bucket = entry
    data = ebl.dcx_decompress(ebl.read_entry(bdt, size, offset, key, path, blob))
    if needle in data:
        print(f"HIT {path}")
    return 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("esd", type=Path, nargs="?", help="omit with --scan-int")
    parser.add_argument("--command", type=int, help="only states calling this command id, and states leading to them")
    parser.add_argument(
        "--scan-int", type=int,
        help="read every map's /ezstate/talk_<map>.esd straight from the archive and name the ones "
        "containing this i32 (a param row id, a flag) -- nothing is written",
    )
    args = parser.parse_args()
    if args.scan_int is not None:
        return scan_int(args.scan_int)
    if args.esd is None:
        parser.error("an .esd path is required unless --scan-int is given")
    esd = Esd(args.esd.read_bytes())
    all_states = list(esd.states())
    names = {off: f"g{gid}/s{st['id']}" for gid, off, st in all_states}
    if args.command is None:
        for gid, _off, st in all_states:
            show_state(esd, gid, st, names)
        return 0
    needle = f"_{args.command}("
    hits = [(gid, off, st) for gid, off, st in all_states
            if any(needle in c for k in ("entry", "while", "exit") for c in st[k])]
    hit_offs = {off for _g, off, _s in hits}

    def leads(off: int) -> bool:
        c = esd.condition(off)
        return c["target"] in hit_offs or any(leads(s) for s in c["sub"])

    for gid, _off, st in hits:
        print("=== CALLS IT")
        show_state(esd, gid, st, names)
    for gid, _off, st in all_states:
        if any(leads(c) for c in st["conditions"]):
            print("=== LEADS TO IT")
            show_state(esd, gid, st, names)
    return 0


if __name__ == "__main__":
    sys.exit(main())
