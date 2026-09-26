// Sample, every 10 ms from attach until the title menu settles, the signals a loading bar would be
// driven by, and log each one when it changes -- the first measurement docs/DS2-LOADING-BAR.md asks
// for.
//
// `python3 scripts/ds2-frida-up.py --allow-early` as soon as the process exists, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/loading-bar-signals.js`
//
// Signals, all read (nothing is hooked, so no hook can collide with boot-timeline's):
//   gmi       GameManagerImp at [exe+0x16148f0] (null until the manager exists)
//   frames    the per-frame counter at gmi+0x104 (advances once per presented frame)
//   operator  [[gmi+0x22e0]+0xd0], its state at +0x30 and flow at +0x38
//   substate  [flow+0x10]+0x0c, the resident substate's id
// Every line carries ms since attach. Once a second while frames advance it prints the frame rate,
// so "does the game present during sound/Katana init" is answered by whether frame lines appear
// before the operator does. Stops after STOP_MS. Read-only.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const GMI_SLOT = exe.add(0x016148f0);
const STOP_MS = 90000;
const t0 = Date.now();
const last = {};
let lastFrames = null;
let lastFrameReport = 0;
let framesAtReport = 0;

function readPtr(p) {
  try { return p.readPointer(); } catch (e) { return null; }
}
function readU32(p) {
  try { return p.readU32(); } catch (e) { return null; }
}
function note(key, value) {
  if (last[key] === value) return;
  last[key] = value;
  console.log('[boot-signals] t=' + (Date.now() - t0) + 'ms ' + key + '=' + value);
}

const timer = setInterval(() => {
  const now = Date.now() - t0;
  if (now > STOP_MS) {
    clearInterval(timer);
    console.log('[boot-signals] stopped at ' + now + 'ms');
    return;
  }
  const gmi = readPtr(GMI_SLOT);
  note('gmi', gmi === null || gmi.isNull() ? 'null' : 'set');
  if (gmi === null || gmi.isNull()) return;

  const frames = readU32(gmi.add(0x104));
  if (frames !== null) {
    if (lastFrames === null || frames !== lastFrames) note('frames-advancing', 'yes');
    if (lastFrames === null) { framesAtReport = frames; lastFrameReport = now; }
    if (now - lastFrameReport >= 1000) {
      const rate = (frames - framesAtReport) * 1000 / (now - lastFrameReport);
      if (rate > 0) console.log('[boot-signals] t=' + now + 'ms fps=' + rate.toFixed(1));
      framesAtReport = frames;
      lastFrameReport = now;
    }
    lastFrames = frames;
  }

  const holder = readPtr(gmi.add(0x22e0));
  if (holder === null || holder.isNull()) { note('operator', 'none'); return; }
  const op = readPtr(holder.add(0xd0));
  if (op === null || op.isNull()) { note('operator', 'none'); return; }
  note('operator', 'present');
  note('operator-state', readU32(op.add(0x30)));
  const flow = readPtr(op.add(0x38));
  if (flow === null || flow.isNull()) { note('substate', 'no-flow'); return; }
  const resident = readPtr(flow.add(0x10));
  if (resident === null || resident.isNull()) { note('substate', 'none'); return; }
  const id = readU32(resident.add(0x0c));
  note('substate', id === null ? 'unreadable' : '0x' + id.toString(16));
}, 10);

console.log('[boot-signals] attached');
