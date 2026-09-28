// Where is every game thread right now? Read-only stack sampling, no hooks, no writes.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/thread-sample.js`
//   optional `--config-json '{"samples": 20, "every_ms": 250}'`
//
// Takes `samples` snapshots `every_ms` apart. Each snapshot reads every thread's context
// (Process.enumerateThreads) and walks a fuzzy backtrace from it. A frame is kept when it lands
// in DarkSoulsII.exe and printed as its RVA. At the end, for every thread that was ever inside
// the exe, prints how often each distinct stack (top 8 exe frames) was seen: a thread spinning or
// waiting on one thing shows the same stack in every sample.

'use strict';

const cfg = globalThis.__ER_FRIDA_CONFIG || {};
const SAMPLES = cfg.samples || 20;
const EVERY_MS = cfg.every_ms || 250;
const exe = Process.getModuleByName('DarkSoulsII.exe');
const lo = exe.base;
const hi = exe.base.add(exe.size);
const inExe = (p) => p.compare(lo) >= 0 && p.compare(hi) < 0;
const rva = (p) => '0x' + p.sub(lo).toString(16);

const seen = new Map(); // tid -> Map(stack -> count)
let taken = 0;

function sample() {
  for (const t of Process.enumerateThreads()) {
    let frames;
    try {
      frames = Thread.backtrace(t.context, Backtracer.FUZZY).filter(inExe);
    } catch (e) {
      continue;
    }
    const pc = t.context.pc;
    if (inExe(pc)) frames.unshift(pc);
    if (frames.length === 0) continue;
    const key = frames.slice(0, cfg.depth || 8).map(rva).join(' ');
    if (!seen.has(t.id)) seen.set(t.id, new Map());
    const m = seen.get(t.id);
    m.set(key, (m.get(key) || 0) + 1);
  }
  taken += 1;
  if (taken < SAMPLES) {
    setTimeout(sample, EVERY_MS);
  } else {
    for (const [tid, m] of seen) {
      const rows = [...m.entries()].sort((a, b) => b[1] - a[1]).slice(0, tid === cfg.tid ? 1000 : 3);
      for (const [stack, n] of rows) console.log('[sample] tid=' + tid + ' ' + n + '/' + SAMPLES + ' ' + stack);
    }
    console.log('[sample] done');
  }
}

console.log('[sample] ' + SAMPLES + ' samples every ' + EVERY_MS + 'ms');
sample();
