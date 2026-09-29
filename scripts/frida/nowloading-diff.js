// Which FeOperatorNowLoading fields change across a load? Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/nowloading-diff.js`
//
// The operator is [[GameManagerImp 0x16148f0] + 0x22e0] + 0xc8 (ds2-rva). Every 100 ms, compares
// its 0x3c0 bytes, dword by dword, with the last read and logs each dword that changed, with the
// time since attach. Dwords that change on most polls (timers) are muted after their third change.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const op = exe.add(0x16148f0).readPointer().add(0x22e0).readPointer().add(0xc8).readPointer();
const LEN = 0x3c0;
const t0 = Date.now();
let last = new Uint32Array(op.readByteArray(LEN));
const changes = new Map();
console.log('[nld] op=' + op + ' watching');
setInterval(() => {
  const now = new Uint32Array(op.readByteArray(LEN));
  for (let i = 0; i < now.length; i++) {
    if (now[i] === last[i]) continue;
    const n = (changes.get(i) || 0) + 1;
    changes.set(i, n);
    if (n <= 3) {
      console.log('[nld] t=' + ((Date.now() - t0) / 1000).toFixed(1) + 's +0x' + (i * 4).toString(16) +
        ' 0x' + last[i].toString(16) + ' -> 0x' + now[i].toString(16) + (n === 3 ? ' (muted from here)' : ''));
    }
  }
  last = now;
}, 100);
