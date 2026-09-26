// What does the frontend stat table hold right now?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/stat-table.js`
//
// docs/DS2-ITEM-REQUIREMENTS.md reads the table at [[GameManagerImp]+0x22e0]+0x138 (stride 0x18,
// the value an i32 at the start of each entry) as a copy of the EFFECTIVE stat block, written by
// FUN_14003ebe0 from the frontend root's update. This prints entries 4..13 once a second -- 8..11
// are STR/DEX/INT/FTH -- so a change of rings or a buff shows up here within a frame or so if the
// reading is right. Read-only; hooks nothing.

'use strict';

const base = Process.getModuleByName('DarkSoulsII.exe').base;
const GAME_MANAGER_IMP = base.add(0x016148f0);
const ROOT_OFFSET = 0x22e0;
const TABLE_OFFSET = 0x138;
const STRIDE = 0x18;
const NAMES = { 8: 'STR', 9: 'DEX', 10: 'INT', 11: 'FTH' };

function sample(tag) {
  try {
    const gmi = GAME_MANAGER_IMP.readPointer();
    const root = gmi.add(ROOT_OFFSET).readPointer();
    const table = root.add(TABLE_OFFSET).readPointer();
    const values = [];
    for (let i = 4; i <= 13; i++) {
      const v = table.add(i * STRIDE).readS32();
      values.push((NAMES[i] || String(i)) + '=' + v);
    }
    console.log('[stat-table] ' + tag + ' table=' + table + ' ' + values.join(' '));
  } catch (e) {
    console.log('[stat-table] ' + tag + ' unreadable: ' + e.message);
  }
}

sample('load');
let n = 0;
setInterval(function () {
  n += 1;
  sample(String(n));
}, 1000);
