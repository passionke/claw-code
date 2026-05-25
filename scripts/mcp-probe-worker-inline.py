#!/usr/bin/env python3
# Author: kejiqing
from importlib.machinery import SourceFileLoader

m = SourceFileLoader("p", "/probe.py").load_module()
AUTH = (
    "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9."
    "eyJhY2Nlc3Nfa2V5IjoiMEp1WlI2MWRXR0NROUpkcmpxbTJoUSIsImlhdCI6MTc3OTQyMjU3OX0."
    "M7jBy6UibAwOcWnk_L4zWwGtXKvOlUwYxEZzufWHJ1c"
)

URLS = [
    "http://host.docker.internal:8001/mcp-streamable",
    "http://host.docker.internal:8000/mcp/mcp-streamable",
    "http://host.docker.internal:8000/mcp-streamable",
    "http://192.168.9.252:8000/mcp/mcp-streamable",
    "http://host.containers.internal:8001/mcp-streamable",
    "http://127.0.0.1:8001/mcp-streamable",
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
            f"mcp_start_call_ok={ms.ok} snippet={ms.snippet[:80]!r}"
        )
    except Exception as e:
        return f"FAIL {type(e).__name__}: {e}"


print("=== from pool worker network namespace ===\n")
for u in URLS:
    print(u)
    print(f"  -> {test(u)}\n")
