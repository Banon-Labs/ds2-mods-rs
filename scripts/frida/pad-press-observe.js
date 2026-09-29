// Press pad buttons one at a time through a faked XInput pad and report, for each press, which
// menu grids' cursors moved and whether any pause-menu action ran.
//
// `... --agent scripts/frida/pad-press-observe.js --config-json '{"presses":["DOWN","UP"]}'`
//
// The fake pad is drive-inventory-tab.js's: `+0x19c` set to 0 at the device update's entry
// (0x140f16cc0) so the poll runs with no pad plugged in, the button word written to `+0x198` after
// the poll (0x140f05540) with a return of 1, and `+0x19c` restored at the update's exit. A grid's
// cursor is read from memory with FEX_GRID_CURRENT_INDEX's own body (0x140022140). Every grid the
// game asked about in the run is diffed, not just the recent ones, so a grid that stops being asked
// about after a press still shows up with its value.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const NAMES = { UP: 0x1, DOWN: 0x2, LEFT: 0x4, RIGHT: 0x8, START: 0x10, BACK: 0x20,
  LB: 0x100, RB: 0x200, A: 0x1000, B: 0x2000, X: 0x4000, Y: 0x8000 };
const presses = (config.presses || []).map((n) => [n, NAMES[n]]);

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const PAD_VTABLE = at(0x01271aa8);

let mask = 0;
let pad = null;
let savedPort = null;
let frames = 0;
let dispatched = [];
const grids = {};

const log = (s) => console.log('[press-observe] ' + s);
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
Interceptor.attach(at(0x00022140), { onEnter(args) { grids[args[0].toString()] = args[0]; } });

// FexGridControl's input slot (0x1400236d0): rcx is the grid, r9 the input event. The event's
// +0 word is "pressed this frame" and +4 "held/repeat"; bits 1/2/4/8 are up/down/left/right
// (read from its body at 0x1400237c0..0x140023880). Every call that carries a direction bit is
// logged once per press with the grid and the return addresses above it, so the route from the
// menu to the grid is read off the stack instead of guessed.
let traced = [];
// The menu event builder 0x14010bc50 ORs event bits from a u64 action word, `[0x1404ffb10()+8]`,
// through the table at 0x141567a40 (UP<-bit 33, DOWN<-34, LEFT<-35, RIGHT<-36). Every distinct
// word seen during a press is logged with the object and its vtable.
let actionWords = {};
Interceptor.attach(at(0x004ffb10), {
  onLeave(ret) {
    if (mask === 0 || ret.isNull()) return;
    try {
      const key = 'obj=' + ret + ' vtable=' + where(ret.readPointer()) + ' word=0x' + ret.add(8).readU64().toString(16);
      actionWords[key] = true;
    } catch (e) { /* unreadable */ }
  },
});
const where = (p) => (p.compare(exe) >= 0 && p.compare(exe.add(0x2000000)) < 0
  ? '0x' + p.sub(exe).add(0x140000000).toString(16) : p.toString());
// FeItemSelectMenu's input slot (vtable +0x10, 0x1400bbea0): r8 is the same 0x1c-byte event. Its
// whole event and a deep stack say which code turned the pad into it.
Interceptor.attach(at(0x000bbea0), {
  onEnter(args) {
    if (mask === 0) return;
    const ev = args[2];
    if (((ev.readU32() | ev.add(4).readU32()) & 0xf) === 0) return;
    const words = [];
    for (let i = 0; i < 7; i++) words.push('0x' + ev.add(i * 4).readU32().toString(16));
    const frames = Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 14).map(where);
    const line = 'inventory-input this=' + args[0] + ' event=[' + words.join(' ') + '] stack=' + frames.join(' < ');
    if (traced.indexOf(line) < 0) traced.push(line);
  },
});
Interceptor.attach(at(0x000236d0), {
  onEnter(args) {
    if (mask === 0) return;
    const ev = args[3];
    const pressed = ev.readU32(), held = ev.add(4).readU32();
    if (((pressed | held) & 0xf) === 0) return;
    const frames = Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 6)
      .map((p) => (p.compare(exe) >= 0 && p.compare(exe.add(0x2000000)) < 0 ? '0x' + p.sub(exe).add(0x140000000).toString(16) : p.toString()));
    const line = 'grid=' + args[0] + ' pressed=0x' + pressed.toString(16) + ' held=0x' + held.toString(16) + ' stack=' + frames.join(' < ');
    if (traced.indexOf(line) < 0) traced.push(line);
  },
});
Interceptor.attach(at(0x000a6090), { onEnter(args) { dispatched.push(args[1].toInt32()); } });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function waitFrames(n) {
  const until = frames + n;
  for (let i = 0; i < 400 && frames < until; i++) await sleep(5);
}
function snapshot() {
  const out = {};
  for (const k in grids) { try { out[k] = cursor(grids[k]); } catch (e) { out[k] = 'unreadable'; } }
  return out;
}

async function main() {
  await sleep(800);
  if (pad === null) { log('FAIL no pad device updated'); return; }
  log('grids at start ' + JSON.stringify(snapshot()));
  for (const [name, bits] of presses) {
    if (bits === undefined) { log('FAIL unknown button ' + name); return; }
    const before = snapshot();
    dispatched = [];
    traced = [];
    actionWords = {};
    mask = bits;
    await waitFrames(4);
    mask = 0;
    await waitFrames(3);
    await sleep(500);
    const after = snapshot();
    const moved = [];
    for (const k in after) if (before[k] !== after[k]) moved.push(k + ' ' + before[k] + '->' + after[k]);
    log(name + ': moved ' + (moved.length ? moved.join(', ') : 'nothing') +
      (dispatched.length ? ' dispatch=' + JSON.stringify(dispatched) : ''));
    for (const t of traced) log('  ' + t);
    for (const w in actionWords) log('  action ' + w);
  }
}

main().catch((e) => log('FAIL exception ' + e.stack));
