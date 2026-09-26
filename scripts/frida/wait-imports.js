// Are these the import slots `ds2-boot-timeline` fronts to time the boot thread's waits?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/wait-imports.js`
//
// The RVAs were read out of the image on disk: each slot's hint/name entry names the function.
// This reads the same slots in the running game, where the Windows loader has filled them in, and
// prints which module and symbol each one points at. It hooks nothing and writes nothing.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const SLOTS = [
  ['Sleep', 0x1aae314],
  ['WaitForSingleObject', 0x1aae264],
  ['WaitForMultipleObjects', 0x1aae05c],
  ['MsgWaitForMultipleObjects', 0x1aae554],
];

for (const [name, rva] of SLOTS) {
  const target = image.base.add(rva).readPointer();
  const module = Process.findModuleByAddress(target);
  const symbol = DebugSymbol.fromAddress(target);
  console.log('[wait-imports] ' + name + ' slot=' + image.base.add(rva) + ' -> ' + target +
    ' module=' + (module === null ? 'none' : module.name) + ' symbol=' + symbol.name);
}
