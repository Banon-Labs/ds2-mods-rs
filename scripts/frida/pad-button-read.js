// Does the game see a START the harness authors on an XInput pad?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/pad-button-read.js`
//
// `PadDevice` vtable slot 27, `0x140f04d40`, answers "is this key id down". On the XInput path
// (`[this+0x314] < 0` and `[this+0x19c] >= 0`) it tests `[this+0x198]` against a mask taken from
// the table at `0x1415f64a0`; START is key id `0x2c`, mask `0x0010`. This logs every call for
// START -- the device, which path it is on, the button word it reads and what it answered -- up
// to a cap, so a `buttons 0x0010 12` from the harness either shows up as `down=1` or shows which
// branch ate it. Nothing else is logged: the function runs for every key of every device every
// frame, and doing work per call is what starves this game's Frida thread.

'use strict';

Interceptor.detachAll();

const image = Process.getModuleByName('DarkSoulsII.exe');
const IS_BUTTON_DOWN = image.base.add(0xf04d40);
const START = 0x2c;
const CAP = 400;

let logged = 0;
let lastLine = '';

Interceptor.attach(IS_BUTTON_DOWN, {
  onEnter(args) {
    // Integer compare first; touching anything else per call is the cost to avoid.
    this.start = args[1].toInt32() === START && logged < CAP;
    if (this.start) {
      this.device = args[0];
    }
  },
  onLeave(retval) {
    if (!this.start) {
      return;
    }
    const d = this.device;
    const line = 'device=' + d + ' third=' + d.add(0x314).readS32() + ' port=' + d.add(0x19c).readS32() +
      ' buttons=0x' + d.add(0x198).readU16().toString(16) + ' down=' + (retval.toInt32() & 0xff);
    if (line !== lastLine) {
      console.log('[pad-button-read] ' + line);
      lastLine = line;
      logged += 1;
    }
  },
});

console.log('[pad-button-read] hooked ' + IS_BUTTON_DOWN);
