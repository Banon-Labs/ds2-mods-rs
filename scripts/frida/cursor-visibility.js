// Who shows and hides the OS pointer, on which thread, and what Windows ends up with.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/cursor-visibility.js`
//
// Written 2026-09-27 because hiding the pointer by clearing `ds2_rva::INPUT_UPDATE_CURSOR_WANTED_
// OFFSET` for the input update did not hide it: the detour logged `pointer hidden` while the owner
// still saw the pointer. Reports, per second: every ShowCursor call (thread, argument, returned
// count, caller), every SetCursor call (thread, handle, caller), the game window's owning thread,
// and GetCursorInfo's flags and handle. Read-only: nothing is written or replaced.

'use strict';

const user32 = Process.getModuleByName('user32.dll');
const f = (name, ret, args) => new NativeFunction(user32.getExportByName(name), ret, args);
const GetCursorInfo = f('GetCursorInfo', 'int', ['pointer']);
const GetForegroundWindow = f('GetForegroundWindow', 'pointer', []);
const GetWindowThreadProcessId = f('GetWindowThreadProcessId', 'uint', ['pointer', 'pointer']);

const game = Process.getModuleByName('DarkSoulsII.exe');
const where = (address) => {
  const m = Process.findModuleByAddress(address);
  return m ? m.name + '+0x' + address.sub(m.base).toString(16) : address.toString();
};

const counts = {};
const bump = (key) => { counts[key] = (counts[key] || 0) + 1; };

Interceptor.attach(user32.getExportByName('ShowCursor'), {
  onEnter(args) {
    this.show = args[0].toInt32();
    this.from = where(this.returnAddress);
  },
  onLeave(ret) {
    bump('ShowCursor(' + this.show + ') tid=' + Process.getCurrentThreadId() + ' from=' + this.from
      + ' -> ' + ret.toInt32());
  },
});

Interceptor.attach(user32.getExportByName('SetCursor'), {
  onEnter(args) {
    bump('SetCursor(' + args[0] + ') tid=' + Process.getCurrentThreadId() + ' from='
      + where(this.returnAddress));
  },
});

const info = Memory.alloc(24); // CURSORINFO: cbSize, flags, hCursor, ptScreenPos
const pidBox = Memory.alloc(4);
setInterval(() => {
  info.writeU32(24);
  GetCursorInfo(info);
  const window = GetForegroundWindow();
  const owner = window.isNull() ? 0 : GetWindowThreadProcessId(window, pidBox);
  const mine = !window.isNull() && pidBox.readU32() === Process.id;
  send({
    cursor_flags: info.add(4).readU32(),
    cursor_handle: info.add(8).readPointer().toString(),
    foreground_owner_tid: mine ? owner : 'not this process',
    calls: counts,
  });
  for (const key of Object.keys(counts)) delete counts[key];
}, 1000);

send({ loaded: true, game_base: game.base.toString() });
