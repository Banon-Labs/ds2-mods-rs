// What does taking an imported weapon off actually do to its inventory entry?
//
// THE QUESTION. A build import equips gear the character already holds -- the run's own log says
// `nothing to grant` and `equipped 10/10`, and the entries it equipped were confirmed live through
// the game's own `ITEM_INVENTORY_ENTRY_BY_HANDLE`. Unequip the sword anyway and it is not in the
// inventory. So the entry the equip used was real, and something between there and the inventory
// screen loses it. Static reading has taken this as far as it goes: the unequip body at 0x1401b4330
// clears a bit, zeroes a slot, and for a weapon cannot delete an entry. This records what it
// actually does.
//
// WHAT IS HOOKED, both bodies rather than the thunks, because the thunks hop the receiver and the
// bodies are where the arguments are in their final form:
//
//   0x1401b3d50  equip    fn(bag, internal_slot, item_id, handle)
//   0x1401b4330  unequip  fn(bag, internal_slot)
//
// The entry is `bag + 0x28 + handle * 0x28`, which is how the equip body itself computes the slot
// pointer it stores -- not a layout guessed here. Fields, from `ds2_rva`:
//
//   +0x10  u8   unread by this repo; the unequip body branches on it being 2
//   +0x14  u32  item id
//   +0x1c  u16  handle, 0xFFFF for none
//   +0x1f  u8   flags, bit 0x02 is "equipped"
//   +0x20  u16  quantity
//
// Both sides of the unequip are recorded -- the entry as it is on entry and as it is on leave --
// because the question is what CHANGED, and a single sample cannot answer that.
//
// NO TIMER, and integer gates before any pointer is materialised
// (`frida-js-thread-wedges-in-ds2`). Menu updates run every frame and this is on the game thread.

'use strict';

Interceptor.detachAll();

const EQUIP_BODY = 0x001b3d50;
const UNEQUIP_BODY = 0x001b4330;
//: `addItemToInventory(container, spawns, count, doSave)`, the body behind `ITEM_GIVE`'s thunk.
//
// Hooked because the container it writes to is the question. On a character that already owns
// everything a build names, the grant never runs -- the mod's `already_held` short-circuits it --
// so the only way to see where an item is deposited is a character that does NOT own it. The
// pointer logged here is compared against the one the equip body indexes: same object and the grant
// and the equip agree, different objects and the item is being put somewhere the player's bag is
// not.
const GIVE_BODY = 0x001a7470;
//: `sItemStruct`, the per-item record the grant loops over. Layout from `addItemToInventory`.
const SPAWN_STRIDE = 0x10;
const SPAWN_ITEM_ID = 0x04;
const SPAWN_AMOUNT = 0x0c;

//: `ItemEntry` layout, mirroring `ds2_rva::ITEM_ENTRY_*`.
const ENTRY_ARRAY_OFFSET = 0x28;
const ENTRY_STRIDE = 0x28;
const ENTRY_UNKNOWN_10 = 0x10;
const ENTRY_ITEM_ID = 0x14;
const ENTRY_HANDLE = 0x1c;
const ENTRY_FLAGS = 0x1f;
const ENTRY_QUANTITY = 0x20;

//: The equipped-slot array inside the bag, in `longlong` units, from the equip body's own
//: `param_1[slot + 0x4b06]`.
const EQUIP_SLOT_ARRAY_QWORD = 0x4b06;

//: Enough to cover an import and several unequips without letting the sample grow unbounded.
const SAMPLE_LIMIT = 200;

let seen = 0;
let dumped = false;

//: How many entries the array holds, `ds2_rva::ITEM_ENTRY_COUNT`.
const ENTRY_COUNT = 3840;
//: A whole bag in one message, capped so a corrupt base cannot flood the host.
const DUMP_LIMIT = 400;

//: The handles worth a full row. 430 is the Vessel Shield and 1669 the Ice Rapier -- the two items
//: that went missing, read from the unequip events. The low ones are controls: gear the player can
//: still see, including two rings that are still worn (0x06). A difference between the two groups
//: is the difference this whole investigation is looking for.
const WATCH_HANDLES = [430, 1669, 144, 160, 0, 1, 2, 17, 300];

//: Every slot in the backing array carrying an item id, in one message.
//
// Reported as the raw fields rather than a verdict: the point is to see which of them differs
// between gear the player can still see and gear that has gone, and a script that decided that in
// advance would only find what it was told to look for.
function dumpBag(bag) {
  const rows = [];
  const byFlags = {};
  let occupied = 0;
  let highest = -1;
  for (let index = 0; index < ENTRY_COUNT; index += 1) {
    const entry = bag.add(ENTRY_ARRAY_OFFSET + index * ENTRY_STRIDE);
    let id = 0;
    try {
      id = entry.add(ENTRY_ITEM_ID).readU32();
    } catch (error) {
      break;
    }
    if (id === 0 || id === 0xffffffff) {
      continue;
    }
    occupied += 1;
    highest = index;
    // The shape of the whole array as counts, rather than two thousand rows nobody can read. If
    // the region the imports landed in differs from the region the player's own gear is in, it
    // differs in one of these.
    let flags = -1;
    try {
      flags = entry.add(ENTRY_FLAGS).readU8();
    } catch (error) {
      flags = -1;
    }
    const key = 'flags_0x' + flags.toString(16);
    byFlags[key] = (byFlags[key] || 0) + 1;
    // The named handles, and only those: the two items that vanished, and controls the player can
    // still see. A row here is a row worth reading.
    if (WATCH_HANDLES.indexOf(index) !== -1 && rows.length < DUMP_LIMIT) {
      const row = describe(entry);
      row.index = index;
      rows.push(row);
    }
  }
  send({
    event: 'bag',
    occupied: occupied,
    highest_index: highest,
    by_flags: byFlags,
    watched: rows,
  });
}

//: Read the five fields that say what an entry is, or `null` if it cannot be read.
function describe(entry) {
  if (entry === null || entry.isNull()) {
    return null;
  }
  try {
    return {
      at: entry.toString(),
      byte10: entry.add(ENTRY_UNKNOWN_10).readU8(),
      item_id: entry.add(ENTRY_ITEM_ID).readU32(),
      handle: entry.add(ENTRY_HANDLE).readU16(),
      flags: entry.add(ENTRY_FLAGS).readU8(),
      quantity: entry.add(ENTRY_QUANTITY).readU16(),
    };
  } catch (error) {
    return { at: entry.toString(), unreadable: String(error) };
  }
}

const module = Process.findModuleByName('DarkSoulsII.exe');
if (module === null) {
  send({ error: 'DarkSoulsII.exe is not loaded' });
} else {
  const equip = module.base.add(EQUIP_BODY);
  const unequip = module.base.add(UNEQUIP_BODY);
  console.log('[equip-entry] equip ' + equip + '  unequip ' + unequip);

  Interceptor.attach(equip, {
    onEnter: function (args) {
      if (seen >= SAMPLE_LIMIT) {
        return;
      }
      seen += 1;
      const bag = args[0];
      const slot = args[1].toInt32();
      const itemId = args[2].toUInt32();
      const handle = args[3].toInt32() & 0xffff;
      const entry = bag.add(ENTRY_ARRAY_OFFSET + handle * ENTRY_STRIDE);
      send({
        event: 'equip',
        // The container, so it can be compared with the one the grant wrote to.
        container: bag.toString(),
        slot: slot,
        asked_item_id: itemId,
        handle: handle,
        entry: describe(entry),
      });
    },
  });

  Interceptor.attach(module.base.add(GIVE_BODY), {
    onEnter: function (args) {
      if (seen >= SAMPLE_LIMIT) {
        this.skip = true;
        return;
      }
      seen += 1;
      this.container = args[0];
      const count = args[2].toUInt32();
      // The ids in this batch, bounded: a corrupt count must not walk the heap.
      const ids = [];
      const capped = count > 64 ? 64 : count;
      for (let i = 0; i < capped; i += 1) {
        try {
          ids.push({
            item_id: args[1].add(i * SPAWN_STRIDE + SPAWN_ITEM_ID).readU32(),
            amount: args[1].add(i * SPAWN_STRIDE + SPAWN_AMOUNT).readU16(),
          });
        } catch (error) {
          break;
        }
      }
      this.ids = ids;
      this.count = count;
      this.doSave = args[3].toInt32();
    },
    onLeave: function (retval) {
      if (this.skip) {
        return;
      }
      // `false` means the batch was refused outright -- `canInventAcceptItemBag` said no, which is
      // what a full container looks like from here, and the grant silently adds nothing.
      send({
        event: 'give',
        container: this.container.toString(),
        count: this.count,
        do_save: this.doSave,
        accepted: retval.toInt32() & 0xff,
        items: this.ids,
      });
    },
  });

  Interceptor.attach(unequip, {
    onEnter: function (args) {
      if (seen >= SAMPLE_LIMIT) {
        this.skip = true;
        return;
      }
      seen += 1;
      const bag = args[0];
      const slot = args[1].toInt32();
      // The slot's own pointer, which is the entry the game is about to take off. Read here
      // because the body zeroes it.
      const held = bag.add((EQUIP_SLOT_ARRAY_QWORD + slot) * 8).readPointer();
      this.bag = bag;
      this.slot = slot;
      this.entry = held;
      this.before = describe(held);
    },
    onLeave: function () {
      if (this.skip) {
        return;
      }
      send({
        event: 'unequip',
        slot: this.slot,
        before: this.before,
        after: describe(this.entry),
      });
      // AND THE WHOLE BAG, once, on the first unequip that reaches here.
      //
      // The first measurement said the entry survives with `quantity: 0`, which is a finding about
      // one item and not yet a cause: a weapon is not a stack, so zero may be what every weapon
      // carries. The control is the player's own gear -- items they can still see in the inventory
      // -- read out of the same array at the same instant. Whatever field separates the visible
      // from the vanished is the one that matters, and nothing but a side-by-side dump can name it.
      if (!dumped) {
        dumped = true;
        dumpBag(this.bag);
      }
    },
  });
}
