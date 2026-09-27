// Read-only: the attunement budget, what each attuned spell costs, what every spell in the bag
// costs, and -- passively, only if someone opens Attune Spell -- the game's own greyed-out
// decision and the attunement grid's items. For ds2-item-warn's spell X.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/spell-attune-read.js`
//
// Writes nothing into the game. Addresses, all read statically out of darksoulsii-deobf.bin:
//
//   inventory = [[GameManagerImp]+0xa8]+0x10         (0x140040420)
//   bag       = [[inventory+0x10]+0x10]              (0x1401ac110 -> 0x1401a6790)
//   budget    = u8 bag+0x259ec                       (0x1401a6790)
//   attuned   = 14 entry pointers at bag+0x25910     (0x1401b26a0 sums their +0x21)
//   entry     +0x14 item id, +0x1c handle, +0x1e type (9 = spell), +0x1f bit 1 equipped,
//             +0x21 slot cost
//   cost      = the frontend column 0x40 = u8 row+1  (0x1400373f0 -> 0x140031b90 / 0x1400312e0)
//
// Passive hooks (UI-only, never per frame):
//   0x1400ceed0  SpellBookItemList::getItem(list, out FeItemData, index). On leave, out+5 is the
//                game's greyed-out flag: set when the entry is equipped, or when
//                free + cost(selected slot's spell) < cost(this spell).
//   0x1400b7680  the cell-view builder. From the attunement grid (return address 0x1400cff09) the
//                slot's FeItemData is at [caller rsp + 0x20].
'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const say = (t) => {
  console.log('[spell-attune] ' + t);
  send({ spell_attune: t });
};

function hex(p, n) {
  const b = new Uint8Array(p.readByteArray(n));
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join(' ');
}

// Live prologues, against the bytes the static image holds at the same RVAs.
for (const [rva, want] of [
  [0x000b7680, '48 89 74 24 10 48 89 7c 24 18 55'],
  [0x000cfcf0, '40 55 53 56 57 41 56 48 8d ac 24'],
  [0x000ceed0, '48 8b c4 48 89 58 20 55'],
  [0x000bc850, '48 89 5c 24 18 55 56 57'],
]) {
  const got = hex(at(rva), want.split(' ').length);
  say('prologue rva=0x' + rva.toString(16) + ' live=[' + got + '] static=[' + want + '] ' +
      (got === want ? 'MATCH' : 'DIFFERS'));
}

function p(base, off) {
  try {
    const v = base.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

const lookup = new NativeFunction(at(0x001abfb0), 'pointer', ['pointer', 'uint16']);
const describe = new NativeFunction(at(0x0003c2d0), 'pointer', ['pointer', 'pointer']);
const rowsOf = new NativeFunction(at(0x00035070), 'uint8', ['pointer', 'pointer', 'pointer']);
const columnOf = new NativeFunction(at(0x000312e0), 'uint64', ['pointer', 'uint32']);
const gameColumn = new NativeFunction(at(0x000373f0), 'uint32', ['pointer', 'uint32']);

// The frontend's own column for `key`, through the chain ds2-item-warn already calls, and through
// the game's greyed-out path (0x1400373f0), so the two can be compared.
function columns(handle, key) {
  const item = Memory.alloc(8);
  item.writeU8(1);
  item.add(2).writeU16(handle);
  item.add(4).writeU16(0);
  const desc = Memory.alloc(0x50);
  describe(item, desc);
  const allocator = desc.add(6 * 8).readPointer();
  if (allocator.isNull()) return null;
  const rows = Memory.alloc(0x48);
  if (rowsOf(allocator, rows, desc) === 0) return null;
  const row = rows.readPointer();
  if (row.isNull()) return null;
  return {
    mine: columnOf(row, key).toNumber() & 0xffff,
    game: gameColumn(rows, key),
  };
}

function snapshot() {
  const gmi = at(0x016148f0).readPointer();
  if (gmi.isNull()) return say('no GameManagerImp yet');
  const gdm = p(gmi, 0xa8);
  const inventory = gdm && p(gdm, 0x10);
  const mid = inventory && p(inventory, 0x10);
  const bag = mid && p(mid, 0x10);
  if (!bag) return say('no bag yet (title screen?)');
  const budget = bag.add(0x259ec).readU8();
  const base = bag.add(0x259ed).readU8();
  let used = 0;
  const attuned = [];
  for (let i = 0; i < 14; i++) {
    const e = p(bag, 0x25910 + i * 8);
    if (!e) break;
    const cost = e.add(0x21).readU8();
    used += cost;
    attuned.push('pos' + i + ' id=' + e.add(0x14).readU32() + ' handle=0x' +
      e.add(0x1c).readU16().toString(16) + ' type=' + e.add(0x1e).readU8() + ' flags=0x' +
      e.add(0x1f).readU8().toString(16) + ' cost=' + cost);
  }
  say('budget=' + budget + ' base=' + base + ' used=' + used + ' free=' + (budget - used) +
      ' attuned=' + attuned.length);
  attuned.forEach(say);
  let spells = 0;
  for (let h = 0; h < 0x1400; h++) {
    const e = lookup(inventory, h);
    if (e.isNull() || e.add(0x1e).readU8() !== 9) continue;
    spells++;
    const cost = e.add(0x21).readU8();
    const c = columns(h, 0x40);
    const intel = columns(h, 0x42);
    const faith = columns(h, 0x43);
    const equipped = (e.add(0x1f).readU8() >> 1) & 1;
    say('spell handle=0x' + h.toString(16) + ' id=' + e.add(0x14).readU32() + ' equipped=' +
        equipped + ' entry+0x21=' + cost + ' col40=' + JSON.stringify(c) + ' int=' +
        JSON.stringify(intel) + ' fth=' + JSON.stringify(faith) + ' over-budget=' +
        (cost > budget) + ' over-free=' + (!equipped && cost > budget - used));
  }
  say('spells in bag=' + spells);
}

try {
  snapshot();
} catch (e) {
  say('snapshot threw ' + e);
}

// Off by default: Interceptor attaches froze the game twice on 2026-09-26 (bd recall
// ds2-frida-attach-froze-game-twice-2026-09-26), so a watch on somebody else's session reads only.
const config = globalThis.__ER_FRIDA_CONFIG || {};
const hooks = config.hooks === true;
say('passive hooks ' + (hooks ? 'ON' : 'OFF (pass --config-json {"hooks":true})'));

let greyLines = 0;
if (hooks) Interceptor.attach(at(0x000ceed0), {
  onEnter(args) {
    this.out = args[1];
    this.index = args[2].toInt32();
  },
  onLeave() {
    if (greyLines++ >= 40) return;
    const o = this.out;
    say('getItem index=' + this.index + ' item=[' + hex(o, 6) + '] handle=0x' +
        o.add(2).readU16().toString(16) + ' greyed(+5)=' + o.add(5).readU8());
  },
});

let gridLines = 0;
if (hooks) Interceptor.attach(at(0x000b7680), {
  onEnter() {
    if (!this.returnAddress.equals(at(0x000cff09))) return;
    if (gridLines++ >= 40) return;
    const item = this.context.rsp.add(8 + 0x20);
    say('grid cell view=' + this.context.rcx + ' slot item=[' + hex(item, 6) + '] handle=0x' +
        item.add(2).readU16().toString(16));
  },
});

setInterval(() => {
  try {
    snapshot();
  } catch (e) {
    say('snapshot threw ' + e);
  }
}, 30000);
say('armed');
