// Count executions of ds2-input-harness's pad stores in the live DLL: the block's zeroing of
// +0x2f8 (DINPUT8.dll+0xe5fde) and +0x198 (+0xe6016), the arm-check reads (+0xe62ae) and the
// authored button store (+0xe62d9). Offsets are for the f9fcc9c build. Prints once a second for
// twenty seconds. Read-only.

'use strict';

const dll = Process.getModuleByName('DINPUT8.dll');
const sites = { 'poll-entry': 0xe5ed0, 'block-2f8': 0xe5fde, 'block-198': 0xe6016, 'arm-check': 0xe62ae, 'store-198': 0xe62d9 };
const hits = {};
const listeners = [];
for (const [name, off] of Object.entries(sites)) {
  hits[name] = 0;
  listeners.push(Interceptor.attach(dll.base.add(off), function () { hits[name] += 1; }));
}
const t0 = Date.now();
const timer = setInterval(() => {
  console.log('[store-hits] t=' + (Date.now() - t0) + ' ' + JSON.stringify(hits));
  for (const k of Object.keys(hits)) hits[k] = 0;
  if (Date.now() - t0 > 20000) {
    clearInterval(timer);
    listeners.forEach((l) => l.detach());
  }
}, 1000);
console.log('[store-hits] attached');
