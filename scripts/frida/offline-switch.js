// Is the branch ds2-offline's login skip patches the one its static reading says it is?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/offline-switch.js`
//
// `FeSubStateTitleUserPolicy`'s enter (0x1400f9040) compares system-data byte +0x136e with zero at
// 0x1400f9070 and skips phase 3 with `je` at 0x1400f9077 when it is zero. `docs/DS2-OFFLINE.md`
// reads that from the disassembly of the image on disk. This checks the same bytes in the running
// process, after Arxan has done whatever it does to `.text`, and walks the pointer chain to the
// byte the branch tests. It hooks nothing and writes nothing.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;

// `cmp byte [rax+0x136e], 0; je +0x12`
const SITE_RVA = 0xf9070;
const EXPECTED = [0x80, 0xb8, 0x6e, 0x13, 0x00, 0x00, 0x00, 0x74, 0x12];
const GAME_MANAGER_IMP = 0x16148f0;

function hex(bytes) {
  return Array.from(bytes, b => ('0' + b.toString(16)).slice(-2)).join(' ');
}

function readPtr(p) {
  try {
    return p.readPointer();
  } catch (e) {
    return null;
  }
}

function sample(tag) {
  const live = new Uint8Array(base.add(SITE_RVA).readByteArray(EXPECTED.length));
  const match = EXPECTED.every((b, i) => live[i] === b);
  console.log('[offline-switch] ' + tag + ' site=' + base.add(SITE_RVA) + ' live=' + hex(live) +
    ' expected=' + hex(EXPECTED) + ' match=' + match);

  const manager = readPtr(base.add(GAME_MANAGER_IMP));
  let sys = null;
  if (manager !== null && !manager.isNull()) {
    const a8 = readPtr(manager.add(0xa8));
    if (a8 !== null && !a8.isNull()) {
      sys = readPtr(a8.add(0xd8));
    }
    const net = readPtr(manager.add(0x22f0));
    const online = net === null || net.isNull() ? 'n/a' : net.add(0x3a).readU8();
    console.log('[offline-switch] ' + tag + ' manager=' + manager + ' net=' + net + ' net+0x3a=' + online);
  }
  if (sys === null || sys.isNull()) {
    console.log('[offline-switch] ' + tag + ' system data not reachable yet');
    return;
  }
  console.log('[offline-switch] ' + tag + ' sys=' + sys + ' sys+0x136d=' + sys.add(0x136d).readU8() +
    ' sys+0x136e=' + sys.add(0x136e).readU8());
}

sample('load');
setInterval(() => sample('tick'), 5000);
