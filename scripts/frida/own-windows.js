// List the game process's OWN top-level windows (title, class, visible, owner) from inside it.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/own-windows.js`
//
// EnumWindows walks every top-level window on the desktop; each is kept only if
// GetWindowThreadProcessId names this process, so nothing belonging to another program is read or
// printed. For finding a modal box a dialog is waiting on when it will not take input. Read-only.

'use strict';

const user32 = Process.getModuleByName('user32.dll');
const f = (name, ret, args) => new NativeFunction(user32.getExportByName(name), ret, args);
const EnumWindows = f('EnumWindows', 'int', ['pointer', 'pointer']);
const GetWindowThreadProcessId = f('GetWindowThreadProcessId', 'uint', ['pointer', 'pointer']);
const GetWindowTextW = f('GetWindowTextW', 'int', ['pointer', 'pointer', 'int']);
const GetClassNameW = f('GetClassNameW', 'int', ['pointer', 'pointer', 'int']);
const IsWindowVisible = f('IsWindowVisible', 'int', ['pointer']);
const GetWindow = f('GetWindow', 'pointer', ['pointer', 'uint']);
const GW_OWNER = 4;

const pidBox = Memory.alloc(4);
const text = Memory.alloc(512);
const cls = Memory.alloc(512);
const found = [];
const callback = new NativeCallback((hwnd) => {
  const thread = GetWindowThreadProcessId(hwnd, pidBox);
  if (pidBox.readU32() !== Process.id) return 1;
  GetWindowTextW(hwnd, text, 256);
  GetClassNameW(hwnd, cls, 256);
  found.push('hwnd=' + hwnd + ' thread=' + thread + ' visible=' + IsWindowVisible(hwnd) +
    ' owner=' + GetWindow(hwnd, GW_OWNER) + ' class="' + cls.readUtf16String() + '" title="' +
    text.readUtf16String() + '"');
  return 1;
}, 'int', ['pointer', 'pointer']);

EnumWindows(callback, ptr(0));
found.forEach((line) => console.log('[own-windows] ' + line));
console.log('[own-windows] ' + found.length + ' window(s) belong to pid ' + Process.id);
