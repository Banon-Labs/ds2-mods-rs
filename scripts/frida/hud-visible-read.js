// Read-only: which field says the in-game player HUD (the HP/stamina bar) is on screen?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/hud-visible-read.js`
//
// Writes nothing, hooks nothing, calls nothing. Every 100 ms it reads the candidates below and
// sends a record only when one of them changed, plus a heartbeat every 5 s.
//
// Chain, from the static read (darksoulsii-deobf.bin):
//   GameManagerImp [0x1416148f0] -> +0x22e0 frontend root -> +0xd8 FeOperatorFrontend
//     (ctor 0x140505d80, alloc 0x470 at 0x14050172a, stored at 0x14050174b; vtable 0x1410fa628,
//     update 0x140507360). It owns the HUD scenes the builder 0x140507ea0 makes:
//     +0x398 FeSceneHpGuage (the player's HP/stamina bar), +0x10..+0xd0 the 25-scene array.
//   op+0x008 u8   forces the auto-HUD machine (0x140507a80) straight to its hide branch
//   op+0x340 u32  the auto-HUD state: 0x66 shown, 0x73/0x74 fading, 0x68 hidden, 0x67 forced off
//   op+0x2e0 f32  the auto-HUD countdown
//   op+0x461 u8   "built" (set 1 at 0x140507f0f by the builder)
//   op+0x46c u32  one-shot request raised by 0x14050717e (state 3), consumed by the update
//   hp = [op+0x398]: hp+0x8 FeLayoutSceneLinked, its +0x18 counter (0x140505d20 dec / d30 inc)
//     and the linked scene's layout: [[linked+0x28]+0x30] vtable + slot 0x50 (state getter).
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const rel = (p) => (p ? '0x' + p.sub(image.base).add(0x140000000).toString(16) : null);

function ptr_(p, off) {
  try {
    const v = p.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}
function u8(p, off) { try { return p.add(off).readU8(); } catch (e) { return null; } }
function u32(p, off) { try { return p.add(off).readU32(); } catch (e) { return null; } }
function f32(p, off) { try { return Math.round(p.add(off).readFloat() * 100) / 100; } catch (e) { return null; } }

function sample() {
  const s = {};
  const gm = ptr_(at(0x16148f0), 0);
  if (!gm) return { gm: null };
  s.player = ptr_(gm, 0xd0) ? 1 : 0;
  const root = ptr_(gm, 0x22e0);
  if (!root) return Object.assign(s, { root: null });
  s.root_30c = u8(root, 0x30c);
  const op = ptr_(root, 0xd8);
  if (!op) return Object.assign(s, { op: null });
  s.op_vt = rel(ptr_(op, 0));
  s.op_08 = u8(op, 0x8);
  s.op_340 = u32(op, 0x340);
  s.op_2e0 = f32(op, 0x2e0);
  s.op_461 = u8(op, 0x461);
  s.op_460 = u8(op, 0x460);
  s.op_462 = u8(op, 0x462);
  s.op_46c = u32(op, 0x46c);
  s.op_468 = u32(op, 0x468);
  s.op_464 = u32(op, 0x464);
  const opts0 = ptr_(gm, 0xa8);
  const opts = opts0 && ptr_(opts0, 0xc8);
  s.opt_12 = opts ? u8(opts, 0x12) : null;
  const hp = ptr_(op, 0x398);
  if (!hp) return Object.assign(s, { hp: null });
  s.hp_vt = rel(ptr_(hp, 0));
  s.hp_e1 = u8(hp, 0xe1);
  s.hp_e2 = u8(hp, 0xe2);
  const linked = ptr_(hp, 0x8);
  if (linked) {
    s.lk_vt = rel(ptr_(linked, 0));
    s.lk_18 = u32(linked, 0x18);
    s.lk_0c = u8(linked, 0xc);
    const x = ptr_(linked, 0x28);
    const y = x && ptr_(x, 0x30);
    if (y) {
      const vt = ptr_(y, 0);
      s.y_vt = rel(vt);
      s.y_slot50 = vt ? rel(ptr_(vt, 0x50)) : null;
      // raw dwords of the layout scene, to find the state id without calling slot 0x50
      const words = [];
      for (let o = 0x8; o < 0x80; o += 4) words.push(u32(y, o));
      s.y_words = words.join(',');
    }
  }
  return s;
}

let last = '';
let beats = 0;
setInterval(() => {
  const s = sample();
  const key = JSON.stringify(s);
  beats++;
  if (key !== last || beats % 50 === 0) {
    last = key;
    console.log('[hud-visible-read] ' + key);
    send({ kind: 'hud', t: Date.now(), s });
  }
}, 100);
