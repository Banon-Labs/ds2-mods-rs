// Change only the player's appearance -- face, sex and body -- through the game's own character
// creator. Class, stats, souls, level and inventory stay as they are.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/appearance-only-creator.js \
//   [--config-json '{"trigger":"bonfire"}']`
//
// Without config it opens the creator at once. With "bonfire" it adds a "Change Appearance" row below
// Item box in the bonfire menu and opens the creator when that row is chosen, any number of times.
//
// Opens the full creator in the world (openCharaMakerWindow 0x1401986c0, as open-chara-maker.js),
// then changes what leaving it commits. Static trace and its gaps: docs/DS2-FACE-ONLY-CREATOR.md.
//
// * enter 0x14004c910: snapshot the player's face block (getter ChrAsmModel vtbl+0x68, length 0xa2)
//   and visible equipment (0x140346940(equip, out, slot), slots 0..0x33, 0x14 each), before the enter
//   strips it.
// * 0x1400de610 (class, stats, souls, gift, class re-equip): skipped while face-only is armed.
// * SetFaceData 0x14033a5b0 called from 0x14004cdec (the face commit): keep a copy of the block as
//   the creator built it, sex +0x92 and body +0x93, +0x95 bits 4-6, +0x99 included (user 2026-10-03:
//   "I want to be able to change body and sex too"). SetFaceData alone does not write sex.
// * after the face commit 0x14004cc90 returns: import that block into the player's face part
//   (vtbl+0x188, 0x140340590), which writes sex (ChrAsm vtbl+0x70 +0x2c) and rebuilds the live
//   face; put the equipment records back with
//   0x1403463d0(equip, slot, record) and 0x14037f9d0(player).
//
// Logs the player's face block, equipment records and PlayerGameData's first 0x400 bytes before and
// after, so the run is judged from memory, not from the screen.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const say = (text) => console.log('[appearance-only] ' + text);

const FACE_LEN = 0xa2;
const EQUIP_SLOTS = 0x34;
const EQUIP_RECORD = 0x14;
const WATCH_BYTES = 0x400;

const openCharaMakerWindow = new NativeFunction(at(0x1986c0), 'void', ['pointer']);
const facePartOf = new NativeFunction(at(0x339cd0), 'pointer', ['pointer']);
const equipRead = new NativeFunction(at(0x346940), 'void', ['pointer', 'pointer', 'int']);
const equipWrite = new NativeFunction(at(0x3463d0), 'void', ['pointer', 'int', 'pointer']);
const playerRefresh = new NativeFunction(at(0x37f9d0), 'void', ['pointer']);
const classGiftCommit = new NativeFunction(at(0xde610), 'void', ['pointer']);
const FACE_COMMIT_CALL_RETURN = at(0x4cdec);

function vcall(obj, offset, ret = 'pointer', args = [], values = []) {
  const fn = new NativeFunction(obj.readPointer().add(offset).readPointer(), ret, ['pointer', ...args]);
  return fn(obj, ...values);
}

function gmi() {
  return at(0x16148f0).readPointer();
}

function player() {
  const m = gmi();
  return m.isNull() ? null : m.add(0xd0).readPointer();
}

function playerGameData() {
  const gdm = gmi().add(0xa8).readPointer();
  return gdm.isNull() ? null : gdm.add(0xc0).readPointer();
}

// chr->vtbl+0x120 is the ChrAsm; ChrAsm vtbl+0x80 the model, vtbl+0x60 the visible equipment.
function chrAsm(chr) { return vcall(chr, 0x120); }
function model(chr) { return vcall(chrAsm(chr), 0x80); }
function equip(chr) { return vcall(chrAsm(chr), 0x60); }
function faceBlock(chr) { return vcall(model(chr), 0x68); }

const hex = (bytes) => Array.from(new Uint8Array(bytes), (b) => b.toString(16).padStart(2, '0')).join('');

function readEquip(chr) {
  const e = equip(chr);
  const out = Memory.alloc(EQUIP_RECORD);
  const records = [];
  for (let slot = 0; slot < EQUIP_SLOTS; slot++) {
    out.writeByteArray(new Array(EQUIP_RECORD).fill(0));
    equipRead(e, out, slot);
    records.push(out.readByteArray(EQUIP_RECORD));
  }
  return records;
}

let armed = false;
let snapshot = null;
let committed = null;
let pgdBefore = null;

Interceptor.attach(at(0x4c910), {
  onEnter() {
    if (!armed) return;
    const p = player();
    snapshot = {
      face: faceBlock(p).readByteArray(FACE_LEN),
      equip: readEquip(p),
    };
    say('enter: face before ' + hex(snapshot.face));
    say('enter: equip before ' + snapshot.equip.map(hex).join(' '));
    this.warehouse = this.context.rcx.sub(0x3e20);
  },
  // Finish creation (0x1400de340) refuses with "Select class and gift" while selection +0x108 (class)
  // or +0x10c (gift) is 0 or +0x131 (a class/gift list is open) is set; nothing on open fills them.
  // Any valid row passes, and 0x1400de610, the only reader that grants anything, is skipped while
  // armed. Written directly rather than through the setter 0x1400e2010, which would also dress the
  // preview in that class's gear. User 2026-10-03: "The game still asked me to set my class and gift
  // before proceeding though."
  onLeave() {
    if (!armed || this.warehouse === undefined) return;
    const sel = this.warehouse.add(0x3b28);
    sel.add(0x108).writeU32(0x6e); // Deprived
    sel.add(0x10c).writeU32(500); // gift: Nothing
    sel.add(0x131).writeU8(0);
    say('class and gift pre-filled so Finish creation is available');
  },
});

// The leave handler 0x1400ec020 calls the face commit 0x14004cc90 FIRST and this one second, so this
// is where the run ends and disarms. Measured 2026-10-03: disarming at the end of the face commit let
// this run for real (no "skipped" line; every equipment slot and pgd+0x64 changed afterwards).
Interceptor.replace(at(0xde610), new NativeCallback((self) => {
  if (!armed) {
    classGiftCommit(self);
    return;
  }
  say('class and gift commit skipped');
  armed = false;
  setTimeout(report, 2000);
}, 'void', ['pointer']));

Interceptor.attach(at(0x33a5b0), {
  onEnter(args) {
    if (!armed || snapshot === null || !this.returnAddress.equals(FACE_COMMIT_CALL_RETURN)) return;
    const block = args[1];
    committed = Memory.alloc(FACE_LEN);
    Memory.copy(committed, block, FACE_LEN);
    say('face commit: block ' + hex(block.readByteArray(FACE_LEN)));
  },
});

Interceptor.attach(at(0x4cc90), {
  onLeave() {
    if (!armed || snapshot === null) return;
    const p = player();
    if (committed !== null) {
      const part = facePartOf(model(p));
      if (part.isNull()) {
        say('player has no face part: import skipped');
      } else {
        vcall(part, 0x188, 'void', ['pointer'], [committed]);
        say('imported into the player face part ' + part);
      }
    }
    const e = equip(p);
    const record = Memory.alloc(EQUIP_RECORD);
    snapshot.equip.forEach((bytes, slot) => {
      record.writeByteArray(bytes);
      equipWrite(e, slot, record);
    });
    playerRefresh(p);
    say('equipment put back');
  },
});

function report() {
  const p = player();
  say('after: face ' + hex(faceBlock(p).readByteArray(FACE_LEN)));
  const now = readEquip(p);
  const moved = now.filter((bytes, slot) => hex(bytes) !== hex(snapshot.equip[slot])).length;
  say('after: equip slots differing from before: ' + moved);
  const pgd = new Uint32Array(playerGameData().readByteArray(WATCH_BYTES));
  for (let i = 0; i < pgd.length; i++) {
    // +0x6c..+0xc4 churns with position and facing.
    if (pgd[i] !== pgdBefore[i] && (i * 4 < 0x6c || i * 4 > 0xc4)) {
      say('after: pgd+0x' + (i * 4).toString(16) + ' 0x' + pgdBefore[i].toString(16) + ' -> 0x' + pgd[i].toString(16));
    }
  }
  say('done');
}

function openArmed() {
  const events = gmi().add(0x70).readPointer();
  const wm = events.isNull() ? ptr(0) : events.add(0x50).readPointer();
  pgdBefore = new Uint32Array(playerGameData().readByteArray(WATCH_BYTES));
  snapshot = null;
  committed = null;
  armed = true;
  openCharaMakerWindow(wm);
  say('creator opened; edit the face and choose Finish creation');
}

if (config.trigger === 'bonfire') {
  // User 2026-10-03: "a menu option when I sit at a bonfire that says "Change Appearance" and have it
  // below the item box". The bonfire menu is built in 0x1400d6dc0 (FeTestBonfireWarehouse) by a
  // FexCommandSelectDialog builder: each row is 0x14002b240(builder, label, &jobCreator), where label
  // is the UTF-16 text itself (the FMG lookup's result, not an id) and jobCreator is a ref-counted
  // object whose vtbl+0x10 (this, &job) returns the job to run. Item box is the call at 0x1400d72f7;
  // one more row is added right after it returns. Choosing a row (0x14001c2bb) assigns the returned
  // job with a plain ref-pointer assign (0x140046de0), so a null job is accepted.
  const addRow = new NativeFunction(at(0x2b240), 'pointer', ['pointer', 'pointer', 'pointer']);
  const ITEM_BOX_ROW_RETURN = at(0xd72fc);
  const label = Memory.allocUtf16String('Change Appearance');
  // openCharaMakerWindow swaps the frontend operator and tears down the current scenes at once, so it
  // must not run inside the dialog's own row handler. Measured 2026-10-03: calling it here crashed
  // in the dialog's update (0x140106c3a, a vtable read from freed memory) right after the creator
  // opened. The row only asks; the per-frame nav update opens it once the bonfire has let go of the
  // frontend. Measured 2026-10-03: opening on the next frame after the row was chosen did nothing
  // (the player stood up, no creator, 0x14004c910 never entered); the frontend root's mode
  // +0x32c stayed 1 (bonfire) after the menu closed, so it is not the signal. The HUD operator's
  // suspended byte ([root+0xd8]+0x08, see ds2-rva FRONTEND_HUD_SUSPENDED_OFFSET) is: the bonfire
  // menu sets it and the frontend resume loop 0x1404ffd90 clears it once the menu stack lets go,
  // and the in-world opens that worked were all made with it clear.
  let lastSuspended = null;
  let openRequested = false;
  const invoke = new NativeCallback((self, out) => {
    out.writePointer(ptr(0));
    openRequested = true;
    say('bonfire row chosen');
    return out;
  }, 'pointer', ['pointer', 'pointer']);
  Interceptor.attach(at(0xbaeb20), {
    onEnter() {
      if (!openRequested) return;
      const hud = gmi().add(0x22e0).readPointer().add(0xd8).readPointer();
      const suspended = hud.isNull() ? -1 : hud.add(0x08).readU8();
      if (suspended !== lastSuspended) {
        say('hud suspended ' + suspended);
        lastSuspended = suspended;
      }
      if (suspended !== 0) return;
      openRequested = false;
      lastSuspended = null;
      openArmed();
    },
  });
  // The creator's bundle menu:/09.febnd.dcx is created by 0x1400e2f00 through 0x1400264d0 with the
  // frontend heap [[0x141616ca8]+0xcb0]->vtbl+0x38 as r9. Measured 2026-10-03: after a bonfire the
  // bundle's data arrived (0x1beeaa bytes) but its 0x765a0-byte .flo did not bind (0x140b00d20 copies
  // it from that heap, vtbl+0x50) and the bundle ended failed (state +0x34 = 3, written at
  // 0x140af6ec5), so the creator's update 0x1400e3060 never built its scene; the in-world opens with
  // no bonfire first bound fine. The resource lookup 0x140b02360 returns a same-named resource
  // without checking its state, so a failed bundle stays failed for the rest of the process. With r9
  // null, 0x1400264d0 uses the resource manager's own default heap (+0xe8), and the bind succeeded.
  // Patched rather than hooked: the agent calls openCharaMakerWindow from inside its own nav-update
  // listener, and Frida does not report hooked functions called from within a listener on the same
  // thread (measured 2026-10-03: hooks on 0x1400e2f00 and 0x1400264d0 stayed silent while the
  // bundle was created). `mov r9, rsi` at 0x1400e2fda -> `xor r9d, r9d`, 3 bytes each.
  const HEAP_ARG = at(0xe2fda);
  const HEAP_ARG_ORIGINAL = [0x4c, 0x8b, 0xce];
  const live = Array.from(new Uint8Array(HEAP_ARG.readByteArray(3)));
  if (live.every((b, i) => b === HEAP_ARG_ORIGINAL[i])) {
    Memory.patchCode(HEAP_ARG, 3, (code) => code.writeByteArray([0x45, 0x31, 0xc9]));
    say('creator bundle now created on the resource manager default heap (0x1400e2fda patched)');
  } else {
    say('0x1400e2fda is not mov r9, rsi (' + live.map((b) => b.toString(16)).join(' ') + '): heap patch skipped');
  }
  Interceptor.attach(at(0xb00d20), {
    onEnter(args) {
      let name = '';
      try { name = args[0].add(8).readPointer().readUtf16String(64) || ''; } catch (e) { /* unnamed */ }
      this.ours = name.indexOf('09.febnd') >= 0;
      if (this.ours) say('flo bind size 0x' + args[2].toInt32().toString(16) + ' heap ' + args[0].add(0xb0).readPointer());
    },
    onLeave(retval) {
      if (this.ours) say('flo bind ' + (retval.toInt32() & 0xff ? 'ok' : 'FAILED'));
    },
  });
  // Slots 0 and 1 are the game's own FeFunctorJobCreator ones (0x1410bad98); the reference count is
  // set high enough that neither destructor is ever reached.
  const vtbl = Memory.alloc(3 * 8);
  vtbl.writePointer(at(0x2d1d0));
  vtbl.add(8).writePointer(at(0xd6cd0));
  vtbl.add(16).writePointer(invoke);
  const creator = Memory.alloc(0x28);
  creator.writePointer(vtbl);
  creator.add(8).writeU32(0x40000000);
  const slot = Memory.alloc(8);
  // Memory.alloc frees its block when the JS value is collected, and the game holds these only as raw
  // pointers. Measured 2026-10-03: without this, choosing the row called through a freed vtbl and
  // crashed at rip 0x30 (call [r8+0x10] at 0x14001c613).
  globalThis.__bonfireRowKeepAlive = [label, invoke, vtbl, creator, slot];
  Interceptor.attach(at(0x2b240), {
    onEnter() {
      this.ours = this.returnAddress.equals(ITEM_BOX_ROW_RETURN);
    },
    onLeave(retval) {
      if (!this.ours) return;
      slot.writePointer(creator); // 0x14002b240 consumes the caller's reference and nulls the slot
      retval.replace(addRow(retval, label, slot));
      say('Change Appearance row added below Item box');
    },
  });
  say('armed: rest at a bonfire and choose Change Appearance');
} else {
  // Open from the per-frame nav update, on the game thread, once the character is in.
  let opened = false;
  const nav = Interceptor.attach(at(0xbaeb20), {
    onEnter() {
      if (opened) return;
      const m = gmi();
      if (m.isNull() || m.add(0x22e0).readPointer().isNull() || player() === null || playerGameData() === null) return;
      opened = true;
      openArmed();
      setTimeout(() => nav.detach(), 0);
    },
  });
  say('armed');
}
