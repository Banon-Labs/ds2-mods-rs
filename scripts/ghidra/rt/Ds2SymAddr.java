// Resolve symbol names to addresses.
//
//   query.sh scripts/ghidra/rt/Ds2SymAddr.java FeObjectButton::Update
//   query.sh scripts/ghidra/rt/Ds2SymAddr.java DLRuntimeClassImpl
//
// The counterpart to Ds2Syms: that one searches by substring, this one resolves an exact name you
// already have. Prints every match with its namespace, because DS2's MSVC symbols collide across
// classes constantly -- an `Update` or a `Destroy` is dozens of unrelated functions, and taking
// "the first one" is how a wrong address gets into a table and stays there.
//
// Ported from er-mods-rs/scripts/ghidra/rt/RtSymAddr.java.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolIterator;
import ghidra.program.model.symbol.SymbolTable;

public class Ds2SymAddr extends GhidraScript {
    @Override public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length == 0) {
            println("usage: Ds2SymAddr <exact-symbol-name> [name ...]   (substring search: Ds2Syms)");
            return;
        }
        SymbolTable st = currentProgram.getSymbolTable();
        for (String nm : args) {
            SymbolIterator it = st.getSymbols(nm);
            int n = 0;
            while (it.hasNext()) {
                Symbol s = it.next();
                println(nm + " -> " + s.getAddress()
                    + " (" + s.getSymbolType() + ", ns=" + s.getParentNamespace().getName() + ")");
                n++;
            }
            if (n == 0) {
                println(nm + " -> (none)");
            } else if (n > 1) {
                println("  ^ " + n + " symbols share this name -- pick by namespace, do not assume the first");
            }
        }
    }
}
