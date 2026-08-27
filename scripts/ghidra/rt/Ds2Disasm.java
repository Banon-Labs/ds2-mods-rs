// Disassemble from each VA.
//
//   query.sh scripts/ghidra/rt/Ds2Disasm.java 0x140832e70
//   DS2_DISASM_COUNT=80 query.sh scripts/ghidra/rt/Ds2Disasm.java 0x140832e70
//
// This is the script that answers "is this site safe to hook", which is the question this repo
// keeps asking. Two things to read in the output:
//
//   * THE PROLOGUE. A clean MSVC entry (`MOV [RSP+8], RBX` / `PUSH RBX; SUB RSP,0x20`) is the
//     trivial MinHook relocation case. A leading `JMP rel32` into the second `.text` block is an
//     Arxan-redirected stub -- 286 functions are, and patching over Arxan's own jump fails for
//     reasons that have nothing to do with your hook.
//   * WHERE THE FIRST FIVE BYTES END. MinHook overwrites five bytes. If an instruction boundary
//     does not fall on or after byte 5, the relocation is the hard case.
//
// Ported from er-mods-rs/scripts/ghidra/rt/RtDisasm.java. The ER version hardcoded 30 instructions
// and stopped at the first `RET`, which truncates any function with an early-out -- and an early
// `RET` is extremely common. Here the stop is the function's own body when one is known, so the
// listing ends where the function ends rather than at its first return.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Listing;

public class Ds2Disasm extends GhidraScript {
    @Override public void run() throws Exception {
        int count = 40;
        String env = System.getenv("DS2_DISASM_COUNT");
        if (env != null && !env.isEmpty()) {
            count = Integer.decode(env);
        }
        Listing lst = currentProgram.getListing();
        for (String a : getScriptArgs()) {
            long va = Long.decode(a);
            Address start = toAddr(va);
            Function f = getFunctionContaining(start);
            println("################ " + a + " -> "
                + (f != null ? f.getName() + " @ " + f.getEntryPoint()
                    + " (" + f.getBody().getNumAddresses() + " bytes)" : "NO_FUNC")
                + " ################");
            // Start at the function entry when there is one -- a VA in the middle of a function is
            // usually a call target you want to see from the top -- and at the raw VA otherwise.
            Address addr = f != null ? f.getEntryPoint() : start;
            for (int i = 0; i < count; i++) {
                Instruction insn = lst.getInstructionAt(addr);
                if (insn == null) {
                    println("  (no instruction at " + addr + " -- undefined bytes or data)");
                    break;
                }
                println("  " + insn.getAddress() + "  " + insn);
                addr = insn.getAddress().add(insn.getLength());
                if (f != null && !f.getBody().contains(addr)) {
                    println("  (end of " + f.getName() + ")");
                    break;
                }
            }
        }
    }
}
