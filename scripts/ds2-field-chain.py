#!/usr/bin/env python3
"""Find code that follows a POINTER CHAIN of struct fields, `[a+off1]` then `[that+off2]` ...

    python3 scripts/ds2-field-chain.py 0xa8 0x40            # loads of [x+0xa8], then [it+0x40]
    python3 scripts/ds2-field-chain.py 0x90 0x68 0x20       # three hops
    python3 scripts/ds2-field-chain.py 0xa8 0x40 --window 20

WHY THIS EXISTS. `ds2-field-xrefs.py` answers "who touches `+0xa8`" and gets 1647 candidates,
because every class in the game has something at `+0xa8`. The question actually being asked is
narrower: "who reads ItemUsageParam", and the game only reaches that through
`holder+0xa8 -> +0x40`. The second hop is what makes the list readable, and it cannot be had from
bytes alone -- it needs the destination register of the first load, so this decodes forward.

HOW. The first hop's candidates come from `ds2-field-xrefs.scan` (8-byte loads only). From each
one, capstone decodes forward up to `--window` instructions, follows the loaded register, and
reports a hit when a later `mov`/`movzx`/`movsxd`/`lea`/`cmp`/`test` uses `[that_reg+next_off]`.
The register is dropped when it is overwritten and the walk stops at `ret`/`jmp`/`call` (a call
clobbers the volatile registers, and following across one is guessing). A `mov other, reg` copy
is followed, since MSVC often moves a pointer to a callee-saved register first.

WHAT IT MISSES. A chain whose hops are split across a call, a spill to the stack, or a branch
join is not seen. Every hit is still a candidate: disassemble it with `scripts/ds2-disasm.py`.

needs: capstone (e.g. `uv pip install capstone` into a scratch venv)
"""
from __future__ import annotations

import argparse
import importlib.util
import sys
from pathlib import Path

import capstone
from capstone import x86

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("ds2_field_xrefs", HERE / "ds2-field-xrefs.py")
fx = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fx)

STOP = {"ret", "jmp", "call", "int3"}
USE = {"mov", "movzx", "movsxd", "movsx", "lea", "cmp", "test", "movss", "movsd", "movups", "movaps"}


def walk(md, image: bytes, start: int, reg: int, offsets: list[int], window: int) -> list[int]:
    """VAs of the instructions that completed the chain, starting just after the first hop."""
    live = {reg}
    want = 0
    hops = []
    code = image[start : start + window * 15]
    for count, ins in enumerate(md.disasm(code, fx.IMAGE_BASE + start)):
        if count == 0:
            continue  # the first hop itself
        if count > window or ins.mnemonic in STOP:
            break
        ops = ins.operands
        mem = [o for o in ops if o.type == x86.X86_OP_MEM]
        if mem and ins.mnemonic in USE and mem[0].mem.base in live and mem[0].mem.disp == offsets[want]:
            hops.append(ins.address)
            want += 1
            if want == len(offsets):
                return hops
            if ops and ops[0].type == x86.X86_OP_REG and ins.mnemonic in ("mov", "lea"):
                live = {ops[0].reg}
                continue
            return []
        # `add reg, imm` on a live pointer is a hop to an embedded subobject (a secondary base
        # class's vtable pointer, say) that never touches memory.
        if (ins.mnemonic == "add" and len(ops) == 2 and ops[0].type == x86.X86_OP_REG
                and ops[0].reg in live and ops[1].type == x86.X86_OP_IMM and ops[1].imm == offsets[want]):
            hops.append(ins.address)
            want += 1
            if want == len(offsets):
                return hops
            live = {ops[0].reg}
            continue
        # A register-to-register copy keeps the pointer alive under a new name.
        if (ins.mnemonic == "mov" and len(ops) == 2 and ops[0].type == x86.X86_OP_REG
                and ops[1].type == x86.X86_OP_REG and ops[1].reg in live):
            live.add(ops[0].reg)
            continue
        # Anything that writes a live register kills it.
        if ops and ops[0].type == x86.X86_OP_REG and ops[0].reg in live and ins.mnemonic not in ("cmp", "test"):
            live.discard(ops[0].reg)
            if not live:
                break
    return []


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("offsets", nargs="+", type=lambda s: int(s, 0))
    ap.add_argument("--window", type=int, default=12, help="instructions to follow per hop. DEFAULT: 12")
    ap.add_argument("--limit", type=int, default=80)
    a = ap.parse_args(argv)
    if len(a.offsets) < 2:
        ap.error("a chain needs at least two offsets")

    image = fx.IMAGE_PATH.read_bytes()
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.detail = True
    hits = []
    for cand in fx.scan(image, a.offsets[0], 8, False):
        if not cand["name"].startswith("mov r"):
            continue
        start = cand["va"] - fx.IMAGE_BASE
        first = next(md.disasm(image[start : start + 15], cand["va"]), None)
        if first is None or first.mnemonic != "mov" or first.operands[0].type != x86.X86_OP_REG:
            continue
        chain = walk(md, image, start, first.operands[0].reg, a.offsets[1:], a.window)
        if chain:
            hits.append((cand["va"], chain))

    print(f"chain {' -> '.join(hex(o) for o in a.offsets)}: {len(hits)} candidate(s)")
    print("EVERY LINE IS A CANDIDATE, NOT A FACT -- confirm with scripts/ds2-disasm.py.")
    for va, chain in hits[: a.limit]:
        print(f"  {va:#x}  " + "  ".join(f"{h:#x}" for h in chain))
    if len(hits) > a.limit:
        print(f"  ... {len(hits) - a.limit} more (raise --limit)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
