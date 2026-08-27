// Scan memory for a masked byte pattern, reporting the containing function for each hit.
//
//   query.sh scripts/ghidra/rt/Ds2ByteScan.java '48 89 5c 24 08'          # exact bytes
//   query.sh scripts/ghidra/rt/Ds2ByteScan.java '83 8? b4 01 00 00 01'    # ? = wildcard nibble
//   query.sh scripts/ghidra/rt/Ds2ByteScan.java '?? 8b d9' 200            # ?? = wildcard byte
//
// This is the one script here that is a REWRITE rather than a port, and deliberately so.
// er-mods-rs/scripts/ghidra/BytePatternScan.java is not a scanner -- it is four hardcoded patterns
// for `[reg+0x1b4]`, an Elden Ring ChrLoadState field. The offset, the field and the whole question
// are Elden Ring facts, so porting it verbatim would have carried a foreign structure layout into
// this repo disguised as a tool. What was worth keeping is the mechanism it demonstrates: masked
// `Memory.findBytes`, which handles the ModRM register field varying across an otherwise fixed
// instruction. That mechanism is here, with the pattern supplied by the caller.
//
// PATTERN SYNTAX: whitespace-separated hex bytes. `??` (or `..`) is a fully wildcarded byte. A `?`
// in either nibble wildcards that nibble -- `8?` matches 0x80..0x8f, which is exactly the ModRM
// case where one instruction encodes eight registers.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.mem.Memory;

public class Ds2ByteScan extends GhidraScript {
    @Override public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            println("usage: Ds2ByteScan '<hex pattern>' [cap]");
            println("  '48 89 5c 24 08'        exact");
            println("  '83 8? b4 01 00 00 01'  ? wildcards a nibble (ModRM register field)");
            println("  '?? 8b d9'              ?? wildcards a whole byte");
            return;
        }
        String pattern = args[0];
        int cap = args.length > 1 ? Integer.decode(args[1]) : 100;

        String[] toks = pattern.trim().split("\\s+");
        byte[] pat = new byte[toks.length];
        byte[] mask = new byte[toks.length];
        for (int i = 0; i < toks.length; i++) {
            String t = toks[i].toLowerCase();
            if (t.length() != 2) {
                println("bad token '" + toks[i] + "' -- each must be exactly two hex chars or wildcards");
                return;
            }
            int value = 0;
            int m = 0;
            for (int nib = 0; nib < 2; nib++) {
                char c = t.charAt(nib);
                value <<= 4;
                m <<= 4;
                if (c == '?' || c == '.') {
                    continue; // value nibble stays 0, mask nibble stays 0 => "don't care"
                }
                int v = Character.digit(c, 16);
                if (v < 0) {
                    println("bad hex digit '" + c + "' in token '" + toks[i] + "'");
                    return;
                }
                value |= v;
                m |= 0xf;
            }
            pat[i] = (byte) value;
            mask[i] = (byte) m;
        }

        StringBuilder shown = new StringBuilder();
        for (int i = 0; i < pat.length; i++) {
            shown.append(String.format("%02x/%02x ", pat[i] & 0xff, mask[i] & 0xff));
        }
        println("### pattern (byte/mask): " + shown.toString().trim() + " ###");

        Memory mem = currentProgram.getMemory();
        FunctionManager fm = currentProgram.getFunctionManager();
        Address cur = currentProgram.getMinAddress();
        int found = 0;
        while (cur != null) {
            Address a = mem.findBytes(cur, pat, mask, true, monitor);
            if (a == null) {
                break;
            }
            found++;
            if (found <= cap) {
                Function f = fm.getFunctionContaining(a);
                Instruction insn = getInstructionAt(a);
                println("  " + a + "  "
                    + (f == null ? "(no func)" : f.getName(true) + " @ " + f.getEntryPoint())
                    + (insn == null ? "" : "  | " + insn));
            }
            if (found == cap + 1) {
                // Once, on crossing the cap -- not once per hit. Scanning CONTINUES past it so the
                // total below is a measurement rather than "at least N".
                println("  ...listing capped at " + cap + "; still counting...");
            }
            // +1, not +pat.length: overlapping matches are real and skipping past them would
            // silently under-count a pattern that occurs inside itself.
            cur = a.add(1);
        }
        println("  total=" + found + (found > cap ? "  (listed " + cap + ")" : ""));
    }
}
