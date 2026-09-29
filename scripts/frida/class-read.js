// Read-only, one shot: the loaded character's starting class, in both places the game keeps it,
// and the PlayerStatusParam row the game's own mapping gives it.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/class-read.js`
//
// Writes nothing, hooks nothing. Calls one pure function of the game's, 0x140203e80, which is a
// jump table from class id to row id and touches no memory.
//
//   player_data = [[GameManagerImp]+0xA8]+0xC0; class = +0x64, u32 (ds2_rva PLAYER_DATA_CLASS_OFFSET)
//   list        = [[GameManagerImp]+0xA8]+0xD8; current slot = +0x1368, i32 in 0..10
//   record      = list + slot*0x1F0; class = +0x1D6 u16, name = +0x18A UTF-16, stats = +0x174
//   stats live  = [[GameManagerImp]+0xD0]+0x490 (PlayerParam) + 0x08, nine u16, game order
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const say = (t) => console.log('[class-read] ' + t);

function ptr_(p, off) {
  try {
    const v = p.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function u16s(p, n) {
  const out = [];
  for (let i = 0; i < n; i++) out.push(p.add(i * 2).readU16());
  return out;
}

function read() {
  const gm = ptr_(at(0x16148f0), 0);
  if (!gm) { say('no GameManagerImp -- not in game'); return; }
  const gdm = ptr_(gm, 0xa8);
  const pd = gdm && ptr_(gdm, 0xc0);
  const list = gdm && ptr_(gdm, 0xd8);
  if (!pd || !list) { say('no player_data or character list'); return; }
  const cls = pd.add(0x64).readU32();
  let name = '';
  try { name = pd.add(0x24).readUtf16String(0x20); } catch (e) { name = '?'; }
  say('player_data ' + pd + ' name=' + JSON.stringify(name) + ' class(+0x64)=' + cls);
  const slot = list.add(0x1368).readS32();
  if (slot < 0 || slot >= 10) { say('current slot ' + slot + ' out of range'); return; }
  const rec = list.add(slot * 0x1f0);
  let rname = '';
  try { rname = rec.add(0x18a).readUtf16String(0x20); } catch (e) { rname = '?'; }
  say('record slot=' + slot + ' ' + rec + ' name=' + JSON.stringify(rname) + ' class(+0x1d6)=' +
      rec.add(0x1d6).readU16() + ' stats(+0x174)=' + u16s(rec.add(0x174), 9).join(','));
  const pc = ptr_(gm, 0xd0);
  const param = pc && ptr_(pc, 0x490);
  if (param) say('live stats ' + u16s(param.add(0x08), 9).join(',') + ' level=' + param.add(0xd0).readU32());
  const row = new NativeFunction(at(0x203e80), 'int', ['int']);
  say('0x140203e80(' + cls + ') = PlayerStatusParam row ' + row(cls));
}

read();
say('done, read-only, no hooks');
