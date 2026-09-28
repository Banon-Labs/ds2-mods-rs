// Which of the player's calls leaves the game's own region music at event volume 0?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-volume-bisect.js`
// on a run with `--no-music-probe`, standing in Majula.
//
// A control run with the player off read m100400001 at Event::getVolume 1, audibility 0.759; a run
// with the player on read volume 0 before it had played anything of its own. The player had by then
// done two things to FMOD besides reading: Event::setMute(false) on the game's event at region
// detection, and a catalog scan of getEventBySystemID(id, INFOONLY) + getInfo over every system id.
// This repeats them one at a time on the sound thread, inside the game's own getState calls, and
// reads the game event's volume and audibility after each, 2 s apart:
//   step 0: nothing (baseline)
//   step 1: Event::setMute(false) on the game's event
//   step 2: the INFOONLY scan over ids 0..11000, 500 ids per tick
// It also logs every Event::setVolume the game makes on a music event, with the value.

'use strict';

const ev = Process.getModuleByName('fmod_event64.dll');
const ex = Process.getModuleByName('fmodex64.dll');
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
function e(m, n) { const p = m.findExportByName(n); if (p === null) throw new Error('missing ' + n); return p; }
const P = 'pointer';
const Q = '@FMOD@@QEAA?AW4FMOD_RESULT@@';
const f = (m, n, args) => new NativeFunction(e(m, n), 'int', args);
const getInfo = f(ev, '?getInfo@Event' + Q + 'PEAHPEAPEADPEAUFMOD_EVENT_INFO@@@Z', [P, P, P, P]);
const setMute = f(ev, '?setMute@Event' + Q + '_N@Z', [P, 'int']);
const evVolume = f(ev, '?getVolume@Event' + Q + 'PEAM@Z', [P, P]);
const bySystemId = f(ev, '?getEventBySystemID@EventSystem' + Q + 'IIPEAPEAVEvent@2@@Z', [P, 'uint', 'uint', P]);
const getChannelGroup = f(ev, '?getChannelGroup@Event' + Q + 'PEAPEAVChannelGroup@2@@Z', [P, P]);
const getChannel = f(ex, '?getChannel@ChannelGroup' + Q + 'HPEAPEAVChannel@2@@Z', [P, 'int', P]);
const chAudibility = f(ex, '?getAudibility@Channel' + Q + 'PEAM@Z', [P, P]);

const a = Memory.alloc(0x100);
const info = Memory.alloc(0x80);
function describe(h) {
  a.writePointer(ptr(0));
  for (let i = 0; i < 0x80; i += 8) info.add(i).writeU64(0);
  if (getInfo(h, ptr(0), a, info) !== 0) return null;
  const np = a.readPointer();
  return { name: np.isNull() ? null : np.readCString(), len: info.add(8).readS32() };
}
// getInfo exactly as ds2-music-probe calls it for a bank name: maxwavebanks = 4 at +0x14 and a
// wavebankinfo array pointer at +0x18, 0x400 bytes per entry.
const banks = Memory.alloc(4 * 0x400);
function describeWithBank(h) {
  a.writePointer(ptr(0));
  for (let i = 0; i < 0x80; i += 8) info.add(i).writeU64(0);
  info.add(0x14).writeS32(4);
  info.add(0x18).writePointer(banks);
  const rc = getInfo(h, ptr(0), a, info);
  return { rc, banks: info.add(0x14).readS32(), bank: rc === 0 ? banks.readCString() : null };
}
const MODE = (typeof globalThis.__ER_FRIDA_CONFIG === 'object' && globalThis.__ER_FRIDA_CONFIG && globalThis.__ER_FRIDA_CONFIG.mode) || 'mute-scan';
function read(h) {
  const v = Memory.alloc(8);
  const out = { volume: evVolume(h, v) === 0 ? v.readFloat() : null };
  if (getChannelGroup(h, a) === 0 && !a.readPointer().isNull() && getChannel(a.readPointer(), 0, a) === 0) {
    out.audibility = chAudibility(a.readPointer(), v) === 0 ? Math.round(v.readFloat() * 1000) / 1000 : null;
  }
  return out;
}
const system = exe.add(0x166dfa8).readPointer().add(0x9d8).readPointer();

let game = null;
let step = 0;
let at = 0;
let scanId = 0;
Interceptor.attach(e(ev, '?getState@Event' + Q + 'PEAI@Z'), { onEnter(args) {
  try {
    if (game === null) {
      const d = describe(args[0]);
      if (d !== null && /^m\d{9}/.test(d.name) && d.len === -1) {
        game = args[0];
        at = Date.now();
        send({ kind: 'found', name: d.name, handle: game.toString(), step0: read(game) });
      }
      return;
    }
    const now = Date.now();
    if (step === 0 && now - at > 2000) {
      send({ kind: 'step0-baseline', state: read(game) });
      if (MODE === 'bank') {
        send({ kind: 'step1-getInfo-with-bank-on-live-event', result: describeWithBank(game) });
      } else {
        const rc = setMute(game, 0);
        send({ kind: 'step1-setMute-false', rc });
      }
      step = 1; at = now;
    } else if (step === 1 && now - at > 2000) {
      send({ kind: 'step1-after-2s', state: read(game) });
      step = 2; at = now;
    } else if (step === 2) {
      for (let i = 0; i < 500 && scanId < 11000; i++, scanId++) {
        if (bySystemId(system, scanId, 4, a) === 0) {
          const h = a.readPointer();
          if (MODE === 'bank') describeWithBank(h); else describe(h);
        }
      }
      if (scanId >= 11000) { send({ kind: 'step2-scan-done', state: read(game) }); step = 3; at = now; }
    } else if (step === 3 && now - at > 2000) {
      send({ kind: 'step2-after-2s', state: read(game) });
      step = 4;
    }
  } catch (err) { send({ kind: 'error', error: String(err), step }); step = 9; }
} });

Interceptor.attach(e(ev, '?setVolume@Event' + Q + 'M@Z'), { onEnter(args) {
  const d = describe(args[0]);
  if (d !== null && /^m\d{9}/.test(d.name)) {
    // The float is in xmm1; read it from the context.
    send({ kind: 'game-setVolume', name: d.name, handle: args[0].toString(), xmm1: this.context.xmm1 ? this.context.xmm1.toString() : null, step });
  }
} });

send({ kind: 'ready' });
