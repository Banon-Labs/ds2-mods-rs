#!/usr/bin/env python3
"""Top-level C++ namespace histogram over a Ghidra symbol dump.

Input is the TSV that `rt/Ds2SymCensus.java dump` writes
(`addr <tab> type <tab> source <tab> qualified-name`), or any file with one
qualified name per line.

    bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2SymCensus.java dump /tmp/ds2-syms.tsv
    python3 scripts/ghidra/sym-namespaces.py /tmp/ds2-syms.tsv --col 3
    python3 scripts/ghidra/sym-namespaces.py /tmp/ds2-syms.tsv --col 3 --type Function
    python3 scripts/ghidra/sym-namespaces.py /tmp/ds2-syms.tsv --col 3 --depth 2 --filter DLRF

WHY TEMPLATES ARE STRIPPED FIRST, and why a plain `sed 's/::.*//'` gets this wrong.
DARK SOULS II's mangled names nest namespaces inside template arguments constantly:

    DLRF::DLConcreteMethodInvoker<class_DLLG::DLAppender,class_DLTX::DLBasicString<...>,...>

A naive split on `::` counts that name under DLRF, DLLG *and* DLTX, so every heavily
templated framework namespace inflates every namespace it touches. Stripping balanced
`<...>` before splitting counts it once, under DLRF, which is the only namespace that
name actually declares. The difference is not cosmetic: DLUT looks like it has hundreds
of classes under the naive split and has 17 under this one, because `DLUT::DLNullType`
and `DLUT::TypeList::DLTypeList` appear as filler arguments in most other templates.

A count here is a count of SYMBOLS in a namespace, not of distinct classes. Feed it the
RTTI type-descriptor names when you want a class inventory.
"""
import argparse
import sys
from collections import Counter


def strip_templates(s: str) -> str:
    """Drop everything inside balanced angle brackets, keeping the outer name."""
    out = []
    depth = 0
    for ch in s:
        if ch == '<':
            depth += 1
        elif ch == '>':
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(ch)
    return ''.join(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('path')
    ap.add_argument('--col', type=int, default=None,
                    help='0-based TSV column holding the name (default: whole line)')
    ap.add_argument('--type', default=None,
                    help='keep only rows whose column 1 equals this (e.g. Function, Label)')
    ap.add_argument('--depth', type=int, default=1,
                    help='namespace segments to group by (default 1)')
    ap.add_argument('--filter', default=None,
                    help='keep only keys starting with this prefix')
    ap.add_argument('--top', type=int, default=0, help='print only the N largest')
    args = ap.parse_args()

    hist = Counter()
    total = 0
    with open(args.path, encoding='utf-8', errors='replace') as fh:
        for line in fh:
            line = line.rstrip('\n')
            if not line:
                continue
            if args.col is not None:
                parts = line.split('\t')
                if len(parts) <= args.col:
                    continue
                if args.type is not None and (len(parts) < 2 or parts[1] != args.type):
                    continue
                name = parts[args.col]
            else:
                name = line
            bare = strip_templates(name)
            if '::' not in bare:
                continue
            key = '::'.join(bare.split('::')[:args.depth])
            if args.filter and not key.startswith(args.filter):
                continue
            hist[key] += 1
            total += 1

    rows = hist.most_common(args.top) if args.top else sorted(
        hist.items(), key=lambda kv: (-kv[1], kv[0]))
    for k, v in rows:
        print(f'{v:7d}  {k}')
    print(f'{total:7d}  (TOTAL namespaced rows, {len(hist)} distinct namespaces)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
