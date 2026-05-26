#!/usr/bin/env python3
# Quick MCP probe for a given Bearer. Author: kejiqing
from __future__ import annotations

import os
import sys
from importlib.machinery import SourceFileLoader

m = SourceFileLoader("p", os.path.join(os.path.dirname(__file__), "probe-sqlbot-chat-id-parallel.py")).load_module()

AUTH = os.environ.get("SQLBOT_MCP_AUTH", "").strip()
if not AUTH:
    print("SQLBOT_MCP_AUTH unset", file=sys.stderr)
    sys.exit(1)
if not AUTH.lower().startswith("bearer "):
    AUTH = f"Bearer {AUTH}"

URLS = [
    "http://127.0.0.1:8001/mcp-streamable",
    "http://127.0.0.1:8000/mcp/mcp-streamable",
    "http://127.0.0.1:8000/mcp-streamable",
    "http://host.docker.internal:8001/mcp-streamable",
    "http://host.docker.internal:8000/mcp/mcp-streamable",
]


def test(url: str) -> str:
    try:
        c = m.McpClient(url, AUTH)
        c.initialize()
        r = c.call("tools/list", {})
        resp = r["response"]
        if resp.get("error"):
            return f"tools/list error: {resp['error']}"
        tools = (resp.get("result") or {}).get("tools") or []
        names = [t.get("name") for t in tools if isinstance(t, dict)]
        ms = c.tools_call("mcp_start", {})
        return (
            f"OK tools={len(names)} has_mcp_start={'mcp_start' in names} "
            f"mcp_start_ok={ms.ok} snippet={(ms.error or ms.snippet)[:100]!r}"
        )
    except Exception as e:
        return f"FAIL {type(e).__name__}: {e}"


label = os.environ.get("PROBE_LABEL", "probe")
print(f"=== {label} ===")
print(f"auth_prefix={AUTH[:40]}...")
for u in URLS:
    print(f"  {u}")
    print(f"    -> {test(u)}")
