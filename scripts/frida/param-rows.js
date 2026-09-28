// Are the regulation numbers the build recommender now uses the ones the running game holds?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/param-rows.js`
//
// scripts/ds2-builds-recommend.py `apply_regulation` reads two things out of enc_regulation.bnd.dcx
// on disk: PhysicalStatsPerLevelStatValuesParam `defense` (the physical stat defense by
// trunc((END+VIT+STR+DEX)/4)) and WeaponStatsAffectParam's Enchanted row (Dagger: statsAffectId
// 1005030 + 8, level 10: STR 0.053, DEX 0.158, INT 0.2). This finds each param's blob in the
// live process by its type name at header +0x0C, checks the header the way ds2-regulation.py does
// (row count at +0x0A, data start == 0x40 + rows * 24), and reads the same rows. A blob that fails
// the header check is reported and skipped, not read. Read-only; hooks nothing.

'use strict';

const TARGETS = [
  {
    type: 'PHYS_STATS_PER_LEVEL_STAT_PARAM', rows: 99,
    reads: [1, 2, 3, 4, 5, 20, 50, 99].map(function (id) {
      return { id: id, field: 'defense', offset: 0x80, kind: 's32' };
    }),
  },
  {
    type: 'WEAPON_STATS_AFFECT_PARAM', rows: 1849,
    // row + 8 + level * 0x24 is the level's nine coefficients (docs/DS2-DPS-MECHANICS.md)
    reads: [
      { id: 1005038, field: 'physicalByStrength10', offset: 8 + 10 * 0x24 + 0, kind: 'f32' },
      { id: 1005038, field: 'physicalByDexterity10', offset: 8 + 10 * 0x24 + 4, kind: 'f32' },
      { id: 1005038, field: 'physicalByEnchant10', offset: 8 + 10 * 0x24 + 32, kind: 'f32' },
      { id: 1005033, field: 'physicalByStrength10 (Fire row)', offset: 8 + 10 * 0x24 + 0, kind: 'f32' },
    ],
  },
];

function ascii(s) {
  const out = [];
  for (let i = 0; i < s.length; i++) out.push(('0' + s.charCodeAt(i).toString(16)).slice(-2));
  return out.join(' ') + ' 00';
}

function readRow(blob, rows, id) {
  for (let k = 0; k < rows; k++) {
    const at = blob.add(0x40 + k * 24);
    if (at.readU64().toNumber() === id) {
      const off = at.add(8).readU64();
      // an offset from the blob in the file; an absolute pointer if the loader relocated it
      return off.compare(uint64(0x10000000)) < 0 ? blob.add(off) : ptr(off.toString());
    }
  }
  return null;
}

function scan(target) {
  const pattern = ascii(target.type);
  let blobs = 0;
  for (const range of Process.enumerateRanges('rw-')) {
    let hits;
    try {
      hits = Memory.scanSync(range.base, range.size, pattern);
    } catch (e) {
      continue;
    }
    for (const hit of hits) {
      const blob = hit.address.sub(0x0c);
      try {
        const rows = blob.add(0x0a).readU16();
        const dataStart = blob.add(0x30).readU64().toNumber();
        if (rows !== target.rows || dataStart !== 0x40 + rows * 24) {
          send({ type: target.type, blob: blob.toString(), skipped: 'header', rows: rows, dataStart: dataStart });
          continue;
        }
        blobs += 1;
        const values = {};
        for (const r of target.reads) {
          const row = readRow(blob, rows, r.id);
          const at = row && row.add(r.offset);
          values[r.id + ' ' + r.field] = at === null ? null
            : (r.kind === 'f32' ? Math.round(at.readFloat() * 1e4) / 1e4 : at.readS32());
        }
        send({ type: target.type, blob: blob.toString(), values: values });
      } catch (e) {
        send({ type: target.type, blob: blob.toString(), skipped: e.message });
      }
    }
  }
  send({ type: target.type, done: true, blobs: blobs });
}

for (const target of TARGETS) scan(target);
