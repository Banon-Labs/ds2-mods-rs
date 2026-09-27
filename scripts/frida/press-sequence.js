// Play a timed sequence of key chords to this repo's dinput8.dll, and to nothing else.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/press-sequence.js \
//   --config-json '{"start_ms":1000,"steps":[{"vks":[18,57],"hold_ms":200,"after_ms":800,"label":"Alt+9"}]}'`
//
// `press-f8.js` answers one key, once, and counts polls. The selector in ds2-net-effects reads
// chords (Alt+9, Alt+', Alt+M) and wants many presses in one session, one attach, so this holds a
// SET of virtual keys for a time instead: while a step is held, `GetAsyncKeyState` calls whose
// return address is inside `dinput8.dll` get 0x8000 for the step's keys, and `GetForegroundWindow`
// calls from there get the game's own window. Calls from anywhere else -- the game's input code
// included -- see nothing. The real keyboard is not involved and no focus changes.
//
// Each step: `vks` held for `hold_ms`, then nothing held for `after_ms`. `label` is only for the log.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const steps = Array.isArray(config.steps) ? config.steps : [];
const startMs = typeof config.start_ms === 'number' ? config.start_ms : 1000;
const ours = Process.getModuleByName('dinput8.dll');
const fromOurs = (ret) => ret.compare(ours.base) >= 0 && ret.compare(ours.base.add(ours.size)) < 0;

const user32 = Process.getModuleByName('user32.dll');
const getWindowThreadProcessId = new NativeFunction(
  user32.getExportByName('GetWindowThreadProcessId'), 'uint32', ['pointer', 'pointer']);
const findWindowExW = new NativeFunction(
  user32.getExportByName('FindWindowExW'), 'pointer', ['pointer', 'pointer', 'pointer', 'pointer']);
const getCurrentProcessId = new NativeFunction(
  Process.getModuleByName('kernel32.dll').getExportByName('GetCurrentProcessId'), 'uint32', []);

function ownWindow() {
  const pid = getCurrentProcessId();
  const out = Memory.alloc(4);
  let hwnd = findWindowExW(NULL, NULL, NULL, NULL);
  while (!hwnd.isNull()) {
    getWindowThreadProcessId(hwnd, out);
    if (out.readU32() === pid) return hwnd;
    hwnd = findWindowExW(NULL, hwnd, NULL, NULL);
  }
  return NULL;
}

const game = ownWindow();
const say = (text) => console.log('[press-sequence] ' + text);
say('dinput8.dll ' + ours.base + ' game window ' + game + ' steps ' + steps.length);

// The schedule, as absolute times from load.
const t0 = Date.now();
let at = startMs;
const plan = steps.map((step, index) => {
  const begin = at;
  const end = begin + (step.hold_ms || 200);
  at = end + (step.after_ms || 600);
  return { index, begin, end, vks: step.vks || [], label: step.label || String(step.vks) };
});
const finish = at;
let polled = plan.map(() => 0);
let announced = plan.map(() => false);

function heldNow() {
  const now = Date.now() - t0;
  for (const p of plan) {
    if (now >= p.begin && now < p.end) return p;
  }
  return null;
}

Interceptor.attach(user32.getExportByName('GetForegroundWindow'), {
  onEnter() { this.ours = fromOurs(this.returnAddress); },
  onLeave(retval) {
    if (this.ours && Date.now() - t0 < finish && !game.isNull()) retval.replace(game);
  },
});

Interceptor.attach(user32.getExportByName('GetAsyncKeyState'), {
  onEnter(args) {
    this.vk = fromOurs(this.returnAddress) ? (args[0].toInt32() & 0xff) : -1;
  },
  onLeave(retval) {
    if (this.vk < 0) return;
    const p = heldNow();
    if (p === null || p.vks.indexOf(this.vk) < 0) return;
    retval.replace(ptr(0x8000));
    polled[p.index] += 1;
    if (!announced[p.index]) {
      announced[p.index] = true;
      say('step ' + p.index + ' ' + p.label + ' held (first poll at +' + (Date.now() - t0) + 'ms)');
    }
  },
});

// One line per step once it is over, with how many of our polls it answered: zero means the DLL
// never asked for those keys during the hold, which is a finding, not a press.
plan.forEach((p) => {
  setTimeout(() => {
    say('step ' + p.index + ' ' + p.label + ' released; answered ' + polled[p.index] + ' polls');
    send({ step: p.index, label: p.label, polls: polled[p.index] });
  }, p.end + 50);
});
setTimeout(() => say('sequence done'), finish + 100);
