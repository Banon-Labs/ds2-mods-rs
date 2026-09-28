// Are the FMOD event import slots still pointing at ds2-music-probe's detours, and is the game
// calling the event API at all?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-iat-check.js`
//
// Reads the four slots ds2-music-probe fronts (Event::start/stop/setPaused/getState) and says which
// module each points into, then counts calls to those exports in fmod_event64.dll for 5 s.
// Read-only.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const ev = Process.getModuleByName('fmod_event64.dll');
const Q = '@FMOD@@QEAA?AW4FMOD_RESULT@@';
const slots = {
  start: [0x1aae764, '?start@Event' + Q + 'XZ'],
  stop: [0x1aae75c, '?stop@Event' + Q + '_N@Z'],
  setPaused: [0x1aae85c, '?setPaused@Event' + Q + '_N@Z'],
  getState: [0x1aae754, '?getState@Event' + Q + 'PEAI@Z'],
};
const out = {};
const counts = {};
for (const [name, [rva, sym]] of Object.entries(slots)) {
  const target = exe.add(rva).readPointer();
  const m = Process.findModuleByAddress(target);
  out[name] = { target: target.toString(), module: m ? m.name + '+0x' + target.sub(m.base).toString(16) : null };
  counts[name] = 0;
  Interceptor.attach(ev.findExportByName(sym), { onEnter() { counts[name] += 1; } });
}
send({ kind: 'slots', slots: out });
setTimeout(() => send({ kind: 'calls-in-5s', counts }), 5000);
