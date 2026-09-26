// Print DS2's live options block once: the Game tab (0x10 bytes), Screen tab (4) and the 0x14 after.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/options-block-read.js`
//
// Chain GAME_MANAGER_IMP (RVA 0x16148f0) -> +0xa8 -> +0xc8, as SaveDataOption reads it. Run it
// before a save and again after the next launch to see whether a setting reached the container.
// Read-only.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe');
const imp = exe.base.add(0x016148f0).readPointer();
if (imp.isNull()) {
  console.log('[options-block] GameManagerImp is null');
} else {
  const block = imp.add(0xa8).readPointer().add(0xc8).readPointer();
  if (block.isNull()) {
    console.log('[options-block] options block is null');
  } else {
    const all = Array.from(new Uint8Array(block.readByteArray(0x28)));
    console.log('[options-block] at=' + block + ' game=[' + all.slice(0, 0x10).join(',') +
      '] screen=[' + all.slice(0x10, 0x14).join(',') + '] rest=[' + all.slice(0x14).join(',') + ']');
  }
}
