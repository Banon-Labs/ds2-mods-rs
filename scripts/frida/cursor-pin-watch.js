// Who keeps the OS pointer pinned to the window centre while the Invadable hosts panel is open?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/cursor-pin-watch.js`
//
// Observers on user32 SetCursorPos and ClipCursor (and win32u's NtUserSetCursorPos /
// NtUserClipCursor, which Wine's user32 forwards to). Every call is counted per caller module+offset
// and reported every 2 s: (export, caller, count, first args). Nothing is changed.

'use strict';

const counts = new Map();

function where(address) {
  const mod = Process.findModuleByAddress(address);
  return mod === null ? address.toString() : mod.name + '+0x' + address.sub(mod.base).toString(16);
}

function watch(moduleName, exportName, describe) {
  let target = null;
  try {
    target = Process.getModuleByName(moduleName).getExportByName(exportName);
  } catch (e) {
    send({ tag: 'missing', export: moduleName + '!' + exportName });
    return;
  }
  Interceptor.attach(target, {
    onEnter(args) {
      const key = exportName + ' <- ' + where(this.returnAddress);
      const seen = counts.get(key);
      if (seen === undefined) {
        const frames = Thread.backtrace(this.context, Backtracer.FUZZY).slice(0, 12).map(where);
        counts.set(key, { n: 1, first: describe(args), frames });
      } else {
        seen.n += 1;
      }
    },
  });
  send({ tag: 'hooked', export: moduleName + '!' + exportName });
}

watch('user32.dll', 'SetCursorPos', (args) => [args[0].toInt32(), args[1].toInt32()]);
watch('user32.dll', 'ClipCursor', (args) => {
  if (args[0].isNull()) return 'NULL (release)';
  return [0, 4, 8, 12].map((o) => args[0].add(o).readS32());
});
watch('win32u.dll', 'NtUserSetCursorPos', (args) => [args[0].toInt32(), args[1].toInt32()]);
watch('win32u.dll', 'NtUserClipCursor', (args) => (args[0].isNull() ? 'NULL' : [0, 4, 8, 12].map((o) => args[0].add(o).readS32())));

setInterval(() => {
  const out = [];
  counts.forEach((value, key) => out.push({ call: key, n: value.n, first: value.first, frames: value.frames }));
  send({ tag: 'cursor-calls', calls: out });
}, 2000);
