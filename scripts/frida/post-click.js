// Post one right-click to DARK SOULS II's own window, and report what the input blocks made of it.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/post-click.js`
//
// For checking ds2-input-harness's click block from outside: with a `block` running, the press
// should arrive at the fold already turned into a release, and bit 0x2 of each block's +0x228
// should stay clear. Without a block it should be set. Right button is guard, the least eventful
// press in-world. It observes the fold with Interceptor.attach and writes nothing into the game.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;

const FOLD_EVENT = 0xb08ef0;
const KATANA_MAIN_APP_SLOT = 0x16751f8;

const postMessage = new NativeFunction(
  Process.getModuleByName('user32.dll').getExportByName('PostMessageW'),
  'int', ['pointer', 'uint', 'pointer', 'pointer']);

function readPtr(p) {
  try {
    const v = p.readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function gameWindow() {
  const app = readPtr(base.add(KATANA_MAIN_APP_SLOT));
  const input = app === null ? null : readPtr(app.add(0x60));
  const cursor = input === null ? null : readPtr(input.add(8));
  const first = cursor === null ? null : readPtr(cursor.add(0xd0));
  const device = first === null ? null : readPtr(first);
  return device === null ? null : readPtr(device.add(0x20));
}

const seen = [];

Interceptor.attach(base.add(FOLD_EVENT), {
  onEnter(args) {
    this.block = args[0];
    this.type = args[1].readU32();
  },
  onLeave() {
    if (this.type === 9) return;
    seen.push('type' + this.type + ':0x228&2=' + (this.block.add(0x228).readU32() & 2));
    if (seen.length === 4) console.log('[post-click] ' + JSON.stringify(seen));
  },
});

const hwnd = gameWindow();
if (hwnd === null) {
  console.log('[post-click] no game window found; nothing posted');
} else {
  const lparam = ptr((200 << 16) | 200);
  postMessage(hwnd, 0x204, ptr(0x2), lparam);
  postMessage(hwnd, 0x205, ptr(0), lparam);
  console.log('[post-click] posted a right-click to hwnd ' + hwnd);
}
