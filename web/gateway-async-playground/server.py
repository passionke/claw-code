#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Static UI + reverse proxy for gateway async playground (browser CORS bypass).

Stdlib Python only (no Rust, no pip deps). Author: kejiqing
"""
from __future__ import annotations

import hashlib
import hmac
import json
import os
import re
import secrets
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from http import cookies
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse

DIR = Path(__file__).resolve().parent
# Vite build output (committed in repo). Override in compose image via PLAYGROUND_ADMIN_DIST.
_admin_dist_env = os.environ.get("PLAYGROUND_ADMIN_DIST", "").strip()
ADMIN_DIST = (
    Path(_admin_dist_env)
    if _admin_dist_env
    else DIR.parent / "gateway-admin" / "dist"
)
ADMIN_INDEX = ADMIN_DIST / "index.html"

_ADMIN_MIME = {
    ".html": "text/html; charset=utf-8",
    ".js": "application/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".json": "application/json; charset=utf-8",
    ".svg": "image/svg+xml",
    ".png": "image/png",
    ".ico": "image/x-icon",
    ".woff2": "font/woff2",
    ".woff": "font/woff",
}

LISTEN_HOST = os.environ.get("PLAYGROUND_LISTEN_HOST", "127.0.0.1")
LISTEN_PORT = int(os.environ.get("PLAYGROUND_LISTEN_PORT", "18765"))

ADMIN_USER = os.environ.get("PLAYGROUND_ADMIN_USER", "admin").strip()
ADMIN_PASSWORD = os.environ.get("PLAYGROUND_ADMIN_PASSWORD", "sunmi123")

SESSION_COOKIE = "claw_pg_admin"
SESSION_TTL_SEC = int(os.environ.get("PLAYGROUND_ADMIN_SESSION_TTL_SEC", str(7 * 86400)))

_DEFAULT_HOSTS = "127.0.0.1,localhost,192.168.9.252,10.200.2.171,gateway-rs"
_DEFAULT_PORTS = "18088,8080,8088"


def _norm_host(hostname: str | None) -> str | None:
    if hostname is None:
        return None
    h = hostname.lower().strip(".")
    if h == "::1":
        return "127.0.0.1"
    return h


def _parse_allowed_hosts() -> frozenset[str]:
    raw = os.environ.get("PLAYGROUND_ALLOWED_HOSTS", _DEFAULT_HOSTS)
    out = set()
    for part in raw.split(","):
        h = _norm_host(part.strip())
        if h:
            out.add(h)
    return frozenset(out)


def _parse_allowed_ports() -> frozenset[int]:
    raw = os.environ.get("PLAYGROUND_ALLOWED_PORTS", _DEFAULT_PORTS)
    out = set()
    for part in raw.split(","):
        part = part.strip()
        if not part:
            continue
        try:
            out.add(int(part))
        except ValueError:
            continue
    return frozenset(out) if out else frozenset({18088})


ALLOWED_HOSTNAMES = _parse_allowed_hosts()
ALLOWED_PORTS = _parse_allowed_ports()


def _resolve_gateway_base_url(raw: str) -> str:
    """Expand compose-style ${GATEWAY_HOST_PORT:-N} in URLs; return normalized base without trailing slash."""
    s = raw.strip().rstrip("/")
    if not s:
        return ""
    if "${" in s:
        port = os.environ.get("GATEWAY_HOST_PORT", "18088").strip() or "18088"
        try:
            port = str(int(port))
        except ValueError:
            port = "18088"
        s = re.sub(r"\$\{GATEWAY_HOST_PORT[^}]*\}", port, s)
    if s.startswith("http://") or s.startswith("https://"):
        return s
    return ""


def _gateway_preset_label(url: str) -> str:
    try:
        p = urlparse(url)
    except ValueError:
        return url
    host = p.hostname or ""
    port = p.port or (443 if p.scheme == "https" else 80)
    if host in ("127.0.0.1", "localhost", "::1"):
        return f"本机 :{port}"
    return f"{host}:{port}"


PUBLIC_GATEWAY_BASE = _resolve_gateway_base_url(
    os.environ.get("PLAYGROUND_PUBLIC_GATEWAY_BASE", "").strip()
    or os.environ.get("PLAYGROUND_GATEWAY_BASE", "").strip()
) or "http://127.0.0.1:18088"

# In compose, browser uses host-mapped URL; playground process must dial gateway-rs:8080.
UPSTREAM_GATEWAY_BASE = _resolve_gateway_base_url(
    os.environ.get("PLAYGROUND_GATEWAY_BASE", "").strip()
)


def _effective_proxy_base(browser_base: str) -> str:
    """Map UI `baseUrl` to an address reachable from this process (container vs host). Author: kejiqing"""
    b = _resolve_gateway_base_url(browser_base) or browser_base.strip().rstrip("/")
    if (
        UPSTREAM_GATEWAY_BASE
        and UPSTREAM_GATEWAY_BASE != b
        and b == PUBLIC_GATEWAY_BASE
    ):
        return UPSTREAM_GATEWAY_BASE
    return b


def _effective_proxy_url(browser_base: str, subpath: str) -> str:
    base = _effective_proxy_base(browser_base).rstrip("/")
    path = subpath if subpath.startswith("/") else "/" + subpath
    return base + path


def _session_secret() -> bytes:
    raw = os.environ.get("PLAYGROUND_ADMIN_SESSION_SECRET", "").strip()
    if raw:
        return raw.encode("utf-8")
    return (ADMIN_PASSWORD + "|" + ADMIN_USER + "|claw-playground").encode("utf-8")


def _safe_admin_next(path: str | None) -> str:
    if path and path.startswith("/admin") and "://" not in path:
        return path
    return "/admin"


def make_session_token(user: str) -> str:
    exp = int(time.time()) + SESSION_TTL_SEC
    payload = f"{user}:{exp}"
    sig = hmac.new(_session_secret(), payload.encode("utf-8"), hashlib.sha256).hexdigest()
    return f"{payload}:{sig}"


def verify_session_token(token: str | None) -> str | None:
    if not token or ":" not in token:
        return None
    try:
        user, exp_s, sig = token.rsplit(":", 2)
        exp = int(exp_s)
    except ValueError:
        return None
    if exp < int(time.time()):
        return None
    payload = f"{user}:{exp}"
    expect = hmac.new(_session_secret(), payload.encode("utf-8"), hashlib.sha256).hexdigest()
    if not secrets.compare_digest(expect, sig):
        return None
    if user != ADMIN_USER:
        return None
    return user


def check_admin_credentials(user: str, password: str) -> bool:
    if user != ADMIN_USER:
        return False
    return secrets.compare_digest(password, ADMIN_PASSWORD)


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


def send_html_bytes(handler: BaseHTTPRequestHandler, status: int, data: bytes) -> None:
    handler.send_response(status)
    handler.send_header("Content-Type", "text/html; charset=utf-8")
    handler.send_header("Content-Length", str(len(data)))
    handler.end_headers()
    handler.wfile.write(data)


def send_static_bytes(
    handler: BaseHTTPRequestHandler, status: int, data: bytes, content_type: str
) -> None:
    handler.send_response(status)
    handler.send_header("Content-Type", content_type)
    handler.send_header("Content-Length", str(len(data)))
    handler.end_headers()
    handler.wfile.write(data)


def _admin_dist_safe_path(rel: str) -> Path | None:
    """Resolve `rel` under ADMIN_DIST; reject path traversal."""
    rel = rel.lstrip("/")
    if rel == "":
        return ADMIN_INDEX if ADMIN_INDEX.is_file() else None
    target = (ADMIN_DIST / rel).resolve()
    root = ADMIN_DIST.resolve()
    try:
        target.relative_to(root)
    except ValueError:
        return None
    return target if target.is_file() else None


def serve_admin_dist(handler: BaseHTTPRequestHandler, subpath: str) -> bool:
    """Serve Vite SPA: static assets or index.html fallback."""
    if not ADMIN_INDEX.is_file():
        send_html_bytes(
            handler,
            503,
            (
                "<h1>admin dist missing</h1>"
                "<p>Run <code>cd web/gateway-admin && npm ci && npm run build</code> "
                "and commit <code>dist/</code>.</p>"
            ).encode("utf-8"),
        )
        return True
    hit = _admin_dist_safe_path(subpath)
    if hit is not None:
        mime = _ADMIN_MIME.get(hit.suffix.lower(), "application/octet-stream")
        send_static_bytes(handler, 200, hit.read_bytes(), mime)
        return True
    send_html_bytes(handler, 200, ADMIN_INDEX.read_bytes())
    return True


def send_redirect(handler: BaseHTTPRequestHandler, location: str, status: int = 302) -> None:
    handler.send_response(status)
    handler.send_header("Location", location)
    handler.send_header("Content-Length", "0")
    handler.end_headers()


def set_session_cookie(handler: BaseHTTPRequestHandler, token: str) -> None:
    c = cookies.SimpleCookie()
    c[SESSION_COOKIE] = token
    c[SESSION_COOKIE]["path"] = "/"
    c[SESSION_COOKIE]["httponly"] = True
    c[SESSION_COOKIE]["samesite"] = "Lax"
    c[SESSION_COOKIE]["max-age"] = str(SESSION_TTL_SEC)
    for morsel in c.values():
        handler.send_header("Set-Cookie", morsel.OutputString())


def clear_session_cookie(handler: BaseHTTPRequestHandler) -> None:
    c = cookies.SimpleCookie()
    c[SESSION_COOKIE] = ""
    c[SESSION_COOKIE]["path"] = "/"
    c[SESSION_COOKIE]["httponly"] = True
    c[SESSION_COOKIE]["max-age"] = "0"
    for morsel in c.values():
        handler.send_header("Set-Cookie", morsel.OutputString())


def read_session_user(handler: BaseHTTPRequestHandler) -> str | None:
    raw = handler.headers.get("Cookie", "")
    jar = cookies.SimpleCookie()
    jar.load(raw)
    if SESSION_COOKIE not in jar:
        return None
    return verify_session_token(jar[SESSION_COOKIE].value)


def playground_config() -> dict:
    """One default gateway (browser → host port). Optional extras via PLAYGROUND_EXTRA_GATEWAY_BASES."""
    presets: list[dict[str, str]] = []
    seen = {PUBLIC_GATEWAY_BASE}
    extra = os.environ.get("PLAYGROUND_EXTRA_GATEWAY_BASES", "").strip()
    for part in extra.split(","):
        url = _resolve_gateway_base_url(part)
        if not url or url in seen:
            continue
        seen.add(url)
        presets.append({"label": _gateway_preset_label(url), "value": url})
    return {
        "listenHost": LISTEN_HOST,
        "listenPort": LISTEN_PORT,
        "defaultGatewayBase": PUBLIC_GATEWAY_BASE,
        "defaultGatewayLabel": _gateway_preset_label(PUBLIC_GATEWAY_BASE),
        "gatewayPresets": presets,
        "adminLoginRequired": True,
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        qs = urllib.parse.parse_qs(parsed.query)

        if path == "/__config__":
            send_json(self, 200, playground_config())
            return

        if path == "/__admin_me__":
            user = read_session_user(self)
            if user:
                send_json(self, 200, {"ok": True, "user": user})
            else:
                send_json(self, 401, {"ok": False, "error": "not logged in"})
            return

        if path == "/admin/login.html":
            send_redirect(self, "/admin/login")
            return

        if path == "/admin.html":
            send_redirect(self, "/admin")
            return

        if path in ("/admin", "/admin/login") or path.startswith("/admin/"):
            if path not in ("/admin/login",) and not read_session_user(self):
                nxt = urllib.parse.quote(path, safe="")
                send_redirect(self, f"/admin/login?next={nxt}")
                return
            if path == "/admin" or path == "/admin/login":
                sub = ""
            else:
                sub = path[len("/admin/") :]
            serve_admin_dist(self, sub)
            return

        if path == "/__proxy_sse__":
            target = (qs.get("target") or [""])[0]
            if target:
                try:
                    pt = urlparse(target)
                    if pt.scheme in ("http", "https") and pt.path:
                        browser_base = f"{pt.scheme}://{pt.netloc}"
                        q = f"?{pt.query}" if pt.query else ""
                        target = _effective_proxy_url(browser_base, pt.path + q)
                except ValueError:
                    pass
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

        if path in ("/", "/index.html"):
            send_html_bytes(self, 200, (DIR / "index.html").read_bytes())
            return

        self.send_error(404, "not found")

    def do_POST(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        if path == "/__admin_login__":
            payload = read_allowed_json_body(self)
            if not isinstance(payload, dict):
                send_json(self, 400, {"error": "invalid JSON body"})
                return
            user = str(payload.get("user") or "").strip()
            password = str(payload.get("password") or "")
            if not check_admin_credentials(user, password):
                send_json(self, 401, {"error": "账号或密码错误"})
                return
            token = make_session_token(user)
            nxt = _safe_admin_next(str(payload.get("next") or "").strip() or None)
            body = json.dumps({"ok": True, "user": user, "next": nxt}, ensure_ascii=False).encode(
                "utf-8"
            )
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            set_session_cookie(self, token)
            self.end_headers()
            self.wfile.write(body)
            return

        if path == "/__admin_logout__":
            body = b'{"ok":true}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            clear_session_cookie(self)
            self.end_headers()
            self.wfile.write(body)
            return

        if path != "/__proxy__":
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

        url = _effective_proxy_url(base, subpath)
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
    print(f"  default gateway: {PUBLIC_GATEWAY_BASE}", flush=True)
    print(f"  admin user: {ADMIN_USER}", flush=True)
    httpd.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
