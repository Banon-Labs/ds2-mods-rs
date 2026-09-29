// Read-only: where each copy of an armour piece's reinforcement level lives, for us and for every
// other player. The armour half of `weapon-sync-read.js`.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/armor-sync-read.js`
//
// Writes nothing. Once a second, when anything changed, it prints for the local player, per armour
// piece k = 0..3 (head, chest, hands, legs; internal equip slot 6 + k):
//   inv      the inventory entry equipped in slot 6+k: [bag + 0x25830 + (6+k)*8]
//            (0x1401b66a0 reads param_1[0x4b06 + slot] and, for slots 6..9, calls the armour
//            update 0x14037f640); +0x14 item id, +0x1e type, +0x25 & 0xF level
//   rec      the ChrAsmCtrl record table [[PlayerCtrl+0x378]+0x20]+8+(6+k)*0x14
//            (0x14037f640 writes record index FUN_14034e2e0(k) = k + 6): +0x04 id, +0x0E level
//   live     ChrAsmEquip [[PlayerCtrl+0x378]+0x28]: +0x290+k*0x30 armour id, +0x2a8+k*0x30 level,
//            +0x2b0+k*0x30 ArmorReinforceParam row (its +0x60 is the piece's max level). These are
//            what 0x1403486b0 hands the defense code (0x140380070, 0x140381350).
// then how many armour entries (type 2..5) the whole bag holds at each level, and for every remote
// PlayerCtrl its record-table armour ids and levels (records 6..9, written by the packet 62
// receiver, 0x140162150 case 0x3e).
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const PLAYER_CTRL_VTABLE = at(0x010e4bb8);
const ENTRY_COUNT = 3840;
const ENTRY_STRIDE = 0x28;
const ENTRY_ARRAY = 0x28; // ds2_rva::ITEM_ENTRY_ARRAY_OFFSET; handle at +0x1c, item id at +0x14
const say = (t) => console.log('[armor-sync-read] ' + t);

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

function armourRecords(chr) {
  const asm = ptr_(chr, 0x378);
  const table = asm && ptr_(asm, 0x20);
  if (!table) return null;
  const out = [];
  for (let k = 0; k < 4; k++) {
    const rec = table.add(8 + (6 + k) * 0x14);
    try {
      out.push({ id: rec.add(4).readS32(), cat: rec.add(8).readU8(), one: rec.add(0xc).readU16(), lvl: rec.add(0xe).readU8() });
    } catch (e) {
      out.push(null);
    }
  }
  return out;
}

function armourLive(chr) {
  const asm = ptr_(chr, 0x378);
  const eq = asm && ptr_(asm, 0x28);
  if (!eq) return null;
  const out = [];
  for (let k = 0; k < 4; k++) {
    const e = eq.add(0x290 + k * 0x30);
    try {
      const reinforce = ptr_(e, 0x20);
      out.push({ id: e.readS32(), lvl: e.add(0x18).readU8(), max: reinforce ? reinforce.add(0x60).readU8() : -1 });
    } catch (x) {
      out.push(null);
    }
  }
  return out;
}

function bag(gm) {
  const gdm = ptr_(gm, 0xa8);
  const inv = gdm && ptr_(gdm, 0x10);
  const mgr0 = inv && ptr_(inv, 0x10);
  return mgr0 && ptr_(mgr0, 0x10);
}

function census(b) {
  const counts = {};
  let total = 0;
  let raw;
  try {
    raw = new Uint8Array(b.add(ENTRY_ARRAY).readByteArray(ENTRY_COUNT * ENTRY_STRIDE));
  } catch (e) {
    return 'unreadable';
  }
  const dv = new DataView(raw.buffer);
  for (let i = 0; i < ENTRY_COUNT; i++) {
    const o = i * ENTRY_STRIDE;
    const id = dv.getUint32(o + 0x14, true);
    const handle = dv.getUint16(o + 0x1c, true);
    const type = raw[o + 0x1e];
    if (id === 0 || id === 0xffffffff || handle !== i || type < 2 || type > 5) continue;
    const lvl = raw[o + 0x25] & 0xf;
    const k = 't' + type + '+' + lvl;
    counts[k] = (counts[k] || 0) + 1;
    total++;
  }
  return 'armour entries=' + total + ' ' + JSON.stringify(counts);
}

let last = '';
function tick() {
  const gm = ptr_(at(0x16148f0), 0);
  if (!gm) { say('no GameManagerImp'); return; }
  const pc = ptr_(gm, 0xd0);
  if (!pc) { if (last !== 'none') { say('no PlayerCtrl'); last = 'none'; } return; }
  const b = bag(gm);
  const lines = ['local ' + name(pc) + ' pc=' + pc + ' bag=' + b];
  const rec = armourRecords(pc) || [];
  const lv = armourLive(pc) || [];
  for (let k = 0; k < 4; k++) {
    const entry = b && ptr_(b, 0x25830 + (6 + k) * 8);
    let inv = 'empty';
    if (entry) {
      try {
        inv = 'id=' + entry.add(0x14).readU32() + ' type=' + entry.add(0x1e).readU8() + ' lvl=' + (entry.add(0x25).readU8() & 0xf) + ' dur=' + entry.add(0x20).readFloat().toFixed(1) + ' handle=' + entry.add(0x1c).readU16() + ' index=' + (entry.sub(b.add(ENTRY_ARRAY)).toInt32() / ENTRY_STRIDE);
      } catch (e) {
        inv = 'unreadable ' + entry;
      }
    }
    const r = rec[k];
    const l = lv[k];
    lines.push('  piece' + k + ' inv{' + inv + '} rec' + (6 + k) + '{' + (r ? 'id=' + r.id + ' cat=' + r.cat + ' u16=' + r.one + ' lvl=' + r.lvl : '-') + '} live{' + (l ? 'id=' + l.id + ' lvl=' + l.lvl + ' max=' + l.max : '-') + '}');
  }
  if (b) lines.push('  ' + census(b));
  const cm = ptr_(gm, 0x18);
  const begin = cm && ptr_(cm, 0x10);
  const end = cm && ptr_(cm, 0x18);
  if (begin && end) {
    const n = Math.min(end.sub(begin).toInt32() / 8, 512);
    for (let i = 0; i < n; i++) {
      const c = ptr_(begin, i * 8);
      if (!c || c.equals(pc)) continue;
      let vt;
      try { vt = c.readPointer(); } catch (x) { continue; }
      if (!vt.equals(PLAYER_CTRL_VTABLE)) continue;
      const blk = ptr_(c, 0xb0);
      let pp = -1;
      try { pp = blk ? blk.add(0x3c).readU8() : -1; } catch (x) { pp = -1; }
      const rr = armourRecords(c) || [];
      const ll = armourLive(c) || [];
      lines.push('  remote ' + name(c) + ' pc=' + c + ' phantom=0x' + pp.toString(16) + ' rec ' + rr.map((x, j) => (6 + j) + ':' + (x ? x.id + '+' + x.lvl : '?')).join(' ') + ' live ' + ll.map((x, j) => j + ':' + (x ? x.id + '+' + x.lvl : '?')).join(' '));
    }
  }
  const text = lines.join('\n');
  if (text !== last) { say('\n' + text); last = text; send({ kind: 'snapshot', text }); }
}

tick();
setInterval(tick, 1000);
