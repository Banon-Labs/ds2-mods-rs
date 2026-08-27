// References TO each address, and how many there are.
//
//   query.sh scripts/ghidra/rt/Ds2Xrefs.java 0x140832e70
//
// The call-site count is the number this repo picks hook sites by. M1 chose RVA 0x00832e70 because
// it has 2052 static call sites and a clean prologue, and rejected 0x00832cb0 (12401 sites) and
// 0x00c2c9e0 (4866) because both are already Arxan-redirected. So the TOTAL is the useful output
// here, and it is printed even when the listing is capped -- a truncated list with an honest total
// is a measurement, a truncated list with no total is a guess.
//
// Merges er-mods-rs's rt/RtCallers.java and CallSitesTo.java, which differed only in whether they
// counted or listed.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.RefType;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceIterator;

public class Ds2Xrefs extends GhidraScript {
    private static final int LIST_CAP = 60;

    @Override public void run() throws Exception {
        for (String a : getScriptArgs()) {
            Address addr = toAddr(Long.decode(a));
            Function tgt = getFunctionContaining(addr);
            println("################ refs TO " + a
                + " (" + (tgt != null ? tgt.getName() : "?") + ") ################");

            ReferenceIterator it = currentProgram.getReferenceManager().getReferencesTo(addr);
            int total = 0;
            int calls = 0;
            int shown = 0;
            while (it.hasNext()) {
                Reference r = it.next();
                total++;
                RefType rt = r.getReferenceType();
                if (rt.isCall()) {
                    calls++;
                }
                if (shown < LIST_CAP) {
                    Function f = getFunctionContaining(r.getFromAddress());
                    println("  " + r.getFromAddress() + "  "
                        + (f != null ? f.getName() + " @ " + f.getEntryPoint() : "?") + "  " + rt);
                    shown++;
                }
            }
            if (total == 0) {
                println("  (no refs)");
            } else {
                println("  total=" + total + "  calls=" + calls
                    + (shown < total ? "  (listed " + shown + ", TRUNCATED)" : ""));
            }
        }
    }
}
