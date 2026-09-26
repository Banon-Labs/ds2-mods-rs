// Answer ds2-build-import's "Load a build from a URL" dialog from inside the process: set the edit
// control's text, then press OK or Cancel.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/answer-url-dialog.js \
//   --config-json '{"text":"https://soulsplanner.com/darksouls2/253","command":1}'`
//
// Without `text` the prefill is left as it is and only printed. `command` is 1 for OK, 2 for
// Cancel, and omitted to leave the dialog open. The ids are ds2_build_url_core::dialog's: the edit
// control is 100. Synthetic typing into Wine dialogs has been unreliable (lost focus, lost Shift),
// so the text is set with SetDlgItemTextW, which goes through the dialog's own thread.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const user32 = Process.getModuleByName('user32.dll');
const f = (name, ret, args) => new NativeFunction(user32.getExportByName(name), ret, args);
const FindWindowW = f('FindWindowW', 'pointer', ['pointer', 'pointer']);
const SetDlgItemTextW = f('SetDlgItemTextW', 'int', ['pointer', 'int', 'pointer']);
const GetDlgItemTextW = f('GetDlgItemTextW', 'uint', ['pointer', 'int', 'pointer', 'int']);
const PostMessageW = f('PostMessageW', 'int', ['pointer', 'uint', 'pointer', 'pointer']);
const ID_EDIT = 100;
const WM_COMMAND = 0x0111;

const hwnd = FindWindowW(ptr(0), Memory.allocUtf16String('Load a build from a URL'));
if (hwnd.isNull()) {
  console.log('[url-dialog] not open');
} else {
  const buf = Memory.alloc(4096);
  GetDlgItemTextW(hwnd, ID_EDIT, buf, 2048);
  console.log('[url-dialog] hwnd=' + hwnd + ' prefill="' + buf.readUtf16String() + '"');
  if (typeof config.text === 'string') {
    const set = SetDlgItemTextW(hwnd, ID_EDIT, Memory.allocUtf16String(config.text));
    GetDlgItemTextW(hwnd, ID_EDIT, buf, 2048);
    console.log('[url-dialog] set=' + set + ' now="' + buf.readUtf16String() + '"');
  }
  if (config.command !== undefined) {
    const posted = PostMessageW(hwnd, WM_COMMAND, ptr(config.command), ptr(0));
    console.log('[url-dialog] command=' + config.command + ' posted=' + posted);
  }
}
