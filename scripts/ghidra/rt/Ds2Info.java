// What program is actually open, and does its address space line up with this repo's numbers?
//
// Run this FIRST on any new project, before trusting a single address from it. Every static fact
// this repo records -- the Arxan footprint, the M1 hook site, the RTTI and DLRF counts -- was
// measured against `darksoulsii-deobf.bin`, a FLAT MAPPED image where file offset == RVA and
// VA == offset + 0x140000000. A Ghidra project built from a different artifact (the shipped PE,
// a runtime dump, a different build) can be perfectly valid and still not share those addresses,
// and nothing about the decompiler output would tell you so. So the program says who it is here.
//
//   query.sh scripts/ghidra/rt/Ds2Info.java
//   query.sh scripts/ghidra/rt/Ds2Info.java 0x140832e70 0x140838
//
// With no arguments it prints identity, image base and the memory map. With arguments it also
// dumps 16 bytes at each VA, which is the actual correspondence test: compare against
// `xxd -s $((VA - 0x140000000)) -l 16 darksoulsii-deobf.bin`. Matching bytes at a known site mean
// the two artifacts agree at that address; they do not prove agreement everywhere, and an
// Arxan-obfuscated image agrees with a deobfuscated one at every site Arxan did not touch.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.mem.MemoryBlock;

public class Ds2Info extends GhidraScript {
    @Override public void run() throws Exception {
        println("name          " + currentProgram.getName());
        println("path          " + currentProgram.getDomainFile().getPathname());
        println("executable    " + currentProgram.getExecutablePath());
        println("format        " + currentProgram.getExecutableFormat());
        println("md5           " + currentProgram.getExecutableMD5());
        println("sha256        " + currentProgram.getExecutableSHA256());
        println("language      " + currentProgram.getLanguageID());
        println("compilerSpec  " + currentProgram.getCompilerSpec().getCompilerSpecID());
        println("imageBase     " + currentProgram.getImageBase());
        println("minAddress    " + currentProgram.getMinAddress());
        println("maxAddress    " + currentProgram.getMaxAddress());
        println("functions     " + currentProgram.getFunctionManager().getFunctionCount());
        println("symbols       " + currentProgram.getSymbolTable().getNumSymbols());

        println("");
        println("memory blocks (name, start, end, size, rwx, initialized)");
        for (MemoryBlock b : currentProgram.getMemory().getBlocks()) {
            println(String.format("  %-12s %s %s %#10x %c%c%c %s",
                b.getName(), b.getStart(), b.getEnd(), b.getSize(),
                b.isRead() ? 'r' : '-', b.isWrite() ? 'w' : '-', b.isExecute() ? 'x' : '-',
                b.isInitialized() ? "init" : "UNINITIALIZED"));
        }

        String[] args = getScriptArgs();
        if (args.length == 0) {
            return;
        }
        println("");
        println("bytes (compare: xxd -s $((VA - 0x140000000)) -l 16 darksoulsii-deobf.bin)");
        for (String a : args) {
            Address va = toAddr(Long.decode(a));
            StringBuilder hex = new StringBuilder();
            try {
                byte[] buf = new byte[16];
                // Partial reads throw rather than silently short-reading, so a VA that runs off the
                // end of a block is reported as unreadable instead of as 16 bytes of anything.
                currentProgram.getMemory().getBytes(va, buf);
                for (byte x : buf) {
                    hex.append(String.format("%02x ", x));
                }
            } catch (Exception e) {
                hex.append("<unreadable: ").append(e.getMessage()).append(">");
            }
            println(String.format("  %s  %s", va, hex.toString().trim()));
        }
    }
}
