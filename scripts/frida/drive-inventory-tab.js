// Drive the pause menu to the Inventory tab and then to one of its category tabs, from Frida alone,
// with every step closed on a value read out of the game rather than on a press count.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/drive-inventory-tab.js \
//   --config-json '{"category":13}'`
//
// INPUT. Buttons go in as an XInput pad word: `PadDevice+0x198` after `PadDevice::Poll`
// (0x140f05540) returns, which slot 27 (0x140f04d40) tests on the XInput arm (docs/DS2-PAD-BUTTONS.md).
// With no pad plugged in the device update (0x140f16cc0) never calls the poll at all: slot +0x10
// (0x140f04de0) answers -1 while `+0x100 == 0`, `+0x19c < 0` and `+0x314 < 0`. So while a button is
// held the update's entry sets `+0x19c = 0`, the poll's exit sets it again (a failed
// XInputGetState may clear it), writes the word and returns 1 (the listener loop only runs on a
// non-zero return), and the update's exit puts `+0x19c` back. The fake pad exists only inside the
// update calls that read it.
//
// STATE. A grid's cursor is `([g+0x1e] != 0 || [g+0xd0] < 0) ? [g+0xcc] : [g+0xd0]`, the whole body
// of FEX_GRID_CURRENT_INDEX (0x140022140), so it is read from memory; hooking the getter only shows
// which grids the game is asking about, and it stops asking about a grid that is not changing.
// `FE_INGAME_MENU_DISPATCH` (0x1400a6090) says which pause-menu action ran (1 = Inventory).
// The live `FeGroupInGameMenuInventory2` has vtable 0x1410b1b38 and its category tabs are the grid
// at +0x1f78: its category getter (vtable +0x140, 0x1400ba9c0) is `tabs[cursor(this+0x1f78)]`, the
// tab list at `[this+0x2098]` (count `+0x2b8`, 0x18-byte entries from `+0x10` aligned to 8, each
// pointing at a byte that indexes the 36-entry tab table at 0x14156b120). Table index 13 is 射撃,
// the bows/crossbows tab.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const WANT_CATEGORY = typeof config.category === 'number' ? config.category : 13;

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const INV_VTABLE = at(0x010b1b38);
const PAD_VTABLE = at(0x01271aa8);
const CATEGORY_GRID = 0x1f78;

const START = 0x0010, A = 0x1000, LB = 0x0100, RB = 0x0200, LEFT = 0x0004, RIGHT = 0x0008;

let mask = 0;
let pad = null;
let savedPort = null;
let frames = 0;
let dispatched = [];
const asked = {};

const log = (s) => console.log('[drive-inv] ' + s);
const cursor = (g) => ((g.add(0x1e).readU8() !== 0 || g.add(0xd0).readS32() < 0)
  ? g.add(0xcc).readS32() : g.add(0xd0).readS32());

const fakePort = (d) => {
  const port = d.add(0x19c).readS32();
  if (port < 0) { if (savedPort === null) savedPort = port; d.add(0x19c).writeS32(0); }
};
Interceptor.attach(at(0x00f16cc0), {
  onEnter(args) {
    this.dev = args[0];
    if (!this.dev.readPointer().equals(PAD_VTABLE)) return;
    if (this.dev.add(0x314).readS32() >= 0 || !this.dev.add(0x100).readPointer().isNull()) return;
    pad = this.dev;
    frames += 1;
    if (mask !== 0) fakePort(this.dev);
  },
  onLeave() {
    if (savedPort !== null && pad !== null && this.dev.equals(pad)) {
      pad.add(0x19c).writeS32(savedPort);
      savedPort = null;
    }
  },
});
Interceptor.attach(at(0x00f05540), {
  onEnter(args) { this.dev = args[0]; },
  onLeave(ret) {
    if (mask === 0 || pad === null || !this.dev.equals(pad)) return;
    fakePort(pad);
    pad.add(0x198).writeU16(mask);
    ret.replace(ptr(1));
  },
});
Interceptor.attach(at(0x00022140), {
  onEnter(args) { asked[args[0].toString()] = { p: args[0], t: Date.now() }; },
});
Interceptor.attach(at(0x000a6090), {
  onEnter(args) { dispatched.push(args[1].toInt32()); log('dispatch action=' + args[1].toInt32()); },
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function waitFrames(n) {
  const until = frames + n;
  for (let i = 0; i < 400 && frames < until; i++) await sleep(5);
}
async function press(m, label) {
  mask = m;
  await waitFrames(4);
  mask = 0;
  await waitFrames(3);
  await sleep(400);
  log('pressed ' + label);
}
function recent() {
  const now = Date.now();
  return Object.values(asked).filter((a) => now - a.t < 500).map((a) => a.p);
}
function inventory() {
  for (const g of recent()) {
    try {
      const obj = g.sub(CATEGORY_GRID);
      if (obj.readPointer().equals(INV_VTABLE)) return obj;
    } catch (e) { /* not a readable owner */ }
  }
  return null;
}
const alive = (inv) => { try { return inv.readPointer().equals(INV_VTABLE); } catch (e) { return false; } };
function tabs(obj) {
  const repo = obj.add(0x2098).readPointer();
  const count = repo.add(0x2b8).readS32();
  let base = repo.add(0x10);
  base = base.add((8 - base.and(7).toInt32()) & 7);
  const out = [];
  for (let i = 0; i < count; i++) out.push(base.add(i * 0x18).readPointer().readU8());
  return out;
}
const show = (grids) => grids.map((g) => g + '=' + cursor(g)).join(' ') || 'none';

async function main() {
  await sleep(600);
  if (pad === null) { log('FAIL no pad device updated'); return; }
  log('pad=' + pad + ' port=' + pad.add(0x19c).readS32() + ' frames=' + frames);

  let inv = inventory();
  if (inv) { log('inventory already open at ' + inv); return categories(inv); }

  // 1. Open the pause menu unless a menu grid is already being asked about.
  let grids = recent();
  log('grids before ' + show(grids));
  if (grids.length === 0) {
    await press(START, 'START');
    await sleep(300);
    grids = recent();
    log('grids after START ' + show(grids));
    if (grids.length === 0) { log('FAIL START opened nothing'); return; }
  }
  if (grids.length !== 1) { log('FAIL expected one tab strip grid, saw ' + grids.length); return; }
  const strip = grids[0];

  // 2. Walk the strip to Inventory (tab 1).
  for (let i = 0; i < 8 && cursor(strip) !== 1; i++) {
    const was = cursor(strip);
    await press(was > 1 ? LB : RB, was > 1 ? 'LB' : 'RB');
    log('strip ' + was + ' -> ' + cursor(strip));
    if (cursor(strip) === was) { log('FAIL strip did not move'); return; }
  }
  if (cursor(strip) !== 1) { log('FAIL strip never reached 1'); return; }

  // 3. Confirm until the menu dispatches action 1 and the inventory group is live.
  dispatched = [];
  for (let i = 0; i < 3 && !inv; i++) {
    await press(A, 'A');
    await sleep(300);
    inv = inventory();
  }
  log('dispatched=' + JSON.stringify(dispatched) + ' inventory=' + inv);
  if (!inv) { log('FAIL inventory group not live'); return; }
  return categories(inv);
}

async function categories(inv) {
  // 4. Walk the category tabs to the wanted table index, one press at a time, reading the cursor
  // out of the grid and stopping on anything but a one-step move with the inventory still alive.
  const list = tabs(inv);
  const grid = inv.add(CATEGORY_GRID);
  const want = list.indexOf(WANT_CATEGORY);
  log('tabs=' + JSON.stringify(list) + ' want table ' + WANT_CATEGORY + ' at tab ' + want + ' current=' + cursor(grid));
  if (want < 0) { log('FAIL this character has no tab for table index ' + WANT_CATEGORY); return; }
  const candidates = [[RIGHT, LEFT, 'RIGHT', 'LEFT'], [RB, LB, 'RB', 'LB']];
  let pick = null;
  for (let i = 0; i < list.length && cursor(grid) !== want; i++) {
    const was = cursor(grid);
    const tries = pick ? [pick] : candidates;
    let moved = false;
    for (const c of tries) {
      const fwd = was < want;
      await press(fwd ? c[0] : c[1], fwd ? c[2] : c[3]);
      if (!alive(inv)) { log('FAIL inventory closed after ' + (fwd ? c[2] : c[3])); return; }
      const now = cursor(grid);
      log('category tab ' + was + ' -> ' + now + ' (table ' + list[now] + ')');
      if (now === was + (fwd ? 1 : -1)) { pick = c; moved = true; break; }
      if (now !== was) { log('FAIL unexpected move'); return; }
    }
    if (!moved) { log('FAIL no button moved the category tabs'); return; }
  }
  const now = cursor(grid);
  log((now === want ? 'DONE' : 'FAIL') + ' category tab=' + now + ' table index=' + list[now] +
    ' (13 = bows/crossbows)');
}

main().catch((e) => log('FAIL exception ' + e.stack));
