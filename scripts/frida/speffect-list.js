// Read the local player's active SpEffect actions out of ChrSpEffectCtrl. Writes nothing.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/speffect-list.js \
//   --config-json '{"watch_id":140001010,"period_ms":100,"seconds":60}'`
//
// Layout read statically (docs in `ds2_rva::SP_EFFECT_ACTION_*`):
//   ChrSpEffectCtrl +0x20 -> the action list (the object 0x14022f820 hands to 0x140220670)
//   list +0x10          inline array of 128 action pointers (DLFixedVector, 0x140221790)
//   list +0x418         u64 count
//   action +0x00        vtable (a SpEffectActionImpl_*)
//   action +0x1c        i32, returned by vtable slot 0x58 (0x1402161a0), compared with an id by
//                       0x14021fec0 / 0x14021fff0
//   action +0x10        i32, returned by vtable slot 0x40 (0x140216180)
//   action +0x20        f32, returned by slot 0x60 (0x1402161c0) unless +0x30 == 2 (then -1.0)
//   action +0x30        u8
//
// Every read is a fault-tolerant try; a hop that fails prints where the chain stopped.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const watchId = config.watch_id === undefined ? 140001010 : config.watch_id;
const period = config.period_ms || 100;
const seconds = config.seconds || 60;
const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;
const say = (text) => console.log('[speffect-list] ' + text);

function rva(p) {
  const off = p.sub(base);
  return off.compare(ptr(image.size)) < 0 && !p.isNull() ? '0x' + off.toString(16) : String(p);
}

function snapshot() {
  const manager = base.add(0x16148f0).readPointer();
  if (manager.isNull()) return { stop: 'GameManagerImp' };
  const player = manager.add(0xd0).readPointer();
  if (player.isNull()) return { stop: 'PlayerCtrl' };
  const ctrl = player.add(0x3e0).readPointer();
  if (ctrl.isNull()) return { stop: 'ChrSpEffectCtrl' };
  if (!dumped) {
    dumped = true;
    const words = [];
    for (let o = 0; o < 0x60; o += 8) words.push('+0x' + o.toString(16) + '=' + rva(ctrl.add(o).readPointer()));
    say('ctrl=' + ctrl + ' ' + words.join(' '));
    // Where is the list? It is recognised by its own shape: u64 count <= 128 at +0x418 and the
    // two 1.0f multipliers 0x140220670 writes at +0x420 / +0x428.
    const looksLikeList = (l) => {
      try {
        const n = l.add(0x418).readU64().toNumber();
        return n <= 128 && l.add(0x420).readU32() === 0x3f800000 && l.add(0x428).readU32() === 0x3f800000;
      } catch (e) {
        return false;
      }
    };
    for (let o = 0; o < 0x1000; o += 8) {
      if (looksLikeList(ctrl.add(o))) say('inline list candidate at ctrl+0x' + o.toString(16));
      let p;
      try {
        p = ctrl.add(o).readPointer();
      } catch (e) {
        continue;
      }
      if (!p.isNull() && looksLikeList(p)) say('pointer list candidate at [ctrl+0x' + o.toString(16) + '] = ' + p);
      for (let q = 0; q < 0x60; q += 8) {
        let pp;
        try {
          pp = p.add(q).readPointer();
        } catch (e) {
          break;
        }
        if (!pp.isNull() && looksLikeList(pp)) say('2-hop list candidate at [[ctrl+0x' + o.toString(16) + ']+0x' + q.toString(16) + '] = ' + pp);
      }
    }
  }
  const holder = ctrl.add(0x10).readPointer();
  if (holder.isNull()) return { stop: 'holder', ctrl };
  const list = holder.add(0x20).readPointer();
  if (list.isNull()) return { stop: 'list', ctrl };
  const count = list.add(0x418).readU64().toNumber();
  const actions = [];
  for (let i = 0; i < Math.min(count, 128); i++) {
    const a = list.add(0x10 + i * 8).readPointer();
    if (a.isNull()) {
      actions.push({ i, null: true });
      continue;
    }
    actions.push({
      i,
      vt: rva(a.readPointer()),
      f10: a.add(0x10).readS32(),
      f14: a.add(0x14).readS32(),
      f18: a.add(0x18).readS32(),
      id: a.add(0x1c).readS32(),
      t20: a.add(0x20).readFloat().toFixed(3),
      t28: a.add(0x28).readFloat().toFixed(3),
      k30: a.add(0x30).readU8(),
    });
  }
  return { ctrl, ctrlVt: rva(ctrl.readPointer()), list, listVt: rva(list.readPointer()), count, actions };
}

let first = true;
let dumped = false;
let lastKey = '';
const t0 = Date.now();
const timer = setInterval(() => {
  let s;
  try {
    s = snapshot();
  } catch (e) {
    if (lastKey !== 'fault') say('read fault: ' + e.message + ' ' + e.stack);
    lastKey = 'fault';
    return;
  }
  if (s.stop) {
    const key = 'stop:' + s.stop;
    if (key !== lastKey) say('chain stops at ' + s.stop);
    lastKey = key;
  } else {
    if (first) {
      say('ctrl=' + s.ctrl + ' vt=' + s.ctrlVt + ' list=' + s.list + ' listVt=' + s.listVt +
        ' count=' + s.count);
      for (const a of s.actions) say('  ' + JSON.stringify(a));
      first = false;
    }
    const hits = s.actions.filter((a) => a.id === watchId || a.f10 === watchId);
    const key = 'n=' + s.count + ' hits=' + JSON.stringify(hits.map((a) => [a.i, a.vt, a.f10, a.id, a.k30]));
    if (key !== lastKey) {
      say('+' + (Date.now() - t0) + 'ms count=' + s.count + ' watch=' + watchId + ' present=' +
        (hits.length > 0) + ' ' + JSON.stringify(hits));
      send({ t: Date.now() - t0, count: s.count, present: hits.length > 0, hits });
    }
    lastKey = key;
  }
  if (Date.now() - t0 > seconds * 1000) {
    say('done');
    clearInterval(timer);
  }
}, period);
say('polling every ' + period + 'ms for ' + seconds + 's, watching id ' + watchId);
