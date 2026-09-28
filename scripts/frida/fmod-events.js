// Which FMOD events does the game start, pause and stop, and what are they called?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/fmod-events.js`
//
// Step 1's measurement for docs/DS2-REGION-MUSIC-PLAYLIST.md: the `ds2-music-probe` crate fronts
// the game's import slots for these same exports, so the names and handles seen here are what it
// will see. Interceptor hooks on fmod_event64.dll's exports (FMOD's code, not Arxan-checked game
// code). Thin: each hook calls `Event::getInfo` for the name and send()s one message; nothing
// polls. The first FIRST_ANY events of any name are sent so an unexpected name format shows up;
// after that only music (`m` + nine digits, or `m_word`) is sent.
//
// Map index: [GameManagerImp @ exe+0x16148f0] + 0x38 (MapManager) -> +0x170, u32
// (ds2_rva::MAP_MANAGER_PLAYER_MAP_INDEX_OFFSET). 0xffffffff at the title / during a load.

'use strict';

const FIRST_ANY = 24;
const mod = Process.getModuleByName('fmod_event64.dll');
const exe = Process.getModuleByName('DarkSoulsII.exe').base;

function exp(name) {
  const p = mod.findExportByName(name);
  if (p === null) throw new Error('missing export ' + name);
  return p;
}

const START = exp('?start@Event@FMOD@@QEAA?AW4FMOD_RESULT@@XZ');
const STOP = exp('?stop@Event@FMOD@@QEAA?AW4FMOD_RESULT@@_N@Z');
const SET_PAUSED = exp('?setPaused@Event@FMOD@@QEAA?AW4FMOD_RESULT@@_N@Z');
const GET_EVENT = exp('?getEvent@EventSystem@FMOD@@QEAA?AW4FMOD_RESULT@@PEBDIPEAPEAVEvent@2@@Z');
const getInfo = new NativeFunction(
  exp('?getInfo@Event@FMOD@@QEAA?AW4FMOD_RESULT@@PEAHPEAPEADPEAUFMOD_EVENT_INFO@@@Z'),
  'int', ['pointer', 'pointer', 'pointer', 'pointer']);

const nameOut = Memory.alloc(8);
const info = Memory.alloc(0x80);

function isMusic(name) {
  if (typeof name !== 'string') return false;
  const leaf = name.split('/').pop();
  return /^m\d{9}/.test(leaf) || /^m_./.test(leaf);
}

function mapIndex() {
  try {
    const gm = exe.add(0x16148f0).readPointer();
    if (gm.isNull()) return null;
    const mm = gm.add(0x38).readPointer();
    if (mm.isNull()) return null;
    return mm.add(0x170).readU32();
  } catch (e) { return null; }
}

// name, positionms (+4), lengthms (+8), projectid (+0x20), systemid (+0x24). The struct is zeroed
// first: maxwavebanks/wavebankinfo/numinstances/instances/guid are inputs to getInfo.
function describe(handle) {
  nameOut.writePointer(ptr(0));
  for (let i = 0; i < 0x80; i += 8) info.add(i).writeU64(0);
  const rc = getInfo(handle, ptr(0), nameOut, info);
  const np = nameOut.readPointer();
  return {
    rc,
    name: np.isNull() ? null : np.readCString(),
    positionms: info.add(4).readS32(),
    lengthms: info.add(8).readS32(),
    projectid: info.add(0x20).readU32(),
    systemid: info.add(0x24).readU32(),
  };
}

let anySent = 0;
function report(kind, handle, extra) {
  let d;
  try { d = describe(handle); } catch (e) { d = { error: String(e) }; }
  const music = isMusic(d.name);
  if (!music && anySent >= FIRST_ANY) return;
  if (!music) anySent += 1;
  send(Object.assign({ kind, handle: handle.toString(), music, map: mapIndex(),
    tid: Process.getCurrentThreadId() }, d, extra || {}));
}

Interceptor.attach(START, { onEnter(args) { report('start', args[0]); } });
Interceptor.attach(STOP, { onEnter(args) { report('stop', args[0], { immediate: args[1].and(0xff).toInt32() }); } });
Interceptor.attach(SET_PAUSED, { onEnter(args) { report('setPaused', args[0], { paused: args[1].and(0xff).toInt32() }); } });
Interceptor.attach(GET_EVENT, {
  onEnter(args) {
    this.name = args[1].isNull() ? null : args[1].readCString();
    this.mode = args[2].toUInt32();
    this.out = args[3];
  },
  onLeave(rv) {
    if (!isMusic(this.name)) return;
    let h = null;
    try { h = this.out.readPointer().toString(); } catch (e) { /* out may be null */ }
    send({ kind: 'getEvent', name: this.name, mode: this.mode, rc: rv.toInt32(), handle: h,
      map: mapIndex() });
  },
});

send({ kind: 'ready', module: mod.base.toString(), map: mapIndex() });
