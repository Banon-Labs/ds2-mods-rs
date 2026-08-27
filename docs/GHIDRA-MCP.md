# The Ghidra MCP daemon

One warm Ghidra process holding the DS2 program, serving every agent that connects.
Tracking issue: `ds2-mods-rs-bc2`.

```bash
bash scripts/ghidra/mcp-daemon.sh start     # read-only, port 8766
bash scripts/ghidra/mcp-daemon.sh status
bash scripts/ghidra/mcp-daemon.sh stop
```

## Why, precisely

Every `scripts/ghidra/query.sh` run spawns its own `analyzeHeadless` and takes an **exclusive**
lock on the project, so two agents cannot use Ghidra at the same time at all. That is not a
theoretical cost: during `ds2-mods-rs-3rr` a background survey agent held the project, and the
title-flow trace was done with `objdump` and two new scripts instead.

The daemon fixes that by inverting it. One process takes the lock and serves a TCP port; each
MCP client spawns its own Go bridge and connects. N agents → N bridges → 1 server → 1 lock.
Queries serialise inside the daemon at millisecond scale, which is concurrent as far as any
caller is concerned.

The second benefit is the decompiler. `getDecompiledCode` returns C with the project's curated
types attached -- `TitleFlowContext`, `ShowProgressJob`, `TitleStepLogInId` -- names that no
amount of `objdump` recovers.

## Read-only by default

This differs deliberately from the er-mods-rs original, which defaults to writable with a 60s
autosave. MCP edits (rename, struct, comment, bookmark) persist into the project, and several
agents mutating one program have no isolation and no merge: last writer wins, silently.
`--writable` exists and is a deliberate single-agent act.

The cost is real and worth stating: **a read-only daemon cannot define new functions**, which
matters more here than it sounds (see below).

## Which Ghidra, and why there are two projects

| | project | Ghidra | used by |
| --- | --- | --- | --- |
| analysis queries | `~/.cache/ds2-ghidra` | 11.3.1 | `query.sh` |
| MCP daemon | `~/ghidra-projects/ds2-121` | 12.1 | `mcp-daemon.sh` |

Ghidra extensions are version-pinned and GhidraMCP-13bm is installed only under 12.1/12.1.2, so
the daemon cannot serve the 11.3.1 project. The 12.1 project is a forward import of the same
`.gzf`; the analysis survives intact, verified against the recorded numbers:

| | 11.3.1 | 12.1 |
| --- | --- | --- |
| functions | 88780 | 88780 |
| imageBase | `0x140000000` | `0x140000000` |
| `.bind` | `0x141d43000` | `0x141d43000` |

Keeping them separate is not redundancy for its own sake: a direct headless tool cannot open a
project the daemon holds, so this is what lets `query.sh` keep working while the daemon is up,
and what stops a downed daemon from meaning no Ghidra at all.

## Traps

**Port 8766, not 8765.** The er-mods-rs daemon uses 8765. When this script was first written it
inherited that port *and* er's `pgrep -f MCPServeHeadless.java` liveness check, so it reported
`already running; port 8765 up` and never started -- while `mcp_query.py` happily answered from
an **ELDEN RING** program. A decompile would have landed Elden Ring code under a DS2 heading.
The daemon is now identified by the pid listening on its own port plus a cmdline check against
its staged script directory, `start` refuses a foreign port holder, and `stop` will not signal a
pid it does not own. If you ever doubt which program answers, ask it:

```bash
python3 scripts/ghidra/mcp_query.py searchFunctionsByName '{"query":"FeSubStateTitleLogo"}'
```

Zero hits from a DS2 daemon means it is not a DS2 daemon.

**Ghidra 12.1 rejects dot-directories in project paths** (`Path element starting with '.' is not
permitted`), which 11.3.1 allowed. That is why the 12.1 project is not under `~/.cache`.

**One OSGi bundle per `-scriptPath` directory.** A compile error in any sibling `.java` fails the
whole bundle and the daemon script never loads, reporting only `Failed to get OSGi bundle
containing script`. `scripts/ghidra/rt/` holds the query scripts, so `mcp-daemon.sh` stages
`MCPServeHeadless.java` alone into a clean directory.

**Lost `+x` on native helpers looks like a decompiler bug.** An install copied off a
Windows/drvfs mount loses the execute bit on `decompile`, `sleigh` and the demanglers. The Java
side still works, so disassembly, xrefs and symbol queries all answer normally while *every*
decompile returns `Decompilation failed`. `mcp-daemon.sh` re-applies the bits on every `start`;
because the decompile process is spawned per request, that fixes it without a restart.

## Not many Fe* virtuals are defined functions

`0x1400febf0` is `FeSubStateTitleLogo`'s update, reached only through a vtable slot, and Ghidra's
analysis never created a function there -- `getFunctionByAddress` returns "No function found".
Vtable-only targets are common in this binary, and the decompiler cannot be pointed at code that
is not in a function.

So the daemon does **not** retire `objdump` or `scripts/ds2-rtti.py` for this class of work. It
answers beautifully where functions exist and says nothing where they do not. Defining functions
at vtable targets is a mutation, which a read-only daemon will not do -- that is a deliberate,
scoped, `--writable` job for one agent, not something to enable by default.

## MCP client wiring

`.mcp.json` points at `${HOME}/tools/GhidraMCP-13bm/mcp-bridge/mcp_bridge` on port 8766. MCP
servers attach at session start, so a session that starts the daemon cannot use the MCP tools in
that same session -- use `scripts/ghidra/mcp_query.py`, which speaks the daemon's wire protocol
directly and needs no bridge.

(For reference, er-mods-rs's own `.mcp.json` points at `~/projects/ghidra-mcp-13bm/...`, a path
that no longer exists. The binary lives under `~/tools/`.)
