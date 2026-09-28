// Can a mod seek the live background track, and does clearing its loop make it end?
//
// `python3 scripts/ds2-frida-up.py --allow-early`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-seek-loop.js`
// Optional: `--config-json '{"lead_ms": 12000, "mode": "loopcount"}'` (mode: loopcount | mode | none)
//
// Step 2's measurement for docs/DS2-REGION-MUSIC-PLAYLIST.md, rows "seek" and "repeat off". Self
// driving, no input: it finds the looping music event through the game's own `Event::getState`
// polls (the sound thread), and on that same thread
//   1. seeks its channel to `length - lead_ms` with `Channel::setPosition(ms, FMOD_TIMEUNIT_MS)`
//      and reads the position back,
//   2. clears the loop (`Channel::setLoopCount(0)`, or `setMode(FMOD_LOOP_OFF)`, or nothing as the
//      control),
//   3. watches the position every 200 ms and reports whether the track ends (channel gone / not
//      playing), wraps back (the loop survived or the event retriggered), and whether the game
//      starts the event again.
// Every FMOD call runs inside the game's own getState call on its sound thread, which is where the
// game itself makes them. One experiment per attach; it sends 'done' and goes quiet.

'use strict';

const cfg = (typeof globalThis.__ER_FRIDA_CONFIG === 'object' && globalThis.__ER_FRIDA_CONFIG) || {};
const LEAD_MS = cfg.lead_ms || 12000;
const MODE = cfg.mode || 'loopcount';
const MS = 1; // FMOD_TIMEUNIT_MS
const FMOD_LOOP_OFF = 0x1;

const ev = Process.getModuleByName('fmod_event64.dll');
const ex = Process.getModuleByName('fmodex64.dll');
function e(m, n) { const p = m.findExportByName(n); if (p === null) throw new Error('missing ' + n); return p; }
const P = 'pointer';
const getInfo = new NativeFunction(e(ev, '?getInfo@Event@FMOD@@QEAA?AW4FMOD_RESULT@@PEAHPEAPEADPEAUFMOD_EVENT_INFO@@@Z'), 'int', [P, P, P, P]);
const getChannelGroup = new NativeFunction(e(ev, '?getChannelGroup@Event@FMOD@@QEAA?AW4FMOD_RESULT@@PEAPEAVChannelGroup@2@@Z'), 'int', [P, P]);
const getState = e(ev, '?getState@Event@FMOD@@QEAA?AW4FMOD_RESULT@@PEAI@Z');
const startEx = e(ev, '?start@Event@FMOD@@QEAA?AW4FMOD_RESULT@@XZ');
const stopEx = e(ev, '?stop@Event@FMOD@@QEAA?AW4FMOD_RESULT@@_N@Z');
const numChannels = new NativeFunction(e(ex, '?getNumChannels@ChannelGroup@FMOD@@QEAA?AW4FMOD_RESULT@@PEAH@Z'), 'int', [P, P]);
const getChannel = new NativeFunction(e(ex, '?getChannel@ChannelGroup@FMOD@@QEAA?AW4FMOD_RESULT@@HPEAPEAVChannel@2@@Z'), 'int', [P, 'int', P]);
const numGroups = new NativeFunction(e(ex, '?getNumGroups@ChannelGroup@FMOD@@QEAA?AW4FMOD_RESULT@@PEAH@Z'), 'int', [P, P]);
const getGroup = new NativeFunction(e(ex, '?getGroup@ChannelGroup@FMOD@@QEAA?AW4FMOD_RESULT@@HPEAPEAV12@@Z'), 'int', [P, 'int', P]);
const getPosition = new NativeFunction(e(ex, '?getPosition@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAII@Z'), 'int', [P, P, 'uint']);
const setPosition = new NativeFunction(e(ex, '?setPosition@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@II@Z'), 'int', [P, 'uint', 'uint']);
const getCurrentSound = new NativeFunction(e(ex, '?getCurrentSound@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAPEAVSound@2@@Z'), 'int', [P, P]);
const getLength = new NativeFunction(e(ex, '?getLength@Sound@FMOD@@QEAA?AW4FMOD_RESULT@@PEAII@Z'), 'int', [P, P, 'uint']);
const setLoopCount = new NativeFunction(e(ex, '?setLoopCount@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@H@Z'), 'int', [P, 'int']);
const getLoopCount = new NativeFunction(e(ex, '?getLoopCount@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAH@Z'), 'int', [P, P]);
const setMode = new NativeFunction(e(ex, '?setMode@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@I@Z'), 'int', [P, 'uint']);
const getMode = new NativeFunction(e(ex, '?getMode@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAI@Z'), 'int', [P, P]);
const isPlaying = new NativeFunction(e(ex, '?isPlaying@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEA_N@Z'), 'int', [P, P]);
const getLoopPoints = new NativeFunction(e(ex, '?getLoopPoints@Channel@FMOD@@QEAA?AW4FMOD_RESULT@@PEAII0I@Z'), 'int', [P, P, 'uint', P, 'uint']);

const out = Memory.alloc(0x100);
const out2 = Memory.alloc(0x10);
const info = Memory.alloc(0x80);

function describe(h) {
  out.writePointer(ptr(0));
  for (let i = 0; i < 0x80; i += 8) info.add(i).writeU64(0);
  const rc = getInfo(h, ptr(0), out, info);
  if (rc !== 0) return null;
  const np = out.readPointer();
  return { name: np.isNull() ? null : np.readCString(), len: info.add(8).readS32(), pos: info.add(4).readS32() };
}
function isMusic(n) { return typeof n === 'string' && (/^m\d{9}/.test(n) || /^m_./.test(n)); }

function firstChannel(group, depth) {
  if (numChannels(group, out) === 0 && out.readS32() > 0 && getChannel(group, 0, out) === 0 && !out.readPointer().isNull()) return out.readPointer();
  if (depth <= 0 || numGroups(group, out) !== 0) return null;
  const n = out.readS32();
  for (let i = 0; i < n; i++) {
    if (getGroup(group, i, out) === 0 && !out.readPointer().isNull()) {
      const c = firstChannel(out.readPointer(), depth - 1);
      if (c !== null) return c;
    }
  }
  return null;
}
function channelOf(h) {
  if (getChannelGroup(h, out) !== 0 || out.readPointer().isNull()) return null;
  return firstChannel(out.readPointer(), 3);
}
function pos(ch) { return getPosition(ch, out2, MS) === 0 ? out2.readU32() : null; }
function playing(ch) { out2.writeU32(0); const rc = isPlaying(ch, out2); return { rc, playing: out2.readU8() }; }

const seen = new Set();
let target = null; // { handle, name, channel, length }
let phase = 'find';
let last = null;
let lastAt = 0;
let endedAt = 0;
let restartWatch = false;

Interceptor.attach(getState, {
  onEnter(args) {
    const h = args[0];
    try {
      if (phase === 'find') {
        const k = h.toString();
        if (seen.has(k)) return;
        seen.add(k);
        const d = describe(h);
        if (d === null || !isMusic(d.name) || d.len !== -1) return;
        const ch = channelOf(h);
        if (ch === null) { send({ kind: 'no-channel', name: d.name, handle: k }); return; }
        getCurrentSound(ch, out);
        const snd = out.readPointer();
        const length = getLength(snd, out2, MS) === 0 ? out2.readU32() : null;
        getLoopCount(ch, out2); const loops = out2.readS32();
        getMode(ch, out2); const mode = out2.readU32();
        const lp = Memory.alloc(8);
        getLoopPoints(ch, lp, MS, lp.add(4), MS);
        target = { handle: h, name: d.name, channel: ch, length };
        send({ kind: 'found', name: d.name, handle: k, channel: ch.toString(), pos: pos(ch), length,
          loopcount: loops, mode: '0x' + mode.toString(16), loopstart: lp.readU32(), loopend: lp.add(4).readU32() });
        const to = Math.max(0, length - LEAD_MS);
        const before = pos(ch);
        const rc = setPosition(ch, to, MS);
        send({ kind: 'seek', name: d.name, before, target: to, rc, after_immediate: pos(ch) });
        phase = 'seeked';
        lastAt = Date.now();
        return;
      }
      if (target === null || !h.equals(target.handle)) return;
      const now = Date.now();
      if (phase === 'seeked' && now - lastAt > 300) {
        const p = pos(target.channel);
        let rc = null;
        if (MODE === 'loopcount') rc = setLoopCount(target.channel, 0);
        else if (MODE === 'mode') rc = setMode(target.channel, FMOD_LOOP_OFF);
        getLoopCount(target.channel, out2); const loops = out2.readS32();
        getMode(target.channel, out2); const mode = out2.readU32();
        send({ kind: 'seek-confirmed', name: target.name, pos_300ms_later: p, loop_mode: MODE, rc, loopcount_now: loops, mode_now: '0x' + mode.toString(16) });
        phase = 'watch'; last = p; lastAt = now;
        return;
      }
      if (phase === 'watch' && now - lastAt >= 200) {
        lastAt = now;
        const pl = playing(target.channel);
        const p = pos(target.channel);
        const d = describe(target.handle);
        const ch2 = channelOf(target.handle);
        if (pl.rc !== 0 || !pl.playing || p === null) {
          send({ kind: 'ended', name: target.name, last_pos: last, isPlaying_rc: pl.rc, playing: pl.playing, event: d, channel_now: ch2 === null ? null : ch2.toString() });
          phase = 'after'; endedAt = now; restartWatch = true;
          return;
        }
        if (last !== null && p + 1000 < last) {
          send({ kind: 'wrapped', name: target.name, from: last, to: p, event: d, same_channel: ch2 !== null && ch2.equals(target.channel) });
          phase = 'after'; endedAt = now; restartWatch = true;
          return;
        }
        last = p;
      }
      if (phase === 'after' && now - endedAt >= 200 && now - endedAt < 8000 && now - lastAt >= 1000) {
        lastAt = now;
        const ch2 = channelOf(target.handle);
        const d = describe(target.handle);
        send({ kind: 'after', t_ms: now - endedAt, event: d, channel: ch2 === null ? null : ch2.toString(), pos: ch2 === null ? null : pos(ch2) });
      }
      if (phase === 'after' && now - endedAt >= 8000) { phase = 'done'; restartWatch = false; send({ kind: 'done' }); }
    } catch (err) {
      send({ kind: 'error', error: String(err), phase });
      phase = 'done';
    }
  },
});

Interceptor.attach(startEx, { onEnter(args) {
  if (!restartWatch) return;
  const d = describe(args[0]);
  if (d !== null && isMusic(d.name)) send({ kind: 'game-start', name: d.name, handle: args[0].toString(), same_handle: target !== null && args[0].equals(target.handle) });
} });
Interceptor.attach(stopEx, { onEnter(args) {
  if (!restartWatch) return;
  const d = describe(args[0]);
  if (d !== null && isMusic(d.name)) send({ kind: 'game-stop', name: d.name, handle: args[0].toString() });
} });

send({ kind: 'ready', lead_ms: LEAD_MS, mode: MODE });
