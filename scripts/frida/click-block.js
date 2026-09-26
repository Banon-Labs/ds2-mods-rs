// Can a block stop DARK SOULS II's mouse clicks at the event the window procedure hands the input
// blocks?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/click-block.js`
//
// Clicks do not go through the DirectInput mouse the harness blanks. The window procedure's app
// handler (FUN_1402ef110) passes each mouse event to FUN_140af41a0, which calls FUN_140b08ef0 once
// for each of the two input blocks (stride 0x288). That function folds the event into the block's
// button word at +0x228: event 0/2 set bit 0x1 (left down/double), 1 clears it; 3/4 set 0x2 (right),
// 5 clears it; 6/7 set 0x4 (middle), 8 clears it; 9 is a move, 10 a wheel.
//
// The candidate block: rewrite a press into the release of the same button, so the button word
// ends up clear. This posts a right-button press and release to the game's own window twice --
// once untouched, as the control, and once with the rewrite on -- and reports bit 0x2 of +0x228
// after each press. Right button is guard, the least eventful thing to press in-world. Nothing
// here needs the window focused or a hand on the mouse.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;

const FOLD_EVENT = 0xb08ef0;
const KATANA_MAIN_APP_SLOT = 0x16751f8;
const WM_RBUTTONDOWN = 0x204;
const WM_RBUTTONUP = 0x205;
const MK_RBUTTON = 0x2;

const RELEASE_OF = { 0: 1, 2: 1, 3: 5, 4: 5, 6: 8, 7: 8 };

const postMessage = new NativeFunction(
  Process.getModuleByName('user32.dll').getExportByName('PostMessageW'),
  'int', ['pointer', 'uint', 'pointer', 'pointer']);

function log(line) {
  console.log('[click-block] ' + line);
}

function readPtr(p) {
  try {
    const v = p.readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

// The game window, from the WindowsMouseDevice the pointer update reads: cursorObj+0xd0 is its
// device vector, and the device keeps the HWND it clamps against at +0x20.
function gameWindow() {
  const app = readPtr(base.add(KATANA_MAIN_APP_SLOT));
  const input = app === null ? null : readPtr(app.add(0x60));
  const cursor = input === null ? null : readPtr(input.add(8));
  const first = cursor === null ? null : readPtr(cursor.add(0xd0));
  const device = first === null ? null : readPtr(first);
  return device === null ? null : readPtr(device.add(0x20));
}

let phase = 'control';
let rewrite = false;
let releases = 0;
const seen = [];

function post(hwnd) {
  const lparam = ptr((200 << 16) | 200);
  postMessage(hwnd, WM_RBUTTONDOWN, ptr(MK_RBUTTON), lparam);
  postMessage(hwnd, WM_RBUTTONUP, ptr(0), lparam);
}

Interceptor.attach(base.add(FOLD_EVENT), {
  onEnter(args) {
    this.block = args[0];
    const event = args[1];
    const type = event.readU32();
    this.type = type;
    if (rewrite && type in RELEASE_OF) {
      event.writeU32(RELEASE_OF[type]);
      event.add(4).writeU8(event.add(4).readU8() & 0xf0);
      this.rewritten = true;
    }
  },
  onLeave() {
    if (this.type === 9) return;
    const word = this.block.add(0x228).readU32();
    seen.push(phase + ':type' + this.type + (this.rewritten ? '->released' : '') +
      ':0x228&2=' + (word & 2));
    // Each event is folded into both input blocks, so a pass is over at its second release.
    if (this.type === 5) releases += 1;
    if (phase === 'control' && releases === 2) {
      phase = 'blocked';
      rewrite = true;
      post(gameWindow());
    } else if (phase === 'blocked' && releases === 4) {
      phase = 'done';
      rewrite = false;
      log('events ' + JSON.stringify(seen));
    } else if (seen.length > 40 && phase !== 'done') {
      phase = 'done';
      rewrite = false;
      log('gave up after 40 events: ' + JSON.stringify(seen));
    }
  },
});

// Whether the posted message reaches the game's window procedure at all.
Interceptor.attach(base.add(0x2eac00), {
  onEnter(args) {
    const msg = args[1].toUInt32();
    if (msg === WM_RBUTTONDOWN || msg === WM_RBUTTONUP) {
      log('window procedure got 0x' + msg.toString(16) + ' hwnd ' + args[0]);
    }
  },
});

const hwnd = gameWindow();
if (hwnd === null) {
  log('no game window found through the WindowsMouseDevice; nothing posted');
} else {
  log('posting the control right-click to hwnd ' + hwnd);
  post(hwnd);
}
