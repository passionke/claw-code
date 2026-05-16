#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Local end-to-end check for GET /v1/biz_advice_report?stream=true (SSE).

Author: kejiqing

Prerequisites
-------------
1) Build gateway::

    cd rust && cargo build --release -p http-gateway-rs

2) Terminal A — mock OpenAI-compatible stream (slow chunks so deltas are spaced)::

    python3 scripts/verify_biz_report_stream.py serve-mock --port 18091

3) Terminal B — gateway (example; adjust pool/git env to your machine)::

    export CLAW_GATEWAY_DEV_BIZ_REPORT_SEED=1
    export OPENAI_API_KEY=test
    export OPENAI_BASE_URL=http://127.0.0.1:18091/v1
    export CLAW_DEFAULT_MODEL=openai/gpt-4o-mini
    export CLAW_HTTP_ADDR=127.0.0.1:18088
    export CLAW_WORK_ROOT=/tmp/claw-biz-stream-verify
    # … plus your usual CLAW_PROJECTS_GIT_* / pool vars from .env.example …
    ./rust/target/release/http-gateway-rs

4) Terminal C — seed + SSE + timings; optional:测试结束关掉监听网关端口的进程::

    python3 scripts/verify_biz_report_stream.py verify --gateway http://127.0.0.1:18088 --teardown

   ``--teardown`` 默认 kill ``--gateway`` URL 里的端口；也可用 ``--teardown-ports 18091,18092`` 指定 mock + 网关。

The script fails if fewer than two ``biz.report.delta`` events arrive or if all deltas
share the same receive timestamp (no progressive streaming).
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn
from typing import Any


class ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    """allow concurrent LLM streams (default HTTPServer is single-threaded)."""

    daemon_threads = True


def _post_json(url: str, payload: dict[str, Any], timeout: float = 60.0) -> dict[str, Any]:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read().decode("utf-8")
    return json.loads(body)


def _parse_sse_block(lines: list[str]) -> tuple[str | None, str | None]:
    event: str | None = None
    data_lines: list[str] = []
    for ln in lines:
        if ln.startswith("event:"):
            event = ln[len("event:") :].strip()
        elif ln.startswith("data:"):
            data_lines.append(ln[len("data:") :].lstrip())
    data = "\n".join(data_lines) if data_lines else None
    return event, data


def _stream_biz_report(gateway: str, task_id: str, timeout: float = 300.0) -> list[tuple[str, float, str | None]]:
    """Return list of (event_name, monotonic_time, raw_data_or_none)."""
    url = f"{gateway.rstrip('/')}/v1/biz_advice_report?task_id={task_id}&stream=true"
    req = urllib.request.Request(url, method="GET")
    out: list[tuple[str, float, str | None]] = []
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        if resp.status != 200:
            raise RuntimeError(f"unexpected status {resp.status}")
        buf: list[str] = []
        while True:
            raw = resp.readline()
            if not raw:
                break
            line = raw.decode("utf-8", errors="replace").rstrip("\r\n")
            if line == "":
                if not buf:
                    continue
                event, data = _parse_sse_block(buf)
                buf = []
                if event:
                    out.append((event, time.monotonic() - t0, data))
                continue
            buf.append(line)
    return out


def _pids_listening_on_tcp_port(port: int) -> list[int]:
    r = subprocess.run(
        ["lsof", "-tiTCP", str(port), "-sTCP:LISTEN"],
        capture_output=True,
        text=True,
        check=False,
    )
    if r.returncode != 0 or not r.stdout.strip():
        return []
    out: list[int] = []
    for x in r.stdout.split():
        if x.isdigit():
            out.append(int(x))
    return out


def _kill_listeners_on_port(port: int) -> list[int]:
    killed: list[int] = []
    for pid in _pids_listening_on_tcp_port(port):
        try:
            os.kill(pid, signal.SIGKILL)
            killed.append(pid)
        except ProcessLookupError:
            pass
    return killed


def _resolve_teardown_ports(args: argparse.Namespace) -> list[int]:
    raw = getattr(args, "teardown_ports", None)
    if raw:
        return [int(x.strip()) for x in raw.split(",") if x.strip().isdigit()]
    from urllib.parse import urlparse

    u = urlparse(args.gateway)
    if u.port is not None:
        return [u.port]
    sys.stderr.write(
        "teardown: gateway URL has no explicit port; pass e.g. --teardown-ports 18088\n",
    )
    return []


def cmd_verify(args: argparse.Namespace) -> int:
    try:
        gw = args.gateway.rstrip("/")
        seed_url = f"{gw}/v1/dev/biz_report_seed_task"
        try:
            seed = _post_json(
                seed_url,
                {
                    "dsId": args.ds_id,
                    "outputText": args.output_text,
                    "outputJson": {"mock": True, "note": "seed for stream verify"},
                },
            )
        except urllib.error.HTTPError as e:
            sys.stderr.write(f"seed HTTP {e.code}: {e.read().decode('utf-8', errors='replace')}\n")
            return 1
        task_id = seed.get("taskId")
        if not isinstance(task_id, str) or not task_id:
            sys.stderr.write(f"unexpected seed response: {seed!r}\n")
            return 1
        print(f"seeded taskId={task_id}")

        try:
            events = _stream_biz_report(gw, task_id, timeout=args.timeout)
        except urllib.error.HTTPError as e:
            sys.stderr.write(f"stream HTTP {e.code}: {e.read().decode('utf-8', errors='replace')}\n")
            return 1

        deltas = [ev for ev in events if ev[0] == "biz.report.delta"]
        print("--- SSE timeline (seconds since stream open) ---")
        for name, t, data in events:
            preview = ""
            if data and len(data) > 120:
                preview = data[:120] + "…"
            else:
                preview = data or ""
            print(f"  {t:8.3f}s  {name:18}  {preview}")

        if len(deltas) < 2:
            sys.stderr.write(f"expected >= 2 biz.report.delta, got {len(deltas)}\n")
            return 1

        times = [t for _, t, _ in deltas]
        spread = max(times) - min(times)
        if spread < args.min_delta_spread_s:
            sys.stderr.write(
                f"delta receive spread {spread:.4f}s < min {args.min_delta_spread_s}s "
                "(deltas may have arrived in one batch; check mock flush / gateway yield)\n"
            )
            return 1

        print(f"OK: {len(deltas)} deltas, time spread {spread:.3f}s (min required {args.min_delta_spread_s}s)")
        return 0
    finally:
        if getattr(args, "teardown", False):
            ports = _resolve_teardown_ports(args)
            for port in ports:
                killed = _kill_listeners_on_port(port)
                if killed:
                    print(f"teardown: SIGKILL listener PID(s) on port {port}: {killed}")


def cmd_serve_mock(args: argparse.Namespace) -> int:
    delay_s = args.chunk_delay_s
    pieces = args.chunks

    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args: Any) -> None:  # noqa: ANN401
            return

        def do_POST(self) -> None:  # noqa: N802
            n = int(self.headers.get("Content-Length", "0"))
            if n:
                self.rfile.read(n)
            if not self.path.rstrip("/").endswith("/chat/completions"):
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            self.end_headers()

            def write_chunk(obj: dict[str, Any]) -> None:
                payload = json.dumps(obj, ensure_ascii=False)
                self.wfile.write(f"data: {payload}\n\n".encode("utf-8"))
                self.wfile.flush()

            for p in pieces:
                write_chunk({"id": "mock", "model": "gpt-4o-mini", "choices": [{"delta": {"content": p}}]})
                time.sleep(delay_s)
            write_chunk({"id": "mock", "choices": [{"delta": {}, "finish_reason": "stop"}]})
            time.sleep(delay_s)
            write_chunk(
                {
                    "id": "mock",
                    "choices": [],
                    "usage": {"prompt_tokens": 1, "completion_tokens": len("".join(pieces))},
                }
            )
            time.sleep(delay_s)
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()

    host = args.bind
    port = args.port
    httpd = ThreadingHTTPServer((host, port), H)
    print(f"mock OpenAI SSE on http://{host}:{httpd.server_port}/v1/chat/completions")
    print("set: export OPENAI_BASE_URL=http://127.0.0.1:%s/v1" % httpd.server_port)
    print("     export OPENAI_API_KEY=test")
    print("     export CLAW_DEFAULT_MODEL=openai/gpt-4o-mini")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("stopped")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description="Biz report SSE local verification")
    sub = p.add_subparsers(dest="cmd", required=True)

    sm = sub.add_parser("serve-mock", help="slow OpenAI-compatible streaming mock")
    sm.add_argument("--bind", default="127.0.0.1")
    sm.add_argument("--port", type=int, default=18091)
    sm.add_argument("--chunk-delay-s", type=float, default=0.12, dest="chunk_delay_s")
    sm.add_argument(
        "--chunks",
        nargs="*",
        default=["【", "模拟", "润色", "】", "结论", "一", "；", "建议", "二", "。"],
        help="text fragments streamed as separate deltas",
    )
    sm.set_defaults(func=cmd_serve_mock)

    v = sub.add_parser("verify", help="POST dev seed + GET biz_advice_report stream=true")
    v.add_argument("--gateway", required=True, help="e.g. http://127.0.0.1:18088")
    v.add_argument("--ds-id", type=int, default=1, dest="ds_id")
    v.add_argument("--output-text", default="mock raw boss output for polish", dest="output_text")
    v.add_argument("--timeout", type=float, default=300.0)
    v.add_argument(
        "--min-delta-spread-s",
        type=float,
        default=0.15,
        dest="min_delta_spread_s",
        help="min (max-min) receive time among biz.report.delta events",
    )
    v.add_argument(
        "--teardown",
        action="store_true",
        help=(
            "after verify exits (any status), SIGKILL LISTEN processes on teardown port(s); "
            "default: explicit TCP port in --gateway URL; override with --teardown-ports"
        ),
    )
    v.add_argument(
        "--teardown-ports",
        default=None,
        metavar="PORTS",
        dest="teardown_ports",
        help="comma-separated listen ports to kill (e.g. 18091,18092); overrides gateway URL port",
    )
    v.set_defaults(func=cmd_verify)

    args = p.parse_args()
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
