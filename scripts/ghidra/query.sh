#!/usr/bin/env bash
# Run a Ghidra postScript against the DARK SOULS II program, headless and read-only.
#
#   bash scripts/ghidra/query.sh <postScript.java> [scriptArg ...]
#   bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Info.java
#   bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Decomp.java 0x140832e70
#
# NOT a copy of er-mods-rs/scripts/ghidra/query.sh. Three things differ on this box and each one
# breaks the ER version outright:
#
#   1. GHIDRA 11.3.1, not 12.1. The ER script hardcodes `~/tools/ghidra_12.1_PUBLIC`. The DS2
#      project was made with 11.3.1 (`~/.config/ghidra/ghidra_11.3.1_PUBLIC/preferences` names it),
#      and Ghidra will not open a project written by a NEWER version -- so pointing 12.1 at it is a
#      hard failure, not a warning. Override with DS2_GHIDRA_INSTALL if you move the install.
#
#   2. A LOCALLY IMPORTED PROJECT, built by `scripts/ghidra/import-gzf.sh`. DARK SOULS II also
#      lives in a Ghidra Server repository (`ghidra://85.215.148.24:13100/From Software`), which is
#      where it was analysed -- but headless cannot reach it without credentials: an
#      unauthenticated run gets `ERROR Server access denied ... Unauthorized` and aborts. Importing
#      the same gzf locally costs one command, needs no secret in the repo, cannot dirty a shared
#      repository, and makes every static query reproducible offline. Run import-gzf.sh first.
#
#   3. `-process` NAMES THE PROGRAM. Bare `-process` means "every file in the project". One program
#      is in there today, so it would work by accident; naming it is what keeps that true after the
#      second one is imported.
#
# Everything else is deliberately identical to the ER script, including the two traps it encodes:
#
#   * `-scriptPath` gets the passed script's OWN directory, and that directory must contain ONLY
#     .java files that compile. Ghidra compiles every .java in a scriptPath dir as one OSGi bundle,
#     so a single broken sibling poisons the bundle and the script you asked for fails with
#     "class could not be found" -- pointing at the wrong file entirely. That is why the scripts
#     live in scripts/ghidra/rt/ and why that dir is curated. See rt/README.md.
#
#   * java.io.tmpdir is forced off /tmp. It is a small tmpfs here and Ghidra overflows it; plain
#     TMPDIR does not reach java.io.tmpdir, so GHIDRA_JAVA_OPTIONS sets it explicitly.
#
# And one this box adds: JAVA 21, NOT THE SYSTEM JAVA. The system JDK is 26, and Ghidra 11.3.1's
# OSGi container does not recognise it -- every script fails to compile with a message that points
# at entirely the wrong thing:
#
#     ERROR SCRIPT ERROR: Ds2Info.java : The class could not be found.
#     Caused by: BundleException: Unable to resolve ... missing requirement osgi.ee;(osgi.ee=UNKNOWN)
#
# "The class could not be found" is the SAME symptom as the poisoned-bundle trap above, so it is
# easy to spend an hour hunting a broken sibling that does not exist. The `osgi.ee=UNKNOWN` line is
# the real cause: Felix cannot match an execution environment for a JVM this new. JDK 21 fixes it.
#
# READ-ONLY BY CONSTRUCTION: -readOnly and -noanalysis, so a query cannot dirty the program or
# silently kick off a multi-hour re-analysis. A script that needs to WRITE has to say so, and must
# not use this wrapper.
set -euo pipefail

GHIDRA_INSTALL=${DS2_GHIDRA_INSTALL:-/home/banon/Downloads/ghidra_11.3.1_PUBLIC}
PROJ_DIR=${DS2_GHIDRA_PROJ_DIR:-$HOME/.cache/ds2-ghidra}
PROJ_NAME=${DS2_GHIDRA_PROJ_NAME:-ds2}
PROGRAM=${DS2_GHIDRA_PROGRAM:-pc_DarkSoulsIISotFS_static_1.0.3.exe}
TMP=${DS2_GHIDRA_TMP:-$HOME/.cache/ds2-ghidra-tmp}

JAVA_HOME=${DS2_GHIDRA_JAVA_HOME:-/home/banon/tools/jdk-21.0.11+10}

HEADLESS="$GHIDRA_INSTALL/support/analyzeHeadless"
if [[ ! -x "$HEADLESS" ]]; then
  echo "ghidra query: no analyzeHeadless at $HEADLESS" >&2
  echo "  set DS2_GHIDRA_INSTALL to a Ghidra 11.3.1 install (a NEWER Ghidra cannot open this project)" >&2
  exit 3
fi
if [[ ! -x "$JAVA_HOME/bin/java" ]]; then
  echo "ghidra query: no JDK at $JAVA_HOME (set DS2_GHIDRA_JAVA_HOME)" >&2
  echo "  must be JDK 21 -- the system JDK 26 makes every script fail with osgi.ee=UNKNOWN" >&2
  exit 3
fi
if [[ ! -d "$PROJ_DIR/$PROJ_NAME.rep" ]]; then
  echo "ghidra query: no project at $PROJ_DIR/$PROJ_NAME.rep" >&2
  echo "  build it first:  bash scripts/ghidra/import-gzf.sh" >&2
  exit 3
fi

if [[ $# -lt 1 ]]; then
  echo "Usage: bash scripts/ghidra/query.sh <postScript.java> [scriptArg ...]" >&2
  exit 2
fi

SCRIPT_FILE="$1"; shift
if [[ ! -f "$SCRIPT_FILE" ]]; then
  echo "postScript not found: $SCRIPT_FILE" >&2
  exit 2
fi
SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_FILE")" && pwd)"
SCRIPT_NAME="$(basename "$SCRIPT_FILE")"

mkdir -p "$TMP"
export TMPDIR="$TMP"
export GHIDRA_JAVA_OPTIONS="-Djava.io.tmpdir=$TMP"
# Both, deliberately. Ghidra's launcher resolves the JVM from JAVA_HOME, but some paths shell out to
# a bare `java`, so PATH has to agree or the two disagree about which JDK is in use.
export JAVA_HOME
export PATH="$JAVA_HOME/bin:$PATH"

if [[ -n "${DS2_GHIDRA_RAW:-}" ]]; then
  exec "$HEADLESS" "$PROJ_DIR" "$PROJ_NAME" \
    -process "$PROGRAM" \
    -noanalysis \
    -readOnly \
    -scriptPath "$SCRIPT_DIR" \
    -postScript "$SCRIPT_NAME" "$@"
fi

# THE OUTPUT FILTER, and why it is not optional ergonomics.
#
# A GhidraScript's println() does NOT go to stdout. It goes through log4j and comes out as
#
#     INFO  Ds2Info.java> imageBase 140000000 (GhidraScript)
#
# buried in ~60 lines of classpath and analysis logging. The obvious reflex -- piping through
# `grep -v '^INFO'` to quiet the noise -- deletes the script's entire output and leaves a run that
# looks like it silently produced nothing. That mistake cost a full round trip while writing this.
# So the wrapper does the extraction itself: script lines, unprefixed, on stdout.
#
# ERROR and Exception lines are ALWAYS passed through to stderr. A filter that can hide a failure is
# worse than no filter, because the run then reads as an empty success. Set DS2_GHIDRA_RAW=1 for the
# unfiltered firehose when diagnosing the harness rather than the binary.
set +e
out=$("$HEADLESS" "$PROJ_DIR" "$PROJ_NAME" \
  -process "$PROGRAM" \
  -noanalysis \
  -readOnly \
  -scriptPath "$SCRIPT_DIR" \
  -postScript "$SCRIPT_NAME" "$@" 2>&1)
status=$?
set -e

printf '%s\n' "$out" \
  | sed -nE 's/^INFO  '"${SCRIPT_NAME//./\\.}"'> ?(.*) \(GhidraScript\) *$/\1/p'

if printf '%s\n' "$out" | grep -qE '^(ERROR|Exception|.*BundleException)'; then
  {
    echo "--- ghidra reported errors (query.sh) ---"
    printf '%s\n' "$out" | grep -E '^(ERROR|Exception)|BundleException|ClassNotFound' | head -20
    echo "--- re-run with DS2_GHIDRA_RAW=1 for the full log ---"
  } >&2
  # A script that failed to load still exits 0 in some Ghidra paths, so the error scan -- not the
  # exit code alone -- is what decides failure here.
  [[ $status -eq 0 ]] && status=1
fi

exit $status
