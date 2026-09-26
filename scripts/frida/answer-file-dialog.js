// Answer the next comdlg32 open/save dialog the game asks for with a fixed path, without showing
// it: GetOpenFileNameW / GetSaveFileNameW are replaced once, write `path` into the caller's
// OPENFILENAMEW buffer, and return TRUE, then put themselves back.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/answer-file-dialog.js \
//   --config-json '{"path":"Z:\\home\\me\\Downloads\\x.sl2"}'`
// then press the save-file row. The agent stays attached until it has answered once.
//
// Why not type into the dialog: measured 2026-09-26, synthetic X keys lose Shift, the name box's
// autocomplete eats a Return, some dialogs take no keys until X-focused, and an untitled message
// box ("File does not exist") can sit on top where a search by title never finds it. Answering in
// the process has none of that and never blocks the game thread on a window. What it skips is the
// dialog itself -- use it to test what the rows do with a path, not the dialog.
//
// OPENFILENAMEW (x64): lpstrFile +0x30, nMaxFile +0x38 (u32 units), Flags +0x60,
// nFileOffset +0x64 (u16), nFileExtension +0x66 (u16).

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const comdlg = Process.getModuleByName('comdlg32.dll');
const targets = ['GetOpenFileNameW', 'GetSaveFileNameW'].map((name) => ({
  name,
  address: comdlg.getExportByName(name),
}));

if (!config.path) {
  console.log('[answer-file-dialog] pass --config-json \'{"path":"..."}\'');
} else {
  const path = config.path;
  const slash = path.lastIndexOf('\\');
  const dot = path.lastIndexOf('.');
  let answered = false;
  for (const target of targets) {
    Interceptor.replace(target.address, new NativeCallback((ofn) => {
      if (answered) return 0;
      answered = true;
      const max = ofn.add(0x38).readU32();
      if (path.length + 1 > max) {
        console.log('[answer-file-dialog] ' + target.name + ': path is ' + (path.length + 1) +
          ' units, the buffer holds ' + max + ' -- answering Cancel');
        return 0;
      }
      ofn.add(0x30).readPointer().writeUtf16String(path);
      ofn.add(0x64).writeU16(slash + 1);
      ofn.add(0x66).writeU16(dot > slash ? dot + 1 : 0);
      console.log('[answer-file-dialog] ' + target.name + ' answered "' + path + '"');
      setTimeout(() => {
        for (const t of targets) Interceptor.revert(t.address);
        console.log('[answer-file-dialog] reverted');
      }, 0);
      return 1;
    }, 'int', ['pointer']));
  }
  console.log('[answer-file-dialog] armed for "' + path + '"');
}
