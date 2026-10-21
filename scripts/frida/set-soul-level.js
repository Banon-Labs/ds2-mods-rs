// Set the live character's soul level by its stats: raise (or lower) the nine attributes
// (PlayerParam +0x08..+0x18, u16, ds2-rva PLAYER_PARAM_STAT_OFFSETS) until their sum supports
// SOUL_LEVEL, then write the level (+0xD0) to match.
//
// Why the stats and not the level alone: a level written by itself was back at 11 by the next
// search (2026-10-21), and the owner's lead is that the game derives the level from the stats.
// The first message reports sum(stats) - level so that lead is checked, not assumed.
//
// Used to find Seamless Co-op's invasion bracket boundaries with ds2sc-lobby-watch.js. Soul memory
// is left as it is (the owner's call, 2026-10-21). SOUL_LEVEL = null only reads.
// Writes u16/u32 fields and hooks nothing, so reloading the file in place is safe.

'use strict';

const SOUL_LEVEL = 155;
const STAT_OFFSETS = [0x08, 0x0a, 0x0c, 0x0e, 0x10, 0x12, 0x14, 0x16, 0x18];
const STAT_NAMES = ['vgr', 'end', 'vit', 'att', 'str', 'dex', 'int', 'fth', 'adp'];
const STAT_MAX = 99;
const STAT_MIN = 1;
const LEVEL_OFFSET = 0xd0;
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const get = new NativeFunction(exe.add(0x001ab660), 'pointer', []);

function stats(param) {
  return STAT_OFFSETS.map((offset) => param.add(offset).readU16());
}

const param = get();
if (param.isNull()) {
  send({ tag: 'set-soul-level', error: 'no character (PlayerParam is null)' });
} else {
  const before = stats(param);
  const level = param.add(LEVEL_OFFSET).readU32();
  const sum = before.reduce((a, b) => a + b, 0);
  const out = {
    tag: 'set-soul-level',
    level,
    stats: Object.fromEntries(STAT_NAMES.map((n, i) => [n, before[i]])),
    sum,
    sum_minus_level: sum - level,
  };
  if (SOUL_LEVEL !== null) {
    let delta = SOUL_LEVEL - level;
    const now = before.slice();
    for (let i = 0; i < now.length && delta !== 0; i++) {
      const room = delta > 0 ? STAT_MAX - now[i] : STAT_MIN - now[i];
      const step = delta > 0 ? Math.min(delta, room) : Math.max(delta, room);
      now[i] += step;
      delta -= step;
    }
    now.forEach((value, i) => param.add(STAT_OFFSETS[i]).writeU16(value));
    param.add(LEVEL_OFFSET).writeU32(SOUL_LEVEL - delta);
    out.after_level = param.add(LEVEL_OFFSET).readU32();
    out.after_stats = Object.fromEntries(STAT_NAMES.map((n, i) => [n, stats(param)[i]]));
  }
  send(out);
}
