// Make GetAsyncKeyState report F8 held for a short burst, then released, once.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/press-f8.js`
//
// Question (vhj): does ds2-voice-chat's key poll turn one F8 press into one Voice chat toggle and
// its log line? The poll reads `GetAsyncKeyState`, which only a focused window's real keyboard
// feeds; this answers that call for F8 alone, inside the game, so no window focus or synthetic
// compositor input is involved. Every other key and every other caller is passed through.

'use strict';

const VK_F8 = 0x77;
const HELD_CALLS = 8;
const target = Module.getExportByName('user32.dll', 'GetAsyncKeyState');
let calls = 0;

Interceptor.attach(target, {
  onEnter(args) {
    this.f8 = (args[0].toInt32() & 0xff) === VK_F8;
  },
  onLeave(retval) {
    if (!this.f8) return;
    calls += 1;
    if (calls <= HELD_CALLS) {
      retval.replace(ptr(0x8000));
      if (calls === 1 || calls === HELD_CALLS) {
        console.log('[press-f8] F8 reported held, call ' + calls);
      }
    } else if (calls === HELD_CALLS + 1) {
      console.log('[press-f8] F8 released after ' + HELD_CALLS + ' polls');
    }
  },
});
console.log('[press-f8] attached at ' + target);
