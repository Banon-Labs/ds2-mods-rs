// Swap two equipped weapons through the game's own equip, on the game thread, and put them back,
// so `ds2-weapon-sync`'s "equipped changed under cap" lines can be seen without a person at the
// controller.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/weapon-sync-swap.js`
//
// WHAT IT DOES. Hooks the one main-loop frame (`app-frame`, `ds2_rva::BOOT_PHASES`, 0x140aeeed0,
// not detoured by any crate in this repo unless `--boot-timeline`) and, from inside it -- the game
// thread, between frames -- calls `ItemInventory2::SetEquip` (`ds2_rva::ITEM_SET_EQUIP`,
// `fn(ItemInventory2*, internal slot, ItemEntry*)`), the path the equipment menu takes:
//
//   frame 60   slot 2 <- a pack weapon that is not equipped
//   frame 240  slot 3 <- another one
//   frame 420  slot 2 <- what slot 2 held before
//   frame 600  slot 3 <- what slot 3 held before
//
// Each step prints the entry it equipped, and what the bag's equipped-entry table holds for that
// slot afterwards. It reads the weapon-sync lines' effect only through the game's own state; the
// proof is in `ds2-loader.log`.
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const APP_FRAME = at(0x00aeeed0);
const SET_EQUIP = new NativeFunction(at(0x001ac510), 'void', ['pointer', 'uint32', 'pointer']);
const say = (t) => console.log('[weapon-sync-swap] ' + t);

function p(base, off) {
  try {
    const v = base.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function chain() {
  const gm = p(at(0x16148f0), 0);
  const gdm = gm && p(gm, 0xa8);
  const inv = gdm && p(gdm, 0x10);
  const mgr = inv && p(inv, 0x10);
  const bag = mgr && p(mgr, 0x10);
  return { inv, bag };
}

function describe(e) {
  if (!e) return 'empty';
  return e + ' id=' + e.add(0x14).readU32() + ' handle=' + e.add(0x1c).readU16() + ' flags=0x' +
    e.add(0x1f).readU8().toString(16) + ' lvl=' + (e.add(0x25).readU8() & 0xf);
}

function equippedEntry(bag, slot) {
  return p(bag, 0x25830 + slot * 8);
}

function spares(bag, count) {
  const worn = [];
  for (let s = 0; s < 6; s++) {
    const e = equippedEntry(bag, s);
    if (e) worn.push(e.add(0x14).readU32());
  }
  const out = [];
  for (let k = 0; k < 3840 && out.length < count; k++) {
    const e = bag.add(0x28 + k * 0x28);
    const id = e.add(0x14).readU32();
    if (id === 0 || id === 0xffffffff) continue;
    if (e.add(0x1e).readU8() !== 0) continue;
    if (e.add(0x1c).readU16() !== k) continue;
    const flags = e.add(0x1f).readU8();
    if (flags & 0x06) continue;
    if (worn.indexOf(id) >= 0) continue;
    out.push(e);
  }
  return out;
}

const { inv, bag } = chain();
if (!bag) {
  say('no bag; not in the world');
} else {
  const before = [equippedEntry(bag, 2), equippedEntry(bag, 3)];
  const picks = spares(bag, 2);
  say('slot2 before ' + describe(before[0]));
  say('slot3 before ' + describe(before[1]));
  picks.forEach((e, i) => say('spare' + i + ' ' + describe(e)));
  const plan = [
    [60, 2, picks[0]],
    [240, 3, picks[1]],
    [420, 2, before[0]],
    [600, 3, before[1]],
  ];
  let frame = 0;
  let step = 0;
  Interceptor.attach(APP_FRAME, {
    onEnter() {
      frame += 1;
      if (step >= plan.length || frame !== plan[step][0]) return;
      const [, slot, entry] = plan[step];
      step += 1;
      if (!entry) {
        say('step ' + step + ' slot' + slot + ': nothing to equip, skipped');
        return;
      }
      try {
        SET_EQUIP(inv, slot, entry);
        say('step ' + step + ' frame ' + frame + ' slot' + slot + ' <- ' + describe(entry) + ' ; now holds ' +
          describe(equippedEntry(bag, slot)));
      } catch (err) {
        say('step ' + step + ' threw ' + err);
      }
      send({ kind: 'step', step });
    },
  });
  say('armed on app-frame; four steps over ~10 s');
}
