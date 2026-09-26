// Find the live FeGroupInGameMenuInventory2 from one of its grids and print its category getter.
//
// `... --agent scripts/frida/inventory-category.js --config-json '{"grid":"0x..."}'`
//
// The object is 0x3ee8 bytes with its primary vtable 0x1410b1b38 at +0 (docs/DS2-INVENTORY-SORT.md);
// a grid inside it is found by scanning back for that vtable. Slot +0x140 returns the category;
// its first instructions are printed so the field it reads can be named. Read-only.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const VTABLE = exe.add(0x010b1b38);
const grid = ptr(config.grid);
let found = null;
for (let off = 0; off < 0x3ee8; off += 8) {
  const at = grid.sub(off);
  try {
    if (at.readPointer().equals(VTABLE)) { found = at; break; }
  } catch (e) { break; }
}
if (!found) {
  console.log('[inv-cat] no Inventory2 vtable within 0x3ee8 before ' + grid);
} else {
  console.log('[inv-cat] object=' + found + ' grid at +' + grid.sub(found));
  let pc = VTABLE.add(0x140).readPointer();
  console.log('[inv-cat] slot +0x140 = ' + pc + ' (rva ' + pc.sub(exe) + ')');
  for (let i = 0; i < 12; i++) {
    const ins = Instruction.parse(pc);
    console.log('[inv-cat]   ' + pc.sub(exe) + ' ' + ins.toString());
    if (ins.mnemonic === 'ret') break;
    pc = ins.next;
  }
}
