// Which tab panels the pause menu opens and closes on a tab switch, and which sprite each play
// reaches. Read-only.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/menu-tab-expand.js`
//
// A tab's list is rolled out or not by sequence plays on the tab's panel, the component the group
// resolves from its layout path at `group + 0x130` (proxy `group + 0x160`):
//
//   0x1400a4d20  per-tab init          plays 0x65 (snap, r8b=1) -- not hooked, see below
//   0x1400a6ca0  group vtable +0xc8    plays 0x66
//   0x1400a7050  group vtable +0xe8    plays 0x68
//   0x1400a6bb0  top select            pops every active group, pushes the tab under the cursor
//
// Each group play is logged with the group's pointer and its path ids, and every
// `FeComponentSprite` play (0x140b6c4f0) made inside it is logged with whether the sprite's own
// sequence table had the id -- a miss is a silent no-op in the game. `FEX_GRID_CURRENT_INDEX` on
// the top select is read at each tab switch, so each line says which tab the cursor was on.

'use strict';

const base = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => base.add(rva);
const currentIndex = new NativeFunction(at(0x00022140), 'int', ['pointer']);

const GROUP_PATH = 0x130;
const SPRITE_RESOURCE = 0x48;
const SPRITE_TABLE = 0x18;
const SPRITE_POSITION = 0x40;

let seq = 0;
const emit = (event) => {
  event.n = ++seq;
  send(event);
  console.log('[tab-expand] ' + JSON.stringify(event));
};

function pathOf(group) {
  try {
    const words = [];
    for (let i = 0; i < 12; i++) words.push(group.add(GROUP_PATH + i * 4).readU32());
    return words.map((w) => '0x' + w.toString(16));
  } catch (e) {
    return ['unreadable'];
  }
}

// Per-thread: the group play currently on the stack, so sprite plays inside it are attributed.
const inside = {};

// The top select the last tab switch was made on, read by the two timers below.
let lastTop = NULL;

function groupHook(rva, name) {
  Interceptor.attach(at(rva), {
    onEnter(args) {
      const tid = Process.getCurrentThreadId();
      this.tid = tid;
      this.group = args[0];
      inside[tid] = { name, group: args[0].toString(), sprites: [] };
    },
    onLeave() {
      const frame = inside[this.tid];
      delete inside[this.tid];
      emit({
        what: name,
        group: this.group.toString(),
        path: pathOf(this.group),
        sprites: frame ? frame.sprites : [],
      });
      if (frame && frame.sprites.length > 0) {
        dumpLater(frame.sprites[0].sprite, name + ' ' + pathOf(this.group)[1]);
      }
    },
  });
}

// Not 0x1400a4d20: ds2-menu-row already detours it, and an Interceptor on top of a MinHook jump is
// a second trampoline over the first.
groupHook(0x000a6ca0, 'enter-0x66');
groupHook(0x000a7050, 'leave-0x68');

Interceptor.attach(at(0x000a6bb0), {
  onEnter(args) {
    this.top = args[0];
    lastTop = args[0];
    let index = -99;
    try { index = currentIndex(args[0]); } catch (e) { /* reported as -99 */ }
    emit({ what: 'switch-begin', top: args[0].toString(), index });
  },
  onLeave() {
    emit({ what: 'switch-end', top: this.top.toString() });
  },
});

Interceptor.attach(at(0x00b6c4f0), {
  onEnter(args) {
    const frame = inside[Process.getCurrentThreadId()];
    if (!frame) return;
    const sprite = args[0];
    const id = args[1].toInt32();
    let hit = 'no-table';
    let count = 0;
    try {
      const table = sprite.add(SPRITE_RESOURCE).readPointer().add(SPRITE_TABLE).readPointer();
      if (!table.isNull()) {
        const entries = table.readPointer();
        count = table.add(8).readU16();
        hit = 'miss';
        for (let i = 0; i < count; i++) {
          if (entries.add(i * 0x10).readS32() === id) {
            hit = 'start=' + entries.add(i * 0x10 + 4).readU16();
            break;
          }
        }
      }
    } catch (e) {
      hit = 'unreadable';
    }
    this.frame = frame;
    this.sprite = sprite;
    this.record = { sprite: sprite.toString(), id: '0x' + id.toString(16), hit, count };
  },
  onLeave() {
    if (!this.frame) return;
    try { this.record.pos = this.sprite.add(SPRITE_POSITION).readFloat(); } catch (e) { /* none */ }
    if (this.frame.sprites.length < 24) this.frame.sprites.push(this.record);
    panels[this.record.sprite] = { group: this.frame.group, last: null };
  },
});

// A panel's subtree at rest: each component's class, element id, frame range of its record, its
// hidden byte (+0x44, what `0x140b6a960` writes), and its own playhead if it is a sprite. Walked
// the way `ds2-menu-row`'s tree.rs walks it: linked children for Object/Scene, the display list
// for a Sprite.
const VT_OBJECT = base.add(0x011ddfa8);
const VT_SCENE = base.add(0x011de158);
const VT_SPRITE = base.add(0x011de318);
function walk(component, depth, out) {
  if (out.length > 300 || depth > 12 || component.isNull()) return;
  let vt;
  try { vt = component.readPointer(); } catch (e) { return; }
  const node = { d: depth, at: component.toString() };
  node.cls = vt.equals(VT_SPRITE) ? 'S' : vt.equals(VT_OBJECT) ? 'O' : vt.equals(VT_SCENE) ? 'C' : vt.sub(base).toString();
  try {
    const rec = component.add(0x48).readPointer();
    node.id = '0x' + rec.add(0x1c).readU32().toString(16);
    node.range = [rec.add(0x14).readU16(), rec.add(0x16).readU16()];
    node.def = '0x' + rec.readU16().toString(16);
  } catch (e) { /* leaf without record */ }
  try { node.hid = component.add(0x44).readU8(); } catch (e) { /* none */ }
  try { node.f30 = component.add(0x30).readU32().toString(16); } catch (e) { /* none */ }
  if (node.cls === 'S') {
    try { node.pos = component.add(SPRITE_POSITION).readFloat(); } catch (e) { /* none */ }
  }
  out.push(node);
  if (node.cls === 'S') {
    const list = component.add(0x70).readPointer();
    const count = component.add(0x66).readU16();
    for (let i = 0; i < count && i < 40; i++) walk(list.add(i * 0x10).readPointer(), depth + 1, out);
  } else if (node.cls === 'O' || node.cls === 'C') {
    let child = component.add(0x38).readPointer();
    let n = 0;
    while (!child.isNull() && n++ < 40) {
      walk(child, depth + 1, out);
      child = child.add(0x28).readPointer();
    }
  }
}
// Which panel owns each component, from the walks, so a draw can be attributed to a panel.
const owner = {};
function dumpLater(sprite, label) {
  setTimeout(() => {
    const out = [];
    try { walk(ptr(sprite), 0, out); } catch (e) { out.push({ error: String(e) }); }
    const panel = label.split(' ')[1];
    for (const node of out) if (node.at) owner[node.at] = { panel, id: node.id, cls: node.cls };
    emit({ what: 'tree', label, sprite, count: out.length });
  }, 1500);
}

// THE DRAW CENSUS. A texture shape's draw (0x140b6f200, `FeComponentTextureShape` slot 46) and a
// text field's (0x140b6d470, `FeComponentTextField` slot 46), counted per owning panel and element
// over one-second windows. What a panel draws while the cursor is on another tab is the whole
// question the playheads could not answer.
const draws = {};
// The text field's slot 46 (0x140b6d470) is `ret 0`; its slot 47 (0x140b6d480) is the one with a
// body, and is counted as its draw.
for (const [rva, kind] of [[0x00b6f200, 'shape'], [0x00b6d480, 'text']]) {
  Interceptor.attach(at(rva), {
    onEnter(args) {
      const o = owner[args[0].toString()];
      if (!o) return;
      const key = o.panel + ' ' + kind + ' ' + args[0];
      draws[key] = (draws[key] || 0) + 1;
    },
  });
}
setInterval(() => {
  const keys = Object.keys(draws);
  if (keys.length === 0) return;
  let index = -99;
  try { index = currentIndex(lastTop); } catch (e) { /* none */ }
  const summary = {};
  for (const k of keys) {
    const panel = k.split(' ')[0] + ' ' + k.split(' ')[1];
    summary[panel] = (summary[panel] || 0) + 1;
    delete draws[k];
  }
  emit({ what: 'drawn', tab: index, components_drawn: summary });
}, 1000);

// Every panel sprite a group play has reached, and where its playhead is. A play only SETS the
// playhead to the sequence's start frame; something else has to advance it to the frame where the
// list is folded away. Reported on change, twice a second.
const panels = {};
setInterval(() => {
  for (const key of Object.keys(panels)) {
    let pos = null;
    let stopped = null;
    try {
      const sprite = ptr(key);
      pos = sprite.add(SPRITE_POSITION).readFloat();
      stopped = sprite.add(0x44).readU8();
    } catch (e) { /* reported as null */ }
    let top60 = null;
    try { top60 = lastTop.add(0x60).readPointer().toString(); } catch (e) { /* none */ }
    const now = pos + '/' + stopped + '/' + top60;
    if (panels[key].last === now) continue;
    panels[key].last = now;
    emit({ what: 'playhead', sprite: key, group: panels[key].group, pos, hidden44: stopped, top60 });
  }
}, 500);

emit({ what: 'attached', base: base.toString() });
