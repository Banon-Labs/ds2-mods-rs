// What volume does the game mean its region music to have, and where does a 0 come from?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-cue-volume.js`
//
// The game's only Event::setVolume (0x1409f9d00) applies, when the cue's dirty byte [cue+0x23a] is
// set, `volume = manager->vfunc[0x190](manager, [cue+0x238] as u16) * [cue+0x394]`, where
// [cue+0x238] is the id the manager's vfunc[0x178] gave the event's category name at [cue+0x230]
// (0x1409f89d3..0x1409f8a02) and [cue+0x370] is the event. This finds the MOFmodCue that owns each
// looping music event -- the register holding a pointer p with [p+0x370] == the event, at the
// game's own getState call -- and reports the event's volume, the cue's id, factor and dirty byte,
// the category name, and what vfunc[0x190] answers for that id. Read-only apart from calling that
// getter.

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
function describe(h) {
  a.writePointer(ptr(0));
  for (let i = 0; i < 0x80; i += 8) info.add(i).writeU64(0);
  if (getInfo(h, ptr(0), a, info) !== 0) return null;
  const np = a.readPointer();
  return { name: np.isNull() ? null : np.readCString(), len: info.add(8).readS32() };
}
const manager = () => exe.add(0x166dfa8).readPointer();

function readable(p) {
  try { p.readU8(); return true; } catch (err) { return false; }
}
function cueFor(ctx, handle) {
  for (const r of ['rbx', 'rdi', 'rsi', 'rbp', 'r12', 'r13', 'r14', 'r15', 'rcx']) {
    const p = ctx[r];
    if (p.isNull() || p.compare(ptr(0x10000)) < 0) continue;
    try {
      if (p.add(0x370).readPointer().equals(handle)) return { reg: r, cue: p };
      // r14 in 0x1409f8e10 is &cue->event.
      if (p.readPointer().equals(handle)) {
        const cue = p.sub(0x370);
        if (readable(cue)) return { reg: r + '-0x370', cue };
      }
    } catch (err) { /* not a pointer */ }
  }
  return null;
}

const reported = new Map();
Interceptor.attach(e(ev, '?getState@Event' + Q + 'PEAI@Z'), { onEnter(args) {
  const h = args[0];
  const k = h.toString();
  const now = Date.now();
  const last = reported.get(k);
  if (last !== undefined && now - last < 5000) return;
  const d = describe(h);
  if (d === null || !/^m\d{9}/.test(d.name) || d.len !== -1) { reported.set(k, now + 1e12); return; }
  reported.set(k, now);
  const out = { kind: 'cue', name: d.name, handle: k, caller: 'exe+0x' + this.returnAddress.sub(exe).toString(16) };
  out.event_volume = evVolume(h, v) === 0 ? v.readFloat() : null;
  const found = cueFor(this.context, h);
  if (found === null) { out.cue = null; send(out); return; }
  const cue = found.cue;
  out.cue = cue.toString();
  out.reg = found.reg;
  out.id_238 = cue.add(0x238).readU16();
  out.dirty_23a = cue.add(0x23a).readU8();
  out.factor_394 = cue.add(0x394).readFloat();
  try {
    const nameField = cue.add(0x230).readPointer();
    out.category_230 = nameField.isNull() ? null : (nameField.readCString() || null);
  } catch (err) { out.category_230 = 'unreadable'; }
  try {
    const mgr = manager();
    const fn = new NativeFunction(mgr.readPointer().add(0x190).readPointer(), 'float', [P, 'uint']);
    out.manager_volume_for_id = fn(mgr, out.id_238);
  } catch (err) { out.manager_volume_for_id = 'error ' + err; }
  send(out);
} });
send({ kind: 'ready' });
