// Set the live character's soul memory (PlayerParam +0xF4 total_get_soul_1 and +0xFC
// total_get_soul_2) to SOUL_MEMORY, to make a character whose level its soul memory cannot pay for.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/set-soul-memory.js`
//
// Used to exercise ds2-soul-memory-guard's "short" verdict: import a high-level build, run this,
// save the character to a separate file with Save Game to File, and load that file back with Load
// Character from File. PlayerParam comes from the game's own getter PLAYER_PARAM_GET (0x1401ab660,
// no arguments, null at the title). Offsets: docs/DS2-SOUL-LEVEL.md.

'use strict';

const SOUL_MEMORY = 5000;
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const get = new NativeFunction(exe.add(0x001ab660), 'pointer', []);
const param = get();
if (param.isNull()) {
  console.log('[set-soul-memory] no character (PlayerParam is null)');
} else {
  const before = [param.add(0xf4).readU32(), param.add(0xfc).readU32()];
  const level = param.add(0xd0).readS32();
  param.add(0xf4).writeU32(SOUL_MEMORY);
  param.add(0xfc).writeU32(SOUL_MEMORY);
  console.log('[set-soul-memory] param=' + param + ' level=' + level + ' soul memory ' +
    JSON.stringify(before) + ' -> [' + param.add(0xf4).readU32() + ', ' +
    param.add(0xfc).readU32() + ']');
}
