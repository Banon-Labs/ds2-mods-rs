// Print, and optionally set, the local player's nine stats, so a requirement check has something to
// fail. `--config-json '{"set":{"intelligence":1,"faith":1}}'`.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/set-player-stats.js`
//
// Chain: [exe+0x16148f0] GameManagerImp -> +0xd0 PlayerCtrl -> +0x490 PlayerParam -> +0x8 u16[9]
// (darksouls2::game::chr::PlayerParam). The order is the one ds2-build-import writes and logs.

'use strict';

const NAMES = ['vigor', 'endurance', 'vitality', 'attunement', 'strength', 'dexterity',
  'intelligence', 'faith', 'adaptability'];
const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const pc = exe.add(0x016148f0).readPointer().add(0xd0).readPointer();
const stats = pc.add(0x490).readPointer().add(0x8);
const show = () => NAMES.map((n, i) => n + '=' + stats.add(i * 2).readU16()).join(' ');
console.log('[stats] before ' + show());
for (const [name, value] of Object.entries(config.set || {})) {
  const i = NAMES.indexOf(name);
  if (i < 0) {
    console.log('[stats] unknown stat ' + name);
  } else {
    stats.add(i * 2).writeU16(value);
  }
}
if (config.set) console.log('[stats] after  ' + show());

// The frontend's own copy, which ds2-item-warn's requirement check reads: GameManagerImp +0x22e0
// -> +0x138, i32 at a 0x18 stride, indexed by FE_STAT_ROW_TABLE (0x14155def0) +0x4 per key.
const fe = exe.add(0x016148f0).readPointer().add(0x22e0).readPointer();
if (fe.isNull()) {
  console.log('[stats] frontend root not built');
} else {
  const table = fe.add(0x138).readPointer();
  const keys = {str: 0x33, dex: 0x34, int: 0x35, fth: 0x36, spell_int: 0x42, spell_fth: 0x43};
  // `{"frontend":{"int":1}}` writes this copy too. PlayerParam alone does not reach it until the
  // game recomputes it (measured 2026-09-26: PlayerParam int=1 while this still read 30).
  for (const [name, value] of Object.entries(config.frontend || {})) {
    const index = exe.add(0x0155def0 + keys[name] * 12 + 4).readS16();
    if (index >= 0) table.add(index * 0x18).writeS32(value);
  }
  const out = Object.entries(keys).map(([name, key]) => {
    const index = exe.add(0x0155def0 + key * 12 + 4).readS16();
    return name + '[' + index + ']=' + (index < 0 ? '-' : table.add(index * 0x18).readS32());
  });
  console.log('[stats] frontend ' + out.join(' '));
}
