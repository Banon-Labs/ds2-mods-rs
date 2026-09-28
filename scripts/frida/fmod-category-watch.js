// Who sets the "music" category's volume, mute or pause, and to what?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-category-watch.js`
//
// The game's region music volume is `manager->vfunc[0x190](category id) * cue factor`, and
// vfunc[0x190] (0x1409e30b0 -> 0x1409e4a30) answers the FMOD EventCategory's own volume, or 0 when
// the category is not in the manager's list at +0xab8. After an autoload the "music" category read 0
// with ds2-music-probe off, and came back to 0.7 when the pause menu opened. This logs every
// EventCategoryI::setVolume / setMute / setPaused with the category's name, the value, the caller
// (module+offset) and the thread, and the music category's volume every 2 s. Read-only.

'use strict';

const ev = Process.getModuleByName('fmod_event64.dll');
const P = 'pointer';
const Q = '@FMOD@@UEAA?AW4FMOD_RESULT@@';
function e(n) { const p = ev.findExportByName(n); if (p === null) throw new Error('missing ' + n); return p; }
const catInfo = new NativeFunction(e('?getInfo@EventCategoryI' + Q + 'PEAHPEAPEAD@Z'), 'int', [P, P, P]);
const catVolume = new NativeFunction(e('?getVolume@EventCategoryI' + Q + 'PEAM@Z'), 'int', [P, P]);
const a = Memory.alloc(16);
const b = Memory.alloc(16);
function nameOf(cat) {
  b.writePointer(ptr(0));
  if (catInfo(cat, a, b) !== 0) return null;
  const p = b.readPointer();
  return p.isNull() ? null : p.readCString();
}
function where(addr) {
  const m = Process.findModuleByAddress(addr);
  return m ? m.name + '+0x' + addr.sub(m.base).toString(16) : addr.toString();
}
const cats = new Map();
function hook(sym, label, read) {
  Interceptor.attach(e(sym), { onEnter(args) {
    const name = nameOf(args[0]);
    cats.set(args[0].toString(), name);
    const out = { kind: label, category: name, cat: args[0].toString(), caller: where(this.returnAddress), tid: Process.getCurrentThreadId(), t: Date.now() };
    Object.assign(out, read(this, args));
    const bt = Thread.backtrace(this.context, Backtracer.FUZZY).slice(0, 6).map(where);
    out.stack = bt;
    send(out);
  } });
}
hook('?setVolume@EventCategoryI' + Q + 'M@Z', 'setVolume', (self) => ({ value: self.context.xmm1 ? null : null, xmm1_bits: null }));
hook('?setMute@EventCategoryI' + Q + '_N@Z', 'setMute', (self, args) => ({ value: args[1].and(0xff).toInt32() }));
hook('?setPaused@EventCategoryI' + Q + '_N@Z', 'setPaused', (self, args) => ({ value: args[1].and(0xff).toInt32() }));
// The float argument is in xmm1, which Frida does not expose on x64 Windows; read the volume back
// after the call instead.
Interceptor.attach(e('?setVolume@EventCategoryI' + Q + 'M@Z'), {
  onEnter(args) { this.cat = args[0]; },
  onLeave() {
    const v = Memory.alloc(8);
    send({ kind: 'setVolume-after', category: nameOf(this.cat), volume: catVolume(this.cat, v) === 0 ? v.readFloat() : null, t: Date.now() });
  },
});
setInterval(() => {
  for (const [k, n] of cats) {
    if (n !== 'music') continue;
    const v = Memory.alloc(8);
    send({ kind: 'music-volume', cat: k, volume: catVolume(ptr(k), v) === 0 ? v.readFloat() : null, t: Date.now() });
  }
}, 2000);
send({ kind: 'ready', t: Date.now() });
