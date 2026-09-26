// Report what a harness drive needs before every press: whether the game's window is the
// foreground window, which pause-menu action was dispatched, and which menu grids are live.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/menu-state.js`
//
// Lines, each printed only when it changes:
//   [menu-state] focus=<true|false> fg=<hwnd>        GetForegroundWindow belongs to this process
//   [menu-state] dispatch action=<n>                  FE_INGAME_MENU_DISPATCH (0x1400a6090)
//   [menu-state] live grids=<ptr:index,...>           grids FEX_GRID_CURRENT_INDEX answered in
//                                                     the last 500 ms, with their cursor
// Actions (docs/DS2-INGAME-MENU.md): 0 Equipment, 1 Inventory, 2/3 Status, 4-6 messages,
// 7/8 settings, 9 return to title. The tab strip is the grid whose index follows RB/LB; no live
// grids means no menu is open. Read-only.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const user32 = Process.getModuleByName('user32.dll');
const GetForegroundWindow = new NativeFunction(user32.getExportByName('GetForegroundWindow'),
  'pointer', []);
const GetWindowThreadProcessId = new NativeFunction(
  user32.getExportByName('GetWindowThreadProcessId'), 'uint', ['pointer', 'pointer']);
const pidOut = Memory.alloc(4);

let seen = {};
Interceptor.attach(exe.add(0x00022140), {
  onEnter(args) { this.grid = args[0].toString(); },
  onLeave(ret) { seen[this.grid] = ret.toInt32(); },
});
Interceptor.attach(exe.add(0x000a6090), {
  onEnter(args) { console.log('[menu-state] dispatch action=' + args[1].toInt32()); },
});

// Every text the frontend binds by FMG id, which is how a popup's message and buttons get their
// words: `FUN_14003d870(command, 7, textId)`, kind 7 = "text by FMG id" (ds2-rva, bindCaptions).
// Printed once per id per burst; look the id up with scripts/ds2-fmg.py. A popup does not show as
// a live grid of its own, so this is the line that says one opened.
let texts = [];
const printedTexts = {};
Interceptor.attach(exe.add(0x0003d870), {
  onEnter(args) {
    const key = args[1].toInt32() + ':0x' + args[2].toUInt32().toString(16);
    if (printedTexts[key]) return;
    printedTexts[key] = true;
    texts.push(key);
  },
});

let lastFocus = null;
let lastGrids = null;
setInterval(() => {
  const fg = GetForegroundWindow();
  GetWindowThreadProcessId(fg, pidOut);
  const focus = pidOut.readU32() === Process.id;
  if (focus !== lastFocus) {
    console.log('[menu-state] focus=' + focus + ' fg=' + fg);
    lastFocus = focus;
  }
  // The cursor is read off the grid, not taken from the call's return: a d-pad move in a row list
  // does not call FEX_GRID_CURRENT_INDEX, so the last return goes stale. The formula is that
  // function's own: `+0x1e != 0 || +0xd0 < 0 ? +0xcc : +0xd0`.
  const cursor = (g) => {
    const p = ptr(g);
    const live = p.add(0xd0).readS32();
    return (p.add(0x1e).readU8() !== 0 || live < 0) ? p.add(0xcc).readS32() : live;
  };
  const grids = Object.keys(seen).sort().map((g) => g + ':' + cursor(g)).join(',') || 'none';
  seen = {};
  if (texts.length) {
    console.log('[menu-state] texts kind:id=' + texts.join(','));
    texts = [];
    for (const k of Object.keys(printedTexts)) delete printedTexts[k];
  }
  if (grids !== lastGrids) {
    console.log('[menu-state] live grids=' + grids);
    lastGrids = grids;
  }
}, 500);
console.log('[menu-state] attached');
