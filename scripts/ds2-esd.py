#!/usr/bin/env python3
"""Decode a DARK SOULS II `fsSL` EzState script (`ezstate/ai%06d.esd`) into readable text.

    python3 scripts/ds2-esd.py ai810011.esd                 # every state machine, state, transition
    python3 scripts/ds2-esd.py ai810011.esd --stats         # which command / function ids it uses
    python3 scripts/ds2-esd.py ai810011.esd --raw           # also print expression bytes

Extract the file first with `scripts/ds2-ebl.py extract /ezstate/ai810011.esd --out DIR`.

LAYOUT (64-bit "fsSL" variant, every offset in the body relative to the end of the 0x6C header):

  header   "fsSL", 1, 2, 2, 0x54, dataSize, 6, 0x48, 1, 0x20, machineCount, 0x48, stateCount,
           0x38, conditionCount, 0x18, commandCount, 0x10, argCount, condOffsetsOff,
           condOffsetsCount, nameOff, nameLen, unkOff, 0, unkOff, 0
  body+0   1, magic[4], 0, machinesOff(q), machineCount(q), nameOff(q), nameLen(q), -1(q), -1(q)
  machine  id(q), statesOff(q), stateCount(q), statesOff(q)                         0x20 bytes
  state    id, condOffsetsOff, condCount, entryOff, entryCount, exitOff, exitCount,
           whileOff, whileCount  (all q)                                            0x48 bytes
  cond     nextStateOff, passCmdsOff, passCmdCount, subCondOffsetsOff, subCondCount,
           exprOff, exprLen  (all q)                                                0x38 bytes
  command  bank(i32), id(i32), argsOff(q), argCount(q)                              0x18 bytes
  arg      exprOff(q), exprLen(q)                                                   0x10 bytes

Every one of those sizes is ASSERTED from the file's own header, which names them, so a file of a
different layout fails loudly instead of decoding to noise.

EXPRESSIONS are a postfix byte code run through a handler table at 0x1415B2920 in
`darksoulsii-deobf.bin` (8-byte entry N = handler for opcode N, 0x00-0xBA; entry 0xA1 is NULL,
the terminator). Every meaning below was read from that handler's disassembly. The public table
for the 64-bit format is off by one for 0x8C-0x97 against this executable. Binary handlers pop the
RIGHT operand first; `[state+0x480]` is the value-stack pointer, 16 bytes per value
(payload, then type: 1=f32 2=i32 3=string).

  0x00-0x7F  push (byte - 0x40)            0x80 push f32   0x81 push f64   0x82 push i32
  0x84-0x8B  call fn with 0..7 args: the function id is pushed first, then the args
  0x8C +   0x8E -   0x8F *   0x90 /        (addsd / subsd / mulsd / divsd)
  0x91 <   0x92 >   0x93 <=  0x94 >=       (comisd + seta/setae, operand order checked)
  0x95 ==  0x96 !=  0x98 &&  0x99 ||  0x9A !      0x8D, 0x97: unary, unused by the AI files
  0xA2, 0xA6 no-op (handler is `ret 0`)    0xA5 push UTF-16 string
  0xA7-0xAE  store top in register 0-7 (value stays)   0xAF-0xB6 load register 0-7
  0xB7       pop; if false, the whole expression is false (`ABORT_IF_FALSE`)
  0xB8       pop n; push the running machine's argument n (`arg[n]`, from `[env+0x90]`)
  0xB9       push the return value of the sub-machine this state called (`CALL_RESULT`)
  0xBA       push 0x7FFFFFFF (`RUNNING`), so `b9 ba 96` = `CALL_RESULT != RUNNING`, i.e.
             "the sub-machine called by this state has returned"
  0xA1       end

The decoder checks that every expression consumes exactly its stated length, ends in 0xA1 and
leaves exactly one value, and reports anything else.
"""

from __future__ import annotations

import argparse
import collections
import struct
import sys
from pathlib import Path

HEADER = 0x6C
BINOPS = {
    0x8C: "+", 0x8E: "-", 0x8F: "*", 0x90: "/", 0x91: "<", 0x92: ">", 0x93: "<=", 0x94: ">=",
    0x95: "==", 0x96: "!=", 0x98: "&&", 0x99: "||",
}


class Esd:
    def __init__(self, data: bytes) -> None:
        if data[:4] != b"fsSL":
            raise SystemExit(f"not a 64-bit fsSL ESD (magic {data[:4]!r})")
        h = struct.unpack_from("<4x27i", data, 0)
        (one, ds_a, ds_b, hsz, self.data_size, six, h48, one2, h20, self.machine_count,
         state_sz, self.state_count, cond_sz, self.cond_count, cmd_sz, self.cmd_count,
         arg_sz, self.arg_count, *_rest) = h
        expect = dict(one=(one, 1), hsz=(hsz, 0x54), six=(six, 6), h48=(h48, 0x48),
                      h20=(h20, 0x20), state=(state_sz, 0x48), cond=(cond_sz, 0x38),
                      cmd=(cmd_sz, 0x18), arg=(arg_sz, 0x10))
        for name, (got, want) in expect.items():
            if got != want:
                raise SystemExit(f"header field {name} = {got:#x}, expected {want:#x}")
        self.version = (ds_a, ds_b)
        self.d = data
        b = HEADER
        _one, *self.magic = struct.unpack_from("<i4i", data, b)
        mach_off, mach_n, name_off, name_len = struct.unpack_from("<4q", data, b + 0x18)
        if mach_n != self.machine_count:
            raise SystemExit(f"body machine count {mach_n} != header {self.machine_count}")
        self.name = data[b + name_off : b + name_off + 2 * name_len].decode("utf-16le", "replace")
        self.machines = []
        for i in range(mach_n):
            mid, soff, scount, soff2 = struct.unpack_from("<4q", data, b + mach_off + i * 0x20)
            states = [self.state(soff + k * 0x48) for k in range(scount)]
            self.machines.append((mid, soff, states))
        self.state_by_off = {}
        for mid, soff, states in self.machines:
            for k, st in enumerate(states):
                self.state_by_off[soff + k * 0x48] = (mid, st["id"])

    def q(self, off: int, n: int) -> tuple:
        return struct.unpack_from(f"<{n}q", self.d, HEADER + off)

    def state(self, off: int) -> dict:
        sid, co, cn, eo, en, xo, xn, wo, wn = self.q(off, 9)
        return dict(id=sid, conds=self.cond_list(co, cn), entry=self.cmds(eo, en),
                    exit=self.cmds(xo, xn), while_=self.cmds(wo, wn))

    def cond_list(self, off: int, n: int) -> list:
        return [self.cond(self.q(off + 8 * i, 1)[0]) for i in range(n)] if n else []

    def cond(self, off: int) -> dict:
        nxt, po, pn, so, sn, eo, el = self.q(off, 7)
        return dict(next=nxt, passcmds=self.cmds(po, pn), sub=self.cond_list(so, sn),
                    expr=self.expr(eo, el))

    def cmds(self, off: int, n: int) -> list:
        out = []
        for i in range(n):
            bank, cid, ao, an = struct.unpack_from("<iiqq", self.d, HEADER + off + i * 0x18)
            args = [self.expr(*self.q(ao + k * 0x10, 2)) for k in range(an)]
            out.append((bank, cid, args))
        return out

    def expr(self, off: int, length: int) -> dict:
        raw = self.d[HEADER + off : HEADER + off + length]
        return dict(raw=raw, **decode_expr(raw))


def decode_expr(raw: bytes) -> dict:
    st: list[str] = []
    calls: list[tuple[int, int]] = []
    bad = []
    i = 0
    ended = False
    while i < len(raw):
        b = raw[i]
        i += 1
        if b <= 0x7F:
            st.append(str(b - 0x40))
        elif b == 0x80:
            st.append(repr(round(struct.unpack_from("<f", raw, i)[0], 4))); i += 4
        elif b == 0x81:
            st.append(repr(struct.unpack_from("<d", raw, i)[0])); i += 8
        elif b == 0x82:
            st.append(str(struct.unpack_from("<i", raw, i)[0])); i += 4
        elif 0x84 <= b <= 0x8B:
            argc = b - 0x84
            args = [st.pop() if st else "?" for _ in range(argc)][::-1]
            fn = st.pop() if st else "?"
            try:
                calls.append((int(fn), argc))
            except ValueError:
                pass
            st.append(f"f{fn}({', '.join(args)})")
        elif b in (0x8D, 0x97):
            st.append(f"op{b:02X}({st.pop() if st else '?'})")
        elif b in BINOPS:
            r = st.pop() if st else "?"
            l = st.pop() if st else "?"
            st.append(f"({l} {BINOPS[b]} {r})")
        elif b == 0x9A:
            st.append(f"!({st.pop() if st else '?'})")
        elif b == 0xA5:
            end = i
            while raw[end : end + 2] != b"\0\0":
                end += 2
            st.append(repr(raw[i:end].decode("utf-16le", "replace"))); i = end + 2
        elif 0xA7 <= b <= 0xAE:
            st.append(f"r{b - 0xA7}:=({st.pop() if st else '?'})")
        elif 0xAF <= b <= 0xB6:
            st.append(f"r{b - 0xAF}")
        elif b == 0xB7:
            st.append(f"ABORT_IF_FALSE({st.pop() if st else '?'})")
        elif b == 0xB8:
            st.append(f"arg[{st.pop() if st else '?'}]")
        elif b == 0xB9:
            st.append("CALL_RESULT")
        elif b == 0xBA:
            st.append("RUNNING")
        elif b in (0xA2, 0xA6):
            pass  # handler is `ret 0` (0x14098F900 / 0x14098EE00)
        elif b == 0xA1:
            ended = True
            if i != len(raw):
                bad.append(f"A1 at {i - 1} of {len(raw)}")
            break
        else:
            bad.append(f"UNKNOWN {b:#04x} at {i - 1}")
            st.append(f"?{b:02X}")
    if not ended:
        bad.append("no A1 terminator")
    if len(st) != 1:
        bad.append(f"stack depth {len(st)} at end")
    return dict(text=" ; ".join(st), calls=calls, bad=bad)


def fmt_cmd(c) -> str:
    bank, cid, args = c
    return f"cmd[{bank}:{cid}]({', '.join(a['text'] for a in args)})"


def dump(esd: Esd, raw: bool) -> None:
    print(f"# {esd.name}  version={esd.version} machines={esd.machine_count} "
          f"states={esd.state_count} conds={esd.cond_count} cmds={esd.cmd_count}")
    for mid, _soff, states in esd.machines:
        label = f"{mid}" if mid < 0x7F000000 else f"{mid:#x} (sub {0x7FFFFFFF - mid})"
        print(f"\n== machine {label}: {len(states)} states")
        for st in states:
            print(f"  -- state {st['id']}")
            for what in ("entry", "while_", "exit"):
                for c in st[what]:
                    print(f"     {what.rstrip('_'):5} {fmt_cmd(c)}")
            for c in st["conds"]:
                print_cond(esd, c, 5, raw)


def print_cond(esd: Esd, c: dict, ind: int, raw: bool) -> None:
    tgt = esd.state_by_off.get(c["next"], None) if c["next"] != -1 else None
    to = f"-> state {tgt[1]}" if tgt else ("-> (sub-conditions)" if c["next"] == -1 else
                                            f"-> ?{c['next']:#x}")
    bad = f"   !! {c['expr']['bad']}" if c["expr"]["bad"] else ""
    print(" " * ind + f"if {c['expr']['text']} {to}{bad}")
    if raw:
        print(" " * ind + "   bytes " + c["expr"]["raw"].hex(" "))
    for p in c["passcmds"]:
        print(" " * ind + f"   pass {fmt_cmd(p)}")
    for s in c["sub"]:
        print_cond(esd, s, ind + 3, raw)


def stats(esd: Esd) -> None:
    cmds = collections.Counter()
    fns = collections.Counter()
    bad = 0

    def walk_expr(e):
        nonlocal bad
        for fn, argc in e["calls"]:
            fns[(fn, argc)] += 1
        bad += bool(e["bad"])

    def walk_cond(c):
        walk_expr(c["expr"])
        for p in c["passcmds"]:
            walk_cmd(p)
        for s in c["sub"]:
            walk_cond(s)

    def walk_cmd(c):
        cmds[(c[0], c[1], len(c[2]))] += 1
        for a in c[2]:
            walk_expr(a)

    for _mid, _o, states in esd.machines:
        for st in states:
            for what in ("entry", "while_", "exit"):
                for c in st[what]:
                    walk_cmd(c)
            for c in st["conds"]:
                walk_cond(c)
    print(f"# {esd.name}: expressions with decode problems: {bad}")
    print("commands (bank:id argc -> uses):")
    for (bank, cid, argc), n in sorted(cmds.items()):
        print(f"  {bank}:{cid} argc={argc}  x{n}")
    print("functions (id argc -> uses):")
    for (fn, argc), n in sorted(fns.items()):
        print(f"  f{fn} argc={argc}  x{n}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("esd", type=Path)
    ap.add_argument("--stats", action="store_true")
    ap.add_argument("--raw", action="store_true")
    a = ap.parse_args()
    esd = Esd(a.esd.read_bytes())
    if a.stats:
        stats(esd)
    else:
        dump(esd, a.raw)
    return 0


if __name__ == "__main__":
    sys.exit(main())
