// Read-only: where each copy of a weapon's upgrade level lives, for us and for every other player.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/weapon-sync-read.js`
//
// Writes nothing. Once a second it prints, for the local player:
//   inv[i]   the inventory entry equipped in internal weapon slot i (0..5), read two ways:
//            bag = [[[[GameManagerImp]+0xA8]+0x10]+0x10]+0x10 (two +0x10 hops), entry = [bag + 0x25830 + i*8]
//            (0x1401b66a0 reads param_1[0x4b06 + slot]); level = entry+0x25 & 0xF
//   rec[r]   the ChrAsmCtrl record table [[PlayerCtrl+0x378]+0x20]+8+r*0x14, r = [1,0,3,2,5,4][i]
//            (+0x04 item id, +0x0E level, +0x0F infusion)
//   live[n]  ChrAsmEquip [[PlayerCtrl+0x378]+0x28] + n*0x48: +0x50 item id, +0x70 level
// and, for every remote PlayerCtrl in the roster, its record-table weapon levels.
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const PLAYER_CTRL_VTABLE = at(0x010e4bb8);
const SLOT_TO_RECORD = [1, 0, 3, 2, 5, 4];
const say = (t) => console.log('[weapon-sync-read] ' + t);

function ptr_(p, off) {
  try {
    const v = p.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function name(chr) {
  try {
    const s = chr.add(0x118);
    const len = s.add(0x10).readU64().toNumber();
    const cap = s.add(0x18).readU64().toNumber();
    if (len === 0 || len > 64) return '';
    return (cap > 7 ? s.readPointer() : s).readUtf16String(len);
  } catch (e) {
    return '?';
  }
}

function records(chr) {
  const asm = ptr_(chr, 0x378);
  if (!asm) return null;
  const table = ptr_(asm, 0x20);
  if (!table) return null;
  const out = [];
  for (let r = 0; r < 6; r++) {
    const rec = table.add(8 + r * 0x14);
    out.push({ id: rec.add(4).readU32(), cat: rec.add(8).readU32(), lvl: rec.add(0xe).readU8(), inf: rec.add(0xf).readU8() });
  }
  return out;
}

function live(chr) {
  const asm = ptr_(chr, 0x378);
  if (!asm) return null;
  const eq = ptr_(asm, 0x28);
  if (!eq) return null;
  const out = [];
  for (let n = 0; n < 6; n++) {
    const e = eq.add(n * 0x48);
    out.push({ id: e.add(0x50).readU32(), lvl: e.add(0x70).readU8(), inf: e.add(0x71).readU8() });
  }
  return out;
}

let last = '';
function tick() {
  const gm = ptr_(at(0x16148f0), 0);
  if (!gm) { say('no GameManagerImp'); return; }
  const pc = ptr_(gm, 0xd0);
  if (!pc) { say('no PlayerCtrl'); return; }
  const lines = [];
  const gdm = ptr_(gm, 0xa8);
  const inv = gdm && ptr_(gdm, 0x10);
  const mgr0 = inv && ptr_(inv, 0x10);
  const mgr = mgr0 && ptr_(mgr0, 0x10);
  const invLv = [];
  for (let i = 0; i < 6; i++) {
    const entry = mgr && ptr_(mgr, 0x25830 + i * 8);
    let ok = false;
    try { ok = entry !== null && entry.add(0x25).readU8() >= 0; } catch (e) { ok = false; }
    invLv.push(!entry ? 'empty' : !ok ? 'unreadable ' + entry : ('entry=' + entry + ' flags1f=0x' + entry.add(0x1f).readU8().toString(16) + ' id=' + entry.add(0x14).readU32() + ' type=' + entry.add(0x1e).readU8() + ' lvl=' + (entry.add(0x25).readU8() & 0xf) + ' raw25=0x' + entry.add(0x25).readU8().toString(16)));
  }
  lines.push('local ' + name(pc) + ' pc=' + pc + ' inv=' + inv + ' mgr=' + mgr);
  const rec = records(pc) || [];
  const lv = live(pc) || [];
  for (let i = 0; i < 6; i++) {
    const r = rec[SLOT_TO_RECORD[i]];
    lines.push('  slot' + i + ' inv{' + invLv[i] + '} rec' + SLOT_TO_RECORD[i] + '{' + (r ? 'id=' + r.id + ' cat=' + r.cat + ' lvl=' + r.lvl + ' inf=' + r.inf : '-') + '}');
  }
  lines.push('  live ' + lv.map((e, n) => n + ':' + e.id + '+' + e.lvl).join(' '));
  const cm = ptr_(gm, 0x18);
  if (cm) {
    const b = ptr_(cm, 0x10), e = ptr_(cm, 0x18);
    if (b && e) {
      const n = Math.min(e.sub(b).toInt32() / 8, 512);
      for (let k = 0; k < n; k++) {
        const c = ptr_(b, k * 8);
        if (!c || c.equals(pc)) continue;
        let vt;
        try { vt = c.readPointer(); } catch (x) { continue; }
        if (!vt.equals(PLAYER_CTRL_VTABLE)) continue;
        const blk = ptr_(c, 0xb0);
        const pp = blk ? blk.add(0x3c).readU8() : -1;
        const rr = records(c) || [];
        lines.push('  remote ' + name(c) + ' pc=' + c + ' phantom=0x' + pp.toString(16) + ' rec ' + rr.map((x, j) => j + ':' + x.id + '+' + x.lvl).join(' '));
      }
    }
  }
  const text = lines.join('\n');
  if (text !== last) { say('\n' + text); last = text; send({ kind: 'snapshot', text }); }
}

tick();
setInterval(tick, 1000);
