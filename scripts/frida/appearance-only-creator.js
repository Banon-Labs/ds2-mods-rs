// Change only the player's appearance -- face, sex and body -- through the game's own character
// creator. Class, stats, souls, level and inventory stay as they are.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/appearance-only-creator.js`
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

// Open from the per-frame nav update, on the game thread, once the character is in.
let opened = false;
const nav = Interceptor.attach(at(0xbaeb20), {
  onEnter() {
    if (opened) return;
    const m = gmi();
    if (m.isNull() || m.add(0x22e0).readPointer().isNull() || player() === null || playerGameData() === null) return;
    opened = true;
    const events = m.add(0x70).readPointer();
    const wm = events.isNull() ? ptr(0) : events.add(0x50).readPointer();
    pgdBefore = new Uint32Array(playerGameData().readByteArray(WATCH_BYTES));
    armed = true;
    openCharaMakerWindow(wm);
    say('creator opened; edit the face and choose Finish creation');
    setTimeout(() => nav.detach(), 0);
  },
});

say('armed');
