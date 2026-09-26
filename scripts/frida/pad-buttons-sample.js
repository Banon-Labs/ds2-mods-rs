// Sample PadDevice+0x198 (XInput button word) and +0x2f8 (third-backend mask) from a timer for
// fifteen seconds, and print per-second counts of the values seen. Timer samples fall between
// frames, so they show what ds2-input-harness left after its poll detour. The pad is found with a
// one-shot hook, detached at once. Read-only.

'use strict';

const POLL = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00f05540);

function run(pad) {
  console.log('[pad-sample] pad=' + pad);
  const t0 = Date.now();
  let seen = {};
  let last = 0;
  const timer = setInterval(() => {
    const key = '198=0x' + pad.add(0x198).readU16().toString(16) + ',2f8=0x' +
      pad.add(0x2f8).readU32().toString(16);
    seen[key] = (seen[key] || 0) + 1;
    const now = Date.now() - t0;
    if (now - last >= 1000) {
      last = now;
      console.log('[pad-sample] t=' + now + ' ' + JSON.stringify(seen));
      seen = {};
    }
    if (now > 15000) clearInterval(timer);
  }, 10);
}

const finder = Interceptor.attach(POLL, {
  onEnter(args) {
    if (args[0].add(0x19c).readS32() < 0) return;
    const pad = args[0];
    finder.detach();
    setTimeout(() => run(pad), 0);
  },
});
