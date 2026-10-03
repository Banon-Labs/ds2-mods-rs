// Read-only probe: after openCharaMakerWindow, is FeOperatorTestCharaMaking's update (0x1400e3060)
// being called? Prints the frontend root's mode, the creator operator and its first bytes, the
// root's operator array, and counts update and nav-update (0x140baeb20) calls over two seconds.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/chara-op-probe.js --role probe`

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const root = at(0x16148f0).readPointer().add(0x22e0).readPointer();
const op = root.add(0xe8).readPointer();
console.log('[probe] mode ' + root.add(0x32c).readU32() + ' op ' + op + ' root+0x3b8 0x' +
  root.add(0x3b8).readU32().toString(16));
// The update waits for op+0x2c == 1, a text-manager idle check (0x140504d90 on [0x141616cb0]) and its
// resource object's state +0x34 == 2 before it builds the scene.
if (!op.isNull()) {
  const q = [];
  for (let o = 0; o < 0x30; o += 8) q.push('+0x' + o.toString(16) + '=' + op.add(o).readPointer());
  console.log('[probe] op ' + q.join(' ') + ' +0x28=' + op.add(0x28).readU8() + ' +0x2c=' + op.add(0x2c).readU32());
  for (let o = 0x8; o < 0x28; o += 8) {
    const p = op.add(o).readPointer();
    try {
      console.log('[probe] op+0x' + o.toString(16) + ' -> +0x34=' + p.add(0x34).readU32() + ' f0..f8 ' +
        p.add(0xf8).readPointer().sub(p.add(0xf0).readPointer()));
    } catch (e) {
      console.log('[probe] op+0x' + o.toString(16) + ' unreadable');
    }
  }
}
// The resource object (0x130 bytes, ctor 0x140affe10): every qword, and any UTF-16 string it points at.
if (!op.isNull() && !op.add(0x10).readPointer().isNull()) {
  const res = op.add(0x10).readPointer();
  for (let o = 0; o < 0x130; o += 8) {
    const v = res.add(o).readPointer();
    let s = '';
    try {
      const t = v.readUtf16String(64);
      if (t && /^[\x20-\x7e]{3,}$/.test(t)) s = ' "' + t + '"';
    } catch (e) { /* not a pointer */ }
    console.log('[probe] res+0x' + o.toString(16) + ' ' + v + s);
  }
}
const textMgr = at(0x1616cb0).readPointer();
console.log('[probe] text manager ' + textMgr + (textMgr.isNull() ? '' : ' busy=' +
  (new NativeFunction(at(0x504d90), 'uint8', ['pointer'])(textMgr))));
const ops = [];
for (let i = 0; i < 5; i++) ops.push(root.add(0x38 + i * 8).readPointer().toString());
console.log('[probe] root ops +0x38: ' + ops.join(' ') + ' +0x58 ' + root.add(0x58).readPointer());
let updates = 0;
let navs = 0;
const h1 = Interceptor.attach(at(0xe3060), { onEnter() { updates++; } });
const h2 = Interceptor.attach(at(0xbaeb20), { onEnter() { navs++; } });
setTimeout(() => {
  h1.detach();
  h2.detach();
  console.log('[probe] in 2s: chara op update calls ' + updates + ', nav updates ' + navs);
}, 2000);
