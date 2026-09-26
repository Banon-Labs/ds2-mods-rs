// Set the local player's current HP (i32 at PlayerCtrl +0x168; max +0x170) and print it, so a
// healing effect has something to show. `--config-json '{"fraction":0.1}'` sets a tenth of max.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/set-player-hp.js`
//
// Chain: [exe+0x16148f0] GameManagerImp -> +0xd0 PlayerCtrl. Measured 2026-09-26: 2461/2461 read
// there on the test character, and a bonfire-rest SpEffect raised a lowered value.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const pc = exe.add(0x016148f0).readPointer().add(0xd0).readPointer();
const max = pc.add(0x170).readS32();
if (typeof config.fraction === 'number') {
  pc.add(0x168).writeS32(Math.max(1, Math.floor(max * config.fraction)));
}
console.log('[set-player-hp] hp=' + pc.add(0x168).readS32() + '/' + max);
