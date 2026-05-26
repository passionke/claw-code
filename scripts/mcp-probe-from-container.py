#!/usr/bin/env python3
# Probe SQLBot MCP URLs from inside pool worker network. Author: kejiqing
"""Usage: podman run --rm --network container:<worker> -v $PWD/scripts/mcp-probe-from-container.py:/probe.py:ro python:3.12-slim python3 /probe.py"""

from __future__ import annotations

import json
import socket
import urllib.error
import urllib.request

AUTH = (
    "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9."
    "eyJhY2Nlc3Nfa2V5IjoiMEp1WlI2MWRXR0NROUpkcmpxbTJoUSIsImlhdCI6MTc3OTQyMjU3OX0."
    "M7jBy6UibAwOcWnk_L4zWwGtXKvOlUwYxEZzufWHJ1c"
)

URLS = [
    "http://host.docker.internal:8000/mcp/mcp-streamable",
    "http://host.docker.internal:8001/mcp-streamable",
    "http://host.docker.internal:8000/mcp-streamable",
    "http://host.containers.internal:8001/mcp-streamable",
    "http://192.168.9.252:8000/mcp/mcp-streamable",
    "http://192.168.9.252:8001/mcp-streamable",
    "http://127.0.0.1:8000/mcp/mcp-streamable",
    "http://127.0.0.1:8001/mcp-streamable",
]


def tcp(host: str, port: int, timeout: float = 2.0) -> str:
    try:
        s = socket.create_connection((host, port), timeout)
        s.close()
        return "open"
    except OSError as e:
        return f"closed ({e})"


def parse_body(raw: str, content_type: str) -> dict:
    if "text/event-stream" in content_type or raw.startswith("event:"):
        for line in raw.splitlines():
            if line.startswith("data:"):
                raw = line[5:].strip()
                break
    if not raw.strip():
        return {}
    return json.loads(raw)


def mcp_post(url: str, method: str, params: dict | None, session_id: str | None) -> tuple[dict, str | None]:
    payload: dict = {"jsonrpc": "2.0", "id": 1, "method": method}
    if params is not None:
        payload["params"] = params
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-06-18",
        "Authorization": AUTH,
    }
    if session_id:
        headers["Mcp-Session-Id"] = session_id
    req = urllib.request.Request(
        url, data=json.dumps(payload).encode(), headers=headers, method="POST"
    )
    with urllib.request.urlopen(req, timeout=12) as resp:
        raw = resp.read(12000).decode("utf-8", errors="replace")
        sid = resp.headers.get("Mcp-Session-Id") or resp.headers.get("mcp-session-id")
        ct = resp.headers.get("content-type", "")
        return parse_body(raw, ct), sid


def mcp_probe(url: str) -> str:
    try:
        init, sid = mcp_post(
            url,
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "probe", "version": "0"},
            },
            None,
        )
        if init.get("error"):
            return f"initialize error: {init['error']}"
        listed, _ = mcp_post(url, "tools/list", {}, sid)
        if listed.get("error"):
            return f"tools/list error: {listed['error']}"
        tools = (listed.get("result") or {}).get("tools") or []
        names = [t.get("name") for t in tools if isinstance(t, dict)]
        return f"OK tools={len(names)} mcp_start={'mcp_start' in names} sample={names[:4]}"
    except urllib.error.HTTPError as e:
        body = e.read(200).decode("utf-8", errors="replace")
        return f"HTTP {e.code} {e.reason} body={body[:80]!r}"
    except Exception as e:
        return f"{type(e).__name__}: {e}"


def main() -> None:
    print("=== DNS (inside worker netns) ===")
    for h in ["host.docker.internal", "host.containers.internal", "192.168.9.252"]:
        try:
            print(f"  {h} -> {socket.gethostbyname(h)}")
        except OSError as e:
            print(f"  {h} -> FAIL {e}")

    print("\n=== TCP (host.docker.internal) ===")
    for p in [8000, 8001, 8088]:
        print(f"  :{p} {tcp('host.docker.internal', p)}")

    print("\n=== MCP POST initialize + tools/list ===")
    for url in URLS:
        print(f"  {url}")
        print(f"    -> {mcp_probe(url)}")


if __name__ == "__main__":
    main()
