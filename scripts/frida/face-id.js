// Give the local player a fixed face model by id, the way NpcPlayer characters get theirs.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/face-id.js`
//   (measure only: logs appearance inits, face file loads and the face-ready check)
// `... --config-json '{"face_id": 7410}'`
//   (on the player's next load, both of its models take the fixed-face path with that id)
//
// Static trace, darksoulsii-deobf.bin:
//   Face mode is ChrAsmModel+0xe88 bits 0x18: 0x08 FaceGen sliders, 0x10 fixed face model by id.
//   Two inits read the equip's type (equip = owner->vtbl[0x70](); type +0x1c, face id +0x30):
//     ChrAsmCopyModel init 0x140339f60, read at 0x14033a018: 3 -> 0x10, 4/5 -> 0x08; both
//       branches meet at 0x14033a046.
//     the other model's init 0x140151850 (sets +0x1040 = r9, the model it takes its face from),
//       read at 0x1401518e5: 4/5 -> 0x08, anything else leaves the mode as constructed; the
//       decision is done at 0x1401518fe.
//   Face-ready check 0x140152370(model): mode 0 or model+0xd08 set -> ready; mode 2 -> ready only
//     once [[model+0x1040]+0xd08]->vtbl[0x50]() is non-null, then 0x140151d70 copies it in.
//   Face file: 0x140345050 (r8d = id, r9b = sex) -> 0x14048be80 "FC_%4d_%s".
//   Player: [[GameManagerImp 0x16148f0] + 0xd0] -> +0x378 ChrAsmCtrl -> +0x28 equip.
//
// MEASURED 2026-09-28 (beads ds2-player-fixed-face-hangs-world-load-2026-09-28): equip type 3
// left on the player hung the world load; type 3 held only across the copy init, restored to 4,
// also hung (the other model then took FaceGen from a copy that has none). So the equip type now
// stays 4 everywhere except the copy init's own read, and the other model is put on 0x10
// directly when the model it takes its face from is one this agent switched.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const FACE_ID = Number.isInteger(config.face_id) ? config.face_id : null;

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const say = (text) => console.log('[face-id] ' + text);
const mode = (model) => (model.add(0xe88).readU8() >> 3) & 3;

function playerEquip() {
  const manager = at(0x16148f0).readPointer();
  if (manager.isNull()) return null;
  const player = manager.add(0xd0).readPointer();
  if (player.isNull()) return null;
  const asm = player.add(0x378).readPointer();
  if (asm.isNull()) return null;
  const equip = asm.add(0x28).readPointer();
  return equip.isNull() ? null : { player, asm, equip };
}

let last = '';
function poll() {
  try {
    const p = playerEquip();
    const now = p ? 'player=' + p.player + ' equip=' + p.equip + ' type=' + p.equip.add(0x1c).readS32() +
      ' face=' + p.equip.add(0x30).readS32() : 'no player';
    if (now !== last) {
      say(now);
      last = now;
    }
  } catch (e) {
    say('poll: ' + e);
  }
}

const held = new Map(); // threadId -> { equip, type }: copy init in progress
const switched = new Set(); // copy models put on the fixed face, as strings

Interceptor.attach(at(0x33a018), {
  onEnter() {
    const equip = this.context.rax;
    const type = equip.add(0x1c).readS32();
    const p = playerEquip();
    const mine = p !== null && p.equip.equals(equip);
    // Before PlayerCtrl exists, a type 4/5 equip is taken to be the player's: offline, nobody
    // else has a FaceGen face.
    const target = mine || (p === null && (type === 4 || type === 5));
    say('copy init model=' + this.context.rdi + ' equip=' + equip + ' type=' + type +
      (mine ? ' (player)' : p === null ? ' (player not up yet)' : ''));
    if (FACE_ID !== null && target) {
      held.set(this.threadId, { equip, type });
      equip.add(0x30).writeS32(FACE_ID);
      equip.add(0x1c).writeS32(3);
    }
  },
});

Interceptor.attach(at(0x33a046), {
  onEnter() {
    const h = held.get(this.threadId);
    if (h === undefined) return;
    held.delete(this.threadId);
    h.equip.add(0x1c).writeS32(h.type);
    switched.add(this.context.rdi.toString());
    say('  copy ' + this.context.rdi + ' mode=' + mode(this.context.rdi) + ', type back to ' + h.type);
  },
});

Interceptor.attach(at(0x1518fe), {
  onEnter() {
    const model = this.context.rdi;
    const source = model.add(0x1040).readPointer();
    const mine = switched.has(source.toString());
    if (mine && FACE_ID !== null) {
      model.add(0xe88).writeU8((model.add(0xe88).readU8() & ~0x18) | 0x10);
    }
    say('model init ' + model + ' source=' + source + ' mode=' + mode(model) + (mine ? ' (forced fixed face)' : ''));
  },
});

Interceptor.attach(at(0x345050), {
  onEnter(args) {
    say('face file load id=' + args[2].toInt32() + ' sex=' + (args[3].toInt32() & 0xff));
  },
});

// The ready check runs every frame for a model that is waiting; log only changes.
const readyState = new Map();
Interceptor.attach(at(0x152370), {
  onEnter() {
    this.model = this.context.rcx;
  },
  onLeave(retval) {
    const model = this.model;
    const m = mode(model);
    if (m !== 2 && !switched.has(model.toString())) return;
    const source = model.add(0x1040).readPointer();
    const part = source.isNull() ? ptr(0) : source.add(0xd08).readPointer();
    const state = 'mode=' + m + ' own+0xd08=' + model.add(0xd08).readPointer() + ' source=' + source +
      (source.isNull() ? '' : ' source mode=' + mode(source)) + ' source part=' + part +
      (part.isNull() ? '' : ' vtbl=0x' + part.readPointer().sub(image.base).toString(16)) +
      ' ready=' + (retval.toInt32() & 0xff);
    if (readyState.get(model.toString()) !== state) {
      readyState.set(model.toString(), state);
      say('ready check ' + model + ' ' + state);
    }
  },
});

// THE WAIT (static trace + measured 2026-09-28): GameManager state 19 (0x1401bea10) leaves for
// state 22 only once 0x14037f5b0(player) is true; after 30 s it gives up, runs 24 -> 31 -> 20 and
// lands in 19 again, so the load loops forever. 0x14037f5b0 takes the player's face part through
// 0x140339cf0 (Arxan-wrapped; its unwrapped neighbour 0x140339d10 hands a part out only in FaceGen
// mode 0x08) and asks its vtbl[0x90]; a fixed-face player has none. Turning that false into true
// (except on its own early return, [[GameManagerImp]+0x80]+0x100 == 2) let the load run
// 19 -> 28 -> 30 and the player came up wearing FC_7410.
if (FACE_ID !== null) {
  let forced = 0;
  Interceptor.attach(at(0x37f5b0), {
    onLeave(retval) {
      if ((retval.toInt32() & 0xff) !== 0 || switched.size === 0) return;
      if (at(0x16148f0).readPointer().add(0x80).readPointer().add(0x100).readS32() === 2) return;
      retval.replace(ptr(1));
      if (forced++ === 0) say('player-ready check forced true');
    },
  });
}

// NOT THE WAIT (measured 2026-09-28): 0x14050c2b7 polls the player model's FaceGen face every
// frame and sets [rdi+0xb8] only when it exists, but rdi is an EquipPanelGroup (vtable
// 0x1410facd8, the equipment menu's preview), not the load; forcing that flag left the load hung.
// The loading screen is shown by GameManager state 31 (0x1401c0210) and hidden by state 28
// (0x1401bf200) -- hardware watchpoint on FeOperatorNowLoading+0xa8, nowloading-flag-watch.js.
// Never put a fixed-face model back to 0x08 to "rescue" it: 0x140339cd0 then hands the fixed-face
// part out as a FaceGen face and a vtbl[0x178] call executes string data (crash 22:29:38Z, pc
// 0x5f004d005f = UTF-16 "_M_").

say('face_id=' + FACE_ID + ' (null: measure only)');
poll();
setInterval(poll, 1000);
