// Read memory as something in particular: pointers, bytes, or a string.
//
//   query.sh scripts/ghidra/rt/Ds2Mem.java ptrs 0x1410a6000 16     # vtable slots, resolved
//   query.sh scripts/ghidra/rt/Ds2Mem.java bytes 0x140832e70 32
//   query.sh scripts/ghidra/rt/Ds2Mem.java str   0x1411234f0
//
// The vtable reader is the mode that earns its place in DARK SOULS II. 5271 MSVC RTTI type
// descriptors and 587 DLRF-registered runtime classes mean a vtable slot usually resolves to a
// NAMED function, so walking a vtable identifies a class by reading rather than by inferring shape.
// See docs/DS2-ENGINE.md.
//
// Ported from er-mods-rs/scripts/ghidra/rt/RtVtbl.java, minus its `disasm`/`refs` modes (those are
// Ds2Disasm and Ds2Xrefs -- the ER original bundled four unrelated tools behind a mode switch) and
// minus `w4`, whose "try it as image-relative, then try it raw" guessing prints two answers and
// leaves the reader to pick. If a table is image-relative, say so with `rva` and get one answer.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;

public class Ds2Mem extends GhidraScript {
    @Override public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            println("usage: Ds2Mem <ptrs|rva|bytes|str> <va> [count]");
            println("  ptrs  <count> 8-byte pointers, each resolved to function/symbol");
            println("  rva   <count> 4-byte image-relative offsets, resolved against the image base");
            println("  bytes <count> raw bytes as hex");
            println("  str   NUL-terminated ASCII (count caps the length, default 256)");
            return;
        }
        String mode = args[0];
        long va = Long.decode(args[1]);
        int count = args.length > 2 ? Integer.decode(args[2]) : 16;

        Memory mem = currentProgram.getMemory();
        SymbolTable st = currentProgram.getSymbolTable();
        long base = currentProgram.getImageBase().getOffset();

        switch (mode) {
            case "ptrs": {
                for (int i = 0; i < count; i++) {
                    Address slot = toAddr(va + (long) i * 8);
                    long p = mem.getLong(slot);
                    println("  [" + slot + "] = " + String.format("%016x", p) + describe(toAddr(p), st));
                }
                break;
            }
            case "rva": {
                println("  (image base " + currentProgram.getImageBase() + ")");
                for (int i = 0; i < count; i++) {
                    Address slot = toAddr(va + (long) i * 4);
                    long off = mem.getInt(slot) & 0xffffffffL;
                    println("  [" + slot + "] rva=" + String.format("%08x", off)
                        + describe(toAddr(base + off), st));
                }
                break;
            }
            case "bytes": {
                byte[] buf = new byte[count];
                mem.getBytes(toAddr(va), buf);
                // ONE println PER ROW, never one println containing newlines. A GhidraScript's
                // println goes through log4j as `INFO  <script>> <text> (GhidraScript)`, and only
                // the FIRST line of a multi-line message carries that prefix -- so query.sh's
                // extractor keeps line one and silently drops the rest. A 48-byte dump came back
                // completely empty because of it, which reads as "unreadable memory" rather than
                // as a formatting bug.
                for (int row = 0; row < buf.length; row += 16) {
                    StringBuilder hex = new StringBuilder();
                    for (int i = row; i < row + 16 && i < buf.length; i++) {
                        hex.append(String.format("%02x ", buf[i]));
                    }
                    println(String.format("  %s  %s", toAddr(va + row), hex.toString().trim()));
                }
                break;
            }
            case "str": {
                int cap = args.length > 2 ? count : 256;
                StringBuilder sb = new StringBuilder();
                Address a = toAddr(va);
                for (int i = 0; i < cap; i++) {
                    byte b = mem.getByte(a.add(i));
                    if (b == 0) {
                        break;
                    }
                    sb.append((char) (b & 0xff));
                }
                println("  str@" + a + " = \"" + sb + "\"");
                break;
            }
            default:
                println("unknown mode: " + mode + " (want ptrs|rva|bytes|str)");
        }
    }

    /** Function and symbol at an address, or empty markers -- never a silent blank. */
    private String describe(Address tgt, SymbolTable st) {
        Function f = getFunctionContaining(tgt);
        Symbol s = st.getPrimarySymbol(tgt);
        return "  -> " + tgt
            + "  fn=" + (f != null ? f.getName() + "@" + f.getEntryPoint() : "-")
            + "  sym=" + (s != null ? s.getName() : "-");
    }
}
