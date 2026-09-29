// Change the player's face id while in the world; the change shows after the next bonfire warp.
//
// MEASURED 2026-09-28: writing the id alone reloads nothing. A bonfire warp re-runs the copy init
// (0x140339f60) on the player's model, ChrAsmCtrl+0x30, which reads the equip again: with face-id.js
// attached at face 8100 the warp loaded FC_8100; with face-id.js detached and the id set back to
// 0 (`{"face_id": 0}`), the warp put the player back on the FaceGen slider face.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/face-id-live.js --config-json '{"face_id": 8100}'`
//
// Player equip: [[GameManagerImp 0x16148f0] + 0xd0] -> +0x378 ChrAsmCtrl -> +0x28 equip (face-id.js).
// Logs the equip's type and face id before the write, the player's models' mode, and every face
// file load (0x140345050) for the next 10 s.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const say = (t) => console.log('[face-live] ' + t);

Interceptor.attach(exe.add(0x345050), {
  onEnter(args) {
    say('face file load id=' + args[2].toInt32() + ' sex=' + (args[3].toInt32() & 0xff));
  },
});

const player = exe.add(0x16148f0).readPointer().add(0xd0).readPointer();
const equip = player.add(0x378).readPointer().add(0x28).readPointer();
say('equip=' + equip + ' type=' + equip.add(0x1c).readS32() + ' face=' + equip.add(0x30).readS32());
if (Number.isInteger(config.face_id)) {
  equip.add(0x30).writeS32(config.face_id);
  say('face id now ' + equip.add(0x30).readS32());
}
