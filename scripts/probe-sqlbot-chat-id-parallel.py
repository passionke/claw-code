#!/usr/bin/env python3
# Probe SQLBot: same chat_id parallel mcp_question_then_analysis. Author: kejiqing
"""Evidence script for SQLBot per-chat_id serialization."""

from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from typing import Any

try:
    import urllib.error
    import urllib.request
except ImportError:  # pragma: no cover
    raise

DEFAULT_URL = os.environ.get(
    "SQLBOT_MCP_URL", "http://127.0.0.1:8001/mcp-streamable"
)
DEFAULT_AUTH = os.environ.get(
    "SQLBOT_MCP_AUTH",
    "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhY2Nlc3Nfa2V5IjoiY1B3X0d6R0F4S29UNnJGRUZaZzd2ZyIsImlhdCI6MTc3ODMxNTkzOX0.0RaWCgQfIGtu-bjl5yRSRr1SNxeO8DmeBRH9x7RTqRQ",
)


@dataclass
class RpcResult:
    req_id: int
    method: str
    sent_ms: float
    recv_ms: float
    ok: bool
    error: str | None
    snippet: str


class McpClient:
    def __init__(self, base_url: str, auth: str) -> None:
        self.base_url = base_url.rstrip("/")
        self.auth = auth
        self.session_id: str | None = None
        self._lock = threading.Lock()
        self._next_id = 1

    def _post(self, payload: dict[str, Any]) -> tuple[dict[str, Any], dict[str, str]]:
        body = json.dumps(payload).encode("utf-8")
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
            "MCP-Protocol-Version": "2025-06-18",
            "Authorization": self.auth,
        }
        if self.session_id:
            headers["Mcp-Session-Id"] = self.session_id
        req = urllib.request.Request(
            self.base_url, data=body, headers=headers, method="POST"
        )
        with urllib.request.urlopen(req, timeout=600) as resp:
            hdrs = {k.lower(): v for k, v in resp.headers.items()}
            if "mcp-session-id" in hdrs:
                self.session_id = hdrs["mcp-session-id"]
            raw = resp.read().decode("utf-8", errors="replace")
        content_type = hdrs.get("content-type", "")
        if "text/event-stream" in content_type or raw.startswith("event:"):
            for line in raw.splitlines():
                if line.startswith("data:"):
                    data = line[5:].strip()
                    if data:
                        return json.loads(data), hdrs
            raise RuntimeError(f"SSE without data line: {raw[:200]!r}")
        if not raw.strip():
            raise RuntimeError("empty MCP response body")
        return json.loads(raw), hdrs

    def call(self, method: str, params: Any = None) -> dict[str, Any]:
        with self._lock:
            req_id = self._next_id
            self._next_id += 1
        payload: dict[str, Any] = {
            "jsonrpc": "2.0",
            "id": req_id,
            "method": method,
        }
        if params is not None:
            payload["params"] = params
        sent = time.time()
        resp, _ = self._post(payload)
        recv = time.time()
        return {"id": req_id, "sent": sent, "recv": recv, "response": resp}

    def initialize(self) -> None:
        r = self.call(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "probe-sqlbot-parallel", "version": "0.1"},
            },
        )
        if r["response"].get("error"):
            raise RuntimeError(f"initialize failed: {r['response']['error']}")
        # notification
        notif = {"jsonrpc": "2.0", "method": "notifications/initialized"}
        body = json.dumps(notif).encode("utf-8")
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
            "MCP-Protocol-Version": "2025-06-18",
            "Authorization": self.auth,
        }
        if self.session_id:
            headers["Mcp-Session-Id"] = self.session_id
        req = urllib.request.Request(
            self.base_url, data=body, headers=headers, method="POST"
        )
        with urllib.request.urlopen(req, timeout=30):
            pass

    def tools_call(self, name: str, arguments: dict[str, Any]) -> RpcResult:
        r = self.call(
            "tools/call",
            {"name": name, "arguments": arguments},
        )
        resp = r["response"]
        err = resp.get("error")
        result = resp.get("result") or {}
        text = ""
        for block in result.get("content") or []:
            if isinstance(block, dict) and block.get("type") == "text":
                text += str(block.get("text", ""))
        snippet = text[:240].replace("\n", " ")
        return RpcResult(
            req_id=r["id"],
            method=name,
            sent_ms=r["sent"] * 1000,
            recv_ms=r["recv"] * 1000,
            ok=err is None and not result.get("isError"),
            error=json.dumps(err) if err else None,
            snippet=snippet,
        )


def mcp_start(client: McpClient) -> tuple[str, int]:
    full = client.call("tools/call", {"name": "mcp_start", "arguments": {}})
    resp = full["response"]
    if resp.get("error"):
        raise RuntimeError(f"mcp_start failed: {resp['error']}")
    result = resp.get("result") or {}
    text = ""
    for block in result.get("content") or []:
        if isinstance(block, dict) and block.get("type") == "text":
            text += str(block.get("text", ""))
    inner = json.loads(text)
    data = inner.get("data") or inner
    token = data["access_token"]
    chat_id = int(data["chat_id"])
    return token, chat_id


def run_parallel_probe(
    base_url: str,
    auth: str,
    n: int,
    questions: list[str],
) -> list[RpcResult]:
    # One shared MCP session + same chat_id; separate HTTP threads (like Claw concurrent calls).
    shared = McpClient(base_url, auth)
    shared.initialize()
    token, chat_id = mcp_start(shared)
    print(f"mcp_start ok chat_id={chat_id} token_prefix={token[:12]}...", flush=True)

    results: list[RpcResult] = []
    barrier = threading.Barrier(n)

    def one(i: int) -> RpcResult:
        # Each worker uses its own MCP HTTP session but same chat_id in args.
        c = McpClient(base_url, auth)
        c.initialize()
        barrier.wait(timeout=30)
        t0 = time.time()
        q = questions[i % len(questions)]
        r = c.tools_call(
            "mcp_question_then_analysis",
            {"token": token, "chat_id": chat_id, "question": q},
        )
        t1 = time.time()
        print(
            f"  worker={i} sent={r.sent_ms:.0f} recv={r.recv_ms:.0f} "
            f"wall={(t1-t0)*1000:.0f}ms ok={r.ok} q={q[:40]!r}",
            flush=True,
        )
        return r

    t_launch = time.time() * 1000
    with ThreadPoolExecutor(max_workers=n) as pool:
        futs = [pool.submit(one, i) for i in range(n)]
        for f in as_completed(futs):
            results.append(f.result())
    t_done = time.time() * 1000
    print(f"batch wall={t_done - t_launch:.0f}ms n={n}", flush=True)
    return results


def run_distinct_chat_probe(
    base_url: str,
    auth: str,
    n: int,
    questions: list[str],
) -> list[RpcResult]:
    """Control: each worker gets its own chat_id via mcp_start."""
    results: list[RpcResult] = []
    barrier = threading.Barrier(n)

    def one(i: int) -> RpcResult:
        c = McpClient(base_url, auth)
        c.initialize()
        token, chat_id = mcp_start(c)
        barrier.wait(timeout=30)
        q = questions[i % len(questions)]
        print(f"  worker={i} distinct chat_id={chat_id}", flush=True)
        return c.tools_call(
            "mcp_question_then_analysis",
            {"token": token, "chat_id": chat_id, "question": q},
        )

    t0 = time.time() * 1000
    with ThreadPoolExecutor(max_workers=n) as pool:
        futs = [pool.submit(one, i) for i in range(n)]
        for f in as_completed(futs):
            r = f.result()
            print(
                f"  done req={r.req_id} dur={r.recv_ms-r.sent_ms:.0f}ms ok={r.ok}",
                flush=True,
            )
            results.append(r)
    print(f"distinct_chat batch wall={time.time()*1000 - t0:.0f}ms n={n}", flush=True)
    return results


def main() -> int:
    p = argparse.ArgumentParser(description="Probe SQLBot same chat_id parallel queries")
    p.add_argument("--url", default=DEFAULT_URL)
    p.add_argument("--auth", default=DEFAULT_AUTH)
    p.add_argument(
        "--mode",
        choices=("same_chat", "distinct_chat"),
        default="same_chat",
        help="same_chat: one mcp_start, N parallel analysis on same chat_id; "
        "distinct_chat: N separate mcp_start + analysis (control)",
    )
    p.add_argument(
        "--questions",
        nargs="*",
        default=[
            "查询门店 S20241007172800004204 昨日订单笔数",
            "查询门店 S20241007172800004204 昨日实收金额",
            "查询门店 S20241007172800004204 昨日退款笔数",
            "查询门店 S20241007172800004204 昨日客单价",
        ],
    )
    args = p.parse_args()
    try:
        if args.mode == "distinct_chat":
            results = run_distinct_chat_probe(args.url, args.auth, args.n, args.questions)
        else:
            results = run_parallel_probe(args.url, args.auth, args.n, args.questions)
    except Exception as e:
        print(f"PROBE FAILED: {e}", file=sys.stderr)
        return 1

    sent = sorted(r.sent_ms for r in results)
    recv = sorted(r.recv_ms for r in results)
    overlap = recv[-1] - sent[0]
    serial_sum = sum(r.recv_ms - r.sent_ms for r in results)
    print("\n=== summary ===")
    print(f"first_sent_ms={sent[0]:.0f} last_recv_ms={recv[-1]:.0f} span={overlap:.0f}ms")
    print(f"sum_individual_durations_ms={serial_sum:.0f}")
    print(f"avg_duration_ms={serial_sum/len(results):.0f}")
    if overlap < serial_sum * 0.7:
        print("VERDICT: requests OVERLAP on server timeline → not strict serial lock")
    else:
        print("VERDICT: span ≈ sum of durations → likely SERIAL per chat_id (or single worker)")
    ok_count = sum(1 for r in results if r.ok)
    print(f"ok={ok_count}/{len(results)}")
    return 0 if ok_count == len(results) else 2


if __name__ == "__main__":
    raise SystemExit(main())
