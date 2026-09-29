// Which appearance/face readiness check keeps saying "not yet" during a hung load? Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/face-ready-probe.js`
//
// Counts calls per second and the distinct results of each candidate below, and prints once a
// second for 10 s. A poll that never finishes shows up as a steady call rate with one result.
//   0x1401528d0 (part) face-part ready: mode (via [part+8]->vtbl[0x80]()+0xe88) then [part+0x12]
//   0x140152370 (model) face-ready step; returns al
//   0x140339cd0 (model) FaceGen face object getter; returns rax
//   0x14033d3d0 (model) FaceGen face build
//   0x140151d70 (model, 0, res) copy a fixed face in

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const stats = new Map();
const note = (name, result) => {
  if (!stats.has(name)) stats.set(name, { calls: 0, results: new Map() });
  const s = stats.get(name);
  s.calls += 1;
  s.results.set(result, (s.results.get(result) || 0) + 1);
};
const mode = (model) => (model.add(0xe88).readU8() >> 3) & 3;

Interceptor.attach(exe.add(0x1528d0), {
  onEnter(args) { this.part = args[0]; },
  onLeave() {
    let m = '?';
    try {
      const owner = this.part.add(8).readPointer();
      const model = new NativeFunction(owner.readPointer().add(0x80).readPointer(), 'pointer', ['pointer'])(owner);
      m = mode(model) + ' model=' + model;
    } catch (e) { m = 'err'; }
    note('part-ready 0x1401528d0', 'mode=' + m + ' ready=' + this.part.add(0x12).readU8());
  },
});
Interceptor.attach(exe.add(0x152370), {
  onEnter(args) { this.model = args[0]; },
  onLeave(r) { note('face-step 0x140152370', 'model=' + this.model + ' mode=' + mode(this.model) + ' al=' + (r.toInt32() & 0xff)); },
});
Interceptor.attach(exe.add(0x339cd0), {
  onEnter(args) { this.model = args[0]; this.from = this.returnAddress.sub(exe); },
  onLeave(r) {
    note('facegen-get 0x140339cd0', 'from=0x' + this.from.toString(16) + ' model=' + this.model + ' ' +
      (r.isNull() ? 'null' : 'obj'));
  },
});
Interceptor.attach(exe.add(0x33d3d0), {
  onEnter(args) { note('facegen-build 0x14033d3d0', 'model=' + args[0]); },
});
Interceptor.attach(exe.add(0x151d70), {
  onEnter(args) { note('fixed-copy 0x140151d70', 'model=' + args[0]); },
});

// The other functions that branch on face mode 1 (model+0xe88 bits 0x18 == 0x08, then +0xd08).
for (const rva of [0x3395c0, 0x33a740, 0x33bf70, 0x33d120]) {
  const name = 'fn 0x' + (0x140000000 + rva).toString(16);
  Interceptor.attach(exe.add(rva), {
    onEnter(args) { this.a0 = args[0]; this.from = this.returnAddress.sub(exe); },
    onLeave(r) {
      let m = '?';
      try { m = mode(this.a0); } catch (e) { m = 'n/a'; }
      note(name, 'from=0x' + this.from.toString(16) + ' a0=' + this.a0 + ' mode=' + m + ' ret=' + r);
    },
  });
}

let ticks = 0;
const timer = setInterval(() => {
  ticks += 1;
  for (const [name, s] of stats) {
    const top = [...s.results.entries()].sort((a, b) => b[1] - a[1]).slice(0, 4)
      .map(([k, n]) => n + 'x ' + k).join(' | ');
    console.log('[ready] t=' + ticks + ' ' + name + ' calls=' + s.calls + ' ' + top);
  }
  stats.clear();
  if (ticks >= 10) {
    clearInterval(timer);
    console.log('[ready] done');
  }
}, 1000);
console.log('[ready] probing');
