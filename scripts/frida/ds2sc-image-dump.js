// Write a loaded module's live image (every section) to one file, for static RE.
//
// For ds2sc.dll: the DLL on disk is packed, so none of its code references the lobby strings
// in .rdata ("_bimm_", "ds2sc_yui_nexus_session_search_"); the code only exists in process memory.
// With MODULE = 'DarkSoulsII.exe' the dump is the game as Seamless and our loader left it, which
// diffed against darksoulsii-deobf.bin lists every prologue someone patched.
// Read-only: no Interceptor, no writes into the game.
//
// Output: <Game>/<module>-image-<base>.bin, base to base+size, unreadable pages zeroed, and beside
// it <module>-image-<base>.ranges.json: every mapped range inside the module with its live
// protection, so code that only becomes executable at runtime (an unpacked section) shows up.
//
// The module comes from the watcher's --config-json, so dumping Seamless needs no edit:
//   --config-json '{"module": "ds2sc.dll"}'
// With no config it dumps DarkSoulsII.exe, as it always did.

'use strict';

const MODULE = (globalThis.__ER_FRIDA_CONFIG || {}).module || 'DarkSoulsII.exe';
const OUT_DIR = 'Z:\\home\\banon\\.local\\share\\Steam\\steamapps\\common\\Dark Souls II Scholar of the First Sin\\Game\\';
const PAGE = 0x1000;

const mod = Process.getModuleByName(MODULE);
const path = OUT_DIR + MODULE.replace(/\.[a-z]+$/i, '') + '-image-' + mod.base.toString(16) + '.bin';
const file = new File(path, 'wb');
const zero = new ArrayBuffer(PAGE);
let readable = 0;
let unreadable = 0;
for (let offset = 0; offset < mod.size; offset += PAGE) {
  let bytes = null;
  try {
    bytes = mod.base.add(offset).readByteArray(PAGE);
  } catch (e) {
    bytes = null;
  }
  if (bytes === null) {
    unreadable += 1;
    file.write(zero);
  } else {
    readable += 1;
    file.write(bytes);
  }
}
file.flush();
file.close();
const end = mod.base.add(mod.size);
const ranges = Process.enumerateRanges('---')
  .filter((r) => r.base.compare(end) < 0 && r.base.add(r.size).compare(mod.base) > 0)
  .map((r) => ({ offset: '0x' + r.base.sub(mod.base).toString(16), size: '0x' + r.size.toString(16), protection: r.protection }));
const rangesPath = path.replace(/\.bin$/, '.ranges.json');
const rangesFile = new File(rangesPath, 'w');
rangesFile.write(JSON.stringify({ module: MODULE, base: mod.base.toString(), size: mod.size, ranges }, null, 1));
rangesFile.flush();
rangesFile.close();
const modules = Process.enumerateModules().map((m) => [m.name, m.base.toString(), m.size]);
send({ dump: path, ranges: rangesPath, base: mod.base.toString(), size: mod.size, readable_pages: readable,
  unreadable_pages: unreadable, executable_ranges: ranges.filter((r) => r.protection[2] === 'x'), modules });
