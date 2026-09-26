// Does [GameManagerImp]+0x104 advance exactly once per frame through the title, a load and play?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/frame-counter-phases.js`
//
// Attach as early in a launch as possible: the point is the stretch frame-counter.js never
// sampled -- the title screen and the load into the world. Timers do not fire reliably in this
// game's Frida runtime, so this is driven by the frame function itself (0x140aeeed0, not an Arxan
// redirect): every call is one frame, and every 120 calls it logs how far the counter moved over
// those calls, the GameManagerImp pointer, and the title-flow flag (FE_OPERATOR_TITLE_ACTIVE).
// A counter that is the frame counter moves by exactly the call count, in every phase; a stall
// during a load shows as a delta of 0 with calls still arriving. Read-only.

'use strict';

const base = Process.getModuleByName('DarkSoulsII.exe').base;
const GAME_MANAGER_IMP = base.add(0x016148f0);
const TITLE_ACTIVE = base.add(0x01614804);
const FRAME = base.add(0x00aeeed0);
const WINDOW = 120;

let calls = 0;
let lastCount = null;
let lastGmi = null;

function sample() {
  let gmi = NULL;
  let count = null;
  try {
    gmi = GAME_MANAGER_IMP.readPointer();
    if (!gmi.isNull()) count = gmi.add(0x104).readU32();
  } catch (e) {}
  const title = TITLE_ACTIVE.readU8();
  const delta = count !== null && lastCount !== null ? count - lastCount : 'n/a';
  const moved = lastGmi !== null && !gmi.equals(lastGmi) ? ' GMI-CHANGED' : '';
  console.log('[phases] calls=' + calls + ' delta=' + delta + '/' + WINDOW + ' count=' + count +
    ' gmi=' + gmi + ' title=' + title + moved);
  lastCount = count;
  lastGmi = gmi;
}

Interceptor.attach(FRAME, {
  onEnter() {
    calls += 1;
    if (calls % WINDOW === 0) sample();
  },
});
sample();
