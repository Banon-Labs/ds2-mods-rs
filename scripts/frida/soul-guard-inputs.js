// What would a soul-memory guard read, and would the game's own cost function be safe to call?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/soul-guard-inputs.js`
//
// Read-only. Hooks nothing, calls nothing in the game, writes nothing. It prints:
//
//   * the first bytes of `PlayerParam::RestoreFromRecord` (0x14038ad20), the level-up cost function
//     (0x14038d140) and its row accessor (0x140358b90), so the prologues a detour and a call check
//     against are measured rather than copied out of a listing;
//   * the chain the cost function walks on its own, `[GameManagerImp] + 0x18` (CharacterManager),
//     `+ 0x580` (the `PlayerLevelUpSoulsParam` container), `+ 0xD8` (the raw param file), and that
//     file's row count, table shape and first rows -- the preconditions under which the function's
//     halving loop terminates (row 0 must exist with a level at or below the one asked for);
//   * the live character's level (`PlayerParam + 0xD0`), both soul-memory fields (`+0xF4`, `+0xFC`)
//     and the nine stats, and the floor the guard would compute from them, walking the table here
//     with the same arithmetic the function uses.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;
const GAME_MANAGER_IMP = 0x016148f0;

function rp(p) {
  try {
    return p.readPointer();
  } catch (e) {
    return null;
  }
}

function hex(p, n) {
  try {
    return Array.from(new Uint8Array(p.readByteArray(n)))
      .map((b) => ('0' + b.toString(16)).slice(-2))
      .join(' ');
  } catch (e) {
    return 'unreadable';
  }
}

function rowAt(file, shape, count, i) {
  if (i < 0 || i >= count) {
    return null;
  }
  const off = shape !== 0 ? file.add(0x40 + i * 0x18 + 8).readU64().toNumber() : file.add(0x40 + i * 8 + 4).readU32();
  const row = file.add(off);
  return { level: row.readU16(), step: row.readS32(), souls: row.add(8).readS32() };
}

// The arithmetic of 0x14038d140, over the rows read above.
function cost(file, shape, count, L) {
  let r = rowAt(file, shape, count, L);
  if (r !== null && r.level === L) {
    return r.souls;
  }
  let i = L;
  while (r === null || r.level > L) {
    i = i >>> 1;
    r = rowAt(file, shape, count, i);
  }
  let c = r;
  while (c.level < L) {
    r = c;
    i += 1;
    c = rowAt(file, shape, count, i);
    if (c === null) {
      break;
    }
  }
  return r.souls + (L - r.level) * r.step;
}

function sample(tag) {
  console.log('[soul-guard] sample ' + tag + ' base=' + base);
  console.log('[soul-guard] prologue RestoreFromRecord 0x14038ad20: ' + hex(base.add(0x38ad20), 12));
  console.log('[soul-guard] prologue LevelUpCost 0x14038d140: ' + hex(base.add(0x38d140), 12));
  console.log('[soul-guard] prologue ParamRowByIndex 0x140358b90: ' + hex(base.add(0x358b90), 12));

  const gm = rp(base.add(GAME_MANAGER_IMP));
  if (gm === null || gm.isNull()) {
    console.log('[soul-guard] GameManagerImp null');
    return;
  }
  const cm = rp(gm.add(0x18));
  const container = cm === null || cm.isNull() ? null : rp(cm.add(0x580));
  const file = container === null || container.isNull() ? null : rp(container.add(0xd8));
  console.log('[soul-guard] GameManagerImp=' + gm + ' CharacterManager=' + cm + ' container=' + container + ' file=' + file);
  if (file === null || file.isNull()) {
    return;
  }
  const count = file.add(0x0a).readU16();
  const shape = file.add(0x2d).readU8();
  console.log('[soul-guard] PlayerLevelUpSoulsParam rows=' + count + ' shape=' + shape);
  for (const i of [0, 1, 2, 3, count - 1]) {
    const r = rowAt(file, shape, count, i);
    console.log('[soul-guard] row[' + i + '] ' + JSON.stringify(r));
  }
  console.log('[soul-guard] cost(1)=' + cost(file, shape, count, 1) + ' cost(2)=' + cost(file, shape, count, 2) + ' cost(12)=' + cost(file, shape, count, 12) + ' cost(837)=' + cost(file, shape, count, 837));

  const ctrl = rp(gm.add(0xd0));
  const param = ctrl === null || ctrl.isNull() ? null : rp(ctrl.add(0x490));
  if (param === null || param.isNull()) {
    console.log('[soul-guard] no PlayerParam (title screen or loading)');
    return;
  }
  const level = param.add(0xd0).readU32();
  const sm1 = param.add(0xf4).readU32();
  const sm2 = param.add(0xfc).readU32();
  const stats = [];
  for (let k = 0; k < 9; k++) {
    stats.push(param.add(0x08 + 2 * k).readU16());
  }
  const sum = stats.reduce((a, b) => a + b, 0);
  console.log('[soul-guard] PlayerParam=' + param + ' level=' + level + ' sum9-53=' + Math.max(1, sum - 53) + ' soul_memory=' + sm1 + ' soul_memory_2=' + sm2 + ' stats=' + stats.join(','));
  for (const start of [1, 14]) {
    let floor = 0;
    for (let L = start; L < level; L++) {
      floor += cost(file, shape, count, L);
    }
    console.log('[soul-guard] floor from start ' + start + ' to ' + level + ' = ' + floor + ' verdict=' + (sm1 >= floor ? 'ok' : 'short'));
  }
}

sample('load');
