#!/usr/bin/env python3
"""Streamable-HTTP twin of mock_mcp.py for e2b e2e: sandboxes reach it over the network.
Exposes `echo_meta`; appends every tools/call params (incl. `_meta`) to $MOCK_MCP_LOG.
Usage: MOCK_MCP_LOG=/tmp/mcp.ndjson mock_mcp_http.py [port]. Author: kejiqing
"""
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = os.environ.get("MOCK_MCP_LOG")
TOOLS = [{"name": "echo_meta", "description": "Echo the call _meta",
          "inputSchema": {"type": "object", "properties": {"note": {"type": "string"}}}}]


def handle(msg):
    method = msg.get("method")
    if method == "initialize":
        return {"protocolVersion": msg.get("params", {}).get("protocolVersion", "2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mock-mcp-http", "version": "0.1.0"}}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        params = msg.get("params", {})
        if LOG:
            with open(LOG, "a", encoding="utf-8") as fh:
                fh.write(json.dumps(params, ensure_ascii=False) + "\n")
        meta = params.get("_meta") or {}
        return {"content": [{"type": "text", "text": "meta_keys=" + ",".join(sorted(meta.keys()))}]}
    return {}


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        msg = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        sys.stderr.write(f"{self.client_address[0]} {msg.get('method')}\n")
        if msg.get("id") is None:
            self.send_response(202)
            self.end_headers()
            return
        body = json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": handle(msg)}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18799
    ThreadingHTTPServer(("0.0.0.0", port), Handler).serve_forever()
