// Return to the title the way the pause menu's Quit Game "Yes" does, then load another slot.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/switch-character.js`
// on a game that has already loaded a character (e.g. `ds2-run.py --continue-slot 1`). SLOT is the
// character to load second.
//
// Question (4ne): after a character switch inside one launch, does the second character's
// PlayerParam hold its own soul memory, or the first one's? Run with `--soul-memory-guard`: its
// verdict line on each load is the reading.
//
// Writes, all copied from the game's own paths:
//   1. On the game thread (inside SaveLoadSystem::update, which the in-game state's handler calls),
//      once, in game state 0x1e with no return already requested: call 0x14006ec90, the body of
//      FeGroupInGameReturnTitleCheck's "Yes". It reads no argument.
//   2. At the title top menu, resting (phase 0): phase = 2, LOAD GAME.
//   3. In the character list, resting (phase 1), once: the list's own close 0x1400f10e0, then the
//      selected slot and phase 2 (LoadProfile) -- what ds2-continue's take_load_branch does.

'use strict';

const SLOT = 0;
const B = Process.getModuleByName('DarkSoulsII.exe').base;
const gmi = () => B.add(0x16148f0).readPointer();
const yes = new NativeFunction(B.add(0x6ec90), 'void', ['pointer']);
const closeList = new NativeFunction(B.add(0xf10e0), 'void', ['pointer']);
const log = (s) => console.log('[switch-character] ' + s);

let quitDone = false;
let topDone = false;
let listDone = false;

const g0 = gmi();
log('preflight state=0x' + g0.add(0x24ac).readS32().toString(16) +
  ' b1=0x' + g0.add(0x24b1).readU8().toString(16));

Interceptor.attach(B.add(0x2e6b00), {
  onEnter() {
    if (quitDone) return;
    const g = gmi();
    if (g.add(0x24ac).readS32() !== 0x1e || (g.add(0x24b1).readU8() & 4)) return;
    quitDone = true;
    yes(ptr(0));
    log('return-to-title requested b1=0x' + g.add(0x24b1).readU8().toString(16) +
      ' countdown=' + g.add(0x24b4).readFloat());
  },
});

Interceptor.attach(B.add(0xff300), {
  onEnter(args) { this.self = args[0]; },
  onLeave() {
    if (!quitDone || topDone) return;
    if (this.self.add(0x10).readS32() !== 0) return;
    topDone = true;
    this.self.add(0x10).writeS32(2);
    log('top menu: LOAD GAME');
  },
});

Interceptor.attach(B.add(0xfba10), {
  onEnter(args) { this.self = args[0]; },
  onLeave() {
    if (!topDone || listDone) return;
    if (this.self.add(0x10).readS32() !== 1) return;
    const ctx = B.add(0x160de10).readPointer();
    const group = ctx.add(0x98).readPointer();
    const rec = gmi().add(0xa8).readPointer().add(0xd8).readPointer().add(SLOT * 0x1f0);
    const flags = rec.add(0x1d9).readU8();
    if ((flags & 1) === 0 || (flags & 2) !== 0) {
      listDone = true;
      log('slot ' + SLOT + ' unusable flags=0x' + flags.toString(16));
      return;
    }
    listDone = true;
    closeList(group);
    ctx.add(0x564).writeS32(SLOT);
    this.self.add(0x10).writeS32(2);
    log('character list: loading slot ' + SLOT);
  },
});
