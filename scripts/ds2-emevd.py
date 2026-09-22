#!/usr/bin/env python3
"""Read DARK SOULS II's `.emevd` event scripts, including the eleven `SpEffect*.emevd`
members that live inside `enc_regulation.bnd.dcx`.

    python3 scripts/ds2-regulation.py --file <enc_regulation.bnd.dcx> extract --out /tmp/reg
    python3 scripts/ds2-emevd.py info /tmp/reg/SpEffectActiveItem.emevd
    python3 scripts/ds2-emevd.py events /tmp/reg/SpEffectActiveItem.emevd --id 60450000
    python3 scripts/ds2-emevd.py banks /tmp/reg/SpEffect*.emevd
    python3 scripts/ds2-emevd.py find /tmp/reg/SpEffect*.emevd --bank 100120

# Provenance of the layout

Nothing here is copied from a paramdef or a community table; every field below was measured
against `SpEffectActiveItem.emevd` (22840 bytes, 173 events, 319 instructions) and each
measurement has a check that fails loudly if it is wrong:

    magic           b"EVD\\0"
    +0x04 u32       0x0000FF00 -- the 64-bit DARK SOULS II flavour
    +0x0c u32       file size (checked against the real length)
    +0x10 u64       event count          173
    +0x18 u64       event table offset   0x90  == the header length
    +0x20 u64       instruction count    319
    +0x28 u64       instruction offset   0x2100 == 0x90 + 173 * 48   <- fixes event stride 48
    +0x30 u64       unk count / +0x38 offset
    +0x40 u64       event-layer count / +0x48 offset
    +0x50 u64       event-parameter count / +0x58 offset
    +0x60 u64       linked-file count / +0x68 offset
    +0x70 u64       arg-data length 0x1010 / +0x78 offset 0x48E0
                    0x2100 + 319 * 32 == 0x48E0                      <- fixes instruction stride 32
                    0x48E0 + 0x1010   == 0x58F0 == the linked-file table
    +0x80 u64       string-data length 0x40 / +0x88 offset 0x58F8
                    0x58F8 + 0x40 == the file size

    event      +0x00 i64 id   +0x08 i64 instruction count
               +0x10 i64 instruction offset (BYTES into the instruction table, not an index)
               +0x18 i64 parameter count    +0x20 i64 parameter offset (-1 when there are none)
               +0x28 u32 rest behaviour     +0x2c u32 padding

    instruction  +0x00 u32 bank   +0x04 u32 index
                 +0x08 i64 arg byte length  +0x10 i64 arg offset (into the arg data block)
                 +0x18 i64 event-layer offset (-1 when there is none)

The instruction offset being a byte offset rather than an index is the field that is easy to get
wrong: the three consecutive events 60430000 / 60450000 / 60470000 carry 0x1760 / 0x1780 / 0x17A0,
which reads as a plausible index sequence right up until you notice the stride is 32, and the arg
offsets (0x8DC + 0x10 == 0x8EC, 0x8EC + 0x0C == 0x8F8) only chain if you resolve the instructions
by byte offset. `--check` re-runs that chain over every event in the file.

Argument types are NOT stored in the file -- the game knows each bank's signature statically. So
every 4 bytes of an instruction's arguments is printed three ways (i32, f32, hex) and you decide.
"""

from __future__ import annotations

import argparse
import glob
import struct
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path

HEADER_LEN = 0x90
EVENT_STRIDE = 48
INSTR_STRIDE = 32

# The "there is none" sentinel is written into the LOW dword only, so a 64-bit read of it comes
# back as 0xFFFFFFFF rather than -1. Both spellings appear; treat either as absent.
NO_OFFSET = (-1, 0xFFFFFFFF)


@dataclass
class Instruction:
    bank: int
    index: int
    arg_len: int
    arg_off: int
    layer_off: int
    args: bytes

    def words(self) -> list[tuple[int, float, int]]:
        out = []
        for i in range(0, len(self.args) - 3, 4):
            (raw,) = struct.unpack_from("<I", self.args, i)
            (sig,) = struct.unpack_from("<i", self.args, i)
            (flt,) = struct.unpack_from("<f", self.args, i)
            out.append((sig, flt, raw))
        return out

    def render_args(self) -> str:
        bits = []
        for sig, flt, raw in self.words():
            if -1e9 < flt < 1e9 and abs(flt) > 1e-6 and raw not in (0, 0xFFFFFFFF):
                bits.append(f"{sig}|{flt:g}f")
            else:
                bits.append(str(sig))
        tail = len(self.args) % 4
        if tail:
            bits.append("+" + self.args[-tail:].hex())
        return "[" + ", ".join(bits) + "]"


@dataclass
class Event:
    id: int
    instr_count: int
    instr_off: int
    param_count: int
    param_off: int
    rest: int
    instructions: list[Instruction]


class Emevd:
    def __init__(self, path: Path):
        self.path = path
        self.data = path.read_bytes()
        d = self.data
        if d[:4] != b"EVD\0":
            raise SystemExit(f"{path}: not an emevd (magic {d[:4]!r})")
        self.version, self.unk08, self.file_size = struct.unpack_from("<III", d, 4)
        if self.version != 0x0000FF00:
            raise SystemExit(
                f"{path}: version 0x{self.version:08x}, this reader only knows the "
                f"64-bit DARK SOULS II flavour 0x0000ff00"
            )
        if self.file_size != len(d):
            raise SystemExit(f"{path}: header says {self.file_size} bytes, file is {len(d)}")

        fields = struct.unpack_from("<10q", d, 0x10)
        (
            self.event_count,
            self.event_off,
            self.instr_count,
            self.instr_off,
            self.unk_count,
            self.unk_off,
            self.layer_count,
            self.layer_off,
            self.param_count,
            self.param_off,
        ) = fields
        (
            self.linked_count,
            self.linked_off,
            self.arg_len,
            self.arg_off,
            self.string_len,
            self.string_off,
        ) = struct.unpack_from("<6q", d, 0x60)

        if self.event_off != HEADER_LEN:
            raise SystemExit(f"{path}: event table at 0x{self.event_off:x}, expected 0x{HEADER_LEN:x}")
        want = self.event_off + self.event_count * EVENT_STRIDE
        if want != self.instr_off:
            raise SystemExit(
                f"{path}: {self.event_count} events of {EVENT_STRIDE} bytes end at 0x{want:x}, "
                f"but the instruction table starts at 0x{self.instr_off:x}"
            )
        want = self.instr_off + self.instr_count * INSTR_STRIDE
        if want != self.arg_off:
            raise SystemExit(
                f"{path}: {self.instr_count} instructions of {INSTR_STRIDE} bytes end at "
                f"0x{want:x}, but the arg data starts at 0x{self.arg_off:x}"
            )
        if self.string_off + self.string_len != len(d):
            raise SystemExit(f"{path}: string block does not reach the end of the file")

        self.events = [self._event(i) for i in range(self.event_count)]

    def _instruction(self, byte_off: int) -> Instruction:
        base = self.instr_off + byte_off
        bank, index = struct.unpack_from("<II", self.data, base)
        arg_len, arg_off, layer_off = struct.unpack_from("<3q", self.data, base + 8)
        if arg_off in NO_OFFSET:
            arg_off = -1
            args = b""
        else:
            args = self.data[self.arg_off + arg_off : self.arg_off + arg_off + arg_len]
        return Instruction(bank, index, arg_len, arg_off, layer_off, args)

    def _event(self, i: int) -> Event:
        base = self.event_off + i * EVENT_STRIDE
        eid, n, off, pc, po = struct.unpack_from("<5q", self.data, base)
        (rest,) = struct.unpack_from("<I", self.data, base + 0x28)
        ins = [self._instruction(off + k * INSTR_STRIDE) for k in range(n)]
        return Event(eid, n, off, pc, po, rest, ins)

    def check(self) -> list[str]:
        """The arg blocks of consecutive instructions must tile the arg region exactly."""
        problems = []
        seen = sorted(
            {(e.instr_off + k * INSTR_STRIDE) for e in self.events for k in range(e.instr_count)}
        )
        if len(seen) != self.instr_count:
            problems.append(
                f"{len(seen)} of {self.instr_count} instructions are reachable from an event"
            )
        cursor = 0
        for off in seen:
            ins = self._instruction(off)
            if ins.arg_off == -1:
                # An instruction that takes no arguments stores -1, not the running cursor.
                if ins.arg_len:
                    problems.append(f"instruction at +0x{off:x} has no arg offset but {ins.arg_len} bytes")
                continue
            if ins.arg_off != cursor:
                problems.append(
                    f"instruction at +0x{off:x} takes args at 0x{ins.arg_off:x}, "
                    f"the previous one ended at 0x{cursor:x}"
                )
            cursor = ins.arg_off + ins.arg_len
        if cursor > self.arg_len:
            problems.append(f"args run to 0x{cursor:x}, past the 0x{self.arg_len:x}-byte block")
        return problems


def load(patterns: list[str]) -> list[Emevd]:
    paths: list[Path] = []
    for pat in patterns:
        hits = sorted(glob.glob(pat))
        if not hits:
            raise SystemExit(f"no file matches {pat}")
        paths += [Path(h) for h in hits]
    return [Emevd(p) for p in paths]


def cmd_info(args) -> None:
    for ev in load(args.files):
        print(ev.path)
        print(f"  version        0x{ev.version:08x}   unk+0x08 {ev.unk08}")
        print(f"  size           {ev.file_size}")
        print(f"  events         {ev.event_count} at 0x{ev.event_off:x}")
        print(f"  instructions   {ev.instr_count} at 0x{ev.instr_off:x}")
        print(f"  arg data       0x{ev.arg_len:x} at 0x{ev.arg_off:x}")
        print(f"  event params   {ev.param_count} at 0x{ev.param_off:x}")
        print(f"  event layers   {ev.layer_count} at 0x{ev.layer_off:x}")
        print(f"  linked files   {ev.linked_count} at 0x{ev.linked_off:x}")
        print(f"  strings        0x{ev.string_len:x} at 0x{ev.string_off:x}")
        ids = [e.id for e in ev.events]
        print(f"  event ids      {min(ids)} .. {max(ids)}")
        bad = ev.check()
        print("  check          " + ("ok" if not bad else f"{len(bad)} PROBLEMS"))
        for b in bad:
            print("    " + b)


def cmd_events(args) -> None:
    wanted = set(args.ids or [])
    for ev in load(args.files):
        shown = 0
        printed_header = False
        for e in ev.events:
            if wanted and e.id not in wanted:
                continue
            if args.bank is not None and not any(i.bank == args.bank for i in e.instructions):
                continue
            if not printed_header:
                print(ev.path)
                printed_header = True
            print(f"  event {e.id}   instructions={e.instr_count} rest={e.rest}")
            for i in e.instructions:
                print(f"    {i.bank}[{i.index}] {i.render_args()}")
            shown += 1
            if args.limit and shown >= args.limit:
                print(f"  ... stopped at --limit {args.limit}")
                break


def cmd_banks(args) -> None:
    counts: Counter = Counter()
    lengths = defaultdict(set)
    for ev in load(args.files):
        for e in ev.events:
            for i in e.instructions:
                counts[(i.bank, i.index)] += 1
                lengths[(i.bank, i.index)].add(i.arg_len)
    for (bank, index), n in sorted(counts.items()):
        lens = ",".join(str(x) for x in sorted(lengths[(bank, index)]))
        print(f"{bank}[{index}]  n={n}  argbytes={lens}")


def cmd_find(args) -> None:
    lo, hi = (args.arg_range or (None, None))
    for ev in load(args.files):
        for e in ev.events:
            for i in e.instructions:
                if args.bank is not None and i.bank != args.bank:
                    continue
                if args.index is not None and i.index != args.index:
                    continue
                if lo is not None:
                    if not any(lo <= w[0] <= hi for w in i.words()):
                        continue
                print(f"{ev.path.name}  event {e.id}  {i.bank}[{i.index}] {i.render_args()}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("info", help="header fields and the self-check")
    p.add_argument("files", nargs="+")
    p.set_defaults(func=cmd_info)

    p = sub.add_parser("events", help="events with their instructions and raw arguments")
    p.add_argument("files", nargs="+")
    p.add_argument("--id", dest="ids", type=int, action="append", help="only these event ids")
    p.add_argument("--bank", type=int, help="only events that call this bank")
    p.add_argument("--limit", type=int, default=0, help="0 for no limit")
    p.set_defaults(func=cmd_events)

    p = sub.add_parser("banks", help="every (bank, index) pair with its argument byte lengths")
    p.add_argument("files", nargs="+")
    p.set_defaults(func=cmd_banks)

    p = sub.add_parser("find", help="instructions by bank/index, or carrying a value in a range")
    p.add_argument("files", nargs="+")
    p.add_argument("--bank", type=int)
    p.add_argument("--index", type=int)
    p.add_argument("--arg-range", nargs=2, type=int, metavar=("LO", "HI"))
    p.set_defaults(func=cmd_find)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    sys.exit(main())
