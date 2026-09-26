// Print the row under the cursor of one pause-menu tab group, once, and exit.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/menu-row-read.js \
//   --config-json '{"group":"0x..."}'`
//
// `group` is the tab group ds2-menu-row names in its log, e.g. the seventh tab's
// `tab served index=6 group=0x...`. The row is what FEX_GRID_CURRENT_INDEX (0x140022140) would
// answer, read directly: `+0x1e != 0 || +0xd0 < 0 ? +0xcc : +0xd0` (decompiled 2026-09-26). The row
// list does not call that function on a d-pad move, so a hook cannot see the row change; a read
// can. On the seventh tab: 0 Load Build from URL, 1 Load Character from File, 2 Save Game to File,
// 3 Quit to Desktop, and the list wraps. Read-only.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
if (!config.group) {
  console.log('[menu-row] pass --config-json \'{"group":"0x..."}\'');
} else {
  const g = ptr(config.group);
  const pinned = g.add(0x1e).readU8();
  const live = g.add(0xd0).readS32();
  const row = (pinned !== 0 || live < 0) ? g.add(0xcc).readS32() : live;
  console.log('[menu-row] group=' + g + ' row=' + row + ' (+0x1e=' + pinned + ' +0xd0=' + live +
    ' +0xcc=' + g.add(0xcc).readS32() + ')');
}
