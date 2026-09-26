// Does the sort dialog on the equip picker offer options that suit the slot, and does choosing one
// reorder the list? (ds2-mods-rs-6t8)
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/sort-dialog-check.js \
//     --role sort-dialog-check --log ~/.cache/er-frida/sort-dialog-check.jsonl`
//
// READ-ONLY. Interceptor hooks and memory reads, nothing else: no write to game memory, no call
// into the game, no retval replacement, no timer (Frida's setTimeout/setInterval never fire in this
// game). Every event is one `send()`.
//
// WHAT IT LOGS
//
//   picker    FeGroupItemEquip::ctor returned. `kind` is [group+0x2008], the equip-slot kind the
//             picker was built for (the ctor stores its third argument there at 0x14008c1cb), and
//             `kind_name`/`category` are that row of the table at 0x141568be0 (stride 0x18, name
//             pointer at +0, category byte at +9 -- FeIngameItemSelectMenu::v56 at 0x14008ead0
//             returns exactly that byte through 0x1401544d0). Read statically from the table:
//             L_hand1..R_hand3 -> 0, Helmet/Armor/Guntlet/Leggings -> 1, Ring[0..3] -> 2,
//             Arrow1/2 Bolt1/2 -> 3, Item[0..9] -> 4, Spell[..] -> 5, Quiver0/1 -> 0xff.
//   open      the sort dialog opener 0x1400747e0 was entered: group, which class, busy word.
//   options   the option slice FUN_1400349d0(out, u8 category) returned to the opener: the category
//             it was asked for and each {u32 key, u32 fmgId} pair in [out[0], out[1]), with the
//             common.fmg label. An empty slice means the opener returns without a dialog.
//   choose    the row's closure 0x1400ba300(group, key) ran -- the option the player picked.
//   set_key   setSortKey 0x1401ac610(inventory, u8 category, i16 key): what the dialog wrote.
//   build     the shared builder 0x140036080(source, out, index) returned: who called it, the
//             category and raw key it read (getSortKey 0x1401ac0a0, called from 0x140036107, so
//             return address 0x14003610c), the effective key it stored at out+0x18 (0 becomes 0x5B),
//             the row count at out+0x4028, and the first FIRST_N rows in display order as
//             {handle, item_id, value}. A sort that works shows a `choose` -> `set_key` -> `build`
//             run whose `build` has the new key and a different first-rows order from the `build`
//             logged when the picker opened.
//
// The builder output is a DLFixedVector<{u16 handle, u16 pad, f32 value}, 0x800>: elements at
// out+0x20 (aligned up to 4), count at out+0x20+0x4008, per 0x140033930 and the push at
// 0x140034420. Item id: handle <= 0xEFF is a bag slot (0x1401b87b0), and the bag's lookup
// ItemInventory2BagList::v2 (0x1401b2290) is `bag + (handle + 1) * 0x28`; the bag is
// [[ItemInventory2 + 0x10] + 0x10] and ItemInventory2 is [[GameManagerImp] + 0xa8] + 0x10
// (0x140040420). The bag is only trusted when its first qword is BagList's vtable 0x1410c4408.
//
// HOOKED ADDRESSES AND THEIR ENTRY BYTES, read from darksoulsii-deobf.bin and re-checked here in
// memory before each attach (a mismatch skips that hook and says so):
//
//   0x14008c0d0  FeGroupItemEquip::ctor   44 89 44 24 18      mov [rsp+0x18],r8d   (ds2_rva)
//   0x1400747e0  sort dialog opener       40 55 53 48 8d      push rbp; push rbx   (ds2_rva)
//   0x1400349d0  option slicer            84 d2 75 19 48 8d   test dl,dl; jne      leaf; own code,
//                                         not an Arxan e9 stub; no branch targets its first bytes
//   0x1400ba300  option closure           48 89 5c 24 08      mov [rsp+0x8],rbx
//   0x1401ac610  setSortKey               48 89 5c 24 10      mov [rsp+0x10],rbx
//   0x1401ac0a0  getSortKey               40 53 48 83 ec      push rbx; sub rsp
//   0x140036080  shared list builder      48 89 5c 24 18      mov [rsp+0x18],rbx
//
// `scripts/ds2-arxan-chain.py` reports NOT REDIRECTED for all but the slicer, where it stops at
// UNKNOWN because `84 d2` is not a frame prologue; its disassembly is the function's own body.
// With ds2-inventory-sort loaded the equip ctor starts with that crate's MinHook `e9` instead; the
// check accepts that one site in that one form and the `armed` event reports `over-minhook`.
//
// Triggering the dialog: ds2-inventory-sort reads its pad button with its own XInputGetState call
// and its key with GetAsyncKeyState, so the input harness's `buttons` verb (a write to
// PadDevice+0x198 after the game's poll) does not reach it. Press L3 on the physical pad, or the
// configured key, with the game focused.

'use strict';

Interceptor.detachAll();

const image = Process.getModuleByName('DarkSoulsII.exe');
const IMAGE = ptr('0x140000000');
const at = (va) => image.base.add(ptr(va).sub(IMAGE));
const rvaOf = (p) => '0x' + p.sub(image.base).toString(16);

//: Hook sites, each with the bytes it must start with.
const SITES = {
  equipCtor: { va: '0x14008c0d0', bytes: [0x44, 0x89, 0x44, 0x24, 0x18], minhooked: true },
  open: { va: '0x1400747e0', bytes: [0x40, 0x55, 0x53, 0x48, 0x8d] },
  slicer: { va: '0x1400349d0', bytes: [0x84, 0xd2, 0x75, 0x19, 0x48, 0x8d] },
  choose: { va: '0x1400ba300', bytes: [0x48, 0x89, 0x5c, 0x24, 0x08] },
  setKey: { va: '0x1401ac610', bytes: [0x48, 0x89, 0x5c, 0x24, 0x10] },
  getKey: { va: '0x1401ac0a0', bytes: [0x40, 0x53, 0x48, 0x83, 0xec] },
  build: { va: '0x140036080', bytes: [0x48, 0x89, 0x5c, 0x24, 0x18] },
};

//: Return addresses that name a caller.
const RET_OPEN_SLICE = at('0x14007482c'); // opener's call to the slicer
const RET_BUILD_GETKEY = at('0x14003610c'); // builder's call to getSortKey
const BUILD_CALLERS = {};
BUILD_CALLERS[at('0x14009742c').toString()] = 'equip-rebuild (FeIngameItemSelectMenu::v57)';
BUILD_CALLERS[at('0x1400744f1').toString()] = 'inventory 0x1400744d0';
BUILD_CALLERS[at('0x1400ba9ae').toString()] = 'inventory 0x1400ba980';
BUILD_CALLERS[at('0x1400c0154').toString()] = 'fillMenuData';
BUILD_CALLERS[at('0x1400c0294').toString()] = '0x1400c0240';
BUILD_CALLERS[at('0x1400c258e').toString()] = '0x1400c2530';
BUILD_CALLERS[at('0x1400d130c').toString()] = '0x1400d12a0';
BUILD_CALLERS[at('0x140034e00').toString()] = '0x140034dc0';

//: Class identity by primary vtable.
const VTABLES = {};
VTABLES[at('0x1410b46c8').toString()] = 'FeGroupItemEquip';
VTABLES[at('0x1410b1b38').toString()] = 'FeGroupInGameMenuInventory2';

//: FeGroupItemEquip fields.
const EQUIP_KIND_OFFSET = 0x2008;
const GROUP_BUSY_OFFSET = 0x58;
//: The equip-slot kind table read by 0x1401544d0.
const KIND_TABLE = at('0x141568be0');
const KIND_STRIDE = 0x18;
const KIND_COUNT = 0x34;
const KIND_CATEGORY_OFFSET = 9;

//: Builder output layout.
const OUT_KEY_OFFSET = 0x18;
const OUT_VECTOR_OFFSET = 0x20;
const VECTOR_COUNT_OFFSET = 0x4008;
const VECTOR_CAPACITY = 0x800;
const ROW_STRIDE = 8;
const FIRST_N = 10;

//: Inventory chain.
const GAME_MANAGER_IMP = at('0x1416148f0');
const BAG_LIST_VTABLE = at('0x1410c4408');
const BAG_HANDLE_MAX = 0xeff;
const ENTRY_STRIDE = 0x28;
const ENTRY_ITEM_ID = 0x14;

//: common.fmg labels for the option table's fmg ids (docs/DS2-INVENTORY-SORT.md).
const LABELS = {
  80050101: 'Default positions', 80050102: 'By effect', 80050103: 'Attack',
  80050104: 'Weight', 80050105: 'Damage reduction',
  80050201: 'Default positions', 80050203: 'Defense', 80050204: 'Weight',
  80050301: 'Default positions', 80050302: 'Weight',
  80050401: 'Default positions', 80050402: 'Attack', 80050403: 'Held',
  80050501: 'Default positions', 80050502: 'Held',
};
const KEY_NAMES = {
  0x5b: 'Default positions', 0x5c: 'Attack', 0x5d: 'Damage reduction', 0x5e: 'Defense',
  0x5f: 'By effect', 0x28: 'Weight', 0x0f: 'Weight', 0x3a: 'Weight', 0x50: 'Held',
};

//: Bounds on output. Identical consecutive builds from one caller are counted, not re-sent.
const BUILD_CAP = 300;
let builds = 0;
const lastBuild = {};
const repeats = {};
//: getSortKey's answer to the builder, per thread, consumed by the builder's onLeave.
const keyRead = {};

function kindRow(kind) {
  if (kind < 0 || kind >= KIND_COUNT) return { kind: kind, kind_name: null, category: null };
  const row = KIND_TABLE.add(kind * KIND_STRIDE);
  let name = null;
  try { name = row.readPointer().readCString(); } catch (e) { name = null; }
  return { kind: kind, kind_name: name, category: row.add(KIND_CATEGORY_OFFSET).readU8() };
}

function describeGroup(group) {
  const out = { group: group.toString() };
  try {
    const vt = group.readPointer();
    out.vtable = rvaOf(vt);
    out.class = VTABLES[vt.toString()] || null;
    out.busy = group.add(GROUP_BUSY_OFFSET).readU64().toString();
    if (out.class === 'FeGroupItemEquip') {
      Object.assign(out, kindRow(group.add(EQUIP_KIND_OFFSET).readS32()));
    }
  } catch (e) {
    out.unreadable = String(e);
  }
  return out;
}

function bag() {
  try {
    const gm = GAME_MANAGER_IMP.readPointer();
    if (gm.isNull()) return null;
    const gdm = gm.add(0xa8).readPointer();
    if (gdm.isNull()) return null;
    const inv = gdm.add(0x10).readPointer();
    if (inv.isNull()) return null;
    const mgr = inv.add(0x10).readPointer();
    if (mgr.isNull()) return null;
    const list = mgr.add(0x10).readPointer();
    if (list.isNull() || !list.readPointer().equals(BAG_LIST_VTABLE)) return null;
    return list;
  } catch (e) {
    return null;
  }
}

function itemIdOf(list, handle) {
  if (list === null || handle > BAG_HANDLE_MAX) return null;
  try {
    return list.add((handle + 1) * ENTRY_STRIDE + ENTRY_ITEM_ID).readU32();
  } catch (e) {
    return null;
  }
}

function attach(name, callbacks) {
  const site = SITES[name];
  const target = at(site.va);
  let seen;
  try {
    seen = Array.from(new Uint8Array(target.readByteArray(site.bytes.length)));
  } catch (e) {
    send({ event: 'hook-skipped', site: name, va: site.va, reason: String(e) });
    return false;
  }
  const clean = seen.every((b, i) => b === site.bytes[i]);
  // ds2-inventory-sort MinHooks the equip ctor, so with it loaded that entry is its `e9` detour
  // jump. Frida relocates a leading jmp rel32 into its own trampoline, so the chain still runs;
  // the event says which case it was. Only sites listed as `minhooked` accept this.
  const minhooked = site.minhooked === true && seen[0] === 0xe9;
  if (!clean && !minhooked) {
    send({ event: 'hook-skipped', site: name, va: site.va, reason: 'prologue mismatch', seen: seen });
    return false;
  }
  Interceptor.attach(target, callbacks);
  return clean ? 'clean' : 'over-minhook';
}

const armed = {};

armed.equipCtor = attach('equipCtor', {
  onEnter(args) {
    this.group = args[0];
    this.kindArg = args[2].toUInt32();
  },
  onLeave() {
    const info = describeGroup(this.group);
    info.event = 'picker';
    info.kind_arg = this.kindArg;
    send(info);
  },
});

armed.open = attach('open', {
  onEnter(args) {
    const info = describeGroup(args[0]);
    info.event = 'open';
    send(info);
  },
});

armed.slicer = attach('slicer', {
  onEnter(args) {
    this.wanted = this.returnAddress.equals(RET_OPEN_SLICE);
    if (!this.wanted) return;
    this.out = args[0];
    this.category = args[1].toUInt32() & 0xff;
  },
  onLeave() {
    if (!this.wanted) return;
    const begin = this.out.readPointer();
    const end = this.out.add(8).readPointer();
    const options = [];
    if (!begin.isNull() && !end.isNull()) {
      const count = end.sub(begin).toInt32() >> 3;
      for (let i = 0; i < count && i < 16; i += 1) {
        const key = begin.add(i * 8).readU32();
        const fmg = begin.add(i * 8 + 4).readU32();
        options.push({ key: key, fmg: fmg, label: LABELS[fmg] || null });
      }
    }
    send({
      event: 'options',
      category: this.category,
      begin: begin.isNull() ? null : rvaOf(begin),
      end: end.isNull() ? null : rvaOf(end),
      options: options,
    });
  },
});

armed.choose = attach('choose', {
  onEnter(args) {
    const key = args[1].toInt32();
    const info = describeGroup(args[0]);
    info.event = 'choose';
    info.key = key;
    info.key_name = KEY_NAMES[key] || null;
    send(info);
  },
});

armed.setKey = attach('setKey', {
  onEnter(args) {
    const key = (args[2].toInt32() << 16) >> 16;
    const abs = key < 0 ? -key : key;
    send({
      event: 'set_key',
      category: args[1].toUInt32() & 0xff,
      key: key,
      key_name: KEY_NAMES[abs] || null,
      descending: key < 0,
      caller: rvaOf(this.returnAddress),
    });
  },
});

armed.getKey = attach('getKey', {
  onEnter(args) {
    this.wanted = this.returnAddress.equals(RET_BUILD_GETKEY);
    if (this.wanted) this.category = args[1].toUInt32() & 0xff;
  },
  onLeave(retval) {
    if (!this.wanted) return;
    keyRead[this.threadId] = { category: this.category, raw_key: (retval.toInt32() << 16) >> 16 };
  },
});

armed.build = attach('build', {
  onEnter(args) {
    this.skip = builds >= BUILD_CAP;
    if (this.skip) return;
    this.out = args[1];
    this.index = args[2].toInt32();
    this.caller = this.returnAddress.toString();
    delete keyRead[this.threadId];
  },
  onLeave() {
    if (this.skip) return;
    const read = keyRead[this.threadId] || { category: null, raw_key: null };
    delete keyRead[this.threadId];
    let key = null;
    let count = null;
    const rows = [];
    try {
      key = (this.out.add(OUT_KEY_OFFSET).readU16() << 16) >> 16;
      const vec = this.out.add(OUT_VECTOR_OFFSET);
      count = vec.add(VECTOR_COUNT_OFFSET).readU64().toNumber();
      const pad = (4 - (vec.and(3).toInt32())) & 3;
      const data = vec.add(pad);
      const list = bag();
      const shown = count > VECTOR_CAPACITY ? 0 : Math.min(count, FIRST_N);
      for (let i = 0; i < shown; i += 1) {
        const handle = data.add(i * ROW_STRIDE).readU16();
        rows.push({
          handle: handle,
          item_id: itemIdOf(list, handle),
          value: data.add(i * ROW_STRIDE + 4).readFloat(),
        });
      }
    } catch (e) {
      rows.push({ unreadable: String(e) });
    }
    const abs = key === null ? null : (key < 0 ? -key : key);
    const signature = [read.category, key, count, JSON.stringify(rows)].join('|');
    if (lastBuild[this.caller] === signature) {
      repeats[this.caller] = (repeats[this.caller] || 0) + 1;
      return;
    }
    lastBuild[this.caller] = signature;
    builds += 1;
    send({
      event: 'build',
      caller: BUILD_CALLERS[this.caller] || rvaOf(ptr(this.caller)),
      index: this.index,
      category: read.category,
      raw_key: read.raw_key,
      key: key,
      key_name: abs === null ? null : (KEY_NAMES[abs] || null),
      descending: key !== null && key < 0,
      count: count,
      identical_builds_skipped: repeats[this.caller] || 0,
      first: rows,
    });
  },
});

send({ event: 'armed', hooks: armed, base: image.base.toString() });
