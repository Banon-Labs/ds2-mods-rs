// Read-only: hexdump an address and name its first qword as a vtable RVA.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/peek.js
//  --config-json '{"addr": "0x7fffd7755000", "len": 512}'`

'use strict';

const cfg = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = ptr(cfg.addr);
const len = cfg.len || 256;
const v = at.readPointer();
console.log('[peek] ' + at + ' vtable=' + v + ' rva=0x' + v.sub(exe).toString(16));
console.log(hexdump(at, { length: len, header: false, ansi: false }));
console.log('[peek] done');
