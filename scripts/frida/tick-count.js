// Does a candidate per-frame seam actually run, and on which thread? Counts calls for each target
// and prints the totals once a second. Writes nothing and calls nothing.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/tick-count.js`
//
// Targets:
//   0x1402c9540  NET_SESSION_UPDATE (net session manager update, (this, f32 delta))
//   0x140baeb20  NvNavigationSystem::Update (the in-world tick ds2-invasion-path uses)
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const targets = { 'net-session-update': 0x2c9540, 'nv-navigation-update': 0xbaeb20 };
const counts = {};
const threads = {};
for (const [name, rva] of Object.entries(targets)) {
  counts[name] = 0;
  threads[name] = {};
  Interceptor.attach(image.base.add(rva), {
    onEnter() {
      counts[name]++;
      threads[name][this.threadId] = true;
    },
  });
}
let ticks = 0;
setInterval(() => {
  const line = Object.keys(targets).map((n) => n + '=' + counts[n] + ' threads=' + Object.keys(threads[n]).join(',')).join(' ');
  console.log('[tick-count] ' + line);
  send({ kind: 'counts', second: ++ticks, counts: Object.assign({}, counts) });
}, 1000);
