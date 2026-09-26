// Which path does a real click take into DARK SOULS II while ds2-input-harness blocks input?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/block-leak-trace.js`
//
// Two paths carry a mouse button out of the window procedure (FUN_140af0380):
//   - FUN_140af0d90 builds an event that FUN_140b08ef0 folds into each input block's +0x228. The
//     harness rewrites presses there during a block.
//   - FUN_140af4550(inputObj, hwnd, msg, wparam) runs for EVERY message and writes a virtual-key
//     table: WM_LBUTTONDOWN sets inputObj+0x219, WM_KEYDOWN sets inputObj+0x218+vk. The harness
//     does not touch it, and FUN_140af4120 reads it.
// This logs every button/key message reaching FUN_140af4550, every non-move event reaching the
// fold (the type it arrives with and the type it leaves with, after the harness), the window-active
// flag BaseP1+0x30 whenever it changes, and whether the DirectInput mouse's X was being zeroed
// (a block in force) at that moment. It writes nothing.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;
const start = Date.now();

const KATANA_MAIN_APP_SLOT = 0x16751f8;
const NAMES = {
  0x100: 'KEYDOWN', 0x101: 'KEYUP', 0x201: 'LDOWN', 0x202: 'LUP', 0x203: 'LDBL',
  0x204: 'RDOWN', 0x205: 'RUP', 0x206: 'RDBL', 0x207: 'MDOWN', 0x208: 'MUP', 0x209: 'MDBL',
  0x20a: 'WHEEL', 0x6: 'ACTIVATE', 0x21: 'MOUSEACTIVATE',
};

function t() {
  return ((Date.now() - start) / 1000).toFixed(2) + 's';
}

function app() {
  try {
    return base.add(KATANA_MAIN_APP_SLOT).readPointer();
  } catch (e) {
    return NULL;
  }
}

let lastActive = -1;

function note(line) {
  console.log('[leak] ' + t() + ' ' + line);
}

Interceptor.attach(base.add(0xaf4550), {
  onEnter(args) {
    const msg = args[2].toUInt32();
    if (msg in NAMES) {
      const a = app();
      const active = a.isNull() ? -1 : a.add(0x30).readU8();
      note('wndmsg ' + NAMES[msg] + ' wparam=0x' + args[3].toString(16) + ' active=' + active);
    }
  },
});

Interceptor.attach(base.add(0xb08ef0), {
  onEnter(args) {
    this.event = args[1];
    this.before = args[1].readU32();
  },
  onLeave() {
    if (this.before === 9) return;
    note('fold type ' + this.before);
  },
});

// The DirectInput mouse poll: after the harness, a block leaves X, Y and the raw state zero.
// Read here only to tell "block in force" from "not", by the raw buttons the harness zeroes.
Interceptor.attach(base.add(0xf074a0), {
  onEnter(args) {
    this.device = args[0];
  },
  onLeave() {
    const a = app();
    if (!a.isNull()) {
      const active = a.add(0x30).readU8();
      if (active !== lastActive) {
        note('window active ' + lastActive + ' -> ' + active);
        lastActive = active;
      }
    }
  },
});

note('attached');
