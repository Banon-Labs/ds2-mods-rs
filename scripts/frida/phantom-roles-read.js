// Read-only: who in the session is the host, who is a summon and who broke in, as the game itself
// records it. The measurement behind weapon/armour sync's "match the highest invader while hosting".
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/phantom-roles-read.js`
//
// Writes nothing. Once a second, when anything changed, it sends:
//   local    GameManagerImp->PlayerCtrl's phantom type, `[PlayerCtrl+0xb0]+0x3c`
//            (`0x14014ed20` reads exactly this byte for the local player in `0x14013d430`), and
//            the team byte beside it at `+0x3d`.
//   remotes  every other PlayerCtrl in the roster: its factory name, phantom type and team.
//   table    the phantom type table `0x1410c0050` (20 entries of 0x10 bytes) as the running image
//            holds it, so it can be compared with `darksoulsii-deobf.bin`. Byte 2 of an entry is
//            2 for the types that arrive by break-in (Paramdex: 5, 6, 8, 10, 11, 12, 14, 15, 16, 17)
//            and 1 for the sign summons (1, 2, 3, 4, 7, 9, 13).
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const GAME_MANAGER_IMP = at(0x016148f0); // ds2_rva::GAME_MANAGER_IMP
const PLAYER_CTRL_VTABLE = at(0x010e4bb8); // ds2_rva::PLAYER_CTRL_VTABLE
const PHANTOM_TABLE = at(0x010c0050);

function ptr_(p, off) {
  try {
    const v = p.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function name(chr) {
  try {
    const s = chr.add(0x118);
    const len = s.add(0x10).readU64().toNumber();
    const cap = s.add(0x18).readU64().toNumber();
    if (len === 0 || len > 64) return '';
    return (cap > 7 ? s.readPointer() : s).readUtf16String(len);
  } catch (e) {
    return '?';
  }
}

function role(chr) {
  const block = ptr_(chr, 0xb0);
  if (!block) return null;
  try {
    return { type: block.add(0x3c).readU8(), team: block.add(0x3d).readU8() };
  } catch (e) {
    return null;
  }
}

function hex(bytes) {
  return Array.from(new Uint8Array(bytes)).map((b) => b.toString(16).padStart(2, '0')).join('');
}

const table = [];
for (let i = 0; i < 20; i++) table.push(hex(PHANTOM_TABLE.add(i * 0x10).readByteArray(8)));

let last = '';
function tick() {
  const manager = ptr_(GAME_MANAGER_IMP, 0);
  const local = manager && ptr_(manager, 0xd0);
  const out = { local: null, remotes: [] };
  if (local) {
    out.local = Object.assign({ name: name(local) }, role(local));
    const characters = ptr_(manager, 0x18);
    const begin = characters && ptr_(characters, 0x10);
    const end = characters && ptr_(characters, 0x18);
    if (begin && end) {
      const count = Math.min(end.sub(begin).toInt32() / 8, 512);
      for (let i = 0; i < count; i++) {
        const chr = ptr_(begin, i * 8);
        if (!chr || chr.equals(local)) continue;
        let vt = null;
        try {
          vt = chr.readPointer();
        } catch (e) {
          continue;
        }
        if (!vt.equals(PLAYER_CTRL_VTABLE)) continue;
        out.remotes.push(Object.assign({ name: name(chr) }, role(chr)));
      }
    }
  }
  const line = JSON.stringify(out);
  if (line !== last) {
    last = line;
    send({ roles: out, table: table });
  }
}

tick();
setInterval(tick, 1000);
