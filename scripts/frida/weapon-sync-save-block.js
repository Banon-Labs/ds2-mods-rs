// Read-only: how the inventory entries the pause menu reads relate to the block the save writes.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/weapon-sync-save-block.js`
//
// Writes nothing and hooks nothing. The save's inventory writer, SaveDataItemInventory2 vtable
// slot 2 (0x1402e53f0), streams 0x100bc bytes from `mgr + 0x30`, where mgr = [ItemInventory2 +
// 0x10] (0x1401aba20 -> 0x1401a6020). The pause menu reads the bag entries at [mgr + 0x10] + 0x28.
// The question: is a bag entry's level (+0x25) inside that block, or a separate copy? Printed:
//   * mgr, bag, the block's range, and whether the bag or any equipped entry lies inside it;
//   * every equipped weapon entry (bag + 0x25830 + i*8): its address, its 0x28 raw bytes;
//   * every place in the block that holds that entry's item id, with 16 bytes either side;
//   * how many weapon entries (type 0/1) the bag holds, and how many carry the not-in-pack bit.
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const say = (t) => console.log('[weapon-sync-save-block] ' + t);
const BLOCK_SIZE = 0x100bc;

function p(base, off) {
  try {
    const v = base.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function hex(ptr, len) {
  try {
    return Array.from(new Uint8Array(ptr.readByteArray(len)))
      .map((b) => b.toString(16).padStart(2, '0'))
      .join(' ');
  } catch (e) {
    return 'unreadable';
  }
}

function inside(x, lo, len) {
  return x !== null && x.compare(lo) >= 0 && x.compare(lo.add(len)) < 0;
}

function run() {
  const gm = p(at(0x16148f0), 0);
  const gdm = gm && p(gm, 0xa8);
  const inv = gdm && p(gdm, 0x10);
  const mgr = inv && p(inv, 0x10);
  const bag = mgr && p(mgr, 0x10);
  if (!bag) {
    say('no bag yet: gm=' + gm + ' inv=' + inv + ' mgr=' + mgr);
    return;
  }
  const block = mgr.add(0x30);
  say('mgr=' + mgr + ' bag=' + bag + ' [mgr+0x28]=' + p(mgr, 0x28) + ' block=' + block + '..' + block.add(BLOCK_SIZE) +
    ' bag-in-block=' + inside(bag, block, BLOCK_SIZE) + ' block-in-bag=' + inside(block, bag, 0x25900));
  const bytes = new Uint8Array(block.readByteArray(BLOCK_SIZE));
  const dv = new DataView(bytes.buffer);
  for (let i = 0; i < 6; i++) {
    const e = p(bag, 0x25830 + i * 8);
    if (!e) {
      say('slot' + i + ' empty');
      continue;
    }
    const id = e.add(0x14).readU32();
    const index = e.sub(bag.add(0x28)).toInt32() / 0x28;
    say('slot' + i + ' entry=' + e + ' index=' + index + ' in-block=' + inside(e, block, BLOCK_SIZE) + ' id=' + id +
      ' lvl=' + (e.add(0x25).readU8() & 0xf) + ' raw=' + hex(e, 0x28));
    let hits = 0;
    for (let o = 0; o + 4 <= BLOCK_SIZE && hits < 6; o += 1) {
      if (dv.getUint32(o, true) === id) {
        hits++;
        const lo = Math.max(0, o - 16);
        say('  block+0x' + o.toString(16) + ': ' + hex(block.add(lo), Math.min(48, BLOCK_SIZE - lo)) + ' (from +0x' + lo.toString(16) + ')');
      }
    }
    if (hits === 0) say('  item id not in the block');
  }
  let weapons = 0;
  let stored = 0;
  let firstRaw = [];
  for (let k = 0; k < 3840; k++) {
    const e = bag.add(0x28 + k * 0x28);
    const id = e.add(0x14).readU32();
    const type = e.add(0x1e).readU8();
    if (id === 0 || id === 0xffffffff || type > 1) continue;
    weapons++;
    if (e.add(0x1f).readU8() & 0x04) stored++;
    if (firstRaw.length < 4) firstRaw.push('#' + k + ' ' + hex(e, 0x28));
  }
  say('bag weapon entries=' + weapons + ' not-in-pack=' + stored);
  firstRaw.forEach((r) => say('  ' + r));
  say('block head: ' + hex(block, 0x60));
  send({ kind: 'done' });
}

run();
