// Do IDXGISwapChain::Present and NvNavigationSystem::Update run on the same thread in DS2?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/tick-threads.js`
//
// ds2-invasion-path's gametick queue sits between the two seams. If both run on one thread the
// queue is belt-and-braces; if not, every try_lock in it matters. Set PRESENT to the address this
// run's ds2-loader.log gives in "Present hooked at 0x..." -- dxgi.dll moves between runs. Counts
// the thread ids each seam is called from, every 100 calls up to 300. Read-only.
//
// Answered 2026-09-26: both seams ran on thread 328 for 300 calls each, so ds2-invasion-path's
// gametick queue is belt-and-braces rather than load-bearing.

'use strict';

const PRESENT = ptr('0x6ffffc99b2b0');
const NAV_UPDATE = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00baeb20);
const LIMIT = 300;

const seen = { present: {}, nav: {} };
const calls = { present: 0, nav: 0 };

function report(key) {
  if (calls[key] % 100 !== 0) return;
  console.log('[tick-threads] ' + key + ' after ' + calls[key] + ' calls, threads ' +
    JSON.stringify(seen[key]));
}

function watch(address, key) {
  Interceptor.attach(address, {
    onEnter() {
      if (calls[key] >= LIMIT) return;
      calls[key] += 1;
      const id = Process.getCurrentThreadId();
      seen[key][id] = (seen[key][id] || 0) + 1;
      report(key);
    },
  });
}

watch(PRESENT, 'present');
watch(NAV_UPDATE, 'nav');
console.log('[tick-threads] attached present=' + PRESENT + ' nav=' + NAV_UPDATE);
