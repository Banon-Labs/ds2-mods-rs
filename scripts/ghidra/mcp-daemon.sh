#!/usr/bin/env bash
# Lifecycle manager for the pre-warmed headless Ghidra MCP server (MCPServeHeadless.java).
#
#   scripts/ghidra/mcp-daemon.sh start   [--proj-dir DIR] [--proj-name NAME] [--port N] [--writable] [--save-interval N]
#   scripts/ghidra/mcp-daemon.sh stop
#   scripts/ghidra/mcp-daemon.sh status
#   scripts/ghidra/mcp-daemon.sh restart
#
# WHY THIS EXISTS. Every `query.sh` run spawns its own analyzeHeadless and takes an EXCLUSIVE
# lock on the project, so two agents cannot use Ghidra at the same time at all. This keeps ONE
# process alive holding that lock and serving a TCP port, so every agent that connects shares the
# same warm program. Queries serialize inside the daemon at millisecond scale, which is
# concurrent as far as any caller is concerned. Warmth is the secondary benefit; sharing is the
# point.
#
# READ-ONLY BY DEFAULT, deliberately, and this differs from the er-mods-rs original. MCP edits
# (rename, struct, comment, bookmark) persist into the project, and several agents mutating one
# program have no isolation and no merge -- last writer wins, silently. `--writable` is available
# and is a deliberate single-agent act, not a default anyone should inherit.
#
# WHICH GHIDRA. 12.1, not the 11.3.1 that query.sh uses, because Ghidra extensions are
# version-pinned and GhidraMCP-13bm is installed only under 12.1/12.1.2. The 12.1 project is a
# forward import of the same gzf and carries the same 11.3.1 analysis (verified: 88780 functions,
# imageBase 140000000, .bind at 141d43000 -- identical to the 11.3.1 project).
#
# THE TWO PROJECTS MUST STAY SEPARATE. A direct headless tool cannot open a project this daemon
# holds. Leaving query.sh on the 11.3.1 project and the daemon on the 12.1 one means neither ever
# blocks the other, and a daemon that is down does not mean no Ghidra at all.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 12.1 is required: it is where GhidraMCP-13bm is installed. Ghidra refuses version-mismatched
# extensions, so pointing this at 11.3.1 yields a daemon with no MCPServer class to reflect on.
GHIDRA_INSTALL=${DS2_GHIDRA_MCP_INSTALL:-/home/banon/tools/ghidra_12.1_PUBLIC}
# JDK 21 rather than the system JDK: same constraint query.sh documents for 11.3.1, and verified
# working for 12.1 here (Ds2Info.java compiled and ran under it against the 12.1 project).
JAVA_HOME=${DS2_GHIDRA_JAVA_HOME:-/home/banon/tools/jdk-21.0.11+10}
# Dot-free by necessity: Ghidra 12.1 rejects any project path containing a dot-directory
# ("Path element starting with '.' is not permitted"), which 11.3.1 allowed. That is why the
# repo default of ~/.cache/ds2-ghidra cannot be reused for the 12.1 project.
PROJ_DIR=${DS2_GHIDRA_MCP_PROJ_DIR:-$HOME/ghidra-projects/ds2-121}
PROJ_NAME=${DS2_GHIDRA_MCP_PROJ_NAME:-ds2}
RUN_DIR=${DS2_GHIDRA_MCP_RUN_DIR:-$HOME/ghidra-projects/ds2-mcp}
TMP=${DS2_GHIDRA_MCP_TMP:-$HOME/.cache/ds2-ghidra-tmp-121}

# 8766, NOT the 8765 the er-mods-rs daemon uses. Measured 2026-08-26: an er-mods-rs daemon was
# already serving ELDEN RING on 8765, and the first version of this script reported "already
# running; port 8765 up" and served that. A decompile would have returned Elden Ring code under a
# DS2 heading. Distinct ports per game, and the identity check below, so that cannot recur.
PORT=${DS2_GHIDRA_MCP_PORT:-8766}
RO="-readOnly"   # see READ-ONLY BY DEFAULT above; --writable opts out
SAVE_SEC=0       # nothing to save read-only; --writable raises this

LOG="$RUN_DIR/daemon.log"
STOPFILE="$RUN_DIR/STOP"
PIDFILE="$RUN_DIR/daemon.pid"

CMD="${1:-}"; shift || true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --proj-dir)      PROJ_DIR="$2"; shift 2 ;;
    --proj-name)     PROJ_NAME="$2"; shift 2 ;;
    --port)          PORT="$2"; shift 2 ;;
    --writable)      RO=""; [[ "$SAVE_SEC" == 0 ]] && SAVE_SEC=60; shift ;;
    --save-interval) SAVE_SEC="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

HEADLESS="$GHIDRA_INSTALL/support/analyzeHeadless"
[[ -x "$HEADLESS" ]] || { echo "mcp-daemon: no analyzeHeadless at $HEADLESS (set DS2_GHIDRA_MCP_INSTALL)" >&2; exit 3; }
[[ -x "$JAVA_HOME/bin/java" ]] || { echo "mcp-daemon: no JDK at $JAVA_HOME (set DS2_GHIDRA_JAVA_HOME)" >&2; exit 3; }
if [[ ! -d "$GHIDRA_INSTALL/Ghidra/Extensions/GhidraMCP-13bm" ]]; then
  echo "mcp-daemon: GhidraMCP-13bm is not installed in $GHIDRA_INSTALL" >&2
  echo "  the daemon reflects on ghidra.mcp.MCPServer, which only exists once the extension is installed" >&2
  exit 3
fi

mkdir -p "$RUN_DIR" "$TMP"
export TMPDIR="$TMP"
export GHIDRA_JAVA_OPTIONS="-Djava.io.tmpdir=$TMP"
export JAVA_HOME

MCP_SCRIPT_DIR="$RUN_DIR/script"

# IDENTIFY THE DAEMON BY ITS LISTENING SOCKET, never by a process-name match. The er-mods-rs
# original used `pgrep -f MCPServeHeadless.java`, which is correct only while exactly one such
# daemon can exist. It matches a SIBLING REPO's daemon just as happily: `start` becomes a silent
# no-op that leaves the caller talking to another game's program, and `stop` would kill someone
# else's server. The listening pid plus a cmdline check against OUR staged script dir is exact.
port_pid() { ss -ltnpH "sport = :$PORT" 2>/dev/null | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2; }
port_up()  { [[ -n "$(port_pid)" ]]; }
is_running() {
  local pid; pid="$(port_pid)"
  [[ -n "$pid" ]] || return 1
  grep -qz -- "$MCP_SCRIPT_DIR" "/proc/$pid/cmdline" 2>/dev/null
}
# True when the port is taken by something that is NOT our daemon.
port_is_foreign() { port_up && ! is_running; }

# Self-heal exec bits on the install's native helpers (decompile, sleigh, demanglers, lzfse). An
# install copied off a Windows/drvfs mount silently loses +x on these; the Java side still works,
# so disasm, xrefs and symbol queries all keep answering while EVERY decompile returns
# "Decompilation failed" -- a failure that looks like a decompiler bug and is a file mode. The
# decompile process is spawned per request, so this fixes it WITHOUT a restart. Idempotent.
fix_native_exec_bits() {
  local root="${HEADLESS%/support/analyzeHeadless}"
  [[ -d "$root" ]] || return 0
  find "$root" -type f -path '*/os/linux_x86_64/*' ! -name '*.txt' ! -perm -u+x \
    -exec chmod a+x {} + 2>/dev/null || true
}

do_start() {
  fix_native_exec_bits
  if is_running; then echo "already running: port $PORT, this project"; return 0; fi
  if port_is_foreign; then
    local pid; pid="$(port_pid)"
    echo "mcp-daemon: port $PORT is held by pid $pid, which is NOT this project's daemon." >&2
    echo "  Serving from it would answer DS2 questions with another program's data." >&2
    echo "  Pick another port with --port N, or stop that daemon in the repo that owns it." >&2
    return 4
  fi
  rm -f "$STOPFILE"
  echo "starting: $PROJ_NAME on port $PORT $([[ -n "$RO" ]] && echo read-only || echo writable)"
  # Ghidra builds ONE OSGi bundle for the ENTIRE -scriptPath directory, so a compile error in any
  # sibling .java fails the whole bundle and this script never loads ("Failed to get OSGi bundle
  # containing script"). scripts/ghidra/rt/ holds the query scripts; staging this one alone into a
  # clean directory removes that coupling entirely.
  mkdir -p "$MCP_SCRIPT_DIR"
  cp -f "$SCRIPT_DIR/MCPServeHeadless.java" "$MCP_SCRIPT_DIR/MCPServeHeadless.java"
  # Detach so the daemon outlives this shell; the stop-file is the clean exit path.
  setsid bash -c "exec '$HEADLESS' '$PROJ_DIR' '$PROJ_NAME' -process -noanalysis $RO \
    -scriptPath '$MCP_SCRIPT_DIR' -postScript MCPServeHeadless.java '$PORT' '$STOPFILE' '$SAVE_SEC'" \
    >"$LOG" 2>&1 < /dev/null &
  echo $! > "$PIDFILE"
  # Block on the daemon's own READY/FAILED line rather than sleeping a guessed interval.
  timeout 120 grep -m1 -E "MCP_HEADLESS: (READY|FAILED)" <(tail -F -n +1 "$LOG" 2>/dev/null) >/dev/null 2>&1 || true
  if grep -q "MCP_HEADLESS: READY" "$LOG" 2>/dev/null; then
    echo "READY: $(grep 'MCP_HEADLESS: READY' "$LOG" | tail -1)"; return 0
  fi
  if grep -q "MCP_HEADLESS: FAILED" "$LOG" 2>/dev/null; then
    echo "FAILED to start; see $LOG" >&2; tail -20 "$LOG" >&2; return 1
  fi
  echo "timed out waiting for READY; see $LOG" >&2; tail -20 "$LOG" >&2; return 1
}

do_stop() {
  if port_is_foreign; then
    echo "mcp-daemon: port $PORT belongs to another project's daemon; refusing to stop it" >&2
    return 4
  fi
  if ! is_running; then echo "not running"; rm -f "$STOPFILE"; return 0; fi
  # Resolve the pid ONCE, before the stop-file drops it, and only ever signal that exact pid.
  local stop_pid; stop_pid="$(port_pid)"
  echo "stopping (clean) ..."
  touch "$STOPFILE"
  if [[ -n "$stop_pid" ]]; then
    timeout 30 tail --pid="$stop_pid" -f /dev/null >/dev/null 2>&1 || true
  fi
  if ! is_running; then echo "stopped"; rm -f "$STOPFILE"; return 0; fi
  echo "clean stop timed out; killing pid $stop_pid" >&2
  [[ -n "$stop_pid" ]] && kill "$stop_pid" 2>/dev/null || true
  rm -f "$STOPFILE"
}

case "$CMD" in
  start)   do_start ;;
  stop)    do_stop ;;
  restart) do_stop; do_start ;;
  status)
    if is_running; then echo "running: port $PORT, this project (pid $(port_pid))"
    elif port_is_foreign; then echo "stopped -- but port $PORT is held by pid $(port_pid) from ANOTHER project"
    else echo "stopped"; fi ;;
  *) echo "usage: $0 {start|stop|status|restart} [--proj-dir DIR] [--proj-name NAME] [--port N] [--writable]" >&2; exit 2 ;;
esac
