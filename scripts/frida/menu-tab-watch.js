// Print the pause menu's selected tab each time it changes.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/menu-tab-watch.js`
//
// FUN_140022140(this) answers the top menu's display index (docs/DS2-INGAME-MENU.md), 0..5 for
// the game's tabs and 6 for ds2-menu-row's seventh. Logging its answer on change tells a harness
// drive which tab it is on without looking at the screen. Read-only.

'use strict';

const TAB_INDEX = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00022140);
let last = -1;

Interceptor.attach(TAB_INDEX, {
  onLeave(ret) {
    const index = ret.toInt32();
    if (index === last) return;
    last = index;
    console.log('[menu-tab] index=' + index);
  },
});
console.log('[menu-tab] attached at ' + TAB_INDEX);
