// Open one of the pause menu's added rows from Frida alone -- by default the seventh tab's Build
// Recommender -- with every step closed on a value read out of the game rather than on a press
// count, so a run with no controller plugged in can still reach a panel the rows open.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/drive-menu-row.js \
//   --config-json '{"tab":6,"row":3,"expect":"ds2-build-recommender: panel open"}'`
//
// INPUT is drive-inventory-tab.js's faked XInput pad, unchanged: while a button is held the device
// update's entry (0x140f16cc0) sets `PadDevice+0x19c = 0` so the poll runs with no pad plugged in,
// the poll's exit (0x140f05540) writes the button word to `+0x198` and returns 1, and the update's
// exit puts `+0x19c` back. See that file for why each step is needed.
//
// STATE. The tab strip's cursor is read the way drive-inventory-tab.js reads it, off the one grid
// FEX_GRID_CURRENT_INDEX (0x140022140) is asked about while the menu is up. The seventh tab's ROW
// list never asks that function on a d-pad move, so its row is read from the tab group directly:
// `+0x1e != 0 || +0xd0 < 0 ? +0xcc : +0xd0` (menu-row-read.js). The group's address is the one
// `ds2-menu-row` logs as `tab offered to top select ... group=0x...` when the menu is built, read
// here out of `ds2-loader.log` beside the executable. A is pressed only once that read says the
// cursor is on `row`, because the next row down is Quit Game. `expect` is a line the row's own
// action logs, and the run ends DONE only when that line is in the log.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const WANT_TAB = typeof config.tab === 'number' ? config.tab : 6;
const WANT_ROW = typeof config.row === 'number' ? config.row : 3;
const EXPECT = typeof config.expect === 'string' ? config.expect : 'ds2-build-recommender: panel open';

const exe = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => exe.base.add(rva);
const PAD_VTABLE = at(0x01271aa8);
const LOG_PATH = exe.path.replace(/[^\\\/]*$/, '') + 'ds2-loader.log';

const START = 0x0010, A = 0x1000, LB = 0x0100, RB = 0x0200, UP = 0x0001, DOWN = 0x0002;

let mask = 0;
let pad = null;
let savedPort = null;
let frames = 0;
const asked = {};

const log = (s) => console.log('[drive-row] ' + s);
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
const show = (grids) => grids.map((g) => g + '=' + cursor(g)).join(' ') || 'none';
function logText() {
  try { return File.readAllText(LOG_PATH); } catch (e) { return ''; }
}
function lastGroup() {
  const found = [...logText().matchAll(/tab offered to top select 0x[0-9a-f]+ group=(0x[0-9a-f]+)/g)];
  return found.length ? ptr(found[found.length - 1][1]) : null;
}

async function main() {
  await sleep(600);
  if (pad === null) { log('FAIL no pad device updated'); return; }
  log('pad=' + pad + ' port=' + pad.add(0x19c).readS32() + ' frames=' + frames + ' log=' + LOG_PATH);
  const before = logText().split(EXPECT).length - 1;

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

  // 2. Walk the strip to the tab.
  for (let i = 0; i < 10 && cursor(strip) !== WANT_TAB; i++) {
    const was = cursor(strip);
    await press(was > WANT_TAB ? LB : RB, was > WANT_TAB ? 'LB' : 'RB');
    log('strip ' + was + ' -> ' + cursor(strip));
    if (cursor(strip) === was) { log('FAIL strip did not move'); return; }
  }
  if (cursor(strip) !== WANT_TAB) { log('FAIL strip never reached ' + WANT_TAB); return; }

  // 3. The row, read off the group ds2-menu-row built for this menu.
  const group = lastGroup();
  if (group === null) { log('FAIL no "tab offered" group in ' + LOG_PATH); return; }
  log('group=' + group + ' row=' + cursor(group));
  for (let i = 0; i < 12 && cursor(group) !== WANT_ROW; i++) {
    const was = cursor(group);
    const down = was < WANT_ROW;
    await press(down ? DOWN : UP, down ? 'DOWN' : 'UP');
    log('row ' + was + ' -> ' + cursor(group));
    if (cursor(group) === was) { log('FAIL row did not move'); return; }
  }
  if (cursor(group) !== WANT_ROW) { log('FAIL row never reached ' + WANT_ROW); return; }

  // 4. Confirm, and wait for the row's own action to say it ran.
  await press(A, 'A');
  for (let i = 0; i < 20; i++) {
    const now = logText().split(EXPECT).length - 1;
    if (now > before) { log('DONE tab=' + WANT_TAB + ' row=' + WANT_ROW + ' -- the log says "' + EXPECT + '"'); return; }
    await sleep(250);
  }
  log('FAIL pressed A on row ' + WANT_ROW + ' but the log never said "' + EXPECT + '"');
}

main().catch((e) => log('FAIL exception ' + e.stack));
