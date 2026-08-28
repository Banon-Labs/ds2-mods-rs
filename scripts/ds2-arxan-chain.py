#!/usr/bin/env python3
"""Walk an Arxan entry-stub chain from a redirected function entry to the real body.

    python3 scripts/ds2-arxan-chain.py 0x14014bec0

286 (or 311 -- see ds2-mods-rs-46z) of this binary's function entries are not the function.
They are a five-byte `jmp` into Arxan's second `.text` at 0x141aaf000-0x141d42fff.

WHAT IS ACTUALLY OVER THERE, measured at applySpEffect: not a relocated copy of the function,
but the function SHATTERED into basic-block fragments scattered across Arxan's section, each
ending in a jump to the next, with pure obfuscation thunks interleaved between them. The
prologue of applySpEffect is real code at 0x141cf8f8f -- `mov [rsp+0x10],rbx`, `sub rsp,0x30`,
`mov rdi,rcx` -- immediately followed by `jmp` to the next fragment, and padded with `push rsp`
/ `pop rsp` gymnastics whose only purpose is to break stack-frame analysis.

CONSEQUENCE FOR HOOKING, which is why this script exists: there is no contiguous relocated body
to detour. The five-byte `e9` at the original entry is the only place all callers funnel
through, so it is the hook site by elimination, not by preference.

WHY IT REPORTS RATHER THAN CONCLUDES. Arxan emits many stub shapes and this recognises a fixed
set of them. An unrecognised hop is printed as UNKNOWN with its bytes and the walk stops -- it
does not guess, and it does not silently treat "I could not decode this" as "the chain ended".
A chain that ends in UNKNOWN has not been walked; it has been abandoned partway, and the output
says so. Treat every landing address as a candidate to verify, not a fact.

It reads `darksoulsii-deobf.bin`, which RETAINS the redirects: the entry at applySpEffect really
does hold `e9 1c 0d 9f 01` there. That makes the chain statically walkable with no game running
and nothing that can contaminate the result.
"""

from __future__ import annotations

import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

IMAGE_BASE = 0x1_4000_0000
SIZE_OF_IMAGE = 0x1D76000
REPO_ROOT = Path(__file__).resolve().parents[1]
IMAGE_PATH = REPO_ROOT / "darksoulsii-deobf.bin"

#: Arxan's own code lives in the second .text section; a hop into it is still inside the chain,
#: and a hop out of it is the signal that we have probably arrived at real game code.
ARXAN_TEXT = (0x141AAF000, 0x141D42FFF)
#: Enough bytes to contain any single stub shape recognised below.
WINDOW = 64
MAX_HOPS = 64

#: Prologues that mean real game code. These are only trusted OUTSIDE Arxan's section: the first
#: version of this script declared BODY at 0x141cf8f8f because `mov [rsp+0x10],rbx` matched, when
#: that address is a shattered FRAGMENT inside Arxan that jumps onward four instructions later.
#: A prologue byte pattern says "real instruction", never "start of a contiguous function".
#:
#: The list is INCOMPLETE BY NATURE and an omission shows up as a false UNKNOWN, never as a false
#: "not redirected" -- which is the safe direction to be wrong in. FeSubStateTitleUserPolicy::v1
#: at 0x1400f9040 opens `mov [rsp+0x20],rbp` and reported UNKNOWN until that pattern was added.
BODY_PROLOGUES = (
    b"\x48\x89\x5c\x24",          # mov [rsp+N], rbx
    b"\x48\x89\x4c\x24",          # mov [rsp+N], rcx
    b"\x48\x89\x54\x24",          # mov [rsp+N], rdx
    b"\x48\x89\x6c\x24",          # mov [rsp+N], rbp
    b"\x48\x89\x74\x24",          # mov [rsp+N], rsi
    b"\x48\x89\x7c\x24",          # mov [rsp+N], rdi
    b"\x4c\x89\x44\x24",          # mov [rsp+N], r8
    b"\x4c\x89\x4c\x24",          # mov [rsp+N], r9
    b"\x40\x53\x48\x83\xec",      # push rbx; sub rsp, N
    b"\x40\x55\x48\x83\xec",      # push rbp; sub rsp, N
    b"\x40\x57\x48\x83\xec",      # push rdi; sub rsp, N
    b"\x48\x83\xec",              # sub rsp, N
    b"\x55\x48\x8b\xec",          # push rbp; mov rbp, rsp
    b"\x48\x8b\xc4",              # mov rax, rsp  (MSVC frame-pointer-omission prologue)
    b"\x40\x56",                  # rex push rsi
    b"\x40\x57",                  # rex push rdi
    b"\x40\x55",                  # rex push rbp
    b"\x40\x53",                  # rex push rbx
    b"\x4c\x8b\x89",              # mov r9, [rcx+N]
    b"\x4c\x8b\x41",              # mov r8, [rcx+N]
    b"\x48\x83\x79",              # cmp qword [rcx+N], M
    b"\x48\x8b\x41",              # mov rax, [rcx+N]
)


def disasm(data: bytes, va: int, length: int = WINDOW) -> list[str]:
    off = va - IMAGE_BASE
    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as fh:
        fh.write(data[off : off + length])
        path = fh.name
    try:
        out = subprocess.run(
            ["objdump", "-D", "-b", "binary", "-m", "i386:x86-64", "-M", "intel",
             f"--adjust-vma={va:#x}", path],
            capture_output=True, text=True, check=False,
        ).stdout
    finally:
        Path(path).unlink(missing_ok=True)
    # objdump prints a header before the listing; keep only the instruction lines.
    return [ln for ln in out.splitlines() if re.match(r"^\s+[0-9a-f]+:\t", ln)]


RE_TARGET = re.compile(r"^\s+([0-9a-f]+):\t([0-9a-f ]+?)\s*\t(\S+)\s*(.*)$")


def parse(lines: list[str]) -> list[tuple[int, str, str, str]]:
    out = []
    for ln in lines:
        m = RE_TARGET.match(ln)
        if m:
            out.append((int(m.group(1), 16), m.group(2).strip(), m.group(3), m.group(4).strip()))
    return out


def next_hop(data: bytes, va: int, entered_arxan: bool) -> tuple[int | None, str]:
    """(target, pattern-name). target None means the walk stops; the name says why.

    `entered_arxan` matters: the ENTRY of a redirected function is itself in game .text, so
    "we are in game .text" only means the chain is over once we have been into Arxan and come
    back. Without that distinction every walk terminates at hop 0 having proved nothing.
    """
    ins = parse(disasm(data, va))
    if not ins:
        return None, "UNDECODABLE"

    inside_arxan = ARXAN_TEXT[0] <= va <= ARXAN_TEXT[1]
    raw = data[va - IMAGE_BASE : va - IMAGE_BASE + 16]
    if entered_arxan and not inside_arxan:
        # Back in game .text having been through Arxan. For the entry-stub scheme this is the
        # terminal state: only the entry region was stolen, and the chain rejoins the ORIGINAL
        # function a short way past its entry, where the rest of the body sits in place.
        return None, "REJOIN (game .text)"
    if not inside_arxan:
        for pro in BODY_PROLOGUES:
            if raw.startswith(pro):
                return None, "NOT REDIRECTED (clean prologue at the entry)"

    addr0, _bytes0, mnem0, ops0 = ins[0]

    # 1. Plain relative jump -- the redirect at a function entry, and the simplest inner hop.
    if mnem0 == "jmp" and re.fullmatch(r"0x[0-9a-f]+", ops0):
        return int(ops0, 16), "jmp rel"

    # 2. push imm32; ret -- the textbook obfuscated jump.
    if mnem0 == "push" and len(ins) > 1 and ins[1][2] == "ret":
        try:
            return int(ops0, 16), "push/ret"
        except ValueError:
            pass

    # 3. The shape seen at applySpEffect: stage a RIP-relative address, swap it onto the stack
    #    under a scratch register (restoring that register), then jump through the slot.
    #    lea <r>,[rip+X] ; xchg [rsp],<r> ; ... ; jmp [rsp-8]
    lea_target = None
    lea_reg = None
    for _a, _b, mnem, ops in ins[:8]:
        m = re.fullmatch(r"(\w+),\[rip\+0x[0-9a-f]+\]\s*#\s*(0x[0-9a-f]+)", ops.replace(" ", ""))
        if mnem == "lea" and m:
            lea_reg, lea_target = m.group(1), int(m.group(2), 16)
        elif mnem == "xchg" and lea_reg and lea_reg in ops and "[rsp]" in ops.replace(" ", ""):
            return lea_target, "stack-swap thunk"

    # 4. A shattered FRAGMENT: real instructions that run until the block's terminating jump.
    #    Following that jump is the walk; counting the instructions before it is how much of the
    #    original function this hop actually contributes.
    if inside_arxan:
        for i, (_a, _b, mnem, ops) in enumerate(ins):
            if mnem == "jmp" and re.fullmatch(r"0x[0-9a-f]+", ops):
                return int(ops, 16), f"fragment ({i} insn)"
            # A conditional branch forks the walk. One linear path cannot represent that, and
            # picking a side silently would fabricate a chain, so stop and say which fork.
            if mnem.startswith("j") and mnem != "jmp" and re.fullmatch(r"0x[0-9a-f]+", ops):
                return None, f"FORK at +{i} ({mnem} -> {ops}); linear walk cannot continue"
            if mnem in ("ret", "retq"):
                return None, f"RET at +{i}; fragment returns"

    return None, "UNKNOWN"


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    data = IMAGE_PATH.read_bytes()
    if len(data) != SIZE_OF_IMAGE:
        print(f"{IMAGE_PATH}: not the flat image (size mismatch)", file=sys.stderr)
        return 1

    va = start = int(argv[1], 0)
    seen: set[int] = set()
    fragment_insns = 0
    entered_arxan = False
    print(f"walking from {va:#x}")
    for hop in range(MAX_HOPS):
        if va in seen:
            print(f"  hop {hop:<3} {va:#x}  CYCLE -- already visited")
            return 0
        seen.add(va)
        raw = data[va - IMAGE_BASE : va - IMAGE_BASE + 8].hex(" ")
        inside = ARXAN_TEXT[0] <= va <= ARXAN_TEXT[1]
        entered_arxan = entered_arxan or inside
        target, why = next_hop(data, va, entered_arxan)
        if why.startswith("fragment"):
            fragment_insns += int(why.split("(")[1].split()[0])
        zone = "arxan" if inside else "game "
        print(f"  hop {hop:<3} {va:#x} [{zone}] {raw}  {why}"
              + (f" -> {target:#x}" if target is not None else ""))
        if target is None:
            if why.startswith("REJOIN"):
                delta = va - start
                where = f" = entry+{delta:#x}" if 0 < delta < 0x400 else ""
                print(f"\nrejoins game .text at {va:#x}{where} after {hop} hop(s), "
                      f"{fragment_insns} instruction(s) of stolen code")
                print("Only the entry region is stolen; the body from here on is in place.")
            elif why.startswith("NOT REDIRECTED"):
                print(f"\n{va:#x} is NOT an Arxan-redirected entry -- its own prologue is here.")
                print("Hooking it never touches Arxan's code. That is why such a site cannot")
                print("test whether Arxan responds to a detour (see ds2-mods-rs-z6m).")
            else:
                print(f"\nSTOPPED at {va:#x}: {why}")
                print("Not a conclusion. Disassemble it and decide; the chain may continue.")
            return 0
        va = target
    print(f"\nhop limit {MAX_HOPS} reached; chain longer than expected")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
