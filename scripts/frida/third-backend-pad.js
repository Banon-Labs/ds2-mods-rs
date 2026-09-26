// Put the live pad on DS2's third input backend for twelve seconds, with a stub backend holding a
// button, so ds2-input-harness's non-XInput branch can be measured on a machine with an XInput pad.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/third-backend-pad.js`
// and drive `block` / `buttons` through scripts/ds2-harness.sh while it runs.
//
// PadDevice::Poll (0x140f05540) takes the third-backend arm when +0x314 >= 0: it calls
// [this+0x340](index, buf[0x78]) and on a zero return copies buf+0 into +0x2f8, the mask the key test
// reads on that arm (0x140f05a64). The stub answers 0 with STUB_MASK in buf+0, centred sticks
// (buf+4..7 = 0x80) and "not connected" at buf+0x4c, so the arm's other paths stay idle.
//
// The pad is found with a one-shot hook that is detached at once. The arm is then switched from a
// timer, not from a per-poll hook: a Frida hook on the poll lands inside ds2-input-harness's MinHook
// detour (the entry's `jmp` is left alone), so its onLeave runs before the harness writes and would
// report the game's value rather than what the harness left. Sampling from a timer between frames
// sees the harness's writes. Afterwards +0x314 and +0x340 are put back.

'use strict';

const POLL = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00f05540);
const STUB_MASK = 0x1;
const LENGTH_MS = 12000;
const SAMPLE_MS = 20;

let calls = 0;
const stub = new NativeCallback((index, buf) => {
  calls += 1;
  buf.writeByteArray(new Array(0x78).fill(0));
  buf.writeU32(STUB_MASK);
  buf.add(4).writeByteArray([0x80, 0x80, 0x80, 0x80]);
  return 0;
}, 'int', ['int', 'pointer']);

function run(pad) {
  const saved = { fn: pad.add(0x340).readPointer(), third: pad.add(0x314).readS32() };
  console.log('[third-backend] pad=' + pad + ' xinput-port=' + pad.add(0x19c).readS32() +
    ' third=' + saved.third);
  pad.add(0x340).writePointer(stub);
  pad.add(0x314).writeS32(0);
  console.log('[third-backend] on: +0x314=0, +0x340=stub, mask 0x' + STUB_MASK.toString(16));
  const t0 = Date.now();
  let zero = 0;
  let nonzero = 0;
  let lastReport = 0;
  const timer = setInterval(() => {
    const now = Date.now() - t0;
    if (pad.add(0x2f8).readU32() === 0) zero += 1; else nonzero += 1;
    if (now - lastReport >= 1000) {
      lastReport = now;
      console.log('[third-backend] t=' + now + 'ms +0x2f8=0x' +
        pad.add(0x2f8).readU32().toString(16) + ' +0x198=0x' +
        pad.add(0x198).readU16().toString(16) + ' +0x314=' + pad.add(0x314).readS32() +
        ' stub-calls=' + calls + ' samples zero=' + zero + ' nonzero=' + nonzero);
    }
    if (now >= LENGTH_MS) {
      clearInterval(timer);
      pad.add(0x314).writeS32(saved.third);
      pad.add(0x340).writePointer(saved.fn);
      console.log('[third-backend] off: restored +0x314=' + saved.third + ' +0x340=' + saved.fn +
        ' stub-calls=' + calls + ' samples zero=' + zero + ' nonzero=' + nonzero);
    }
  }, SAMPLE_MS);
}

// `--config-json '{"pad":"0x..."}'` acts on that pad directly -- use the pointer the harness's own
// `status` line names (`pad=0x...`). Without it the pad is found by hooking the poll, and on
// 2026-09-26 that found a different object from the one the harness writes.
const config = globalThis.__ER_FRIDA_CONFIG || {};
if (config.pad) {
  setTimeout(() => run(ptr(config.pad)), 0);
  console.log('[third-backend] using the harness pad ' + config.pad);
} else {
  const finder = Interceptor.attach(POLL, {
    onEnter(args) {
      if (args[0].add(0x19c).readS32() < 0) return;
      const pad = args[0];
      finder.detach();
      setTimeout(() => run(pad), 0);
    },
  });
  console.log('[third-backend] attached poll=' + POLL);
}
