// Read, and optionally press a button on, one of the game process's own Win32 dialogs from inside
// the process.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/dialog-command.js \
//   --config-json '{"title":"Load a character from a save file","command":2}'`
// `... --config-json '{"hwnd":"0x150124"}'`   (no command: only print the dialog's texts)
//
// The dialog is named by exact title (FindWindowW) or by hwnd (scripts/frida/own-windows.js lists
// the process's windows, untitled ones included). Every child's class and text is printed, then,
// if `command` is given, WM_COMMAND is posted with it: 1 IDOK, 2 IDCANCEL, 6 IDYES, 7 IDNO.
//
// For when a dialog will not take synthetic X input. On 2026-09-26 a ds2-save-file load dialog
// ignored typed keys, Returns and a real click on Cancel, and a posted IDCANCEL too: it was waiting
// on an untitled message box of its own, which a search by title never finds.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const user32 = Process.getModuleByName('user32.dll');
const f = (name, ret, args) => new NativeFunction(user32.getExportByName(name), ret, args);
const FindWindowW = f('FindWindowW', 'pointer', ['pointer', 'pointer']);
const PostMessageW = f('PostMessageW', 'int', ['pointer', 'uint', 'pointer', 'pointer']);
const EnumChildWindows = f('EnumChildWindows', 'int', ['pointer', 'pointer', 'pointer']);
const GetWindowTextW = f('GetWindowTextW', 'int', ['pointer', 'pointer', 'int']);
const GetClassNameW = f('GetClassNameW', 'int', ['pointer', 'pointer', 'int']);
const GetDlgCtrlID = f('GetDlgCtrlID', 'int', ['pointer']);
const WM_COMMAND = 0x0111;

let hwnd = ptr(0);
if (config.hwnd) {
  hwnd = ptr(config.hwnd);
} else if (config.title) {
  hwnd = FindWindowW(ptr(0), Memory.allocUtf16String(config.title));
}

if (hwnd.isNull()) {
  console.log('[dialog-command] no dialog: pass "title" or "hwnd" in --config-json');
} else {
  const text = Memory.alloc(1024);
  const cls = Memory.alloc(256);
  const onChild = new NativeCallback((child) => {
    GetWindowTextW(child, text, 512);
    GetClassNameW(child, cls, 128);
    const words = text.readUtf16String();
    if (words) {
      console.log('[dialog-command]   id=' + GetDlgCtrlID(child) + ' ' + cls.readUtf16String() +
        ' "' + words + '"');
    }
    return 1;
  }, 'int', ['pointer', 'pointer']);
  console.log('[dialog-command] hwnd=' + hwnd);
  EnumChildWindows(hwnd, onChild, ptr(0));
  if (config.command !== undefined) {
    const posted = PostMessageW(hwnd, WM_COMMAND, ptr(config.command), ptr(0));
    console.log('[dialog-command] command=' + config.command + ' posted=' + posted);
  }
}
