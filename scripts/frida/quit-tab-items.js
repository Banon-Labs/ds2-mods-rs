// What the pause menu's quit tab holds right now: its item vector, read out of the live object.
//
// `uv run -q --with frida python3 scripts/ds2-frida-up.py --force`, then
// `uv run -q --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/quit-tab-items.js`
//
// There is no global that leads to `FeGroupInGameTopSelect`, so this finds it by its vtable
// (`0x1410b67f8`, see `ds2_rva` beside `FE_TOP_MENU_UPDATE`) in the process's writable memory, and
// accepts a hit only when the quit tab subobject at `+0x738` carries `FeGroupInGameGroupSelect`'s
// vtable (`0x1410b6658`). From that tab: the item `DLFixedVector` at `+0xf8`, its count at
// `+0xf8 + 0x30`, one `(action u32, gate u32)` pair per 8-byte slot. The shipped game leaves three
// entries there, actions 7, 8 and 9. It hooks nothing and writes nothing.
//
// The scan checks itself against `GameManagerImp`: the global `0x1416148f0` points at a live heap
// object, so that object's own vtable must turn up at least once. An agent-owned allocation cannot
// serve as the check, because Frida cloaks its own memory from `enumerateRanges`.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;
const GAME_MANAGER_GLOBAL_RVA = 0x016148f0;
const TOP_SELECT_VTABLE_RVA = 0x010b67f8;
const GROUP_SELECT_VTABLE_RVA = 0x010b6658;
const QUIT_TAB_OFFSET = 0x738;
const ITEM_VECTOR_OFFSET = 0xf8;
const ITEM_COUNT_OFFSET = 0x30;
const ITEM_STRIDE = 8;
const ITEM_CAPACITY = 5;

function readPtr(p) {
  try {
    return p.readPointer();
  } catch (e) {
    return null;
  }
}

function pattern(pointer) {
  // Little-endian bytes of the 8-byte pointer value, as a scan pattern.
  const bytes = [];
  let value = uint64(pointer.toString());
  for (let i = 0; i < 8; i += 1) {
    bytes.push(value.and(0xff).toNumber().toString(16).padStart(2, '0'));
    value = value.shr(8);
  }
  return bytes.join(' ');
}

function scanAll(needle) {
  const found = [];
  for (const range of Process.enumerateRanges('rw-')) {
    try {
      for (const match of Memory.scanSync(range.base, range.size, needle)) {
        if (match.address.and(7).isNull()) {
          found.push(match.address);
        }
      }
    } catch (e) {
      // A range that went away or is unreadable is skipped.
    }
  }
  return found;
}

function describeTab(tag, top) {
  const tab = top.add(QUIT_TAB_OFFSET);
  const tabVtable = readPtr(tab);
  if (tabVtable === null || !tabVtable.equals(base.add(GROUP_SELECT_VTABLE_RVA))) {
    return false;
  }
  const vector = tab.add(ITEM_VECTOR_OFFSET);
  const count = vector.add(ITEM_COUNT_OFFSET).readU64().toNumber();
  // Inline storage, aligned to four: elements at `vector + (-vector & 3) + n * 8`.
  const first = vector.add(uint64(0).sub(uint64(vector.toString())).and(3).toNumber());
  const items = [];
  for (let i = 0; i < Math.min(count, ITEM_CAPACITY); i += 1) {
    const slot = first.add(i * ITEM_STRIDE);
    items.push('(' + slot.readU32() + ',' + slot.add(4).readU32() + ')');
  }
  console.log('[quit-tab] ' + tag + ' top=' + top + ' tab=' + tab + ' count=' + count +
    ' items=' + items.join(' '));
  return true;
}

function sample(tag) {
  const manager = readPtr(base.add(GAME_MANAGER_GLOBAL_RVA));
  const managerVtable = manager === null || manager.isNull() ? null : readPtr(manager);
  const selfCheck = managerVtable === null ? 'no GameManagerImp' :
    'GameManagerImp vtable hits=' + scanAll(pattern(managerVtable)).length;
  const tops = scanAll(pattern(base.add(TOP_SELECT_VTABLE_RVA)));
  const groups = scanAll(pattern(base.add(GROUP_SELECT_VTABLE_RVA)));
  let accepted = 0;
  for (const top of tops) {
    if (describeTab(tag, top)) {
      accepted += 1;
    }
  }
  console.log('[quit-tab] ' + tag + ' ' + selfCheck + ' top-select hits=' + tops.length +
    ' accepted=' + accepted + ' group-select hits=' + groups.length);
}

sample('load');
let n = 0;
setInterval(function () {
  n += 1;
  sample(String(n));
}, 5000);
