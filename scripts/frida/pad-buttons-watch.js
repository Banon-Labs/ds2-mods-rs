// Count, per second, the pad polls that end with a nonzero button word at PadDevice+0x198.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/pad-buttons-watch.js`
//
// Hooks PadDevice::Poll (0x140f05540) outside ds2-input-harness's own detour, so onLeave sees what
// the harness left there. Twenty seconds, then detaches. Read-only.

'use strict';

const POLL = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00f05540);
const t0 = Date.now();
let last = 0;
let seen = {};
let pads = {};

const listener = Interceptor.attach(POLL, {
  onEnter(args) { this.self = args[0]; },
  onLeave() {
    const key = this.self.toString();
    pads[key] = (pads[key] || 0) + 1;
    const word = this.self.add(0x198).readU16();
    if (word !== 0) seen[key + ':0x' + word.toString(16)] = (seen[key + ':0x' + word.toString(16)] || 0) + 1;
    const now = Date.now();
    if (now - last >= 1000) {
      last = now;
      console.log('[pad-buttons] t=' + (now - t0) + ' xinput-port=' + this.self.add(0x19c).readS32() +
        ' third=' + this.self.add(0x314).readS32() + ' polls=' + JSON.stringify(pads) +
        ' nonzero=' + JSON.stringify(seen));
      seen = {};
      pads = {};
    }
    if (now - t0 > 20000) listener.detach();
  },
});
console.log('[pad-buttons] attached poll=' + POLL);
