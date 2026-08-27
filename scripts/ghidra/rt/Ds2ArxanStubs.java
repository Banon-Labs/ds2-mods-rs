// Which functions have been redirected by Arxan? Answers it for the whole image, or for a
// specific list of VAs.
//
//   query.sh scripts/ghidra/rt/Ds2ArxanStubs.java                       # census: how many, and where
//   query.sh scripts/ghidra/rt/Ds2ArxanStubs.java list                  # every redirected function, named
//   query.sh scripts/ghidra/rt/Ds2ArxanStubs.java 0x14014bec0 0x1402206d0   # verdict per VA
//
// WHAT IT LOOKS FOR, and why that is the right test. `docs/PORTING.md` records 48 Arxan stubs and
// 286 redirected functions. The redirect has one visible shape: the function's first instruction is
// an unconditional `JMP rel32` whose target is in the image's SECOND `.text` block -- the extra
// executable block Arxan appends, at 0x141aaf000 here. A function that starts that way has no
// prologue left to hook: MinHook's five bytes would land on Arxan's own jump, which is a different
// problem from a hard relocation and fails for different reasons. `scripts/ghidra/README.md` tells
// you to check with Ds2Disasm before trusting a site; this is that check, made countable and made
// runnable over every candidate at once instead of one at a time.
//
// THE BLOCK BOUNDARY IS READ, NOT ASSUMED. The script finds the .text blocks by name and treats
// every .text block after the first as redirect territory. Hardcoding 0x141aaf000 would bake one
// build's layout into a tool, which is the mistake `README.md` describes BytePatternScan making.
//
// A NEGATIVE HERE IS NOT A GUARANTEE THE SITE IS SAFE. It says the entry point is not a redirect
// stub. Arxan's integrity checks can still cover a byte range that includes a clean function, and
// whether a detour survives them is an open question in this repo (`docs/DS2-ENGINE.md`,
// "Still unverified"). This tool answers "is the prologue still the game's own", nothing more.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.MemoryBlock;

import java.util.ArrayList;
import java.util.List;

public class Ds2ArxanStubs extends GhidraScript {
    @Override public void run() throws Exception {
        // Every .text block after the first is where Arxan's redirected bodies live.
        List<MemoryBlock> textBlocks = new ArrayList<>();
        for (MemoryBlock b : currentProgram.getMemory().getBlocks()) {
            if (b.getName().equals(".text")) {
                textBlocks.add(b);
            }
        }
        if (textBlocks.size() < 2) {
            println("only " + textBlocks.size() + " .text block(s) -- this image has no second"
                + " executable block, so there is nothing for this test to find");
            return;
        }
        List<MemoryBlock> redirectBlocks = textBlocks.subList(1, textBlocks.size());
        for (MemoryBlock b : redirectBlocks) {
            println("redirect territory: " + b.getName() + " " + b.getStart() + "-" + b.getEnd());
        }

        String[] args = getScriptArgs();
        boolean listMode = args.length == 1 && args[0].equals("list");

        if (args.length > 0 && !listMode) {
            for (String a : args) {
                Address va = toAddr(Long.decode(a));
                Function f = getFunctionContaining(va);
                String target = redirectTargetOf(va, redirectBlocks);
                println(a + "  " + (f == null ? "(no function)" : f.getName())
                    + (target == null ? "  CLEAN (entry is not a redirect stub)"
                                      : "  ARXAN-REDIRECTED -> " + target));
            }
            return;
        }

        long total = 0;
        long redirected = 0;
        FunctionIterator fi = currentProgram.getFunctionManager().getFunctions(true);
        while (fi.hasNext()) {
            Function f = fi.next();
            total++;
            String target = redirectTargetOf(f.getEntryPoint(), redirectBlocks);
            if (target == null) {
                continue;
            }
            redirected++;
            if (listMode) {
                println("  " + f.getEntryPoint() + "  " + f.getName(true) + "  -> " + target);
            }
        }
        println("functions            " + total);
        println("arxan-redirected     " + redirected);
    }

    // Returns the jump target as a string if the instruction at `entry` is an unconditional JMP
    // into one of `redirectBlocks`, or null if it is anything else.
    private String redirectTargetOf(Address entry, List<MemoryBlock> redirectBlocks) {
        Instruction insn = getInstructionAt(entry);
        if (insn == null || !insn.getMnemonicString().equalsIgnoreCase("JMP")) {
            return null;
        }
        for (Address flow : insn.getFlows()) {
            for (MemoryBlock b : redirectBlocks) {
                if (flow.compareTo(b.getStart()) >= 0 && flow.compareTo(b.getEnd()) <= 0) {
                    return flow.toString();
                }
            }
        }
        return null;
    }
}
