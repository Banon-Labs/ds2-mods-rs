// Do the singleton accessors the Ghidra project names read what their disassembly says they read?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/singletons.js`
//
// Each accessor below is a two- or three-instruction function over a global, read statically on
// 2026-09-25 (docs in `ds2-rva`). This walks the same pointer chains in the live process and
// prints every hop with the vtable at the end of it, so a chain that is right shows a live object
// with a vptr inside the image, and a chain that is wrong shows null or a pointer to nowhere. It
// hooks nothing and writes nothing; it reads once at load and again every five seconds, because a
// manager that is null at the title screen may exist once a character is loaded.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const base = image.base;

function inImage(p) {
  return p.compare(base) >= 0 && p.compare(base.add(image.size)) < 0;
}

function readPtr(p) {
  try {
    return p.readPointer();
  } catch (e) {
    return null;
  }
}

// One chain: a global RVA, then a list of offsets to dereference in turn.
function walk(name, rva, offsets) {
  const hops = [];
  let p = readPtr(base.add(rva));
  hops.push('[' + base.add(rva) + ']=' + p);
  for (const off of offsets) {
    if (p === null || p.isNull()) {
      break;
    }
    p = readPtr(p.add(off));
    hops.push('+0x' + off.toString(16) + '=' + p);
  }
  let vptr = 'n/a';
  if (p !== null && !p.isNull()) {
    const v = readPtr(p);
    vptr = v === null ? 'unreadable' : v + (inImage(v) ? ' (in image, rva 0x' + v.sub(base).toString(16) + ')' : ' (outside image)');
  }
  console.log('[singletons] ' + name + ': ' + hops.join(' -> ') + ' vptr=' + vptr);
}

function sample(tag) {
  console.log('[singletons] sample ' + tag + ' base=' + base);
  // getSaveLoadSystem 0x1401ab6e0: [GameManagerImp]+0xb8.
  walk('SaveLoadSystem', 0x016148f0, [0xb8]);
  // getMapItemPackManager 0x1401e6550: [GameManagerImp]+0x38 (MapManager) +0x1c8.
  walk('MapManager', 0x016148f0, [0x38]);
  walk('MapItemPackManager', 0x016148f0, [0x38, 0x1c8]);
  // getNetSessionManager 0x1402d85e0: [[0x141616cf8]] -- the global holds a pointer to the pointer.
  walk('NetSessionManager', 0x01616cf8, [0x0]);
}

sample('load');
let n = 0;
setInterval(function () {
  n += 1;
  sample(String(n));
}, 5000);
