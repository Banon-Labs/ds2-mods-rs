// Decompile the function containing each VA.
//
//   query.sh scripts/ghidra/rt/Ds2Decomp.java 0x140832e70 0x1408389e0
//
// Ported from er-mods-rs/scripts/ghidra/rt/RtDecomp.java. Renamed off the `Rt` prefix on purpose:
// there it meant "runtime dump project", and this project is a static import, so keeping `Rt`
// would assert something about the artifact that is not true.
//
// Addresses are VAs, not RVAs. This repo records RVAs (the deobf image is flat-mapped, so its file
// offset IS the RVA); add the image base to get a VA. `Ds2Info.java` prints the base -- do not
// assume 0x140000000 without looking, DllCharacteristics has DYNAMIC_BASE set.

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileOptions;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;

public class Ds2Decomp extends GhidraScript {
    @Override public void run() throws Exception {
        DecompInterface di = new DecompInterface();
        di.setOptions(new DecompileOptions());
        di.openProgram(currentProgram);
        try {
            for (String a : getScriptArgs()) {
                Address addr = toAddr(Long.decode(a));
                Function f = getFunctionContaining(addr);
                println("################ " + a + " -> "
                    + (f != null ? f.getName() + " @ " + f.getEntryPoint() : "NO_FUNC") + " ################");
                if (f == null) {
                    continue;
                }
                DecompileResults r = di.decompileFunction(f, 120, monitor);
                if (r == null) {
                    println("(null results)");
                } else if (!r.decompileCompleted()) {
                    println("(FAILED: " + r.getErrorMessage() + ")");
                } else {
                    println(r.getDecompiledFunction().getC());
                }
            }
        } finally {
            // The ER original leaks this. One decompiler process per headless run is survivable
            // because the JVM exits, but it costs nothing to close and it matters the moment
            // anything here loops over hundreds of functions.
            di.dispose();
        }
    }
}
