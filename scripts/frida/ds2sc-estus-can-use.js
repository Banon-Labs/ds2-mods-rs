// Read-only: does Seamless Co-op let an invader drink Estus, and why or why not?
//
// `python3 scripts/ds2-frida-up.py --force` once the game is in a world, then attach a FROZEN COPY
// (bd ds2-frida-hot-reload-kills-game):
//   cp -f scripts/frida/ds2sc-estus-can-use.js ~/.cache/er-frida/ds2sc-estus-can-use-frozen.js
//   uv run --with frida python3 scripts/ds2-frida-watch.py \
//     --agent ~/.cache/er-frida/ds2sc-estus-can-use-frozen.js --log ~/.cache/er-frida/estus-can-use.jsonl
//
// One Interceptor.attach, on ds2sc.dll's own replacement for the game's item can-use check. Writes
// nothing, calls nothing in the game, and sends only when something changes.
//
// `--config-json '{"observe_only": true}'` installs no hook at all: the attach report and the
// phantom-type/session-flag timer only. That is the shape to use when the hook itself is not the
// question, since the two recorded attach freezes (bd ds2-frida-attach-froze-game-twice) were both
// at attaches that installed Interceptor hooks and read-only agents never froze.
//
// # What is hooked, read off ds2sc.dll 0.0.6 on disk (this function is not packed)
//
//   0x180060bc0(rcx = registry, rdx = ctx, r8d = item id)      reached by `jmp` from 0x180072553
//     looks the item up in two tables; if 0x1800890f0 says a handler took it:
//       ebx = item id; setne bl                ; return = (item & ~0xff) | usable
//     else:
//       call [registry+0x78](ctx, item)        ; return = whatever the original returns
//
// `setne bl` only writes the low byte, so on the handler path the upper 24 bits of the return are
// the item id's own. That is how `path` below is told apart without a second hook: upper bits equal
// to the item's means a ds2sc handler answered, anything else means the original did. (An original
// whose return happened to carry the item id's upper bits would be misread; nothing suggests one
// does, and the raw return is sent beside it.)
//
// The session flag is NOT the word at [registry+0x18]: 0x180061160, which reads it, does
//   mov r8,[0x1803c0e58]; mov rdx,[r8+0x18]; test byte ptr [rdx],0x10
// so [registry+0x18] is a pointer and the flag is the byte it points at. Both are sent.
//
// # What it sends
//
//   attach    once: ds2sc base/size, whether 0x60bc0's prologue matches the 0.0.6 file (no hook if
//             not), the registry, [registry+0x18] and the flag byte, [registry+0x78] beside
//             DarkSoulsII.exe+0x1a8b90, where that pointer's code resumes in the game, and where
//             the game's own 0x1a8b90 entry jumps to now.
//   registry  once, at the first call: whether the rcx 0x60bc0 got is [ds2sc+0x3c0e58].
//   can-use   per item id, only when (usable, path, flag byte, local phantom type) changes.
//   calls     every 5 s, only if any count moved: calls per item id since attach.
//   state     once a second, only on change: local phantom type and the session flag byte (and its
//             pointer), so the role is recorded even when no item check ever fires.
//
// Local phantom type is [[GameManagerImp]+0xd0 -> +0xb0]+0x3c (ds2_rva::PHANTOM_BLOCK_PHANTOM_PARAM_
// OFFSET; 0x14014ed20 reads this byte). Read live only as the host so far (type 0, 2026-10-01); an
// invader's value has not been observed, so `invader` is the ds2_rva::INVADER_PHANTOM_TYPES table,
// not a measurement.
'use strict';

const game = Process.getModuleByName('DarkSoulsII.exe');
const sc = Process.findModuleByName('ds2sc.dll');

// ds2sc.dll 0.0.6 RVAs, all read from the file in SeamlessCoop/.
const SC_CAN_USE = 0x60bc0;
const SC_REGISTRY_GLOBAL = 0x3c0e58; // written by 0x1800603c0, read by 0x180061165
const REGISTRY_FLAG_POINTER_OFFSET = 0x18;
const REGISTRY_ORIGINAL_OFFSET = 0x78;
const SESSION_FLAG_BIT = 0x10;
// push rsi; push rdi; push rbp; push rbx; sub rsp,0x48; mov ebx,r8d
const SC_CAN_USE_PROLOGUE = [0x56, 0x57, 0x55, 0x53, 0x48, 0x83, 0xec, 0x48, 0x44, 0x89, 0xc3];

// DarkSoulsII.exe, darksoulsii-deobf.bin. The inferred target of ds2sc's hook: (rcx = ctx, edx = item),
// the same two arguments 0x60bc0 hands [registry+0x78].
const GAME_CAN_USE = 0x1a8b90;
// mov [rsp+0x10],rsi; push rdi; sub rsp,0x20
const GAME_CAN_USE_PROLOGUE = [0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20];

// ds2-rva names, same values.
const GAME_MANAGER_IMP = 0x016148f0;
const GAME_MANAGER_PLAYER_CTRL_OFFSET = 0xd0;
const CHARACTER_CTRL_PHANTOM_BLOCK_OFFSET = 0xb0;
const PHANTOM_BLOCK_PHANTOM_PARAM_OFFSET = 0x3c;
const INVADER_PHANTOM_TYPES = [8, 10, 11, 12, 14, 15, 16, 17];

const ESTUS_FLASK = 60155000;
const LIFEGEM = 60010000;
const NAMES = { [ESTUS_FLASK]: 'estus', [LIFEGEM]: 'lifegem' };

const SUMMARY_EVERY_MS = 5000;
const PHANTOM_EVERY_MS = 1000;
const MAX_HOPS = 4;
const MAX_TRAMPOLINE_INSNS = 12;

function hex(n) { return '0x' + (n >>> 0).toString(16); }
function bytesAt(p, n) {
  try { return Array.from(new Uint8Array(p.readByteArray(n))); } catch (e) { return null; }
}
function hexBytes(bs) { return bs === null ? 'unreadable' : bs.map((b) => b.toString(16).padStart(2, '0')).join(' '); }
function same(a, b) { return a !== null && a.length === b.length && a.every((x, i) => x === b[i]); }
function readPtr(p) {
  try { const v = p.readPointer(); return v.isNull() ? null : v; } catch (e) { return null; }
}
function where(p) {
  if (p === null) return 'null';
  const m = Process.findModuleByAddress(p);
  return m === null ? p.toString() : m.name + '+0x' + p.sub(m.base).toString(16);
}

// Where an unconditional jmp goes, or null. Handles `jmp rel32` and `jmp qword ptr [rip+d]`.
function jmpTarget(insn) {
  if (insn.mnemonic !== 'jmp' || insn.operands.length !== 1) return null;
  const op = insn.operands[0];
  if (op.type === 'imm') return ptr(op.value.toString());
  if (op.type === 'mem' && op.value.base === 'rip' && op.value.index === undefined) {
    return readPtr(insn.next.add(op.value.disp));
  }
  return null;
}

// Follow a jmp chain from `start`, at most MAX_HOPS hops. Every hop is reported.
function followJumps(start) {
  const hops = [];
  let at = start;
  for (let i = 0; i < MAX_HOPS && at !== null; i++) {
    let insn;
    try { insn = Instruction.parse(at); } catch (e) { hops.push({ at: where(at), error: String(e) }); break; }
    const to = jmpTarget(insn);
    hops.push({ at: where(at), insn: insn.mnemonic + ' ' + insn.opStr });
    if (to === null) break;
    at = to;
  }
  return hops;
}

// Walk a hook trampoline: the copied prologue, then a jmp back into the hooked function.
function decodeTrampoline(start) {
  const copied = [];
  let at = start;
  for (let i = 0; i < MAX_TRAMPOLINE_INSNS; i++) {
    let insn;
    try { insn = Instruction.parse(at); } catch (e) { return { copied: copied, error: String(e) }; }
    const to = jmpTarget(insn);
    if (to !== null) return { copied: copied, resumesAt: to, copiedBytes: at.sub(start).toInt32() };
    copied.push(insn.mnemonic + ' ' + insn.opStr);
    at = insn.next;
  }
  return { copied: copied, error: 'no jmp in ' + MAX_TRAMPOLINE_INSNS + ' instructions' };
}

function localPhantom() {
  const manager = readPtr(game.base.add(GAME_MANAGER_IMP));
  const local = manager && readPtr(manager.add(GAME_MANAGER_PLAYER_CTRL_OFFSET));
  const block = local && readPtr(local.add(CHARACTER_CTRL_PHANTOM_BLOCK_OFFSET));
  if (!block) return null;
  try { return block.add(PHANTOM_BLOCK_PHANTOM_PARAM_OFFSET).readU8(); } catch (e) { return null; }
}

function sessionFlag(registry) {
  const flagPtr = registry && readPtr(registry.add(REGISTRY_FLAG_POINTER_OFFSET));
  if (!flagPtr) return { flagPtr: null, flag: null };
  try { return { flagPtr: flagPtr, flag: flagPtr.readU8() }; } catch (e) { return { flagPtr: flagPtr, flag: null }; }
}

if (sc === null) {
  send({ kind: 'attach', ok: false, error: 'ds2sc.dll is not loaded; nothing to hook' });
  throw new Error('ds2sc.dll is not loaded');
}

// ---- once, at attach ----------------------------------------------------------------------------
const canUse = sc.base.add(SC_CAN_USE);
const registryGlobal = sc.base.add(SC_REGISTRY_GLOBAL);
const gameCanUse = game.base.add(GAME_CAN_USE);

const prologue = bytesAt(canUse, SC_CAN_USE_PROLOGUE.length);
const prologueOk = same(prologue, SC_CAN_USE_PROLOGUE);
const registryAtAttach = readPtr(registryGlobal);
const flagAtAttach = sessionFlag(registryAtAttach);
const original = registryAtAttach && readPtr(registryAtAttach.add(REGISTRY_ORIGINAL_OFFSET));

const attach = {
  kind: 'attach',
  ok: prologueOk && registryAtAttach !== null,
  ds2sc: { base: sc.base.toString(), size: hex(sc.size), path: sc.path },
  canUse: where(canUse),
  canUsePrologue: prologueOk ? 'ok' : 'MISMATCH saw=' + hexBytes(prologue) + ' want=' + hexBytes(SC_CAN_USE_PROLOGUE),
  registry: registryAtAttach === null ? null : registryAtAttach.toString(),
  flagPointer: where(flagAtAttach.flagPtr),
  flagByte: flagAtAttach.flag === null ? null : hex(flagAtAttach.flag),
  flagBit0x10: flagAtAttach.flag === null ? null : (flagAtAttach.flag & SESSION_FLAG_BIT) !== 0,
  original: where(original),
  inferredOriginal: where(gameCanUse),
  originalIsInferred: original !== null && original.equals(gameCanUse),
  gameEntryPrologue: hexBytes(bytesAt(gameCanUse, GAME_CAN_USE_PROLOGUE.length)),
  gameEntryMatchesDeobf: same(bytesAt(gameCanUse, GAME_CAN_USE_PROLOGUE.length), GAME_CAN_USE_PROLOGUE),
  gameEntryJumps: followJumps(gameCanUse),
  localPhantomType: localPhantom(),
};
if (original !== null && !attach.originalIsInferred) {
  const t = decodeTrampoline(original);
  attach.originalTrampoline = {
    copied: t.copied,
    copiedBytes: t.copiedBytes === undefined ? null : t.copiedBytes,
    resumesAt: t.resumesAt === undefined ? null : where(t.resumesAt),
    // The inference holds if the trampoline resumes in DarkSoulsII.exe at 0x1a8b90 + what it copied.
    resumesInsideInferred: t.resumesAt !== undefined && t.copiedBytes !== undefined &&
      t.resumesAt.equals(gameCanUse.add(t.copiedBytes)),
    error: t.error === undefined ? null : t.error,
  };
}
send(attach);

// ---- per call -----------------------------------------------------------------------------------
let phantom = attach.localPhantomType;
const lastState = new Map();
const calls = new Map();
let callsMoved = false;
let registryChecked = false;

const observeOnly = (globalThis.__ER_FRIDA_CONFIG || {}).observe_only === true;
if (observeOnly) {
  send({ kind: 'hooked', at: null, why: 'observe_only: no Interceptor installed' });
} else if (prologueOk) {
  Interceptor.attach(canUse, {
    onEnter(args) {
      this.item = args[2].toUInt32();
      if (!registryChecked) {
        registryChecked = true;
        const global = readPtr(registryGlobal);
        send({ kind: 'registry', rcx: args[0].toString(), global: global === null ? null : global.toString(),
          equal: global !== null && args[0].equals(global) });
      }
    },
    onLeave(retval) {
      const item = this.item;
      const ret = retval.toUInt32();
      calls.set(item, (calls.get(item) || 0) + 1);
      callsMoved = true;
      const usable = (ret & 0xff) !== 0;
      const path = (ret >>> 8) === (item >>> 8) ? 'ds2sc-handler' : 'original';
      const f = sessionFlag(readPtr(registryGlobal));
      const key = usable + '|' + path + '|' + f.flag + '|' + phantom;
      if (lastState.get(item) === key) return;
      lastState.set(item, key);
      send({
        kind: 'can-use',
        item: item,
        name: NAMES[item] || null,
        ret: hex(ret),
        usable: usable,
        path: path,
        flagByte: f.flag === null ? null : hex(f.flag),
        flagBit0x10: f.flag === null ? null : (f.flag & SESSION_FLAG_BIT) !== 0,
        localPhantomType: phantom,
        invader: phantom === null ? null : INVADER_PHANTOM_TYPES.indexOf(phantom) >= 0,
        tid: Process.getCurrentThreadId(),
      });
    },
  });
  send({ kind: 'hooked', at: where(canUse) });
}

let lastStateKey = '';
function stateTick() {
  phantom = localPhantom();
  const f = sessionFlag(readPtr(registryGlobal));
  const key = phantom + '|' + f.flag + '|' + where(f.flagPtr);
  if (key === lastStateKey) return;
  lastStateKey = key;
  send({
    kind: 'state',
    localPhantomType: phantom,
    invader: phantom === null ? null : INVADER_PHANTOM_TYPES.indexOf(phantom) >= 0,
    flagPointer: where(f.flagPtr),
    flagByte: f.flag === null ? null : hex(f.flag),
    flagBit0x10: f.flag === null ? null : (f.flag & SESSION_FLAG_BIT) !== 0,
  });
}
stateTick();
setInterval(stateTick, PHANTOM_EVERY_MS);
setInterval(() => {
  if (!callsMoved) return;
  callsMoved = false;
  const out = {};
  for (const [item, n] of calls) out[item] = n;
  send({ kind: 'calls', perItem: out });
}, SUMMARY_EVERY_MS);
