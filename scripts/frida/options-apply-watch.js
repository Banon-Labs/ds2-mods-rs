// Log every call to the Game tab's options commit, with the working copy it applies.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/options-apply-watch.js`
//
// GAME_OPTION_GAME_TAB_APPLY (RVA 0x19d7d0) is what the options menu calls on confirm and what
// ds2-voice-chat calls on its key. Each call prints the live block before and after and the return
// address, so a menu confirm and an F8 press can be told apart and byte 0x0b (voice chat) read
// off both. Read-only.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe');
const APPLY = exe.base.add(0x0019d7d0);

function bytes(p) {
  return Array.from(new Uint8Array(p.readByteArray(0x10))).join(',');
}

Interceptor.attach(APPLY, {
  onEnter(args) {
    this.block = args[0];
    const ret = this.returnAddress;
    const off = ret.sub(exe.base);
    const inExe = off.compare(ptr(exe.size)) < 0 && ret.compare(exe.base) >= 0;
    console.log('[options-apply] caller=' + (inExe ? 'exe+0x' + off.toString(16) : ret) +
      ' before=[' + bytes(this.block) + '] copy=[' + bytes(args[1]) + ']');
  },
  onLeave() {
    console.log('[options-apply] after=[' + bytes(this.block) + '] voice_chat_byte=' +
      this.block.add(0x0b).readU8());
  },
});
console.log('[options-apply] attached apply=' + APPLY);
