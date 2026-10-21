// Make the local player use a goods item the way the belt's use button does, without input.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/use-item.js`
//
// # How (static RE 2026-10-21, darksoulsii-deobf.bin)
//
// Both the belt button and the inventory menu's Use end in the frontend gate 0x140500c40
// (frontendRoot, i16 handle, u16 count). The gate, once its checks pass, writes three fields of the
// use request at [PlayerCtrl+0xC8]: +0x40 u16 handle, +0x42 u16 count, and bit 0x800 of the u32 at
// +0xFC. ChrItemCtrl's state 0 (0x14032fa00) picks the request up on its next update and starts the
// use through 0x14032f450, which re-checks the bag quantity, the param rows and the ladder rule
// and refuses on any of them. This agent writes those three fields itself: plain data on a struct
// the game polls, so nothing runs on this thread, and the game's own start path still decides.
// It skips the gate's ItemUseCheckDialogParam Yes/No window and its player-state checks.
//
// Modes (edit and reload; this agent hooks nothing, so reloading in place is safe):
//   GOODS_ID = null          list the bag: every entry with a nonzero id, its handle and quantity
//   GOODS_ID = <id>          use one of that item
//
// Every write is followed by a read of ChrItemCtrl's state over the next 3 s, so a request the
// game never took shows as never taken, not as success.

'use strict';

const GOODS_ID = null;
const LIST_FILTER = null; // e.g. [60000000, 70000000] to list only ids in that range

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const GAME_MANAGER_IMP = 0x016148f0;
const GAME_DATA_MANAGER_OFFSET = 0xa8; // GameManagerImp -> GameDataManager
const ITEM_INVENTORY_OFFSET = 0x10; // GameDataManager -> ItemInventory2
const ITEM_BAG_LIST_OFFSET = 0x10; // applied twice: ItemInventory2 -> manager -> bag
const ITEM_ENTRY_ARRAY_OFFSET = 0x28;
const ITEM_ENTRY_STRIDE = 0x28;
const ITEM_ENTRY_COUNT = 3840;
const ITEM_ENTRY_ITEM_ID_OFFSET = 0x14;
const ITEM_ENTRY_HANDLE_OFFSET = 0x1c;
const ITEM_ENTRY_TYPE_OFFSET = 0x1e;
const ITEM_ENTRY_FLAGS_OFFSET = 0x1f;
const ITEM_ENTRY_QUANTITY_OFFSET = 0x20;
const PLAYER_CTRL_OFFSET = 0xd0;
const USE_REQUEST_OFFSET = 0xc8; // PlayerCtrl -> use request struct
const REQUEST_HANDLE_OFFSET = 0x40;
const REQUEST_COUNT_OFFSET = 0x42;
const REQUEST_FLAGS_OFFSET = 0xfc;
const REQUEST_USE_BIT = 0x800;
const ITEM_USE_STATE_OFFSET = 0xb8; // PlayerCtrl -> item-use state block
const ITEM_USE_ACTIVE_OFFSET = 0x6dd;
const ITEM_USE_ITEM_ID_OFFSET = 0x700;

function rp(p) {
  try {
    return p.readPointer();
  } catch (e) {
    return null;
  }
}

function gm() {
  return rp(exe.add(GAME_MANAGER_IMP));
}

function bag() {
  let at = rp(gm().add(GAME_DATA_MANAGER_OFFSET));
  at = rp(at.add(ITEM_INVENTORY_OFFSET));
  at = rp(at.add(ITEM_BAG_LIST_OFFSET));
  return rp(at.add(ITEM_BAG_LIST_OFFSET));
}

function entries() {
  const base = bag().add(ITEM_ENTRY_ARRAY_OFFSET);
  const out = [];
  for (let i = 0; i < ITEM_ENTRY_COUNT; i++) {
    const e = base.add(i * ITEM_ENTRY_STRIDE);
    const id = e.add(ITEM_ENTRY_ITEM_ID_OFFSET).readU32();
    if (id === 0 || id === 0xffffffff) continue;
    out.push({
      index: i,
      id,
      handle: e.add(ITEM_ENTRY_HANDLE_OFFSET).readU16(),
      type: e.add(ITEM_ENTRY_TYPE_OFFSET).readU8(),
      flags: e.add(ITEM_ENTRY_FLAGS_OFFSET).readU8(),
      quantity: e.add(ITEM_ENTRY_QUANTITY_OFFSET).readU32(),
    });
  }
  return out;
}

const ctrl = rp(gm().add(PLAYER_CTRL_OFFSET));
if (ctrl === null || ctrl.isNull()) {
  send({ tag: 'use-item', error: 'no PlayerCtrl' });
} else if (GOODS_ID === null) {
  let list = entries();
  if (LIST_FILTER !== null) list = list.filter((e) => e.id >= LIST_FILTER[0] && e.id < LIST_FILTER[1]);
  for (let i = 0; i < list.length; i += 40) send({ tag: 'bag', entries: list.slice(i, i + 40) });
  send({ tag: 'bag-done', count: list.length });
} else {
  const entry = entries().find((e) => e.id === GOODS_ID && e.quantity > 0);
  if (entry === undefined) {
    send({ tag: 'use-item', error: 'no entry with goods id ' + GOODS_ID });
  } else {
    const req = rp(ctrl.add(USE_REQUEST_OFFSET));
    const state = rp(ctrl.add(ITEM_USE_STATE_OFFSET));
    const before = req.add(REQUEST_FLAGS_OFFSET).readU32();
    req.add(REQUEST_HANDLE_OFFSET).writeU16(entry.handle);
    req.add(REQUEST_COUNT_OFFSET).writeU16(1);
    req.add(REQUEST_FLAGS_OFFSET).writeU32(before | REQUEST_USE_BIT);
    send({ tag: 'use-item', wrote: { handle: entry.handle, goods: GOODS_ID }, flags_before: before });
    let ticks = 0;
    const timer = setInterval(() => {
      ticks += 1;
      const active = state.add(ITEM_USE_ACTIVE_OFFSET).readU8();
      const using = state.add(ITEM_USE_ITEM_ID_OFFSET).readU32();
      const pending = (req.add(REQUEST_FLAGS_OFFSET).readU32() & REQUEST_USE_BIT) !== 0;
      if (active || ticks === 30) {
        send({ tag: 'use-item', taken: active !== 0, item_in_use: using, request_still_pending: pending, after_ms: ticks * 100 });
        clearInterval(timer);
      }
    }, 100);
  }
}
