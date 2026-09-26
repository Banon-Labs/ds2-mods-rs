// Why do DARK SOULS II's input polls go quiet when its window loses focus?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/input-focus.js`
//
// The DLUserInput manager lives at `[0x141875278]`. `FUN_140ef6640` stores the game window at
// `+0x80`, clears `+0x16d` when `GetActiveWindow()` is some other window, and reads three options
// into `+0x16e`/`+0x16f`/`+0x170`: `Ext.UserInput.CooperativeLevel.SetForeGround.Pad`,
// `.Keyboard` and `.Mouse`. The pad poll (`0x140f05540`) returns without reading the pad when
// `+0x16e` is set and `+0x16d` is clear. This prints those bytes, and whether the game window is
// the foreground one, once at load. It does not sample on a timer because timers do not fire in
// this game's Frida runtime; attach again to read again. It hooks nothing and writes nothing.
//
// Read on 2026-09-26 with the window unfocused: `+0x16d=0 +0x16e=0 +0x16f=1 +0x170=0`. The pad
// ignores focus; the keyboard does not.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;
const MANAGER_SLOT = 0x1875278;

const getForegroundWindow = new NativeFunction(
  Process.getModuleByName('user32.dll').getExportByName('GetForegroundWindow'), 'pointer', []);

function sample() {
  let manager;
  try {
    manager = base.add(MANAGER_SLOT).readPointer();
  } catch (e) {
    console.log('[input-focus] manager slot unreadable');
    return;
  }
  if (manager.isNull()) {
    console.log('[input-focus] manager not constructed');
    return;
  }
  const hwnd = manager.add(0x80).readPointer();
  const bytes = [];
  for (let off = 0x169; off <= 0x170; off++) {
    bytes.push('+0x' + off.toString(16) + '=' + manager.add(off).readU8());
  }
  const foreground = getForegroundWindow();
  console.log('[input-focus] hwnd=' + hwnd + ' foreground-is-game=' + foreground.equals(hwnd) +
    ' ' + bytes.join(' '));
}

sample();
