// Every virtual call on one object, with return values, for a few seconds. Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/model-vtable-probe.js
//  --config-json '{"this": "0x7fffd959ade0", "slots": [0x33a240, ...]}'`
//
// Hooks each slot RVA, keeps calls whose rcx is `this`, and prints once a second for 6 s:
// per slot, calls and the distinct (caller, return) pairs. Slots too short to hook are skipped.

'use strict';

const cfg = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const self = ptr(cfg.this);
const stats = new Map();
(cfg.slots || []).forEach((rva, i) => {
  const name = 'slot+0x' + (i * 8).toString(16) + ' 0x' + (0x140000000 + rva).toString(16);
  try {
    Interceptor.attach(exe.add(rva), {
      onEnter(args) { this.mine = args[0].equals(self); this.from = this.returnAddress.sub(exe); },
      onLeave(r) {
        if (!this.mine) return;
        if (!stats.has(name)) stats.set(name, new Map());
        const k = 'from=0x' + this.from.toString(16) + ' ret=' + r;
        stats.get(name).set(k, (stats.get(name).get(k) || 0) + 1);
      },
    });
  } catch (e) {
    console.log('[vt] skip ' + name + ': ' + e.message);
  }
});
let t = 0;
const timer = setInterval(() => {
  t += 1;
  for (const [k, m] of stats) console.log('[vt] t=' + t + ' ' + k + ' ' + [...m.entries()].map(([v, n]) => n + 'x ' + v).join(' | '));
  stats.clear();
  if (t >= 6) { clearInterval(timer); console.log('[vt] done'); }
}, 1000);
