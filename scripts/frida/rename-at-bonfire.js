// A "Rename Character" row below Item box in the bonfire menu, opening the game's own name entry.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/rename-at-bonfire.js`
// with the DLL's own row off (`scripts/ds2-run.py --no-change-appearance`): both hook 0x14002b240.
//
// openNameWindow 0x140198f70 is openCharaMakerWindow 0x1401986c0 with one difference: it stores
// FeOperatorTestCharaMaking+0x28 = 1, and the operator's update 0x1400e3060 then pushes name entry
// (0x1400e3620, FeGroupCreateNameEntry) instead of the full creator. The game calls it only from a
// talk-script case (0x1404631e7). It ignores its argument, as openCharaMakerWindow does.
//
// Everything else is appearance-only-creator.js's proven bonfire mode (docs/DS2-FACE-ONLY-CREATOR.md
// section 6): the row after Item box's (return 0x1400d72fc), a job creator whose invoke returns no
// job, the open deferred to the nav update 0x140baeb20 until the HUD's suspended byte is 0, and the
// creator bundle's heap argument at 0x1400e2fda patched (same operator, same bundle).
//
// Logs the player's name (PlayerGameData+0x24, UTF-16) before the open and whenever it changes.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const say = (text) => console.log('[rename] ' + text);

const NAME_OFFSET = 0x24;

function gmi() {
  return at(0x16148f0).readPointer();
}

function playerName() {
  const m = gmi();
  if (m.isNull()) return null;
  const gdm = m.add(0xa8).readPointer();
  if (gdm.isNull()) return null;
  const pgd = gdm.add(0xc0).readPointer();
  return pgd.isNull() ? null : pgd.add(NAME_OFFSET).readUtf16String(16);
}

const openNameWindow = new NativeFunction(at(0x198f70), 'void', ['pointer']);
const addRow = new NativeFunction(at(0x2b240), 'pointer', ['pointer', 'pointer', 'pointer']);
const ITEM_BOX_ROW_RETURN = at(0xd72fc);
const label = Memory.allocUtf16String('Rename Character');

let openRequested = false;
let lastName = null;

const invoke = new NativeCallback((self, out) => {
  out.writePointer(ptr(0));
  openRequested = true;
  say('row chosen');
  return out;
}, 'pointer', ['pointer', 'pointer']);

Interceptor.attach(at(0xbaeb20), {
  onEnter() {
    const name = playerName();
    if (name !== null && name !== lastName) {
      say('name ' + JSON.stringify(lastName) + ' -> ' + JSON.stringify(name));
      lastName = name;
    }
    if (!openRequested) return;
    const hud = gmi().add(0x22e0).readPointer().add(0xd8).readPointer();
    if (hud.isNull() || hud.add(0x08).readU8() !== 0) return;
    openRequested = false;
    openNameWindow(ptr(0));
    say('name entry opened');
  },
});

const HEAP_ARG = at(0xe2fda);
const live = Array.from(new Uint8Array(HEAP_ARG.readByteArray(3)));
if ([0x4c, 0x8b, 0xce].every((b, i) => b === live[i])) {
  Memory.patchCode(HEAP_ARG, 3, (code) => code.writeByteArray([0x45, 0x31, 0xc9]));
  say('bundle heap patched');
} else {
  say('0x1400e2fda reads ' + live.map((b) => b.toString(16)).join(' ') + ': heap patch skipped');
}

// The name entry group's own enter, to see that it is the one pushed.
Interceptor.attach(at(0xe3620), {
  onEnter() {
    say('operator pushed name entry');
  },
});

const vtbl = Memory.alloc(3 * 8);
vtbl.writePointer(at(0x2d1d0));
vtbl.add(8).writePointer(at(0xd6cd0));
vtbl.add(16).writePointer(invoke);
const creator = Memory.alloc(0x28);
creator.writePointer(vtbl);
creator.add(8).writeU32(0x40000000);
const slot = Memory.alloc(8);
globalThis.__renameRowKeepAlive = [label, invoke, vtbl, creator, slot];

Interceptor.attach(at(0x2b240), {
  onEnter() {
    this.ours = this.returnAddress.equals(ITEM_BOX_ROW_RETURN);
  },
  onLeave(retval) {
    if (!this.ours) return;
    slot.writePointer(creator);
    retval.replace(addRow(retval, label, slot));
    say('row added below Item box');
  },
});

say('loaded');
