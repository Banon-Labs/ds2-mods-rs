// What does the NowLoading operator do each frame while a load hangs? Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/nowloading-probe.js`
//
// The operator is [[GameManagerImp 0x16148f0] + 0x22e0] + 0xc8 (ds2-rva
// FRONTEND_NOW_LOADING_OPERATOR_OFFSET), vtable 0x1410fa0c8. Hooks every slot of that vtable,
// keeps only calls whose `this` is the operator, and prints once a second for 8 s: calls and the
// distinct return values per slot, with the caller.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const gm = exe.add(0x16148f0).readPointer();
const op = gm.add(0x22e0).readPointer().add(0xc8).readPointer();
console.log('[nl] operator=' + op + ' vtable rva=0x' + op.readPointer().sub(exe).toString(16) +
  ' +0x08..+0x40: ' + hexdump(op.add(8), { length: 0x38, header: false, ansi: false }).replace(/\n/g, ' | '));

const stats = new Map();
const slots = [0x4fdf00, 0xa7c00, 0x4c690, 0xa7bc0, 0xa7c30, 0x52510, 0x522c0, 0x500570, 0x4c6b0, 0x4c6a0, 0x4c6c0];
slots.forEach((rva, i) => {
  try { Interceptor.attach(exe.add(rva), {
    onEnter(args) { this.mine = args[0].equals(op); this.from = this.returnAddress.sub(exe); },
    onLeave(r) {
      if (!this.mine) return;
      const k = 'slot+0x' + (i * 8).toString(16) + ' fn=0x' + (0x140000000 + rva).toString(16);
      if (!stats.has(k)) stats.set(k, new Map());
      const v = 'from=0x' + this.from.toString(16) + ' ret=' + r;
      stats.get(k).set(v, (stats.get(k).get(v) || 0) + 1);
    },
  }); } catch (e) { console.log("[nl] skip 0x" + (0x140000000 + rva).toString(16) + ": " + e.message); }
});

let t = 0;
const timer = setInterval(() => {
  t += 1;
  for (const [k, m] of stats) {
    console.log('[nl] t=' + t + ' ' + k + ' ' + [...m.entries()].map(([v, n]) => n + 'x ' + v).join(' | '));
  }
  stats.clear();
  if (t >= 8) { clearInterval(timer); console.log('[nl] done'); }
}, 1000);
