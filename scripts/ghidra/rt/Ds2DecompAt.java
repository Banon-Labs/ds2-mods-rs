// Decompile the function AT each VA, creating it in memory first when analysis never made one.
//
//   query.sh scripts/ghidra/rt/Ds2DecompAt.java 0x140f49df0 0x141004620
//
// Why this exists beside Ds2Decomp: many targets in this binary are reached only through a data
// table or a vtable slot (the FFX action handler table at 0x1415f9b70 is one), so analysis never
// created a function there and Ds2Decomp prints NO_FUNC. This one disassembles and creates the
// function first. query.sh runs -readOnly, so the created function lives only for this run and is
// never saved into the project.
//
// If the VA is inside an existing function, that function is decompiled instead and the banner
// says so -- a mid-function VA is usually a wrong guess at an entry point, and silently creating
// an overlapping function there would decompile garbage.

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileOptions;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;

public class Ds2DecompAt extends GhidraScript {
    @Override public void run() throws Exception {
        DecompInterface di = new DecompInterface();
        di.setOptions(new DecompileOptions());
        di.openProgram(currentProgram);
        try {
            for (String a : getScriptArgs()) {
                Address addr = toAddr(Long.decode(a));
                Function f = getFunctionAt(addr);
                String how = "existing";
                if (f == null) {
                    Function c = getFunctionContaining(addr);
                    if (c != null) {
                        f = c;
                        how = "inside " + c.getName() + " @ " + c.getEntryPoint();
                    } else {
                        disassemble(addr);
                        f = createFunction(addr, "fn_" + a.replace("0x", ""));
                        how = "created";
                    }
                }
                println("################ " + a + " (" + how + ") ################");
                if (f == null) {
                    println("(could not create a function here)");
                    continue;
                }
                DecompileResults r = di.decompileFunction(f, 120, monitor);
                if (r == null || !r.decompileCompleted()) {
                    println("(FAILED: " + (r == null ? "null results" : r.getErrorMessage()) + ")");
                    continue;
                }
                // One println per line: query.sh keeps only lines carrying the GhidraScript suffix,
                // which log4j attaches to the first line of a message only (see Ds2Decomp).
                for (String line : r.getDecompiledFunction().getC().split("\n", -1)) {
                    println(line);
                }
            }
        } finally {
            di.dispose();
        }
    }
}
