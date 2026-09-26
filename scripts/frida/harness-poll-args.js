// Count calls into ds2-input-harness's shared poll body (DINPUT8.dll+0xe5ed0 on the f9fcc9c build)
// by their first two arguments, to see which device indices reach it. Ten seconds. Read-only.

'use strict';

const dll = Process.getModuleByName('DINPUT8.dll');
const seen = {};
const l = Interceptor.attach(dll.base.add(0xe5ed0), {
  onEnter(args) {
    const key = 'rcx=' + args[0] + ' rdx=' + args[1];
    seen[key] = (seen[key] || 0) + 1;
  },
});
setTimeout(() => { l.detach(); console.log('[poll-args] ' + JSON.stringify(seen)); }, 10000);
console.log('[poll-args] attached');
