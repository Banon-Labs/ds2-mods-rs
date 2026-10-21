// Write a loaded module's live image (every section) to one file, for static RE.
//
// Default ds2sc.dll: the DLL on disk is packed, so none of its code references the lobby strings
// in .rdata ("_bimm_", "ds2sc_yui_nexus_session_search_"); the code only exists in process memory.
// With MODULE = 'DarkSoulsII.exe' the dump is the game as Seamless and our loader left it, which
// diffed against darksoulsii-deobf.bin lists every prologue someone patched.
// Read-only: no Interceptor, no writes into the game.
//
// Output: <Game>/<module>-image-<base>.bin, base to base+size, unreadable pages zeroed.

'use strict';

const MODULE = 'DarkSoulsII.exe';
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
const modules = Process.enumerateModules().map((m) => [m.name, m.base.toString(), m.size]);
send({ dump: path, base: mod.base.toString(), size: mod.size, readable_pages: readable, unreadable_pages: unreadable, modules });
