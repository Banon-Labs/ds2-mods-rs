// When does the region music's volume go to 0 and back, and what else changes at that moment?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-music-timeline.js`
// Attach at the title (or in the world before a reload), then load into the region.
//
// Every looping music event the game polls, through its own getState on the sound thread: sends a
// line whenever any of these changes -- Event::getVolume, the owning MOFmodCue's dirty byte
// [cue+0x23a], id [cue+0x238] and factor [cue+0x394], what the sound manager's vfunc[0x190] gives
// for that id (the volume the game's only Event::setVolume applies, 0x1409f9d00), the channel
// group's volume, the channel's audibility, the event's 3D position and the listener's. Also every
// Event::setVolume on a music event with the volume FMOD holds after it, and the caller. Read-only
// apart from calling the vfunc[0x190] getter.

'use strict';

const ev = Process.getModuleByName('fmod_event64.dll');
const ex = Process.getModuleByName('fmodex64.dll');
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
function e(m, n) { const p = m.findExportByName(n); if (p === null) throw new Error('missing ' + n); return p; }
const P = 'pointer';
const Q = '@FMOD@@QEAA?AW4FMOD_RESULT@@';
const f = (m, n, args) => new NativeFunction(e(m, n), 'int', args);
const getInfo = f(ev, '?getInfo@Event' + Q + 'PEAHPEAPEADPEAUFMOD_EVENT_INFO@@@Z', [P, P, P, P]);
const evVolume = f(ev, '?getVolume@Event' + Q + 'PEAM@Z', [P, P]);
const ev3d = f(ev, '?get3DAttributes@Event' + Q + 'PEAUFMOD_VECTOR@@00@Z', [P, P, P, P]);
const listener = f(ev, '?get3DListenerAttributes@EventSystem' + Q + 'HPEAUFMOD_VECTOR@@000@Z', [P, 'int', P, P, P, P]);
const getChannelGroup = f(ev, '?getChannelGroup@Event' + Q + 'PEAPEAVChannelGroup@2@@Z', [P, P]);
const groupVolume = f(ex, '?getVolume@ChannelGroup' + Q + 'PEAM@Z', [P, P]);
const getChannel = f(ex, '?getChannel@ChannelGroup' + Q + 'HPEAPEAVChannel@2@@Z', [P, 'int', P]);
const chAudibility = f(ex, '?getAudibility@Channel' + Q + 'PEAM@Z', [P, P]);

const a = Memory.alloc(0x100);
const info = Memory.alloc(0x80);
const v = Memory.alloc(8);
const p0 = Memory.alloc(12), p1 = Memory.alloc(12), p2 = Memory.alloc(12), p3 = Memory.alloc(12);
function describe(h) {
  a.writePointer(ptr(0));
  for (let i = 0; i < 0x80; i += 8) info.add(i).writeU64(0);
  if (getInfo(h, ptr(0), a, info) !== 0) return null;
  const np = a.readPointer();
  return { name: np.isNull() ? null : np.readCString(), len: info.add(8).readS32() };
}
const r3 = (x) => Math.round(x * 1000) / 1000;
const vec = (p) => [r3(p.readFloat()), r3(p.add(4).readFloat()), r3(p.add(8).readFloat())];
const manager = () => exe.add(0x166dfa8).readPointer();

function cueFor(ctx, handle) {
  for (const r of ['rbx', 'rdi', 'rsi', 'rbp', 'r12', 'r13', 'r14', 'r15']) {
    const p = ctx[r];
    if (p.isNull() || p.compare(ptr(0x10000)) < 0) continue;
    try { if (p.add(0x370).readPointer().equals(handle)) return p; } catch (err) { /* not a pointer */ }
  }
  return null;
}

const events = new Map(); // handle -> { name, cue, last }
let lastPoll = 0;
Interceptor.attach(e(ev, '?getState@Event' + Q + 'PEAI@Z'), { onEnter(args) {
  const h = args[0];
  const k = h.toString();
  let rec = events.get(k);
  if (rec === undefined) {
    const d = describe(h);
    rec = (d !== null && /^m\d{9}/.test(d.name) && d.len === -1) ? { name: d.name, cue: null, last: '' } : null;
    events.set(k, rec);
    if (rec !== null) send({ kind: 'seen', name: rec.name, handle: k, t: Date.now() });
  }
  if (rec === null) return;
  if (rec.cue === null) rec.cue = cueFor(this.context, h);
  const now = Date.now();
  if (now - lastPoll < 250) return;
  lastPoll = now;
  for (const [hk, r] of events) {
    if (r === null) continue;
    const hh = ptr(hk);
    const s = {};
    s.volume = evVolume(hh, v) === 0 ? r3(v.readFloat()) : 'gone';
    if (s.volume === 'gone') { events.set(hk, null); send({ kind: 'gone', name: r.name, handle: hk, t: now }); continue; }
    if (r.cue !== null) {
      try {
        s.dirty = r.cue.add(0x23a).readU8();
        s.id = r.cue.add(0x238).readU16();
        s.factor = r3(r.cue.add(0x394).readFloat());
        const mgr = manager();
        s.manager_volume = r3(new NativeFunction(mgr.readPointer().add(0x190).readPointer(), 'float', [P, 'uint'])(mgr, s.id));
      } catch (err) { s.cue_error = String(err); }
    }
    if (getChannelGroup(hh, a) === 0 && !a.readPointer().isNull()) {
      const g = a.readPointer();
      s.group_volume = groupVolume(g, v) === 0 ? r3(v.readFloat()) : null;
      if (getChannel(g, 0, a) === 0 && !a.readPointer().isNull()) s.audibility = chAudibility(a.readPointer(), v) === 0 ? r3(v.readFloat()) : null;
    }
    const key = JSON.stringify(s);
    if (key !== r.last) {
      r.last = key;
      const out = Object.assign({ kind: 'state', name: r.name, handle: hk, t: now }, s);
      if (ev3d(hh, p0, p1, p2) === 0) out.pos3d = vec(p0);
      try {
        const system = manager().add(0x9d8).readPointer();
        if (listener(system, 0, p0, p1, p2, p3) === 0) out.listener = vec(p0);
      } catch (err) { /* no system */ }
      send(out);
    }
  }
} });

Interceptor.attach(e(ev, '?setVolume@Event' + Q + 'M@Z'), {
  onEnter(args) { this.h = args[0]; this.caller = this.returnAddress.sub(exe); },
  onLeave() {
    const r = events.get(this.h.toString());
    if (!r) return;
    send({ kind: 'setVolume', name: r.name, handle: this.h.toString(), after: evVolume(this.h, v) === 0 ? r3(v.readFloat()) : null, caller: 'exe+0x' + this.caller.toString(16), t: Date.now() });
  },
});
send({ kind: 'ready', t: Date.now() });
