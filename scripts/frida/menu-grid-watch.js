// Print every menu grid's current index when it changes, keyed by the grid object.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/menu-grid-watch.js`
//
// FEX_GRID_CURRENT_INDEX (0x140022140) answers the cursor position of whichever FexGridControl it
// is handed: the pause menu's tab strip (0..6, the seventh being ds2-menu-row's) and each tab's row
// list alike. Logging per grid lets a harness drive know the tab AND the row under the cursor
// instead of counting presses -- a counted drive on 2026-09-26 landed on Quit to Desktop. Lines:
// `[menu-grid] grid=<ptr> index=<n>`; a grid whose index follows RB/LB is the tab strip, one that
// follows the d-pad is the row list. Read-only.

'use strict';

const CURRENT_INDEX = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00022140);
const last = {};

Interceptor.attach(CURRENT_INDEX, {
  onEnter(args) { this.grid = args[0].toString(); },
  onLeave(ret) {
    const index = ret.toInt32();
    if (last[this.grid] === index) return;
    last[this.grid] = index;
    console.log('[menu-grid] grid=' + this.grid + ' index=' + index);
  },
});
console.log('[menu-grid] attached at ' + CURRENT_INDEX);
