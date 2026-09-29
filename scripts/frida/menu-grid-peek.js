// Read, without pressing anything, where the pause menu's cursors are: every grid the game asks
// FEX_GRID_CURRENT_INDEX (0x140022140) about in one second, with its index read straight out of the
// grid, plus any grids named in the config. Read-only.
//
// `... --agent scripts/frida/menu-grid-peek.js --config-json '{"grids":["0x..."],"inventory":"0x..."}'`
//
// The getter is a pure read -- `([g+0x1e] != 0 || [g+0xd0] < 0) ? [g+0xcc] : [g+0xd0]` -- so the
// index is read here from memory rather than by calling it, and a grid the game is not currently
// asking about still reads correctly. `inventory` is a FeGroupInGameMenuInventory2 pointer: its
// vtable (0x1410b1b38) is checked and its category tab grid at +0x1f78 is read.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const index = (g) => ((g.add(0x1e).readU8() !== 0 || g.add(0xd0).readS32() < 0)
  ? g.add(0xcc).readS32() : g.add(0xd0).readS32());

for (const g of config.grids || []) {
  try {
    const p = ptr(g);
    // +0xc8 item count (what the cursor may reach), +0xd4 drawable extent (docs/DS2-INGAME-MENU.md);
    // +0x1e / +0xcc / +0xd0 are the fields the index getter chooses between.
    console.log('[grid-peek] named ' + g + ' index=' + index(p) + ' 1e=' + p.add(0x1e).readU8() +
      ' c8=' + p.add(0xc8).readS32() + ' cc=' + p.add(0xcc).readS32() + ' d0=' + p.add(0xd0).readS32() +
      ' d4=' + p.add(0xd4).readS32());
  } catch (e) { console.log('[grid-peek] named ' + g + ' unreadable'); }
}
if (config.inventory) {
  const inv = ptr(config.inventory);
  try {
    const live = inv.readPointer().equals(exe.add(0x010b1b38));
    console.log('[grid-peek] inventory ' + inv + ' vtable-ok=' + live + (live ? ' category-tab=' + index(inv.add(0x1f78)) : ''));
  } catch (e) { console.log('[grid-peek] inventory ' + inv + ' unreadable'); }
}
const asked = {};
const hook = Interceptor.attach(exe.add(0x00022140), { onEnter(args) { asked[args[0].toString()] = args[0]; } });
setTimeout(() => {
  hook.detach();
  const parts = Object.keys(asked).map((k) => k + '=' + index(asked[k]));
  console.log('[grid-peek] asked in 1s: ' + (parts.length ? parts.join(' ') : 'none'));
}, 1000);
