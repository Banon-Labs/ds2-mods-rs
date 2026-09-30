// What scale does DARK SOULS II draw its UI at, and which unit are FeFont's line heights in?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/ui-scale.js`
//
// docs/DS2-HP-GAUGE.md reads `0x1410ae458` as render height / 720, shared by 22 readers, and the
// .flo coordinates this repo has measured all sit on a 1280x720 canvas. docs/DS2-UI-DESIGN.md
// sizes the mods' text from FeFont_Big / FeFont_Small's `.ccm` line heights (41 / 28) on the
// assumption that they are in those same 720p units. This reads the float once a second, with the
// game window's client size beside it, so the ratio is measured instead of assumed: if the float
// times 720 equals the client height, the UI is authored at 720 lines and a 28-unit line is
// 28 * scale pixels on screen.
//
// Read-only. NO TIMER beyond one setInterval that only reads memory (the notes on wedged Frida JS
// threads in DS2 are about work done inside hooks, not about a sampler).

'use strict';

const base = Process.getModuleByName('DarkSoulsII.exe').base;
const UI_SCALE = base.add(0x010ae458);

const user32 = Process.getModuleByName('user32.dll');
const GetForegroundWindow = new NativeFunction(
  user32.getExportByName('GetForegroundWindow'), 'pointer', []);
const GetWindowThreadProcessId = new NativeFunction(
  user32.getExportByName('GetWindowThreadProcessId'), 'uint32', ['pointer', 'pointer']);
const GetClientRect = new NativeFunction(
  user32.getExportByName('GetClientRect'), 'int', ['pointer', 'pointer']);
const pid = Process.id;

// The game's own window: the foreground one if it belongs to this process, else none.
function clientSize() {
  const hwnd = GetForegroundWindow();
  if (hwnd.isNull()) {
    return null;
  }
  const owner = Memory.alloc(4);
  GetWindowThreadProcessId(hwnd, owner);
  if (owner.readU32() !== pid) {
    return null;
  }
  const rect = Memory.alloc(16);
  if (!GetClientRect(hwnd, rect)) {
    return null;
  }
  return { w: rect.add(8).readS32(), h: rect.add(12).readS32() };
}

let samples = 0;
let last = null;
setInterval(() => {
  const scale = UI_SCALE.readFloat();
  const client = clientSize();
  const key = `${scale}|${client ? client.w + 'x' + client.h : 'none'}`;
  samples += 1;
  if (key === last && samples % 10 !== 0) {
    return;
  }
  last = key;
  send({
    event: 'ui-scale',
    va: UI_SCALE.toString(),
    scale: scale,
    scale_times_720: scale * 720,
    client: client,
    matches_client_height: client ? Math.abs(scale * 720 - client.h) < 1.0 : null,
    ccm_small_px: 28 * scale,
    ccm_big_px: 41 * scale,
    samples: samples,
  });
}, 1000);
