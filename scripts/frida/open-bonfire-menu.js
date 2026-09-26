// Open the bonfire menu wherever the player stands, by calling the game's own `bonfireWindow`
// (0x140198680) once, on the game thread. For QA of anything that lives in the bonfire menu
// (Attune Spells) on a character that has no bonfire nearby.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/open-bonfire-menu.js \
//   [--config-json '{"bonfire":0}']`
//
// Read in Ghidra 2026-09-26: `bonfireRest` (0x14017f610) ends in
// `bonfireWindow([[GameManagerImp+0x70]+0x50], bonfireId)` at 0x14017f813, and `bonfireWindow` is
// `FUN_1404fdf30([GameManagerImp+0x22e0], id); openWindow(wm, 10)`. This skips the rest itself: no
// sit animation, no rest SpEffect, no refill, no enemy respawn. The id names the bonfire the menu
// shows; 0 is none. The call is made from inside the per-frame nav update (0x140baeb20), which runs
// on the game thread, rather than from Frida's own thread.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const bonfireWindow = new NativeFunction(exe.add(0x00198680), 'void', ['pointer', 'uint16']);
const id = typeof config.bonfire === 'number' ? config.bonfire : 0;

let done = false;
const hook = Interceptor.attach(exe.add(0x00baeb20), {
  onEnter() {
    if (done) return;
    done = true;
    const gmi = exe.add(0x016148f0).readPointer();
    const events = gmi.add(0x70).readPointer();
    const wm = events.isNull() ? ptr(0) : events.add(0x50).readPointer();
    if (wm.isNull() || gmi.add(0x22e0).readPointer().isNull()) {
      console.log('[bonfire-menu] not opened: event window manager or frontend root is null');
    } else {
      bonfireWindow(wm, id);
      console.log('[bonfire-menu] bonfireWindow(' + wm + ', ' + id + ') called on thread ' +
        Process.getCurrentThreadId());
    }
    setTimeout(() => hook.detach(), 0);
  },
});
console.log('[bonfire-menu] armed');
