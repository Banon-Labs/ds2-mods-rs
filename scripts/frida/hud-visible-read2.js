// Read-only, second pass of hud-visible-read.js: the per-scene state behind the HUD's show/hide.
//
// Writes nothing, hooks nothing, calls nothing. Sends a record when anything changed (every 100 ms
// poll) and a heartbeat every 5 s.
//
//   op = [[[0x1416148f0]+0x22e0]+0xd8]  FeOperatorFrontend
//   for each scene pointer in op+0x10 .. op+0xd0 (25 slots):
//     linked = [scene+8] (FeLayoutSceneLinked), cnt = linked+0x18
//     y = [[linked+0x28]+0x30] (FeComponentScene), z = [y+0x38]; state getter is z's vslot 0x50
//       (0x140afdb30 -> 0x140b50800 -> 0x140b6b860 -> z->vslot 0x50)
//   the auto-HUD list: op+0x2e8, count at op+0x338
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const rel = (p) => (p ? '0x' + p.sub(image.base).add(0x140000000).toString(16) : null);
function ptr_(p, off) {
  try { const v = p.add(off).readPointer(); return v.isNull() ? null : v; } catch (e) { return null; }
}
function u8(p, off) { try { return p.add(off).readU8(); } catch (e) { return null; } }
function u32(p, off) { try { return p.add(off).readU32(); } catch (e) { return null; } }

let zinfo = null;
function sample() {
  const gm = ptr_(at(0x16148f0), 0);
  const root = gm && ptr_(gm, 0x22e0);
  const op = root && ptr_(root, 0xd8);
  if (!op) return { op: null };
  const s = { player: ptr_(gm, 0xd0) ? 1 : 0, op_08: u8(op, 8), op_340: u32(op, 0x340), op_338: u32(op, 0x338) };
  const cnt = [];
  for (let i = 0; i < 25; i++) {
    const sc = ptr_(op, 0x10 + i * 8);
    const lk = sc && ptr_(sc, 8);
    cnt.push(lk ? u32(lk, 0x18) : -1);
  }
  s.cnt = cnt.join(',');
  const hp = ptr_(op, 0x398);
  const lk = hp && ptr_(hp, 8);
  const x = lk && ptr_(lk, 0x28);
  const y = x && ptr_(x, 0x30);
  const z = y && ptr_(y, 0x38);
  if (z) {
    const vt = ptr_(z, 0);
    if (!zinfo) {
      zinfo = { z_vt: rel(vt), z_slot50: rel(ptr_(vt, 0x50)), y_vt: rel(ptr_(y, 0)), x_vt: rel(ptr_(x, 0)) };
      send({ kind: 'zinfo', zinfo });
      console.log('[hud-visible-read2] zinfo ' + JSON.stringify(zinfo));
    }
    const w = [];
    for (let o = 0x8; o < 0x100; o += 4) w.push(u32(z, o));
    s.z = w.join(',');
    const yw = [];
    for (let o = 0x8; o < 0xa0; o += 4) yw.push(u32(y, o));
    s.y = yw.join(',');
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
    console.log('[hud-visible-read2] ' + key);
    send({ kind: 'hud2', t: Date.now(), s });
  }
}, 100);
