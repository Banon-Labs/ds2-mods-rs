// Which FeStateFlow substate is resident right now? Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/state-flow-probe.js`
//
// FeStateFlow::update (ds2-rva FE_STATE_FLOW_UPDATE, 0x140104540) reads the resident substate
// from [this+0x10]. For 5 s, prints each distinct (flow, resident vtable RVA) and how often it ran;
// name the vtables with scripts/ds2-rtti.py.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const seen = new Map();
Interceptor.attach(exe.add(0x104540), {
  onEnter(args) {
    try {
      const resident = args[0].add(0x10).readPointer();
      const k = 'flow=' + args[0] + ' resident=' + resident + ' vtable=0x' +
        (resident.isNull() ? '0' : resident.readPointer().sub(exe).toString(16)) +
        ' from=0x' + this.returnAddress.sub(exe).toString(16);
      seen.set(k, (seen.get(k) || 0) + 1);
    } catch (e) {
      seen.set('err ' + e.message, 1);
    }
  },
});
setTimeout(() => {
  for (const [k, n] of seen) console.log('[flow] ' + n + 'x ' + k);
  console.log('[flow] done');
}, 5000);
