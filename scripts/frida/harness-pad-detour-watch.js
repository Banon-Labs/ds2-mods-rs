// Hook ds2-input-harness's own pad-poll detour (found by following PadDevice::Poll's entry jump
// chain into DINPUT8.dll) and print, once a second, the +0x198 / +0x2f8 / +0x314 / +0x19c each
// device carries when the detour returns -- i.e. after the harness has written. Twenty seconds.
// Read-only.

'use strict';

let at = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00f05540);
for (let hop = 0; hop < 3; hop++) {
  const insn = Instruction.parse(at);
  if (insn.mnemonic !== 'jmp') break;
  at = insn.opStr === 'qword ptr [rip]' ? insn.next.readPointer() : ptr(insn.opStr);
}
const mod = Process.findModuleByAddress(at);
console.log('[harness-detour] at ' + at + ' ' + (mod ? mod.name + '+0x' + at.sub(mod.base).toString(16) : '?'));

const t0 = Date.now();
let seen = {};
let last = 0;
const l = Interceptor.attach(at, {
  onEnter(args) { this.self = args[0]; },
  onLeave() {
    const s = this.self;
    const key = s + ' 198=0x' + s.add(0x198).readU16().toString(16) + ' 2f8=0x' +
      s.add(0x2f8).readU32().toString(16) + ' 314=' + s.add(0x314).readS32() + ' 19c=' +
      s.add(0x19c).readS32();
    seen[key] = (seen[key] || 0) + 1;
    const now = Date.now() - t0;
    if (now - last >= 1000) {
      last = now;
      console.log('[harness-detour] t=' + now + ' ' + JSON.stringify(seen));
      seen = {};
    }
    if (now > 20000) l.detach();
  },
});
