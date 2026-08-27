// Count symbols, or dump the whole symbol table to a file for offline slicing.
//
//   query.sh scripts/ghidra/rt/Ds2SymCensus.java count DLRF@@ DLUT@@ FD4
//   query.sh scripts/ghidra/rt/Ds2SymCensus.java dump /tmp/ds2-syms.tsv
//   query.sh scripts/ghidra/rt/Ds2SymCensus.java dump /tmp/ds2-rtti.tsv RTTI_Type_Descriptor
//
// WHY THIS EXISTS ALONGSIDE Ds2Syms. Ds2Syms answers "show me symbols like X" and caps at 400 --
// correct for reading, useless for a census. A survey question ("how many DLRF classes are there,
// and what namespaces exist at what counts") needs a TOTAL, and needs it for many patterns at once
// without 400-line walls of output between them. `count` gives one line per pattern.
//
// WHY `dump` WRITES A FILE INSTEAD OF PRINTING. This program has 426625 symbols. Pushing those
// through println means pushing them through log4j and then through query.sh's line extractor, and
// the whole log ends up captured in a shell variable -- tens of MB for one query. Worse, it makes
// every follow-up question ("now group those by namespace") another 15s Ghidra round trip. One dump
// to a TSV turns the rest of a survey into grep/awk, where each number is a short command that can
// be quoted verbatim in a document. The file is an analysis artifact written to a path the caller
// names; nothing about the program is modified, so this is still safe under query.sh's -readOnly.
//
// The dumped columns are: address, symbol type, source, and the FULLY QUALIFIED name. Qualification
// matters here: DS2's functions are `FUN_<va>` sitting inside a real class namespace
// (`EventCameraOperator::FUN_1400011b0`), so the unqualified name throws away the only class
// information the binary still carries. See docs/DS2-ENGINE.md.

import ghidra.app.script.GhidraScript;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolIterator;

import java.io.BufferedWriter;
import java.io.FileWriter;
import java.io.PrintWriter;

public class Ds2SymCensus extends GhidraScript {
    @Override public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            println("usage: Ds2SymCensus count <pattern> [pattern ...]");
            println("       Ds2SymCensus dump <outfile.tsv> [pattern ...]");
            println("  count   symbols whose fully-qualified name contains each pattern (case-insensitive)");
            println("  dump    write addr/type/source/qualified-name TSV; no pattern means every symbol");
            return;
        }
        String mode = args[0];

        if (mode.equals("count")) {
            if (args.length < 2) {
                println("count needs at least one pattern");
                return;
            }
            String[] pats = new String[args.length - 1];
            long[] hits = new long[pats.length];
            for (int i = 0; i < pats.length; i++) {
                pats[i] = args[i + 1].toLowerCase();
            }
            long total = 0;
            SymbolIterator si = currentProgram.getSymbolTable().getAllSymbols(true);
            while (si.hasNext()) {
                String n = si.next().getName(true).toLowerCase();
                total++;
                for (int i = 0; i < pats.length; i++) {
                    if (n.contains(pats[i])) {
                        hits[i]++;
                    }
                }
            }
            for (int i = 0; i < pats.length; i++) {
                println(String.format("%-40s %d", args[i + 1], hits[i]));
            }
            println(String.format("%-40s %d", "(symbols scanned)", total));
            return;
        }

        if (mode.equals("dump")) {
            if (args.length < 2) {
                println("dump needs an output path");
                return;
            }
            String out = args[1];
            String[] pats = new String[args.length - 2];
            for (int i = 0; i < pats.length; i++) {
                pats[i] = args[i + 2].toLowerCase();
            }
            long written = 0;
            long total = 0;
            try (PrintWriter w = new PrintWriter(new BufferedWriter(new FileWriter(out)))) {
                SymbolIterator si = currentProgram.getSymbolTable().getAllSymbols(true);
                while (si.hasNext()) {
                    Symbol s = si.next();
                    total++;
                    String qn = s.getName(true);
                    if (pats.length > 0) {
                        String lc = qn.toLowerCase();
                        boolean hit = false;
                        for (String p : pats) {
                            if (lc.contains(p)) {
                                hit = true;
                                break;
                            }
                        }
                        if (!hit) {
                            continue;
                        }
                    }
                    // Tabs are the separator, so any tab inside a name would corrupt a column.
                    // MSVC symbols do not contain them; replace rather than trust that.
                    w.println(s.getAddress() + "\t" + s.getSymbolType() + "\t" + s.getSource()
                        + "\t" + qn.replace('\t', ' '));
                    written++;
                }
            }
            println("wrote " + written + " of " + total + " symbols to " + out);
            return;
        }

        println("unknown mode: " + mode + " (want count|dump)");
    }
}
