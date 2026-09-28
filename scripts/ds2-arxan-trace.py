#!/usr/bin/env python3
"""Linearise a function that Arxan has shattered into fragments, from `darksoulsii-deobf.bin`.

    python3 scripts/ds2-arxan-trace.py 0x14039041a            # from one fragment
    python3 scripts/ds2-arxan-trace.py 0x14039041a --max-blocks 400

WHY THIS EXISTS. `scripts/ds2-arxan-chain.py` walks an entry stub to the first real fragment and
stops. Some game functions -- the weapon attack-rating builder around 0x140390400 is the one that
prompted this -- are shattered all the way through: every few real instructions end in a `jmp` to
the next fragment, and every branch is an obfuscated stub instead of a `jcc`. Ghidra makes no
function there (`Ds2Decomp` says NO_FUNC) and `ds2-disasm.py` prints one fragment at a time. This
follows the fragments and prints only the real instructions, one basic block per label.

WHAT IT RECOGNISES, and nothing else:

* `jmp rel` outside a stub: the next fragment. Followed, not printed.
* The Arxan branch stub. It pushes a default target with `movabs reg, A` + `xchg [rsp],reg`,
  optionally overwrites that slot with a second `movabs reg, B` under `cmovCC`, and leaves through
  `ret` or `jmp [rsp-0x8]`. Printed as `jCC B  (else A)` or `jmp A`; both targets are queued. A
  stub may itself be split by a `jmp rel` in the middle, so stub state survives a `jmp`.
* Stub noise: any operand on `[rsp-..]`, `lea rsp,[rsp+-8]`, `xchg [rsp],reg`, and push/pop or
  `[rsp+0x10]` shuffles while a stub is open. Real x64 code here has no red zone, so a negative
  rsp offset is never game data.

WHAT IT DOES NOT DO: it does not emulate. A stub shape it does not recognise ends the block with
`?? UNRESOLVED` and the bytes, instead of guessing a successor. A `call` is printed and stepped
over; its callee is not traced. The output is a reading aid whose every line is a real instruction
at a real address -- verify a load-bearing line with `scripts/ds2-disasm.py <addr>` before quoting
it. Same caveat as its siblings: the image is the file on disk (docs/ARXAN-FOOTPRINT.md).
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from collections import deque
from pathlib import Path

IMAGE_BASE = 0x1_4000_0000
SIZE_OF_IMAGE = 0x1D76000
REPO_ROOT = Path(__file__).resolve().parents[1]
IMAGE_PATH = REPO_ROOT / "darksoulsii-deobf.bin"
#: Bytes disassembled per fragment read. Fragments are short; a long one is read again from where
#: the window ran out.
WINDOW = 0x60
#: A stub loads its target either as `movabs reg, imm64` or as `lea rbp,[rip+disp] # 0x...`.
IMM_RE = re.compile(r"(?:movabs\s+(\w+),(0x[0-9a-f]+)|lea\s+(rbp),\[rip\+0x[0-9a-f]+\]\s*#\s*(0x[0-9a-f]+))")
CMOV_RE = re.compile(r"^cmov(\w+)\s")
JCC_RE = re.compile(r"^(j(?!mp)\w+)\s+(0x[0-9a-f]+)")
JMP_REL_RE = re.compile(r"^jmp\s+(0x[0-9a-f]+)$")
CALL_REL_RE = re.compile(r"^call\s+(0x[0-9a-f]+)$")


def objdump(data: bytes, va: int, length: int) -> list[tuple[int, str, str]]:
    off = va - IMAGE_BASE
    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as fh:
        fh.write(data[off: off + length])
        tmp = fh.name
    try:
        proc = subprocess.run(
            ["objdump", "-D", "-b", "binary", "-m", "i386:x86-64", "-M", "intel",
             f"--adjust-vma={va:#x}", tmp],
            capture_output=True, text=True, check=True)
    finally:
        os.unlink(tmp)
    out: list[tuple[int, str, str]] = []
    for line in proc.stdout.splitlines():
        m = re.match(r"^\s+([0-9a-f]+):\t([0-9a-f ]+)\t(.*)$", line)
        if m:
            out.append((int(m.group(1), 16), m.group(2).strip(), re.sub(r"\s+", " ", m.group(3).strip())))
            continue
        # objdump wraps an instruction longer than 7 bytes (movabs is 10) onto a bytes-only line;
        # those bytes belong to the previous instruction, or its size -- and every address after
        # it -- comes out wrong.
        m = re.match(r"^\s+([0-9a-f]+):\t([0-9a-f ]+)$", line)
        if m and out:
            a, raw, text = out[-1]
            out[-1] = (a, f"{raw} {m.group(2).strip()}", text)
    return out


def in_image(v: int) -> bool:
    return IMAGE_BASE <= v < IMAGE_BASE + SIZE_OF_IMAGE


def is_stub_noise(text: str, stub_open: bool) -> bool:
    if "[rsp-0x" in text or re.match(r"^lea rsp,\[rsp[+-]0x8\]$", text):
        return True
    if text.startswith("xchg QWORD PTR [rsp],") or re.match(r"^mov QWORD PTR \[rsp\],r(bp|cx|dx|ax)$", text):
        return True
    if stub_open and (re.match(r"^(push|pop) r(ax|bx|cx|dx|bp)$", text)
                      or "[rsp+0x10]" in text or "[rsp+0x8]" in text or "QWORD PTR [rsp]" in text):
        return True
    return False


def trace(data: bytes, start: int, max_blocks: int) -> list[str]:
    lines: list[str] = []
    queue: deque[int] = deque([start])
    seen: set[int] = set()
    labels = 0
    while queue and labels < max_blocks:
        blk = queue.popleft()
        if blk in seen:
            continue
        seen.add(blk)
        labels += 1
        lines.append(f"L_{blk:x}:")
        pc = blk
        pushed: list[int] = []       # movabs targets pushed by an open stub, in order
        cond: str | None = None      # cmovCC condition seen in the open stub
        steps = 0
        while steps < 400:
            steps += 1
            insns = objdump(data, pc, WINDOW)
            if not insns:
                lines.append(f"  ?? no decode at {pc:#x}")
                break
            advanced = False
            for addr, raw, text in insns[:-1]:  # the last one may be cut by the window
                if addr != pc:
                    continue
                size = len(raw.split())
                m_imm = IMM_RE.match(text)
                imm = int(m_imm.group(2) or m_imm.group(4), 16) if m_imm else None
                if imm is not None and in_image(imm):
                    pushed.append(imm)
                    pc += size
                    advanced = True
                    continue
                m_cmov = CMOV_RE.match(text)
                if m_cmov and pushed:
                    cond = m_cmov.group(1)
                    pc += size
                    advanced = True
                    continue
                if text == "ret" or text.startswith("repz ret") or text == "jmp QWORD PTR [rsp-0x8]":
                    if pushed:
                        if cond and len(pushed) >= 2:
                            a, b = pushed[0], pushed[-1]
                            lines.append(f"  {addr:x}: j{cond} L_{b:x}   (else L_{a:x})   [arxan stub]")
                            queue.extend([b, a])
                        else:
                            lines.append(f"  {addr:x}: jmp L_{pushed[0]:x}   [arxan stub]")
                            queue.append(pushed[0])
                    else:
                        lines.append(f"  {addr:x}: ret")
                    pc = None
                    break
                m_jmp = JMP_REL_RE.match(text)
                if m_jmp:
                    tgt = int(m_jmp.group(1), 16)
                    if not in_image(tgt):
                        lines.append(f"  {addr:x}: ?? UNRESOLVED jmp out of image {text}")
                        pc = None
                        break
                    pc = tgt  # next fragment (stub state survives)
                    advanced = True
                    break
                if is_stub_noise(text, bool(pushed)):
                    pc += size
                    advanced = True
                    continue
                if text in ("int3", "(bad)") or text.startswith("udb"):
                    lines.append(f"  {addr:x}: ?? UNRESOLVED hit {text} [{raw}]")
                    pc = None
                    break
                m_jcc = JCC_RE.match(text)
                if m_jcc:
                    tgt = int(m_jcc.group(2), 16)
                    lines.append(f"  {addr:x}: {m_jcc.group(1)} L_{tgt:x}")
                    queue.append(tgt)
                    pc += size
                    advanced = True
                    continue
                m_call = CALL_REL_RE.match(text)
                lines.append(f"  {addr:x}: {text}" + ("" if not m_call else ""))
                pc += size
                advanced = True
                if text.startswith("jmp "):  # indirect jmp that is not the stub exit
                    lines.append("  ?? UNRESOLVED indirect jmp")
                    pc = None
                    break
            if pc is None:
                break
            if not advanced:
                lines.append(f"  ?? stuck at {pc:#x}")
                break
            if pc in seen and pc != blk and not pushed:
                lines.append(f"  -> L_{pc:x}")
                break
    if queue:
        lines.append(f"... {len(queue)} queued blocks not printed (--max-blocks)")
    return lines


def refs(data: bytes, target: int) -> list[str]:
    """Every byte position that could transfer control to `target`: a `call`/`jmp rel32`, a
    `jcc rel32`, a short `jmp`/`jcc`, a `movabs` imm64 or a `lea reg,[rip+disp]`. Walking a
    shattered function BACKWARDS needs this, because its fragments have no `.pdata` and Ghidra
    made no function over them. Candidates, not proof: a hit inside junk bytes is possible, so
    read each one with `ds2-disasm.py` before relying on it."""
    import numpy as np

    a = np.frombuffer(data, dtype=np.uint8)
    n = len(a) - 8
    idx = np.arange(n, dtype=np.int64)
    t = target - IMAGE_BASE

    def rel32(at: int) -> np.ndarray:
        r = (a[at:n + at].astype(np.int64) | (a[at + 1:n + at + 1].astype(np.int64) << 8)
             | (a[at + 2:n + at + 2].astype(np.int64) << 16) | (a[at + 3:n + at + 3].astype(np.int64) << 24))
        return np.where(r >= 2**31, r - 2**32, r)

    out = []
    op = a[:n]
    for h in np.flatnonzero(((op == 0xE8) | (op == 0xE9)) & (idx + 5 + rel32(1) == t)).tolist():
        out.append(f"{IMAGE_BASE + h:#x}  {'call' if data[h] == 0xE8 else 'jmp'} rel32")
    jcc = (op == 0x0F) & (a[1:n + 1] >= 0x80) & (a[1:n + 1] <= 0x8F)
    for h in np.flatnonzero(jcc & (idx + 6 + rel32(2) == t)).tolist():
        out.append(f"{IMAGE_BASE + h:#x}  jcc rel32")
    lea = (op >= 0x48) & (op <= 0x4F) & (a[1:n + 1] == 0x8D) & ((a[2:n + 2] & 0xC7) == 0x05)
    for h in np.flatnonzero(lea & (idx + 7 + rel32(3) == t)).tolist():
        out.append(f"{IMAGE_BASE + h:#x}  lea rip")
    r8 = a[1:n + 1].astype(np.int64)
    r8 = np.where(r8 >= 128, r8 - 256, r8)
    short = (op == 0xEB) | ((op >= 0x70) & (op <= 0x7F))
    for h in np.flatnonzero(short & (idx + 2 + r8 == t)).tolist():
        out.append(f"{IMAGE_BASE + h:#x}  short jmp/jcc")
    needle = target.to_bytes(8, "little")
    i = 0
    while (i := data.find(needle, i)) >= 0:
        out.append(f"{IMAGE_BASE + i:#x}  imm64")
        i += 1
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("address", help="virtual address of a fragment, e.g. 0x14039041a")
    ap.add_argument("--max-blocks", type=int, default=120)
    ap.add_argument("--refs", action="store_true",
                    help="list what can transfer control TO the address instead of tracing from it")
    args = ap.parse_args(argv[1:])
    data = IMAGE_PATH.read_bytes()
    if len(data) != SIZE_OF_IMAGE:
        raise SystemExit(f"{IMAGE_PATH} is {len(data)} bytes, expected {SIZE_OF_IMAGE}")
    va = int(args.address, 0)
    print("\n".join(refs(data, va) if args.refs else trace(data, va, args.max_blocks)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
