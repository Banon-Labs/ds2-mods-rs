// Who sets a music event's volume, to what, and from which cue fields?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-setvolume-watch.js`
//
// The game has one Event::setVolume call, in 0x1409f9d00: volume = vfunc[0x190](manager, id) *
// [cue+0x394], applied when the dirty byte [cue+0x23a] is set; `rbx` is the MOFmodCue there. This
// logs every setVolume on a music event: the caller, the volume FMOD holds afterwards, and the
// cue's id and its own factor, so a volume of 0 can be put on one of the two. Read-only.

'use strict';

const ev = Process.getModuleByName('fmod_event64.dll');
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
function e(m, n) { const p = m.findExportByName(n); if (p === null) throw new Error('missing ' + n); return p; }
const P = 'pointer';
const Q = '@FMOD@@QEAA?AW4FMOD_RESULT@@';
const getInfo = new NativeFunction(e(ev, '?getInfo@Event' + Q + 'PEAHPEAPEADPEAUFMOD_EVENT_INFO@@@Z'), 'int', [P, P, P, P]);
const evVolume = new NativeFunction(e(ev, '?getVolume@Event' + Q + 'PEAM@Z'), 'int', [P, P]);
const a = Memory.alloc(0x100);
const info = Memory.alloc(0x80);
const v = Memory.alloc(8);
function nameOf(h) {
  a.writePointer(ptr(0));
  if (getInfo(h, ptr(0), a, info) !== 0) return null;
  const np = a.readPointer();
  return np.isNull() ? null : np.readCString();
}
let sent = 0;
Interceptor.attach(e(ev, '?setVolume@Event' + Q + 'M@Z'), {
  onEnter(args) {
    this.h = args[0];
    this.name = nameOf(args[0]);
    this.caller = this.returnAddress.sub(exe);
    this.rbx = this.context.rbx;
  },
  onLeave() {
    if (this.name === null || !/^m/.test(this.name) || sent > 200) return;
    sent += 1;
    const out = { kind: 'setVolume', name: this.name, handle: this.h.toString(), caller: 'exe+0x' + this.caller.toString(16) };
    out.volume_after = evVolume(this.h, v) === 0 ? v.readFloat() : null;
    try {
      const cue = this.rbx;
      out.cue = cue.toString();
      out.cue_event = cue.add(0x370).readPointer().toString();
      out.cue_id = cue.add(0x238).readU16();
      out.cue_factor_394 = cue.add(0x394).readFloat();
    } catch (err) { out.cue_error = String(err); }
    send(out);
  },
});
// Every 2 s, the volume FMOD holds for each looping music event the game polls, so a load can be
// watched from start to finish.
const tracked = new Map();
let lastDump = 0;
Interceptor.attach(e(ev, '?getState@Event' + Q + 'PEAI@Z'), { onEnter(args) {
  const k = args[0].toString();
  if (!tracked.has(k)) {
    const n = nameOf(args[0]);
    tracked.set(k, n !== null && /^m\d{9}/.test(n) && info.add(8).readS32() === -1 ? n : null);
  }
  const now = Date.now();
  if (now - lastDump < 2000) return;
  lastDump = now;
  const out = {};
  for (const [h, n] of tracked) {
    if (n === null) continue;
    out[n + '@' + h] = evVolume(ptr(h), v) === 0 ? Math.round(v.readFloat() * 1000) / 1000 : 'gone';
  }
  send({ kind: 'volumes', t: now, volumes: out });
} });
Interceptor.attach(e(ev, '?start@Event' + Q + 'XZ'), { onEnter(args) {
  const n = nameOf(args[0]);
  if (n !== null && /^m/.test(n)) send({ kind: 'start', name: n, handle: args[0].toString(), volume_at_start: evVolume(args[0], v) === 0 ? v.readFloat() : null });
} });
send({ kind: 'ready' });
