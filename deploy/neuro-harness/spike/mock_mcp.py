#!/usr/bin/env python3
"""Minimal stdio MCP server exposing `echo_meta`; appends every tools/call params (incl. `_meta`)
to $MOCK_MCP_LOG so spikes can prove `_meta` reached the real MCP server. Author: kejiqing
"""
import json
import os
import sys

LOG = os.environ.get("MOCK_MCP_LOG")


def reply(msg_id, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg_id, "result": result}) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method, msg_id = msg.get("method"), msg.get("id")
    if method == "initialize":
        reply(msg_id, {"protocolVersion": msg.get("params", {}).get("protocolVersion", "2025-06-18"),
                       "capabilities": {"tools": {}},
                       "serverInfo": {"name": "mock-mcp", "version": "0.1.0"}})
    elif method == "tools/list":
        reply(msg_id, {"tools": [{"name": "echo_meta", "description": "Echo the call _meta",
                                  "inputSchema": {"type": "object",
                                                  "properties": {"note": {"type": "string"}}}}]})
    elif method == "tools/call":
        params = msg.get("params", {})
        if LOG:
            with open(LOG, "a", encoding="utf-8") as fh:
                fh.write(json.dumps(params, ensure_ascii=False) + "\n")
        meta = params.get("_meta") or {}
        reply(msg_id, {"content": [{"type": "text",
                                    "text": "meta_keys=" + ",".join(sorted(meta.keys()))}]})
    elif msg_id is not None:
        reply(msg_id, {})
