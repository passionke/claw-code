#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Local static + reverse proxy for the gateway async playground (browser CORS bypass).

Stdlib Python only (no Rust, no pip deps). The Rust gateway is only the upstream HTTP
service under test. Author: kejiqing
"""
from __future__ import annotations

import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse

DIR = Path(__file__).resolve().parent
LISTEN_HOST = "127.0.0.1"
LISTEN_PORT = 18765

ALLOWED_HOSTNAMES = frozenset(
    {
        "127.0.0.1",
        "localhost",
        "192.168.9.252",
        "10.200.2.171",
    }
)
ALLOWED_PORTS = frozenset({18088})


def _norm_host(hostname: str | None) -> str | None:
    if hostname is None:
        return None
    h = hostname.lower().strip(".")
    if h == "::1":
        return "127.0.0.1"
    return h


def is_allowed_upstream(url: str) -> bool:
    try:
        p = urlparse(url)
    except ValueError:
        return False
    if p.scheme not in ("http", "https"):
        return False
    host = _norm_host(p.hostname)
    if host not in ALLOWED_HOSTNAMES:
        return False
    port = p.port or (443 if p.scheme == "https" else 80)
    return port in ALLOWED_PORTS


def read_allowed_json_body(handler: BaseHTTPRequestHandler, max_bytes: int = 2_000_000) -> dict | None:
    length = handler.headers.get("Content-Length")
    if not length:
        return None
    try:
        n = int(length)
    except ValueError:
        return None
    if n < 0 or n > max_bytes:
        return None
    raw = handler.rfile.read(n)
    try:
        return json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None


def send_json(handler: BaseHTTPRequestHandler, status: int, obj: dict) -> None:
    body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json; charset=utf-8")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        if path == "/__proxy_sse__":
            qs = urllib.parse.parse_qs(parsed.query)
            target = (qs.get("target") or [""])[0]
            if not target or not is_allowed_upstream(target):
                self.send_error(400, "bad or disallowed target")
                return
            try:
                req = urllib.request.Request(
                    target,
                    method="GET",
                    headers={"Accept": "text/event-stream"},
                )
                upstream = urllib.request.urlopen(req, timeout=600)
            except urllib.error.HTTPError as e:
                self.send_response(e.code)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.end_headers()
                self.wfile.write(e.read())
                return
            except urllib.error.URLError as e:
                self.send_error(502, str(e.reason if hasattr(e, "reason") else e))
                return

            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            self.send_header("X-Accel-Buffering", "no")
            self.end_headers()
            try:
                while True:
                    chunk = upstream.read(4096)
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    self.wfile.flush()
            finally:
                upstream.close()
            return

        if path in ("/admin", "/admin.html"):
            data = (DIR / "admin.html").read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return

        if path in ("/", "/index.html"):
            data = (DIR / "index.html").read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return

        self.send_error(404, "not found")

    def do_POST(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path != "/__proxy__":
            self.send_error(404, "not found")
            return

        payload = read_allowed_json_body(self)
        if not isinstance(payload, dict):
            send_json(self, 400, {"error": "invalid JSON body"})
            return

        base = str(payload.get("baseUrl") or "").strip().rstrip("/")
        method = str(payload.get("method") or "GET").upper()
        subpath = str(payload.get("path") or "")
        if not subpath.startswith("/"):
            send_json(self, 400, {"error": "path must start with /"})
            return

        url = base + subpath
        if not is_allowed_upstream(url):
            send_json(self, 400, {"error": "upstream host/port not allowed"})
            return

        body = payload.get("body")
        body_bytes: bytes | None
        if body is None:
            body_bytes = None
        elif isinstance(body, str):
            body_bytes = body.encode("utf-8")
        elif isinstance(body, dict):
            body_bytes = json.dumps(body, ensure_ascii=False).encode("utf-8")
        else:
            send_json(self, 400, {"error": "body must be string, object, or null"})
            return

        headers: dict[str, str] = {}
        raw_headers = payload.get("headers")
        if isinstance(raw_headers, dict):
            for k, v in raw_headers.items():
                if isinstance(k, str) and isinstance(v, str):
                    lk = k.lower()
                    if lk in ("host", "connection", "content-length"):
                        continue
                    headers[k] = v

        if body_bytes is not None and not any(
            k.lower() == "content-type" for k in headers
        ):
            headers["Content-Type"] = "application/json; charset=utf-8"

        try:
            req = urllib.request.Request(url, data=body_bytes, method=method, headers=headers)
            resp = urllib.request.urlopen(req, timeout=600)
        except urllib.error.HTTPError as e:
            raw = e.read()
            text = raw.decode("utf-8", errors="replace")
            send_json(
                self,
                e.code,
                {
                    "ok": False,
                    "status": e.code,
                    "headers": dict(e.headers.items()) if e.headers else {},
                    "bodyText": text,
                },
            )
            return
        except urllib.error.URLError as e:
            reason = getattr(e, "reason", e)
            send_json(self, 502, {"ok": False, "error": str(reason)})
            return

        try:
            out = resp.read()
            rh: dict[str, str] = {}
            if resp.headers:
                for k, v in resp.headers.items():
                    # Hop-by-hop headers stripped for JSON relay
                    lk = k.lower()
                    if lk in (
                        "transfer-encoding",
                        "connection",
                        "keep-alive",
                        "content-encoding",
                    ):
                        continue
                    rh[k] = v
            ct = resp.headers.get("Content-Type", "") if resp.headers else ""
            text = out.decode("utf-8", errors="replace")
            send_json(
                self,
                resp.status,
                {
                    "ok": 200 <= resp.status < 300,
                    "status": resp.status,
                    "headers": rh,
                    "bodyText": text,
                    "contentType": ct,
                },
            )
        finally:
            resp.close()


def main(argv: list[str]) -> int:
    host = LISTEN_HOST
    port = LISTEN_PORT
    if len(argv) >= 2:
        port = int(argv[1])
    httpd = ThreadingHTTPServer((host, port), Handler)
    print(f"gateway-async-playground: http://{host}:{port}/", flush=True)
    httpd.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
