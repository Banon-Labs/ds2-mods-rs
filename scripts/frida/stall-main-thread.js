// Freeze DS2's main thread once, for long enough that ds2-crash-logging-core's hang watchdog fires.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/stall-main-thread.js`
//
// NvNavigationSystem::Update runs once per frame on the thread that runs Present (tick-threads.js,
// 2026-09-26), so sleeping inside it once stops the frame counter at GameManagerImp+0x104 for the
// whole sleep. With hang_stall_seconds = 30, a 40 s sleep should leave a "hang watchdog" stall
// report in ds2-crash-log.txt. The game resumes afterwards; nothing is written to game state.

'use strict';

const NAV_UPDATE = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00baeb20);
const STALL_SECONDS = 40;

let done = false;
const listener = Interceptor.attach(NAV_UPDATE, {
  onEnter() {
    if (done) return;
    done = true;
    console.log('[stall-main-thread] sleeping ' + STALL_SECONDS + ' s on thread ' +
      Process.getCurrentThreadId());
    Thread.sleep(STALL_SECONDS);
    console.log('[stall-main-thread] resumed');
  },
});
console.log('[stall-main-thread] attached nav=' + NAV_UPDATE);
