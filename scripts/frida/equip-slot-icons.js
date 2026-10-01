// Which icons the pause menu's Equipment page asks for as it opens, read off the game's own icon
// requests. Written to find the art an EMPTY slot shows, on the guess that it was an item
// category's icon (`Item_Category/IC_CA_%05d.tpf`, kind 12).
//
// MEASURED 2026-09-30, and the guess was wrong. Opening the page (dispatch action 0) asked for
// kind 6 -- the equipped items, each by item id, from `0x3052e` -- plus kinds 0 and 15, and never
// for kind 12. An empty slot's silhouette is not an icon at all: it is a frame of a layout sprite,
// which `ds2_rva::FE_EQUIP_EMPTY_ART` traces from the page's setup code.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/equip-slot-icons.js`
//
// INPUT is drive-inventory-tab.js's faked XInput pad, unchanged (see that file): START opens the
// pause menu, LB/RB walk its tab strip to tab 0, and A opens it. Each step is closed on a value read
// out of the game: the strip's cursor off the one grid FEX_GRID_CURRENT_INDEX (0x140022140) is
// asked about, and the pause menu's own dispatch (0x1400a6090) naming the action that ran.
//
// WHAT IS LOGGED. Every icon key `{kind u32, id u32}` from the moment A is pressed:
// * `seticon` -- FUN_14001e200(holder, &key), the one call every icon component is set through
//   (its callers were read statically: it wraps FUN_14002fb80, which wraps the FeIconProxy
//   maker FUN_14002fa50);
// * `acquire` -- FUN_14002fdd0(cache, &key, flags), the icon texture cache every proxy reads
//   through, which also sees a key a proxy was already holding.
// Kind 12 is an item category: its id is a `MenuCategoryIconParam` row and a tab of the 36-entry
// table at 0x14156b120 (150 = shield, 400..430 = head, chest, hands, legs, 500 = ring,
// 600 = arrows and bolts). Kinds 0..10 are item icons, `IC_%010d.tpf`. The caller's RVA is logged
// with each, so the code that chose a category can be read statically afterwards. Read-only.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
// `{"drive": false}` presses nothing and records from the start, for a screen opened by hand: every
// key names itself and the dispatch hook names the screen, so no press of ours is needed to read
// them.
const DRIVE = config.drive !== false;

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const PAD_VTABLE = at(0x01271aa8);
const START = 0x0010, A = 0x1000, LB = 0x0100, RB = 0x0200;
const WANT_TAB = 0;

let mask = 0;
let pad = null;
let savedPort = null;
let frames = 0;
let dispatched = [];
let recording = !DRIVE;
let seq = 0;
const asked = {};

const log = (s) => console.log('[equip-icons] ' + s);
const cursor = (g) => ((g.add(0x1e).readU8() !== 0 || g.add(0xd0).readS32() < 0)
  ? g.add(0xcc).readS32() : g.add(0xd0).readS32());
const rva = (p) => '0x' + p.sub(exe).toString(16);

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

function key(p) {
  try {
    return { kind: p.readU32(), id: p.add(4).readU32() };
  } catch (e) {
    return null;
  }
}
Interceptor.attach(at(0x0001e200), {
  onEnter(args) {
    if (!recording) return;
    const k = key(args[1]);
    if (k) log(`#${++seq} seticon kind=${k.kind} id=${k.id} holder=${args[0]} from=${rva(this.returnAddress)}`);
  },
});
Interceptor.attach(at(0x0002fdd0), {
  onEnter(args) {
    if (!recording) return;
    const k = key(args[1]);
    if (k) log(`#${++seq} acquire kind=${k.kind} id=${k.id} from=${rva(this.returnAddress)}`);
  },
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

async function main() {
  await sleep(600);
  if (pad === null) { log('FAIL no pad device updated'); return; }
  log('pad=' + pad + ' frames=' + frames);
  let grids = recent();
  if (grids.length === 0) {
    await press(START, 'START');
    await sleep(300);
    grids = recent();
    if (grids.length === 0) { log('FAIL START opened nothing'); return; }
  }
  if (grids.length !== 1) { log('FAIL expected one tab strip grid, saw ' + grids.length); return; }
  const strip = grids[0];
  for (let i = 0; i < 8 && cursor(strip) !== WANT_TAB; i++) {
    const was = cursor(strip);
    await press(was > WANT_TAB ? LB : RB, was > WANT_TAB ? 'LB' : 'RB');
    log('strip ' + was + ' -> ' + cursor(strip));
    if (cursor(strip) === was) { log('FAIL strip did not move'); return; }
  }
  if (cursor(strip) !== WANT_TAB) { log('FAIL strip never reached ' + WANT_TAB); return; }
  dispatched = [];
  recording = true;
  await press(A, 'A');
  await sleep(3000);
  recording = false;
  log(`DONE dispatched=${JSON.stringify(dispatched)} keys=${seq}`);
}

if (DRIVE) {
  main().catch((e) => log('FAIL exception ' + e.stack));
} else {
  log('recording without driving: open the screen by hand');
}
