// Who writes FeOperatorNowLoading+0xa8 (1 while the loading screen is up)? Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/nowloading-flag-watch.js`
//
// MEASURED 2026-09-28 (nowloading-diff.js, one normal load): +0xa8 went 0 -> 1 when the load
// started and 1 -> 0 when it ended, with +0x48 (a pointer) and +0x124 (1 -> 0) alongside.
// Arms a hardware write watchpoint on +0xa8 in every thread. On each hit, logs the thread, the new
// value and a fuzzy backtrace of DarkSoulsII.exe frames, then re-arms.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe');
const inExe = (p) => p.compare(exe.base) >= 0 && p.compare(exe.base.add(exe.size)) < 0;
const rva = (p) => '0x' + p.sub(exe.base).toString(16);
const op = exe.base.add(0x16148f0).readPointer().add(0x22e0).readPointer().add(0xc8).readPointer();
const flag = op.add(0xa8);

function armAll() {
  for (const t of Process.enumerateThreads()) {
    try { t.setHardwareWatchpoint(0, flag, 1, 'w'); } catch (e) { /* thread gone */ }
  }
}

Process.setExceptionHandler((details) => {
  if (details.type !== 'single-step' && details.type !== 'breakpoint') return false;
  if (!details.memory || !details.memory.address || !details.memory.address.equals(flag)) {
    // Hardware watchpoint hits arrive as single-step on some platforms without memory info.
  }
  const bt = Thread.backtrace(details.context, Backtracer.FUZZY).filter(inExe).slice(0, 14).map(rva).join(' ');
  console.log('[flag] hit pc=' + rva(details.context.pc) + ' value=' + flag.readU8() + ' bt=' + bt);
  return true;
});

armAll();
setInterval(armAll, 2000); // new threads, and threads the handler disarmed

// A watchpoint left armed after detach has no handler: the next write to the flag (quit to title,
// 2026-09-28) raised an unhandled single-step (0x80000004) in GameManager state 31 and killed the
// game. Clear the slot in every thread on unload.
rpc.exports.dispose = () => {
  for (const t of Process.enumerateThreads()) {
    try { t.unsetHardwareWatchpoint(0); } catch (e) { /* thread gone */ }
  }
};
console.log('[flag] op=' + op + ' watching ' + flag + ' value=' + flag.readU8());
