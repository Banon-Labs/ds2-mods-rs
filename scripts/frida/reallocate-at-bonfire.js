// A "Reallocate Stats" row below Item box in the bonfire menu: what choosing "Reallocate points"
// from the Things Betwixt firekeepers does once flag 102181 is set.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/reallocate-at-bonfire.js`
// with the DLL's own rows off (`--no-change-appearance --no-rename-character`): both hook 0x14002b240.
//
// The firekeepers' talk script (talk_m10_02_00_00.esd, group 2147483619, read with
// scripts/ds2-esd.py): a yes/no with the Soul Vessel; on yes, f131401(50960000, 1, 1, 0) -- the
// vessel count -- refuses with message 1206 when there is none; otherwise c1_130455(0, 220, 0),
// whose handler (0x14046332e) is `openAttributeMenu([[GMI+0x70]+0x50], mode 0, buf)` after
// `fillWindowDataBuffer(buf, talker, 220, 0.0, true, 0.0)`. Mode 0 pushes menu 0x1a, the Reallocate
// screen (FeGroupTestBonfireLevelUp with the Soul Vessel cost); its own confirm asks "Reallocation
// with these attributes consumes %s. Okay?" and its commit (0x1400ca0ac) removes one vessel.
//
// The window buffer (fillWindowDataBuffer 0x14019b750, 32 bytes, 16-aligned): +0x00 the talker's
// position (the player's when there is no talker), +0x10 the squared distance at which the window
// closes, EventManager value 7 squared when the distance argument is <= 0, +0x14 -1.0 (the `true`
// argument), +0x18 not written. Here the position is the player's: there is no talker.
//
// Logs the vessel count and soul level when the row is chosen and whenever either changes.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const say = (text) => console.log('[reallocate] ' + text);

const SOUL_VESSEL = 50960000;

const gmi = () => at(0x16148f0).readPointer();
const itemCount = new NativeFunction(at(0x40440), 'uint32', ['uint32']);
const eventValue = new NativeFunction(at(0x44ea70), 'float', ['pointer', 'uint32']);
const openAttributeMenu = new NativeFunction(at(0x1992c0), 'void', ['pointer', 'int', 'pointer']);
const addRow = new NativeFunction(at(0x2b240), 'pointer', ['pointer', 'pointer', 'pointer']);
const ITEM_BOX_ROW_RETURN = at(0xd72fc);
const label = Memory.allocUtf16String('Reallocate Stats');

function soulLevel() {
  const player = gmi().add(0xd0).readPointer();
  if (player.isNull()) return null;
  const param = player.add(0x490).readPointer();
  return param.isNull() ? null : param.add(0xd0).readU32();
}

let openRequested = false;
let last = null;

function state() {
  return 'vessels=' + itemCount(SOUL_VESSEL) + ' level=' + soulLevel();
}

const invoke = new NativeCallback((self, out) => {
  out.writePointer(ptr(0));
  openRequested = true;
  say('row chosen, ' + state());
  return out;
}, 'pointer', ['pointer', 'pointer']);

// 32 bytes, 16-aligned: the callee reads it with movaps.
const window = Memory.alloc(0x40);
const buf = window.add((16 - (window.toUInt32() & 15)) & 15);

Interceptor.attach(at(0xbaeb20), {
  onEnter() {
    const now = state();
    if (now !== last) {
      if (last !== null) say(last + ' -> ' + now);
      last = now;
    }
    if (!openRequested) return;
    const hud = gmi().add(0x22e0).readPointer().add(0xd8).readPointer();
    if (hud.isNull() || hud.add(0x08).readU8() !== 0) return;
    openRequested = false;
    if (itemCount(SOUL_VESSEL) === 0) {
      say('no Soul Vessel: not opened (the firekeepers refuse here with message 1206)');
      return;
    }
    const player = gmi().add(0xd0).readPointer();
    const events = gmi().add(0x70).readPointer();
    const reach = eventValue(events, 7);
    for (let i = 0; i < 4; i++) buf.add(i * 4).writeFloat(player.add(0x90 + i * 4).readFloat());
    buf.add(0x10).writeFloat(reach * reach);
    buf.add(0x14).writeFloat(-1.0);
    buf.add(0x18).writeU64(0);
    openAttributeMenu(events.add(0x50).readPointer(), 0, buf);
    say('reallocate opened, close distance ' + reach.toFixed(2));
  },
});

// The menu id the screen factory is asked for: 0x1a is Reallocate, 0x19 Level Up.
Interceptor.attach(at(0xc63c0), {
  onEnter(args) {
    const kind = args[2].toUInt32();
    if (kind === 0x1a || kind === 0x19) say('screen 0x' + kind.toString(16) + ' built');
  },
});

// The commit's vessel spend.
Interceptor.attach(at(0xca0ac), {
  onEnter() {
    say('commit spends a Soul Vessel');
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
globalThis.__reallocateRowKeepAlive = [label, invoke, vtbl, creator, slot, window];

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

say('loaded, ' + state());
