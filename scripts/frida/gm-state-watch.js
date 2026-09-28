// GameManagerImp's state machine across a load. Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/gm-state-watch.js`
//
// The update dispatches `call [0x1410c49e0 + state*8]` (0x1401c3320..0x1401c3332) and reads the
// next state from +0x24ac. Measured 2026-09-28 with a hardware watchpoint on the loading screen's
// visible flag: state 31 (0x1401c0210) showed it, state 28 (0x1401bf200) hid it. Polls the dwords
// at +0x24a0..+0x24c0 every 50 ms and logs each change with the time since attach.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const gm = exe.add(0x16148f0).readPointer();
const OFF = 0x24a0;
const N = 8;
const t0 = Date.now();
const read = () => Array.from({ length: N }, (_, i) => gm.add(OFF + i * 4).readS32());
let last = read();
console.log('[gm] ' + gm + ' +0x24a0..: ' + last.join(' '));
setInterval(() => {
  const now = read();
  for (let i = 0; i < N; i++) {
    if (now[i] !== last[i]) {
      console.log('[gm] t=' + ((Date.now() - t0) / 1000).toFixed(2) + 's +0x' + (OFF + i * 4).toString(16) + ' ' + last[i] + ' -> ' + now[i]);
    }
  }
  last = now;
}, 50);
