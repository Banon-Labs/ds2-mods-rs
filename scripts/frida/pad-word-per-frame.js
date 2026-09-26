// Read the pad's +0x198 button word once per frame from inside NvNavigationSystem::Update
// (0x140baeb20, game thread, once per frame), which is not an input poll and not hooked by
// ds2-input-harness, so no hook ordering can hide what the harness left there. The pad pointer
// comes from a one-shot hook on the poll. Counts values per second for twenty seconds. Read-only.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const POLL = exe.add(0x00f05540);
const NAV = exe.add(0x00baeb20);
let pad = null;
const finder = Interceptor.attach(POLL, {
  onEnter(args) {
    if (args[0].add(0x19c).readS32() < 0) return;
    pad = args[0];
    finder.detach();
  },
});
const t0 = Date.now();
let seen = {};
let last = 0;
const nav = Interceptor.attach(NAV, {
  onEnter() {
    if (pad === null) return;
    const key = '0x' + pad.add(0x198).readU16().toString(16);
    seen[key] = (seen[key] || 0) + 1;
    const now = Date.now() - t0;
    if (now - last >= 1000) {
      last = now;
      console.log('[pad-word] t=' + now + ' ' + JSON.stringify(seen));
      seen = {};
    }
    if (now > 20000) nav.detach();
  },
});
console.log('[pad-word] attached');
