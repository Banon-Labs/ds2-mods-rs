// Quit DARK SOULS II the orderly way, without touching the window or its focus.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/request-quit.js`
//
// The title menu's QUIT GAME row and the window procedure's WM_CLOSE handler (0x1402eacfe..
// 0x1402ead0e) both end the same way: they write 1 to `[KatanaMainApp]+0x13a`, the app being
// `[0x1416751f8]`. The main loop then leaves through `ExitProcess`, which is what runs every DLL's
// DLL_PROCESS_DETACH -- the path a killed process never takes. This writes that one byte, once, at
// load. THIS ENDS THE GAME.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const app = image.base.add(0x16751f8).readPointer();
if (app.isNull()) {
  console.log('[request-quit] REFUSED: KatanaMainApp is null');
} else {
  const before = app.add(0x13a).readU8();
  app.add(0x13a).writeU8(1);
  console.log('[request-quit] app=' + app + ' +0x13a ' + before + ' -> ' + app.add(0x13a).readU8());
}
