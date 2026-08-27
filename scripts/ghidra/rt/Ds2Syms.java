// List symbols whose name contains any of the given keywords (case-insensitive).
//
//   query.sh scripts/ghidra/rt/Ds2Syms.java FeObject
//   query.sh scripts/ghidra/rt/Ds2Syms.java DLRuntimeClass DLReferencePointer
//
// This is the single most useful script against DARK SOULS II, and the reason is a property of
// THIS binary rather than of Ghidra: DS2 is heavily symbolised. 5271 MSVC RTTI type descriptors and
// 587 DLRF-registered runtime classes mean identifying a class is usually READING ITS NAME, not
// inferring it from vtable shape. See docs/DS2-ENGINE.md.
//
// Ported from er-mods-rs/scripts/ghidra/rt/RtSyms.java, with the cap made explicit rather than
// silent -- the original stopped at 400 and printed the count, which reads identically whether it
// found 400 or ran out at 400. Silent truncation is how a sweep gets mistaken for exhaustive.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolIterator;

public class Ds2Syms extends GhidraScript {
    private static final int CAP = 400;

    @Override public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length == 0) {
            println("usage: Ds2Syms <keyword> [keyword ...]");
            return;
        }
        String[] kws = new String[args.length];
        for (int i = 0; i < args.length; i++) {
            kws[i] = args[i].toLowerCase();
        }

        SymbolIterator si = currentProgram.getSymbolTable().getAllSymbols(true);
        int shown = 0;
        int matched = 0;
        while (si.hasNext()) {
            Symbol s = si.next();
            String n = s.getName().toLowerCase();
            boolean hit = false;
            for (String k : kws) {
                if (n.contains(k)) {
                    hit = true;
                    break;
                }
            }
            if (!hit) {
                continue;
            }
            matched++;
            if (shown < CAP) {
                println("  " + s.getAddress() + "  " + s.getSymbolType() + "  " + s.getName());
                shown++;
            }
        }
        if (matched > shown) {
            println("(shown " + shown + " of " + matched + " -- TRUNCATED at the " + CAP
                + " cap; narrow the keyword, this is not the whole answer)");
        } else {
            println("(shown " + shown + ", complete)");
        }
    }
}
