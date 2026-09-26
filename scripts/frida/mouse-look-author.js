// Does a delta added to DLUID::MouseDevice+0x108 after its poll turn DARK SOULS II's camera?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/mouse-look-author.js`
//
// docs/DS2-MOUSE-LOOK.md reads the camera's mouse-look out of the binary: the DirectInput poll
// (0x140f074a0) writes relative X/Y as floats at +0x108/+0x10c, the device's Update maps them into
// cursorObj+0x18/+0x1c, and the camera stage turns on those only while BaseP1+0x30 (window active)
// and BaseP1+0x133 (keyboard/mouse enabled) are set. The one link it could not trace instruction by
// instruction is the mapping's read of +0x108. This measures it, and the turn it produces.
//
// It does nothing until the window is active. Then it needs QUIET_FRAMES frames in which the
// poll's own +0x108/+0x10c are zero (nobody touching the mouse), and the camera heading taken
// across that stretch is the control: how far the camera drifts with no input at all. Then it adds
// DX to +0x108 for INJECT_FRAMES frames, still checking the poll's own value every frame -- a
// non-zero one means a hand was on the mouse, and the result is reported as contaminated rather
// than as a measurement. The heading is read again SETTLE_FRAMES after the last write.
//
// Timers do not fire in this game's Frida runtime, so everything runs from the poll's own onLeave.
// It hooks with Interceptor.attach only; hot-reloading an Interceptor.replace killed the game once.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;

const MOUSE_DEVICE_POLL = 0xf074a0;
const KATANA_MAIN_APP_SLOT = 0x16751f8;
const GAME_MANAGER_IMP_SLOT = 0x16148f0;
const GAME_MANAGER_CAMERA_MANAGER_OFFSET = 0x20;
const CAMERA_MANAGER_OPERATOR_OFFSETS = [0x18, 0x20, 0x28];
const CAMERA_OPERATOR_VIEW_OFFSET = 0x10;

const DX = 20.0;
const QUIET_FRAMES = 60;
const INJECT_FRAMES = 60;
const SETTLE_FRAMES = 30;

function readPtr(p) {
  try {
    const value = p.readPointer();
    return value.isNull() ? null : value;
  } catch (e) {
    return null;
  }
}

function gates() {
  const app = readPtr(base.add(KATANA_MAIN_APP_SLOT));
  if (app === null) return null;
  const input = readPtr(app.add(0x60));
  const cursor = input === null ? null : readPtr(input.add(8));
  return {
    active: app.add(0x30).readU8(),
    enabled: app.add(0x133).readU8(),
    cursor,
  };
}

// Heading of every camera operator's view, in degrees. The view is row-vector, so the camera's
// forward axis in world space is the rotation's third column.
function headings() {
  const manager = readPtr(base.add(GAME_MANAGER_IMP_SLOT));
  const cameras = manager === null ? null : readPtr(manager.add(GAME_MANAGER_CAMERA_MANAGER_OFFSET));
  const out = {};
  if (cameras === null) return out;
  for (const offset of CAMERA_MANAGER_OPERATOR_OFFSETS) {
    const operator = readPtr(cameras.add(offset));
    if (operator === null) continue;
    const m = operator.add(CAMERA_OPERATOR_VIEW_OFFSET);
    const fx = m.add(2 * 4).readFloat();
    const fz = m.add(10 * 4).readFloat();
    out['op+0x' + offset.toString(16)] = Math.atan2(fx, fz) * 180 / Math.PI;
  }
  return out;
}

function wrap(d) {
  let x = d % 360;
  if (x > 180) x -= 360;
  if (x < -180) x += 360;
  return x;
}

function diff(a, b) {
  const out = {};
  for (const key of Object.keys(a)) {
    if (key in b) out[key] = wrap(b[key] - a[key]).toFixed(2);
  }
  return JSON.stringify(out);
}

function fmt(h) {
  const out = {};
  for (const key of Object.keys(h)) out[key] = h[key].toFixed(2);
  return JSON.stringify(out);
}

let phase = 'wait-focus';
let count = 0;
let quietStart = null;
let injectStart = null;
let contaminated = 0;
let cursorSeen = [];
let polls = 0;

function log(line) {
  console.log('[mouse-look-author] ' + line);
}

log('attached; waiting for the window to be active (BaseP1+0x30) with mouse-look enabled (+0x133)');

Interceptor.attach(base.add(MOUSE_DEVICE_POLL), {
  onEnter(args) {
    this.device = args[0];
  },
  onLeave() {
    polls += 1;
    const device = this.device;
    if (device.isNull()) return;
    const g = gates();
    if (g === null) return;
    const ownX = device.add(0x108).readFloat();
    const ownY = device.add(0x10c).readFloat();
    if (polls === 1 || polls % 300 === 0) {
      log('poll ' + polls + ' phase=' + phase + ' active=' + g.active + ' enabled=' + g.enabled +
        ' own=(' + ownX + ',' + ownY + ')');
    }

    if (phase === 'wait-focus') {
      if (g.active && g.enabled) {
        phase = 'quiet';
        count = 0;
        quietStart = null;
        log('gates open at poll ' + polls + '; need ' + QUIET_FRAMES + ' frames with no hand on the mouse');
      }
      return;
    }
    if (phase === 'done') return;

    if (!g.active || !g.enabled) {
      log('gate closed during ' + phase + ' (active=' + g.active + ' enabled=' + g.enabled + '); starting over');
      phase = 'wait-focus';
      return;
    }

    if (phase === 'quiet') {
      if (ownX !== 0 || ownY !== 0) {
        count = 0;
        quietStart = null;
        return;
      }
      if (quietStart === null) quietStart = headings();
      count += 1;
      if (count >= QUIET_FRAMES) {
        injectStart = headings();
        log('control: ' + QUIET_FRAMES + ' quiet frames drifted ' + diff(quietStart, injectStart) +
          ' deg; heading ' + fmt(injectStart));
        phase = 'inject';
        count = 0;
        contaminated = 0;
        cursorSeen = [];
      }
      return;
    }

    if (phase === 'inject') {
      if (ownX !== 0 || ownY !== 0) contaminated += 1;
      if (g.cursor !== null && count < 4) {
        // The previous frame's mapped value: this frame's mapping has not run yet.
        cursorSeen.push(g.cursor.add(0x18).readFloat().toFixed(2));
      }
      device.add(0x108).writeFloat(ownX + DX);
      count += 1;
      if (count >= INJECT_FRAMES) {
        phase = 'settle';
        count = 0;
      }
      return;
    }

    if (phase === 'settle') {
      if (count === 0 && g.cursor !== null) {
        cursorSeen.push('last=' + g.cursor.add(0x18).readFloat().toFixed(2));
      }
      if (ownX !== 0 || ownY !== 0) contaminated += 1;
      count += 1;
      if (count >= SETTLE_FRAMES) {
        const after = headings();
        log('injected +' + DX + ' at +0x108 for ' + INJECT_FRAMES + ' frames: heading moved ' +
          diff(injectStart, after) + ' deg; cursorObj+0x18 during injection ' +
          JSON.stringify(cursorSeen) + '; frames with a hand on the mouse ' + contaminated +
          (contaminated === 0 ? ' (clean)' : ' (CONTAMINATED, discard)'));
        phase = 'done';
      }
    }
  },
});
