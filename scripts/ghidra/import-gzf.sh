#!/usr/bin/env bash
# One-shot import of the DARK SOULS II gzf into a reusable LOCAL Ghidra project, so
# `scripts/ghidra/query.sh` has something to query. Idempotent: `-overwrite` re-imports over an
# existing program of the same name.
#
#   bash scripts/ghidra/import-gzf.sh                      # default gzf, default project
#   GZF=/path/to/other.gzf bash scripts/ghidra/import-gzf.sh
#
# WHY LOCAL AND NOT THE SHARED SERVER. The program also lives in a Ghidra Server repository
# (`ghidra://85.215.148.24:13100/From Software`, folder `Dark Souls II SotFS`), and that is where
# it was originally analysed. Headless cannot reach it without credentials -- an unauthenticated
# run gets `ERROR Server access denied ... Unauthorized` and aborts -- and `analyzeHeadless` has no
# non-interactive password path short of `-connect <user> -p` (a prompt) or a `-keystore`. Rather
# than put a credential in the repo, or make every static query depend on a remote host being up,
# the queries run against a local import of the same gzf. Two further reasons this is the better
# default even once credentials exist: a local project cannot dirty a repository other people check
# out, and it makes a static query reproducible offline.
#
# THE GZF IS NOT IN THE REPO and never will be: it is game-derived data. Supply your own.
#
# `-noanalysis` because a gzf is a Ghidra export that is ALREADY analysed -- it carries the
# functions, symbols and types with it. Re-analysing would take hours and overwrite exactly the
# curation that makes the gzf worth having.
#
# Same java.io.tmpdir gotcha as `query.sh`: /tmp is a small tmpfs here and Ghidra overflows it
# unpacking a ~140MB gzf. Plain TMPDIR does not reach java.io.tmpdir, so set it explicitly.
set -euo pipefail

GZF=${GZF:-/home/banon/Downloads/pc_DarkSoulsIISotFS_static_1.0.3.exe.gzf}
PROJ_DIR=${DS2_GHIDRA_PROJ_DIR:-$HOME/.cache/ds2-ghidra}
PROJ_NAME=${DS2_GHIDRA_PROJ_NAME:-ds2}
TMP=${DS2_GHIDRA_TMP:-$HOME/.cache/ds2-ghidra-tmp}
# Ghidra 11.3.1, not 12.x: the gzf and the shared project were made with 11.3.1, and Ghidra refuses
# to open anything written by a newer version. See query.sh.
GHIDRA_INSTALL=${DS2_GHIDRA_INSTALL:-/home/banon/Downloads/ghidra_11.3.1_PUBLIC}
# JDK 21, not the system JDK 26: Ghidra 11.3.1's OSGi container cannot match an execution
# environment for a JVM that new and every postScript then fails to compile. See query.sh for the
# misleading error it produces. The import itself does not compile scripts, so this matters less
# here -- it is set anyway so both entry points agree about which JVM runs Ghidra.
JAVA_HOME=${DS2_GHIDRA_JAVA_HOME:-/home/banon/tools/jdk-21.0.11+10}
HEADLESS="$GHIDRA_INSTALL/support/analyzeHeadless"

if [[ ! -x "$HEADLESS" ]]; then
  echo "import: no analyzeHeadless at $HEADLESS (set DS2_GHIDRA_INSTALL)" >&2
  exit 3
fi
if [[ ! -x "$JAVA_HOME/bin/java" ]]; then
  echo "import: no JDK at $JAVA_HOME (set DS2_GHIDRA_JAVA_HOME); must be JDK 21" >&2
  exit 3
fi
if [[ ! -f "$GZF" ]]; then
  echo "import: gzf not found: $GZF" >&2
  echo "  set GZF=/path/to/pc_DarkSoulsIISotFS_static_1.0.3.exe.gzf -- game-derived, not in the repo" >&2
  exit 2
fi

mkdir -p "$TMP" "$PROJ_DIR"
export TMPDIR="$TMP"
export GHIDRA_JAVA_OPTIONS="-Djava.io.tmpdir=$TMP"
export JAVA_HOME
export PATH="$JAVA_HOME/bin:$PATH"

echo "importing $GZF"
echo "      -> $PROJ_DIR/$PROJ_NAME"
"$HEADLESS" "$PROJ_DIR" "$PROJ_NAME" \
  -import "$GZF" \
  -noanalysis \
  -overwrite

echo
echo "done. verify what landed before trusting an address from it:"
echo "  bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Info.java 0x140832e70"
