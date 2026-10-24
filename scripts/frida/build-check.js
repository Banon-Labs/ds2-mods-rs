// Read-only, one shot: what the loaded character IS, to check a save written by
// scripts/ds2-sl2-builds.py against what the game made of it.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/build-check.js`
//
// Hooks nothing, sets no timer (equipped-read.js's discipline). Prints one JSON line:
//
//   level, stats      PlayerParam (+0x490 of PlayerCtrl) +0xD0 u32, nine u16 at +0x08
//   soul_memory       PlayerParam +0xF4 and +0xFC (ds2_rva PLAYER_PARAM_SOUL_MEMORY_OFFSETS)
//   hp                PlayerCtrl +0x168 current, +0x170 max (set-player-hp.js)
//   attunement_slots  bag +0x259EC u8 (ds2_rva ITEM_BAG_ATTUNEMENT_SLOTS_OFFSET)
//   equipped          every bag entry with the equipped flag: +0x14 id, +0x25 level, +0x26 infusion
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const say = (t) => console.log('[build-check] ' + t);

function ptr_(p, off) {
  try {
    const v = p.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function read() {
  const gm = ptr_(image.base.add(0x16148f0), 0);
  if (!gm) { say('no GameManagerImp -- not in game'); return; }
  const pc = ptr_(gm, 0xd0);
  const param = pc && ptr_(pc, 0x490);
  if (!param) { say('no PlayerParam -- no character loaded'); return; }
  const names = ['vigor', 'endurance', 'vitality', 'attunement', 'strength', 'dexterity',
    'intelligence', 'faith', 'adaptability'];
  const out = {
    level: param.add(0xd0).readU32(),
    stats: Object.fromEntries(names.map((n, i) => [n, param.add(0x08 + i * 2).readU16()])),
    soul_memory: [param.add(0xf4).readU32(), param.add(0xfc).readU32()],
    souls: param.add(0xec).readU32(),
    hp: [pc.add(0x168).readS32(), pc.add(0x170).readS32()],
  };
  const gdm = ptr_(gm, 0xa8);
  const inv = gdm && ptr_(gdm, 0x10);
  const mgr0 = inv && ptr_(inv, 0x10);
  const bag = mgr0 && ptr_(mgr0, 0x10);
  if (bag) {
    out.attunement_slots = bag.add(0x259ec).readU8();
    out.equipped = [];
    for (let i = 0; i < 3840; i++) {
      const e = bag.add(0x28 + i * 0x28);
      let id, flags;
      try { id = e.add(0x14).readU32(); flags = e.add(0x1f).readU8(); } catch (err) { break; }
      if (id !== 0 && id !== 0xffffffff && (flags & 0x02) !== 0) {
        out.equipped.push([id, e.add(0x25).readU8(), e.add(0x26).readU8() & 0x0f]);
      }
    }
  } else {
    out.equipped = null;
  }
  say(JSON.stringify(out));
}

read();
say('done, read-only, no hooks');
