// Does Event::setMute(false) make a muted music event audible again, and where is the listener?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-unmute-test.js
//  --config-json '{"game": "0x29d40001", "ours": "0x53380001"}'`
//
// On the game's sound thread: reads the listener (EventSystem::get3DListenerAttributes, listener 0),
// the game event's 3D position, mute, channel mute and audibility; unmutes the game event; reads
// them again 600 ms later; then puts the mute back so the running player's state is unchanged. It
// also reads the listener against our instance's position.

'use strict';

const cfg = (typeof globalThis.__ER_FRIDA_CONFIG === 'object' && globalThis.__ER_FRIDA_CONFIG) || {};
const GAME = ptr(cfg.game);
const OURS = cfg.ours ? ptr(cfg.ours) : null;
const ev = Process.getModuleByName('fmod_event64.dll');
const ex = Process.getModuleByName('fmodex64.dll');
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
function e(m, n) { const p = m.findExportByName(n); if (p === null) throw new Error('missing ' + n); return p; }
const P = 'pointer';
const Q = '@FMOD@@QEAA?AW4FMOD_RESULT@@';
const f = (m, n, args) => new NativeFunction(e(m, n), 'int', args);
const setMute = f(ev, '?setMute@Event' + Q + '_N@Z', [P, 'int']);
const evMute = f(ev, '?getMute@Event' + Q + 'PEA_N@Z', [P, P]);
const ev3d = f(ev, '?get3DAttributes@Event' + Q + 'PEAUFMOD_VECTOR@@00@Z', [P, P, P, P]);
const listener = f(ev, '?get3DListenerAttributes@EventSystem' + Q + 'HPEAUFMOD_VECTOR@@000@Z', [P, 'int', P, P, P, P]);
const getChannelGroup = f(ev, '?getChannelGroup@Event' + Q + 'PEAPEAVChannelGroup@2@@Z', [P, P]);
const getChannel = f(ex, '?getChannel@ChannelGroup' + Q + 'HPEAPEAVChannel@2@@Z', [P, 'int', P]);
const chAudibility = f(ex, '?getAudibility@Channel' + Q + 'PEAM@Z', [P, P]);
const chMute = f(ex, '?getMute@Channel' + Q + 'PEA_N@Z', [P, P]);

const a = Memory.alloc(64);
function vec(p) { return [p.readFloat(), p.add(4).readFloat(), p.add(8).readFloat()].map((v) => Math.round(v * 100) / 100); }
function state(h) {
  const out = {};
  const pos = Memory.alloc(12), z = Memory.alloc(12), o = Memory.alloc(12);
  out.pos3d = ev3d(h, pos, z, o) === 0 ? vec(pos) : null;
  a.writeU32(0); out.mute = evMute(h, a) === 0 ? a.readU8() : null;
  if (getChannelGroup(h, a) === 0 && !a.readPointer().isNull() && getChannel(a.readPointer(), 0, a) === 0) {
    const ch = a.readPointer();
    const b = Memory.alloc(8);
    out.ch_mute = chMute(ch, b) === 0 ? b.readU8() : null;
    out.audibility = chAudibility(ch, b) === 0 ? Math.round(b.readFloat() * 1000) / 1000 : null;
  }
  return out;
}
function listenerPos() {
  const mgr = exe.add(0x166dfa8).readPointer();
  const system = mgr.add(0x9d8).readPointer();
  const pos = Memory.alloc(12), v = Memory.alloc(12), fw = Memory.alloc(12), up = Memory.alloc(12);
  const rc = listener(system, 0, pos, v, fw, up);
  return rc === 0 ? vec(pos) : 'rc' + rc;
}

let phase = 0;
let at = 0;
Interceptor.attach(e(ev, '?getState@Event' + Q + 'PEAI@Z'), { onEnter() {
  const now = Date.now();
  try {
    if (phase === 0) {
      send({ kind: 'before', listener: listenerPos(), game: state(GAME), ours: OURS ? state(OURS) : null });
      // Is the mute a count? Unmute until getMute says 0, at most 64 times.
      let calls = 0;
      let rc = 0;
      while (calls < 64) {
        rc = setMute(GAME, 0);
        calls += 1;
        a.writeU32(0);
        if (evMute(GAME, a) === 0 && a.readU8() === 0) break;
      }
      send({ kind: 'unmuted', rc, calls_until_unmuted: calls, game: state(GAME) });
      phase = 1; at = now;
    } else if (phase === 1 && now - at > 600) {
      send({ kind: 'after-600ms', listener: listenerPos(), game: state(GAME) });
      const rc = setMute(GAME, 1);
      send({ kind: 'remuted', rc, game: state(GAME) });
      phase = 2;
    }
  } catch (err) { send({ kind: 'error', error: String(err) }); phase = 2; }
} });
send({ kind: 'ready' });
