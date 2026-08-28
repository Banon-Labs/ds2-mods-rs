#!/usr/bin/env python3
"""Minimal TCP client for the 13bm Ghidra MCP daemon (ghidra.mcp.MCPServer).

The daemon (scripts/ghidra/mcp-daemon.sh) keeps the DS2 program warm and answers MCP tool
calls over a TCP socket on 127.0.0.1:8766. This client speaks that raw wire protocol directly,
with no Go bridge and no MCP tool surface involved:

  wire: 4-byte big-endian length prefix + UTF-8 JSON
  json: JSON-RPC {"id": "<n>", "method": "<cmd>", "params": {...}}

Use it when the harness does not expose the ghidra MCP tools but the daemon is up -- which is
always the case in the session that FIRST starts the daemon, since MCP servers are attached at
session start. It is also the honest way to verify the daemon actually serves decompiled C
before asking anyone to restart a session for it.
Common methods (see MCPClientHandler.java dispatch): decompileFunctionByName,
getDecompiledCode, disassembleFunction, getFunctionByAddress, getXrefsTo, getXrefsFrom,
getFunctionXrefs, searchFunctionsByName, getStructure, listStructures, ping.

CLI: python3 scripts/ghidra/mcp_query.py <method> '<json-params>'
"""
import json
import socket
import struct
import sys

#: 8766 IS THE DS2 DAEMON. 8765 IS ELDEN RING'S, and it is usually up on this machine too.
#: This default was 8765 -- inherited from the er-mods-rs original -- so every query that did not
#: pass an explicit port was answered by ELDEN RING while being read as DS2. Measured, not feared:
#: `getFunctionByAddress 0x1402e6fc1` returns `IsArrivedPathNotNew(AiPathData *, float)` on 8765
#: and `FUN_1402e6e60` on 8766. Both daemons answer `ping` with "pong", so liveness proves
#: nothing about WHICH GAME is answering -- see the "Port 8766, not 8765" trap in
#: `docs/GHIDRA-MCP.md`, which this line was contradicting.
HOST, PORT = "127.0.0.1", 8766


def call(method, params=None, timeout=25.0, host=HOST, port=PORT):
    s = socket.create_connection((host, port), timeout=timeout)
    s.settimeout(timeout)
    try:
        req = json.dumps({"id": "1", "method": method, "params": params or {}}).encode("utf-8")
        s.sendall(struct.pack(">i", len(req)) + req)
        hdr = b""
        while len(hdr) < 4:
            chunk = s.recv(4 - len(hdr))
            if not chunk:
                raise EOFError("closed before length")
            hdr += chunk
        (n,) = struct.unpack(">i", hdr)
        buf = b""
        while len(buf) < n:
            chunk = s.recv(min(65536, n - len(buf)))
            if not chunk:
                break
            buf += chunk
        return json.loads(buf.decode("utf-8", "replace"))
    finally:
        s.close()


def main(argv):
    if len(argv) < 2:
        print("usage: mcp_tcp_query.py <method> '<json-params>'", file=sys.stderr)
        return 2
    method = argv[1]
    params = json.loads(argv[2]) if len(argv) > 2 else {}
    r = call(method, params)
    if isinstance(r, dict) and "result" in r:
        res = r["result"]
        print(res if isinstance(res, str) else json.dumps(res, indent=2))
    else:
        print(json.dumps(r, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
