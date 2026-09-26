// Which player stat does each item requirement key compare against, read from the running game?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/stat-row-keys.js`
//
// `FE_STAT_ROW_TABLE` (`0x14155def0`) holds 12-byte entries, one per `FE_ITEM_PARAM_TYPE` key, with
// the player-stat index as an `i16` at `+0x04` (negative: not a requirement). ds2-item-warn reads it
// for the weapon keys `0x33..0x36`; the armour badge would use `0x11..0x14` and the spell badge
// `0x42`/`0x43`. This prints the stat index for all ten once. It writes nothing.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const table = image.base.add(0x155def0);
const keys = [0x11, 0x12, 0x13, 0x14, 0x33, 0x34, 0x35, 0x36, 0x42, 0x43];
const parts = keys.map(k => '0x' + k.toString(16) + '->' + table.add(k * 12 + 4).readS16());
console.log('[stat-row-keys] ' + parts.join(' '));
