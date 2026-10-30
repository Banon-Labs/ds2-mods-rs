#!/usr/bin/env python3
"""Run `scripts/frida/ds2sc-estus-can-use.js` against a planted fake image in a local `sleep`.

    uv run --with frida python3 scripts/ds2sc-estus-can-use-selftest.py

Not the game, and not evidence about the game. It proves the agent's own logic with no game
running: the prologue gate, the registry and flag-pointer reads, the trampoline decode, the jmp
chain, the path classification from the return's upper bits, and the state-change dedup. The fake
`0x60bc0` keeps the real 0.0.6 prologue and then returns `(item & ~0xff) | 1` for Estus, the shape
`setne bl` leaves on ds2sc's handler path, and a plain `1` for anything else.

Exits non-zero if any expected message is missing or wrong.
"""

from __future__ import annotations

import pathlib
import sys
import threading

import frida

AGENT = pathlib.Path(__file__).resolve().parent / "frida" / "ds2sc-estus-can-use.js"

SHIM = r"""
'use strict';
const SIZE = 0x1800000 + 0x400000;
const block = Memory.alloc(SIZE);
Memory.protect(block, SIZE, 'rwx');
const gameBase = block;
const scBase = block.add(0x1800000);
globalThis.__keep = block;
// Process's methods are read-only, so the agent runs inside a function whose `Process` is this.
globalThis.__fakeProcess = {
  getModuleByName: (n) => n === 'DarkSoulsII.exe'
    ? { name: n, base: gameBase, size: 0x1800000, path: 'fake' } : Process.getModuleByName(n),
  findModuleByName: (n) => n === 'ds2sc.dll'
    ? { name: n, base: scBase, size: 0x400000, path: 'fake-ds2sc' } : Process.findModuleByName(n),
  findModuleByAddress: (p) => Process.findModuleByAddress(p),
  getCurrentThreadId: () => Process.getCurrentThreadId(),
};
scBase.add(0x60bc0).writeByteArray([0x56,0x57,0x55,0x53,0x48,0x83,0xec,0x48,0x44,0x89,0xc3,
  0x89,0xd8, 0x81,0xfb,0x78,0xe4,0x95,0x03, 0x74,0x02, 0x31,0xc0, 0xb0,0x01,
  0x48,0x83,0xc4,0x48, 0x5b,0x5d,0x5f,0x5e, 0xc3]);
const registry = Memory.alloc(0x100); globalThis.__reg = registry;
const flag = Memory.alloc(8); globalThis.__flag = flag; flag.writeU8(0x10);
scBase.add(0x3c0e58).writePointer(registry);
registry.add(0x18).writePointer(flag);
const tramp = scBase.add(0x200000);
tramp.writeByteArray([0x48,0x89,0x74,0x24,0x10, 0xff,0x25,0,0,0,0]);
tramp.add(11).writePointer(gameBase.add(0x1a8b95));
registry.add(0x78).writePointer(tramp);
const entry = gameBase.add(0x1a8b90);
const relay = scBase.add(0x72520);
entry.writeU8(0xe9); entry.add(1).writeS32(relay.sub(entry.add(5)).toInt32());
relay.writeByteArray([0xff,0x25,0,0,0,0]); relay.add(6).writePointer(scBase.add(0x60bc0));
const mgr = Memory.alloc(0x200), pc = Memory.alloc(0x200), pb = Memory.alloc(0x100);
globalThis.__objs = [mgr, pc, pb];
gameBase.add(0x016148f0).writePointer(mgr); mgr.add(0xd0).writePointer(pc);
pc.add(0xb0).writePointer(pb); pb.add(0x3c).writeU8(8);
"""

# The calls run on a thread of their own: Frida does not report calls made from its own JS thread
# (bd frida-listener-calls-are-not-intercepted), and the game's calls never come from there either.
TAIL = r"""
globalThis.__worker = new NativeCallback(() => {
  // This host is SysV: Interceptor reads args[0..2] from rdi/rsi/rdx, while the fake body (the real
  // win64 prologue) takes the item from r8d. Five arguments put the registry in rdi and rcx and the
  // item in rdx and r8, so both readings see the game's (registry, ctx, item).
  const g = new NativeFunction(globalThis.__fakeProcess.findModuleByName('ds2sc.dll').base.add(0x60bc0), 'uint32',
    ['pointer', 'pointer', 'uint32', 'pointer', 'uint32']);
  const reg = globalThis.__reg;
  const f = (r, c, item) => g(r, c, item, r, item);
  for (let i = 0; i < 3; i++) f(reg, ptr(0), 60155000);
  f(reg, ptr(0), 60010000);
  globalThis.__flag.writeU8(0);
  f(reg, ptr(0), 60155000);
  return ptr(0);
}, 'pointer', ['pointer']);
setTimeout(() => {
  const create = new NativeFunction(Module.getGlobalExportByName('pthread_create'), 'int',
    ['pointer', 'pointer', 'pointer', 'pointer']);
  globalThis.__tid = Memory.alloc(8);
  create(globalThis.__tid, ptr(0), globalThis.__worker, ptr(0));
  setTimeout(() => send({ kind: 'test-done' }), 5500);
}, 200);
"""


def main() -> int:
    messages: list[dict] = []
    done = threading.Event()

    def on_message(message, _data):
        payload = message.get("payload", message)
        print("AGENT", payload, flush=True)
        messages.append(payload if isinstance(payload, dict) else {"raw": payload})
        if isinstance(payload, dict) and payload.get("kind") == "test-done":
            done.set()

    pid = frida.spawn(["/usr/bin/sleep", "30"])
    try:
        session = frida.attach(pid)
        agent = "(function (Process) {\n" + AGENT.read_text(encoding="utf-8") + "\n})(globalThis.__fakeProcess);\n"
        script = session.create_script(SHIM + "\n" + agent + TAIL)
        script.on("message", on_message)
        script.load()
        done.wait(15)
        session.detach()
    finally:
        frida.kill(pid)

    def kind(k):
        return [m for m in messages if m.get("kind") == k]

    attach = (kind("attach") or [{}])[0]
    uses = kind("can-use")
    estus = [m for m in uses if m.get("item") == 60155000]
    gem = [m for m in uses if m.get("item") == 60010000]
    calls = kind("calls")
    states = kind("state")
    tramp = attach.get("originalTrampoline") or {}
    checks = [
        ("attach ok", attach.get("ok") is True),
        ("prologue gate passed", attach.get("canUsePrologue") == "ok"),
        ("flag byte read through [registry+0x18]", attach.get("flagByte") == "0x10" and attach.get("flagBit0x10") is True),
        ("original is not the game function itself", attach.get("originalIsInferred") is False),
        ("trampoline resumes at inferred+copied", tramp.get("resumesInsideInferred") is True and tramp.get("copiedBytes") == 5),
        ("game entry followed two jmps to 0x60bc0", len(attach.get("gameEntryJumps", [])) == 3),
        ("local phantom type read", attach.get("localPhantomType") == 8),
        ("hooked", len(kind("hooked")) == 1),
        ("registry rcx equals global", (kind("registry") or [{}])[0].get("equal") is True),
        ("three identical estus calls, one message, then one on flag change", len(estus) == 2),
        ("estus first: handler path, usable, bit set, invader",
         bool(estus) and estus[0].get("path") == "ds2sc-handler" and estus[0].get("usable") is True
         and estus[0].get("flagBit0x10") is True and estus[0].get("invader") is True
         and estus[0].get("ret") == "0x395e401"),
        ("estus second: bit clear", len(estus) > 1 and estus[1].get("flagBit0x10") is False),
        ("lifegem: original path", len(gem) == 1 and gem[0].get("path") == "original" and gem[0].get("ret") == "0x1"),
        ("call counts summarised", bool(calls) and calls[-1].get("perItem", {}).get("60155000") == 4),
        ("state sent at attach: phantom 8, flag set",
         bool(states) and states[0].get("localPhantomType") == 8 and states[0].get("flagBit0x10") is True),
        ("state re-sent when the flag cleared, with no item check needed",
         len(states) == 2 and states[1].get("flagByte") == "0x0"),
    ]
    failed = 0
    for name, ok in checks:
        print(("ok   " if ok else "FAIL ") + name)
        failed += 0 if ok else 1
    print(f"{len(checks) - failed}/{len(checks)} checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
