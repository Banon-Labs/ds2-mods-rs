// Disassemble up to 400 instructions from ds2-input-harness's pad detour (DINPUT8.dll+0xe50e0, then the shared poll body at +0xe5ed0),
// printing only calls and stores with a displacement, to find where it writes the pad's fields.
// Read-only.

'use strict';

const dll = Process.getModuleByName('DINPUT8.dll');
let at = dll.base.add(0xe5ed0);
for (let i = 0; i < 3000; i++) {
  const insn = Instruction.parse(at);
  const text = insn.mnemonic + ' ' + insn.opStr;
  if (insn.mnemonic === 'call' || insn.mnemonic === 'jmp' || /0x198|0x2f8|0x314|0x19c/.test(insn.opStr)) {
    console.log('[dump] +0x' + at.sub(dll.base).toString(16) + ' ' + text);
  }
  if (insn.mnemonic === 'ret' || insn.mnemonic === 'int3') {
    console.log('[dump] +0x' + at.sub(dll.base).toString(16) + ' ' + text);
    if (insn.mnemonic === 'int3') break;
  }
  at = insn.next;
}
