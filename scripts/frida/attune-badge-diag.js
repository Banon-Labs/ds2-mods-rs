// Why ds2-item-warn's X does not draw in the bonfire Attune Spell picker. Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/attune-badge-diag.js \
//   [--config-json '{"open_bonfire":true}']`
//
// Three things, logged as they happen:
//   build   every call of the container builder FUN_140b50f20 (0x00b50f20) whose definition is an
//           infusion container (9 or 10 children, first record id 0x5f5c3e9), with its caller;
//   list    SpellBookItemList::getItem (0x000ceed0) firing -- proof the picker is open;
//   cell    after the inventory cell bind FUN_1400bc850 (0x000bc850) for a picker row: the ids of the
//           children the cell's infusion container component actually holds (0x5f5c3ef is ours), and
//           the item's greyed byte.
// The container component is reached the way FE_ELEMENT_SET_VISIBLE reaches it: accessor+0x08 is a
// holder whose first virtual returns the component; children are first=[c+0x38], next=[c+0x28], and
// a child's id is [[child+0x48]+0x1c].
'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const say = (t) => {
  console.log('[attune-diag] ' + t);
  send({ attune_diag: t });
};

function p(x) {
  try {
    const v = x.readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function componentOf(accessor) {
  const holder = p(accessor.add(8));
  if (!holder) return null;
  const vt = p(holder);
  if (!vt) return null;
  const get = p(vt);
  if (!get) return null;
  try {
    const c = new NativeFunction(get, 'pointer', ['pointer'])(holder);
    return c.isNull() ? null : c;
  } catch (e) {
    return null;
  }
}

function kids(comp) {
  const out = [];
  let c = p(comp.add(0x38));
  for (let i = 0; c && i < 24; i++) {
    const rec = p(c.add(0x48));
    let id = 'null';
    try {
      if (rec) id = '0x' + rec.add(0x1c).readU32().toString(16);
    } catch (e) {
      id = '?';
    }
    out.push(id);
    c = p(c.add(0x28));
  }
  return out;
}

let builds = 0;
let allBuilds = 0;
let allKeys = [];
setInterval(() => {
  if (allKeys.length === 0) return;
  say('all builds (def:children:first-id) ' + allKeys.join(' '));
  allKeys = [];
}, 3000);
Interceptor.attach(at(0x00b50f20), {
  onEnter(args) {
    const def = args[2];
    try {
      const count = def.add(2).readU16();
      if (allBuilds++ < 400) {
        let first = 0;
        try {
          first = def.add(8).readPointer().add(0x1c).readU32();
        } catch (e) { /* */ }
        allKeys.push('0x' + def.readU16().toString(16) + ':' + count + ':' + first.toString(16));
      }
      if (count !== 9 && count !== 10) return;
      const first = def.add(8).readPointer().add(0x1c).readU32();
      if (first !== 0x5f5c3e9) return;
      if (builds++ >= 80) return;
      say('build def=' + def + ' children=' + count + ' doc=' + args[1] + ' ret=+0x' +
          this.returnAddress.sub(exe).toString(16));
    } catch (e) { /* not a definition */ }
  },
});

let listed = 0;
Interceptor.attach(at(0x000ceed0), {
  onLeave() {
    if (listed++ < 3) say('list getItem fired (picker open) n=' + listed);
  },
});

let cells = 0;
Interceptor.attach(at(0x000bc850), {
  onEnter(args) {
    this.cell = args[0];
    this.item = args[1];
  },
  onLeave() {
    if (listed === 0 || cells >= 24) return;
    cells++;
    let greyed = '?';
    let handle = '?';
    try {
      greyed = this.item.add(5).readU8();
      handle = '0x' + this.item.add(2).readU16().toString(16);
    } catch (e) { /* */ }
    const comp = componentOf(this.cell.add(0x2d0));
    // Which layout the row comes from: for each of the view's accessors, the record its component
    // was built from -- the definition index at rec+0, the id at rec+0x1c, and xy off rec+8.
    const described = [0x90, 0x120, 0x1b0, 0x240, 0x2d0, 0x360, 0x3f0, 0x480].map((off) => {
      const c = componentOf(this.cell.add(off));
      if (!c) return '+0x' + off.toString(16) + '=none';
      const rec = p(c.add(0x48));
      if (!rec) return '+0x' + off.toString(16) + '=norec';
      try {
        const xf = p(rec.add(8));
        return '+0x' + off.toString(16) + '=def0x' + rec.readU16().toString(16) + '/id0x' +
          rec.add(0x1c).readU32().toString(16) + (xf ? '@(' + xf.readFloat().toFixed(2) + ',' +
          xf.add(4).readFloat().toFixed(2) + ')' : '') + ' kids=[' + kids(c).join(',') + ']';
      } catch (e) {
        return '+0x' + off.toString(16) + '=?';
      }
    });
    say('cell view=' + this.cell + ' handle=' + handle + ' greyed=' + greyed + ' container=' + comp +
        (comp ? ' children=[' + kids(comp).join(',') + ']' : '') + ' | ' + described.join(' '));
  },
});

setInterval(() => say('tick builds=' + builds + ' getItem=' + listed + ' cells=' + cells), 15000);

if (config.open_bonfire === true) {
  const bonfireWindow = new NativeFunction(at(0x00198680), 'void', ['pointer', 'uint16']);
  let done = false;
  const hook = Interceptor.attach(at(0x00baeb20), {
    onEnter() {
      if (done) return;
      done = true;
      const gmi = at(0x016148f0).readPointer();
      const events = gmi.add(0x70).readPointer();
      const wm = events.isNull() ? ptr(0) : events.add(0x50).readPointer();
      if (wm.isNull() || gmi.add(0x22e0).readPointer().isNull()) {
        say('bonfire not opened: event window manager or frontend root is null');
      } else {
        bonfireWindow(wm, 0);
        say('bonfireWindow(' + wm + ', 0) called');
      }
      setTimeout(() => hook.detach(), 0);
    },
  });
}
say('armed');
