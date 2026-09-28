// Read-only, one shot: what the local player has equipped, and their soul level.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/equipped-read.js`
//
// Writes nothing, hooks nothing, sets no timer. The two recorded attach freezes (2026-09-26) were both
// agents that installed Interceptor hooks on functions the game calls every frame; read-only agents
// attached cleanly many times. This reads once, at load, on Frida's own thread, and says so.
//
//   bag    = [[[[GameManagerImp]+0xA8]+0x10]+0x10]+0x10, the chain weapon-sync-read.js proved
//   entry  = bag + 0x28 + i*0x28, i < 3840 (ds2_rva ITEM_ENTRY_*): +0x14 item id, +0x1F flags,
//            bit 0x02 equipped
//   SL     = [[GameManagerImp]+0xD0] (PlayerCtrl) + 0x490 (PlayerParam) + 0xD0, u32
//            (ds2_rva PLAYER_PARAM_OFFSET, PLAYER_PARAM_SOUL_LEVEL_OFFSET)
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const say = (t) => console.log('[equipped-read] ' + t);

function ptr_(p, off) {
  try {
    const v = p.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function read() {
  const gm = ptr_(at(0x16148f0), 0);
  if (!gm) { say('no GameManagerImp -- not in game'); return; }
  const pc = ptr_(gm, 0xd0);
  const param = pc && ptr_(pc, 0x490);
  let sl = null;
  try { sl = param ? param.add(0xd0).readU32() : null; } catch (e) { sl = null; }
  say('soul level ' + sl);
  // ds2_rva PLAYER_PARAM_STAT_OFFSETS / _NAMES: nine u16 at +0x08..+0x18, in the game's order.
  const names = ['vigor', 'endurance', 'vitality', 'attunement', 'strength', 'dexterity',
    'intelligence', 'faith', 'adaptability'];
  try {
    if (param) say('stats ' + names.map((n, i) => n + '=' + param.add(0x08 + i * 2).readU16()).join(' '));
  } catch (e) { say('stats unreadable'); }
  const gdm = ptr_(gm, 0xa8);
  const inv = gdm && ptr_(gdm, 0x10);
  const mgr0 = inv && ptr_(inv, 0x10);
  const bag = mgr0 && ptr_(mgr0, 0x10);
  if (!bag) { say('no inventory bag'); return; }
  const ids = [];
  for (let i = 0; i < 3840; i++) {
    const e = bag.add(0x28 + i * 0x28);
    let id, flags;
    try { id = e.add(0x14).readU32(); flags = e.add(0x1f).readU8(); } catch (err) { break; }
    if (id !== 0 && id !== 0xffffffff && (flags & 0x02) !== 0) ids.push(id);
  }
  say('equipped ids ' + ids.join(','));
}

read();
say('done, read-only, no hooks');
