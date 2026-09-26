// Follow PadDevice::Poll's entry jump chain and name the module each hop lands in, to see which
// detour owns the poll. Read-only.

'use strict';

let at = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00f05540);
for (let hop = 0; hop < 4; hop++) {
  const insn = Instruction.parse(at);
  const mod = Process.findModuleByAddress(at);
  console.log('[poll-entry] hop ' + hop + ' at ' + at + (mod ? ' (' + mod.name + '+0x' +
    at.sub(mod.base).toString(16) + ')' : ' (no module)') + ': ' + insn.mnemonic + ' ' + insn.opStr);
  if (insn.mnemonic !== 'jmp') break;
  const m = insn.opStr.match(/^qword ptr \[rip(?: ([+-]) (0x[0-9a-f]+))?\]$/);
  if (m) {
    const disp = m[2] ? parseInt(m[2], 16) * (m[1] === '-' ? -1 : 1) : 0;
    at = insn.next.add(disp).readPointer();
  } else {
    at = ptr(insn.opStr);
  }
}
