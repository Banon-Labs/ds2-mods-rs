// Does the game's FMOD event system exist, and is anything playing?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-system-check.js`
//
// Reads MOFmodSoundManager (the global at exe+0x166dfa8), its EventSystem (+0x9d8), System (+0x9e0)
// and master ChannelGroup (+0x9f8), the master volume the game applied (+0x930), and asks FMOD for
// the event count and the number of channels playing. Read-only.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const ev = Process.getModuleByName('fmod_event64.dll');
const ex = Process.getModuleByName('fmodex64.dll');
const Q = '@FMOD@@QEAA?AW4FMOD_RESULT@@';
const P = 'pointer';
const numEvents = new NativeFunction(ev.findExportByName('?getNumEvents@EventSystem' + Q + 'PEAH@Z'), 'int', [P, P]);
const channelsPlaying = new NativeFunction(ex.findExportByName('?getChannelsPlaying@System' + Q + 'PEAH@Z'), 'int', [P, P]);
const groupVolume = new NativeFunction(ex.findExportByName('?getVolume@ChannelGroup' + Q + 'PEAM@Z'), 'int', [P, P]);
const out = { kind: 'system' };
const mgr = exe.add(0x166dfa8).readPointer();
out.manager = mgr.toString();
if (!mgr.isNull()) {
  const es = mgr.add(0x9d8).readPointer();
  const sys = mgr.add(0x9e0).readPointer();
  const master = mgr.add(0x9f8).readPointer();
  out.event_system = es.toString();
  out.system = sys.toString();
  out.master = master.toString();
  out.master_volume_applied = mgr.add(0x930).readFloat();
  const n = Memory.alloc(8);
  if (!es.isNull()) out.num_events = numEvents(es, n) === 0 ? n.readS32() : 'rc';
  if (!sys.isNull()) out.channels_playing = channelsPlaying(sys, n) === 0 ? n.readS32() : 'rc';
  if (!master.isNull()) out.master_volume = groupVolume(master, n) === 0 ? n.readFloat() : 'rc';
}
send(out);
