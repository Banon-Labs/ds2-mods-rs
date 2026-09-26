// Make ds2-voice-chat see one F8 press: F8 held for a short burst, then released, once.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/press-f8.js`
//
// Question (vhj): does ds2-voice-chat's key poll turn one F8 press into one Voice chat toggle and
// its log line? The poll runs only while `GetForegroundWindow` belongs to the game and then reads
// `GetAsyncKeyState`. Both are answered here, and only for calls whose return address is inside
// this repo's `dinput8.dll`, so the game's own focus and keyboard handling see nothing. No window
// focus changes and no compositor input is sent. What this does NOT test is the real keyboard
// reaching `GetAsyncKeyState` -- that is Windows' part, not the crate's.

'use strict';

// Any other key: `--config-json '{"vk": 118}'` (118 is F7, ds2-inventory-sort's default key).
const config = globalThis.__ER_FRIDA_CONFIG || {};
const VK_F8 = typeof config.vk === 'number' ? config.vk : 0x77;
const HELD_CALLS = 8;
const ours = Process.getModuleByName('dinput8.dll');
const fromOurs = (ret) => ret.compare(ours.base) >= 0 && ret.compare(ours.base.add(ours.size)) < 0;

const user32 = Process.getModuleByName('user32.dll');
const getWindowThreadProcessId = new NativeFunction(
  user32.getExportByName('GetWindowThreadProcessId'), 'uint32', ['pointer', 'pointer']);
const findWindowExW = new NativeFunction(
  user32.getExportByName('FindWindowExW'), 'pointer', ['pointer', 'pointer', 'pointer', 'pointer']);
const getCurrentProcessId = new NativeFunction(
  Process.getModuleByName('kernel32.dll').getExportByName('GetCurrentProcessId'), 'uint32', []);

// A top-level window this process owns, found the way any Win32 program would.
function ownWindow() {
  const pid = getCurrentProcessId();
  const out = Memory.alloc(4);
  let hwnd = findWindowExW(NULL, NULL, NULL, NULL);
  while (!hwnd.isNull()) {
    getWindowThreadProcessId(hwnd, out);
    if (out.readU32() === pid) return hwnd;
    hwnd = findWindowExW(NULL, hwnd, NULL, NULL);
  }
  return NULL;
}

const game = ownWindow();
console.log('[press-f8] dinput8.dll ' + ours.base + ' game window ' + game);
let calls = 0;

Interceptor.attach(user32.getExportByName('GetForegroundWindow'), {
  onEnter() { this.ours = fromOurs(this.returnAddress); },
  onLeave(retval) {
    if (this.ours && calls <= HELD_CALLS && !game.isNull()) retval.replace(game);
  },
});

Interceptor.attach(user32.getExportByName('GetAsyncKeyState'), {
  onEnter(args) {
    this.f8 = fromOurs(this.returnAddress) && (args[0].toInt32() & 0xff) === VK_F8;
  },
  onLeave(retval) {
    if (!this.f8) return;
    calls += 1;
    if (calls <= HELD_CALLS) {
      retval.replace(ptr(0x8000));
      if (calls === 1 || calls === HELD_CALLS) console.log('[press-f8] vk 0x' + VK_F8.toString(16) + ' reported held, poll ' + calls);
    } else if (calls === HELD_CALLS + 1) {
      console.log('[press-f8] vk 0x' + VK_F8.toString(16) + ' released after ' + HELD_CALLS + ' polls');
    }
  },
});
