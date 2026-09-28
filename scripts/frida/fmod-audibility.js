// Why is a music event the player started inaudible while its channel position advances?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-audibility.js
//  --config-json '{"ours": "0x346e0001", "game": "0x29d40001"}'`
//
// Read-only. On the game's sound thread (inside its own Event::getState calls) it dumps, for our
// instance and the game's region instance, every event property (name, type, value), every event
// parameter (name, value, range), volume, mute, paused, 3D attributes, and the channel's volume,
// audibility, mode and paused state, and the channel group's volume. It also logs what the game
// itself calls on music events: setVolume, setPropertyByIndex, getParameter(name) and setValue,
// set3DAttributes -- which is what a started-by-us instance never receives.

'use strict';

const cfg = (typeof globalThis.__ER_FRIDA_CONFIG === 'object' && globalThis.__ER_FRIDA_CONFIG) || {};
const OURS = cfg.ours ? ptr(cfg.ours) : null;
const GAME = cfg.game ? ptr(cfg.game) : null;

const ev = Process.getModuleByName('fmod_event64.dll');
const ex = Process.getModuleByName('fmodex64.dll');
function e(m, n) { const p = m.findExportByName(n); if (p === null) throw new Error('missing ' + n); return p; }
const P = 'pointer';
const Q = '@FMOD@@QEAA?AW4FMOD_RESULT@@';
const f = (m, n, sig, args) => new NativeFunction(e(m, n), 'int', args);
const getInfo = f(ev, '?getInfo@Event' + Q + 'PEAHPEAPEADPEAUFMOD_EVENT_INFO@@@Z', 0, [P, P, P, P]);
const getNumProperties = f(ev, '?getNumProperties@Event' + Q + 'PEAH@Z', 0, [P, P]);
const getPropertyInfo = f(ev, '?getPropertyInfo@Event' + Q + 'PEAHPEAPEADPEAW4FMOD_EVENTPROPERTY_TYPE@@@Z', 0, [P, P, P, P]);
const getPropertyByIndex = f(ev, '?getPropertyByIndex@Event' + Q + 'HPEAX_N@Z', 0, [P, 'int', P, 'int']);
const getNumParameters = f(ev, '?getNumParameters@Event' + Q + 'PEAH@Z', 0, [P, P]);
const getParameterByIndex = f(ev, '?getParameterByIndex@Event' + Q + 'HPEAPEAVEventParameter@2@@Z', 0, [P, 'int', P]);
const paramInfo = f(ev, '?getInfo@EventParameter' + Q + 'PEAHPEAPEAD@Z', 0, [P, P, P]);
const paramValue = f(ev, '?getValue@EventParameter' + Q + 'PEAM@Z', 0, [P, P]);
const paramRange = f(ev, '?getRange@EventParameter' + Q + 'PEAM0@Z', 0, [P, P, P]);
const evVolume = f(ev, '?getVolume@Event' + Q + 'PEAM@Z', 0, [P, P]);
const evMute = f(ev, '?getMute@Event' + Q + 'PEA_N@Z', 0, [P, P]);
const evPaused = f(ev, '?getPaused@Event' + Q + 'PEA_N@Z', 0, [P, P]);
const ev3d = f(ev, '?get3DAttributes@Event' + Q + 'PEAUFMOD_VECTOR@@00@Z', 0, [P, P, P, P]);
const getChannelGroup = f(ev, '?getChannelGroup@Event' + Q + 'PEAPEAVChannelGroup@2@@Z', 0, [P, P]);
const evCategory = f(ev, '?getCategory@Event' + Q + 'PEAPEAVEventCategory@2@@Z', 0, [P, P]);
const numChannels = f(ex, '?getNumChannels@ChannelGroup' + Q + 'PEAH@Z', 0, [P, P]);
const getChannel = f(ex, '?getChannel@ChannelGroup' + Q + 'HPEAPEAVChannel@2@@Z', 0, [P, 'int', P]);
const groupVolume = f(ex, '?getVolume@ChannelGroup' + Q + 'PEAM@Z', 0, [P, P]);
const chVolume = f(ex, '?getVolume@Channel' + Q + 'PEAM@Z', 0, [P, P]);
const chAudibility = f(ex, '?getAudibility@Channel' + Q + 'PEAM@Z', 0, [P, P]);
const chMode = f(ex, '?getMode@Channel' + Q + 'PEAI@Z', 0, [P, P]);
const chPaused = f(ex, '?getPaused@Channel' + Q + 'PEA_N@Z', 0, [P, P]);
const chMute = f(ex, '?getMute@Channel' + Q + 'PEA_N@Z', 0, [P, P]);
const chPosition = f(ex, '?getPosition@Channel' + Q + 'PEAII@Z', 0, [P, P, 'uint']);

const a = Memory.alloc(0x100);
const b = Memory.alloc(0x100);
const c = Memory.alloc(0x100);

function name(h) {
  a.writePointer(ptr(0));
  const info = Memory.alloc(0x80);
  if (getInfo(h, ptr(0), a, info) !== 0) return null;
  const np = a.readPointer();
  return np.isNull() ? null : np.readCString();
}
function f32(fn, h) { b.writeFloat(NaN); const rc = fn(h, b); return rc === 0 ? b.readFloat() : 'rc' + rc; }
function bool(fn, h) { b.writeU32(0); const rc = fn(h, b); return rc === 0 ? b.readU8() : 'rc' + rc; }
function vec(p) { return [p.readFloat(), p.add(4).readFloat(), p.add(8).readFloat()].map((v) => Math.round(v * 100) / 100); }

function dump(label, h) {
  const out = { label, handle: h.toString(), name: name(h) };
  if (out.name === null) { out.error = 'getInfo failed'; return out; }
  out.volume = f32(evVolume, h);
  out.mute = bool(evMute, h);
  out.paused = bool(evPaused, h);
  const pos = Memory.alloc(12), vel = Memory.alloc(12), ori = Memory.alloc(12);
  const rc3 = ev3d(h, pos, vel, ori);
  out.pos3d = rc3 === 0 ? vec(pos) : 'rc' + rc3;
  a.writePointer(ptr(0));
  out.category = evCategory(h, a) === 0 ? a.readPointer().toString() : null;
  const props = {};
  if (getNumProperties(h, a) === 0) {
    const n = a.readS32();
    for (let i = 0; i < n; i++) {
      a.writeS32(i); b.writePointer(ptr(0)); c.writeU32(99);
      if (getPropertyInfo(h, a, b, c) !== 0) continue;
      const pn = b.readPointer().isNull() ? 'p' + i : b.readPointer().readCString();
      const type = c.readU32();
      const v = Memory.alloc(16);
      const rc = getPropertyByIndex(h, i, v, 1);
      if (rc !== 0) { props[pn] = 'rc' + rc; continue; }
      props[pn] = type === 0 ? v.readS32() : type === 1 ? Math.round(v.readFloat() * 1000) / 1000 : 'str';
    }
  }
  out.props = props;
  const params = {};
  if (getNumParameters(h, a) === 0) {
    const n = a.readS32();
    for (let i = 0; i < n; i++) {
      if (getParameterByIndex(h, i, a) !== 0) continue;
      const p = a.readPointer();
      b.writePointer(ptr(0));
      paramInfo(p, c, b);
      const pn = b.readPointer().isNull() ? 'param' + i : b.readPointer().readCString();
      const v = Memory.alloc(8); paramValue(p, v);
      const lo = Memory.alloc(4), hi = Memory.alloc(4); paramRange(p, lo, hi);
      params[pn] = [v.readFloat(), lo.readFloat(), hi.readFloat()];
    }
  }
  out.params = params;
  if (getChannelGroup(h, a) === 0 && !a.readPointer().isNull()) {
    const g = a.readPointer();
    out.group_volume = f32(groupVolume, g);
    if (numChannels(g, b) === 0 && b.readS32() > 0 && getChannel(g, 0, c) === 0) {
      const ch = c.readPointer();
      out.channel = { volume: f32(chVolume, ch), audibility: f32(chAudibility, ch), paused: bool(chPaused, ch), mute: bool(chMute, ch) };
      if (chMode(ch, b) === 0) out.channel.mode = '0x' + b.readU32().toString(16);
      if (chPosition(ch, b, 1) === 0) out.channel.pos = b.readU32();
    }
  }
  return out;
}

let last = 0;
let dumps = 0;
Interceptor.attach(e(ev, '?getState@Event' + Q + 'PEAI@Z'), {
  onEnter() {
    const now = Date.now();
    if (dumps >= 3 || now - last < 3000) return;
    last = now; dumps += 1;
    try {
      if (OURS) send(dump('ours', OURS));
      if (GAME) send(dump('game', GAME));
    } catch (err) { send({ kind: 'error', error: String(err) }); dumps = 99; }
  },
});

// What the game itself does to music events.
function hookGame(sym, argsOf) {
  Interceptor.attach(e(ev, sym), { onEnter(args) {
    const n = name(args[0]);
    if (n !== null && /^m\d{9}/.test(n)) send(Object.assign({ kind: 'game-call', call: sym.split('@')[0], name: n, handle: args[0].toString() }, argsOf(args)));
  } });
}
hookGame('?setVolume@Event' + Q + 'M@Z', () => ({}));
hookGame('?setPropertyByIndex@Event' + Q + 'HPEAX_N@Z', (args) => ({ index: args[1].toInt32(), value_i: args[2].isNull() ? null : args[2].readS32(), value_f: args[2].isNull() ? null : args[2].readFloat() }));
hookGame('?getParameter@Event' + Q + 'PEBDPEAPEAVEventParameter@2@@Z', (args) => ({ param: args[1].readCString() }));
hookGame('?set3DAttributes@Event' + Q + 'PEBUFMOD_VECTOR@@00@Z', (args) => ({ pos: args[1].isNull() ? null : vec(args[1]) }));

send({ kind: 'ready', ours: cfg.ours || null, game: cfg.game || null });
