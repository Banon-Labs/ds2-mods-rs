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
// Byte +0xa0 of the builder's last row is "shown" (0x14002c680: 0 hides the row, measured);
// byte +0xa1 is "enabled" (0x14002baf0), read only by the row's draw (0x14001d405), which picks
// text style 0x70 or 0x7a from it. Both start at 1 (FexCommandSelectDialog::Command, 0x14002a8e0).
const setRowEnabled = new NativeFunction(at(0x2baf0), 'pointer', ['pointer', 'uint8']);

// The styled add: `builder*(builder, Descriptor*, JobCreator** slot)`, 0x14002b3e0. Descriptor:
// +0x00 a DL wstring (allocator at +0x20, a flag byte at +0x28 its constructor sets), +0x30 the pair
// (u32 frame, u32 text field). The row's draw (0x140027e70) sends the label's style child 0x5f5c5ad
// to `frame` and writes the text into child `field` -- the colour is the frame's art. A row added by
// pointer (0x14002b240) is drawn by 0x140027aa0 instead, which has no style.
const addStyledRow = new NativeFunction(at(0x2b3e0), 'pointer', ['pointer', 'pointer', 'pointer']);
// `wstring*(wstring* out, const wchar_t*)`: constructs `out` on the frontend allocator.
const wstringFrom = new NativeFunction(at(0x3d830), 'pointer', ['pointer', 'pointer']);
// Pairs the game uses: 0x67/0x5f5c5b2 every plain row; 0x98/0x5f5c5b7 a cost it cannot pay (red,
// 0x1400ba570); 0x99/0x5f5c5bc beside it in the stat colours (0x1400bec80).
const STYLES = [[0x98, 0x5f5c5b7], [0x99, 0x5f5c5bc], [0x67, 0x5f5c5b2]];
let styleTurn = 0;
const STYLED = false;
// Test switch: build the row disabled whatever the character, to see the grey on any save.
const FORCE_DISABLED = false;
const descriptor = Memory.alloc(0x40);

// PlayerStatusParam base spreads, game order, by class id (crates/ds2-build-import-core class.rs).
const BASE = {
  1: [7, 6, 6, 5, 15, 11, 5, 5, 5],
  2: [12, 6, 7, 4, 11, 8, 3, 6, 9],
  4: [9, 7, 11, 2, 9, 14, 1, 8, 3],
  6: [10, 3, 8, 10, 11, 5, 4, 12, 4],
  7: [5, 6, 5, 12, 3, 7, 14, 4, 8],
  8: [7, 6, 9, 7, 6, 6, 5, 5, 12],
  9: [4, 8, 4, 6, 9, 16, 7, 5, 6],
  10: [6, 6, 6, 6, 6, 6, 6, 6, 6],
};

// Why the row is off, or null when it is on: a level above the class base and a Soul Vessel.
function whyNot() {
  const data = gmi().add(0xa8).readPointer().add(0xc0).readPointer();
  const cls = data.add(0x64).readU32();
  const param = gmi().add(0xd0).readPointer().add(0x490).readPointer();
  const stats = [0, 1, 2, 3, 4, 5, 6, 7, 8].map((i) => param.add(0x08 + i * 2).readU16());
  const base = BASE[cls];
  if (!base) return 'unknown class ' + cls;
  if (stats.some((s, i) => s < base[i])) return 'below class base ' + stats;
  if (stats.every((s, i) => s === base[i])) return 'no level above base';
  if (itemCount(SOUL_VESSEL) === 0) return 'no Soul Vessel';
  return null;
}
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
  const why = whyNot();
  if (why !== null) {
    say('row chosen while greyed (' + why + '): nothing opened');
    return out;
  }
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

// The styled label draw: (textNode, proxy, descriptor) -> found. Logged for any non-plain pair, to
// see whether ours reaches it and whether the style child 0x5f5c5ad was found (returns 1).
Interceptor.attach(at(0x27e70), {
  onEnter(args) {
    this.frame = args[2].add(0x30).readU32();
    this.field = args[2].add(0x34).readU32();
  },
  onLeave(retval) {
    if (this.frame === 0x67) return;
    say('styled draw frame 0x' + this.frame.toString(16) + ' field 0x' + this.field.toString(16) +
      ' -> ' + retval.toInt32());
  },
});

// The class of the scene node our label's text lands in: proxy vfunc 0 resolves it, its vtable
// slot 0x148 takes the text. Logged once, with the RTTI name, to find a colour setter beside it.
function rttiName(vtable) {
  const col = vtable.sub(8).readPointer();
  const typeDesc = exe.add(col.add(12).readU32());
  return typeDesc.add(16).readCString();
}
let nodeSaid = false;
// FeColorSetParam: row `id` via 0x1404ff7b0(frontendRoot = [GMI+0x22e0], id); four values at
// +0/+4/+8/+0xc. 0x140041410(node, id) hands them to node vtable +0x100 (FeComponentObject
// 0x140b78390 forwards to every child component). Callers pass 1 for plain and 2 for the other
// state (0x140509658, 0x14007cca1).
const colourSetRow = new NativeFunction(at(0x4ff7b0), 'pointer', ['pointer', 'uint32']);
const applyColourSet = new NativeFunction(at(0x41410), 'void', ['pointer', 'uint32']);
// Measured 2026-10-03: set 1 = 255 255 255 255, set 2 = 128 128 128 255, sets 0 and 3..8 absent.
// The user, with set 2 applied to the disabled row: "It is greyed out".
const GREY_SET = 2;
let disabledLabel = false;
function sayColourSets() {
  const root = gmi().add(0x22e0).readPointer();
  for (let id = 0; id <= 8; id++) {
    const row = colourSetRow(root, id);
    say('colour set ' + id + ': ' + (row.isNull() ? 'none'
      : [0, 4, 8, 12].map((o) => row.add(o).readU32()).join(' ')));
  }
}
// A row added by pointer has its text set by 0x140027aa0 through 0x1400299c0(proxy, wchar_t*).
// Read at 0x1400299d1, just after the proxy resolved the node: rax is the node, rbx the text.
// Calls nothing: the resolver takes `*arg0`, not arg0, and calling it wrongly faulted once.
Interceptor.attach(at(0x299d1), function () {
  const ctx = this.context;
  if (!ctx.rbx.equals(label)) return;
  const node = ctx.rax;
  if (!node.isNull()) applyColourSet(node, disabledLabel ? GREY_SET : 1);
  if (nodeSaid) return;
  nodeSaid = true;
  sayColourSets();
  if (node.isNull()) {
    say('label node: null');
    return;
  }
  {
    const vt = node.readPointer();
    say('label node ' + node + ' vtable 0x' + vt.sub(exe).toString(16));
    try {
      say('label node class ' + rttiName(vt));
    } catch (e) {
      say('label node class unreadable: ' + e.message);
    }
    // The node's slots, as RVAs, to find a colour setter beside setText (+0x140).
    const slots = [];
    for (let i = 0; i < 0x60; i++) slots.push(vt.add(i * 8).readPointer().sub(exe).toString(16));
    say('label node slots ' + slots.join(' '));
  }
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
    const why = FORCE_DISABLED ? 'forced for the colour test' : whyNot();
    let builder;
    let style = 'plain';
    // The styled add stays off: after five opens with frames 0x98/0x99 on this dialog's style
    // child, the frontend's scene walk faulted (DarkSoulsII.exe+0xb67075, 2026-10-03), and the
    // frames made no visible difference before that.
    if (why === null || !STYLED) {
      builder = addRow(retval, label, slot);
    } else {
      const [frame, field] = STYLES[styleTurn % STYLES.length];
      styleTurn++;
      // Constructs the whole string, its own +0x28 byte included; the add frees it after copying.
      wstringFrom(descriptor, label);
      descriptor.add(0x30).writeU32(frame);
      descriptor.add(0x34).writeU32(field);
      builder = addStyledRow(retval, descriptor, slot);
      style = 'style 0x' + frame.toString(16) + '/0x' + field.toString(16);
    }
    setRowEnabled(builder, why === null ? 1 : 0);
    disabledLabel = why !== null;
    retval.replace(builder);
    say('row added below Item box, ' + (why === null ? 'enabled' : 'disabled: ' + why) + ', ' + style);
  },
});

say('loaded, ' + state());
