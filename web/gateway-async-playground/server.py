#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Static UI + reverse proxy for gateway async playground (browser CORS bypass).

Public gateway path: ``/gateway/<upstream path>`` → default upstream (strip prefix),
no admin login. Browser Admin UI still uses ``/__proxy__`` (envelope + baseUrl).

Stdlib Python only (no Rust, no pip deps). Author: kejiqing
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import re
import secrets
import socket
import struct
import sys
import threading
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

_DEFAULT_HOSTS = "127.0.0.1,localhost,192.168.9.252,10.200.2.171,10.22.28.94,gateway-rs,openvscode-server"
_DEFAULT_PORTS = "18088,18089,8080,8088,3000,13000"


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

OVS_UPSTREAM_BASE = _resolve_gateway_base_url(
    os.environ.get("PLAYGROUND_OVS_BASE", "http://openvscode-server:3000").strip()
) or "http://openvscode-server:3000"

OVS_PUBLIC_BASE = (
    os.environ.get("PLAYGROUND_PUBLIC_OVS_BASE", "").strip()
    or f"http://127.0.0.1:{os.environ.get('CLAW_OVS_HOST_PORT', '13000').strip() or '13000'}"
).rstrip("/")

OVS_FROM_GATEWAY = os.environ.get("PLAYGROUND_OVS_FROM_GATEWAY", "").strip().lower() in (
    "1",
    "true",
    "yes",
)

OVS_WORKSPACE_ROOT = os.environ.get("PLAYGROUND_OVS_WORKSPACE_ROOT", "/home/workspace").rstrip("/")


def _ovs_upstream_netloc() -> tuple[str | None, int | None]:
    try:
        p = urlparse(OVS_UPSTREAM_BASE)
    except ValueError:
        return None, None
    host = _norm_host(p.hostname)
    if not host:
        return None, None
    port = p.port or (443 if p.scheme == "https" else 80)
    return host, port


def _upstream_host_port_allowed(url: str) -> bool:
    try:
        p = urlparse(url)
    except ValueError:
        return False
    if p.scheme not in ("http", "https"):
        return False
    host = _norm_host(p.hostname)
    port = p.port or (443 if p.scheme == "https" else 80)
    if port not in ALLOWED_PORTS:
        ovs_host, ovs_port = _ovs_upstream_netloc()
        if not (ovs_host and ovs_port == port and host == ovs_host):
            return False
    if host in ALLOWED_HOSTNAMES:
        return True
    ovs_host, _ = _ovs_upstream_netloc()
    if ovs_host and host == ovs_host:
        return True
    return _is_private_lan_host(host)


def _loopback_gateway_key(url: str) -> tuple[str, int] | None:
    """(scheme, port) for loopback browser gateway URLs; None if not loopback."""
    try:
        p = urlparse(url)
    except ValueError:
        return None
    host = _norm_host(p.hostname)
    if host not in ("127.0.0.1", "localhost", "::1"):
        return None
    port = p.port or (443 if p.scheme == "https" else 80)
    return (p.scheme, port)


def _gateway_port_key(url: str) -> tuple[str, int] | None:
    try:
        p = urlparse(url)
    except ValueError:
        return None
    if not p.scheme:
        return None
    port = p.port or (443 if p.scheme == "https" else 80)
    return (p.scheme, port)


def _effective_proxy_base(browser_base: str) -> str:
    """Map UI `baseUrl` to an address reachable from this process (container vs host). Author: kejiqing"""
    b = _resolve_gateway_base_url(browser_base) or browser_base.strip().rstrip("/")
    if not UPSTREAM_GATEWAY_BASE or UPSTREAM_GATEWAY_BASE == b:
        return b
    pub = _loopback_gateway_key(PUBLIC_GATEWAY_BASE)
    br = _loopback_gateway_key(b)
    if pub is not None and br is not None and pub == br:
        return UPSTREAM_GATEWAY_BASE
    if b == PUBLIC_GATEWAY_BASE:
        return UPSTREAM_GATEWAY_BASE
    # Pool registry LAN URL (e.g. 10.x:18088) while PUBLIC is 127.0.0.1:18088 — dial compose gateway-rs. kejiqing
    if (
        UPSTREAM_GATEWAY_BASE
        and _gateway_port_key(b) == _gateway_port_key(PUBLIC_GATEWAY_BASE)
        and pub is not None
    ):
        try:
            br_host = _norm_host(urlparse(b).hostname)
        except ValueError:
            br_host = None
        if br_host and _is_private_lan_host(br_host):
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
    """Normalize post-login redirect for React Router (basename /admin)."""
    if not path or "://" in path:
        return "/"
    p = path if path.startswith("/") else f"/{path}"
    if p.startswith("/admin/"):
        p = p[len("/admin") :] or "/"
    elif p == "/admin":
        p = "/"
    if p.startswith("/login"):
        return "/"
    return p


def _admin_requires_login(path: str) -> bool:
    """Chat SPA is public; project management and other /admin routes need login."""
    if path in ("/admin/login",):
        return False
    if path.startswith("/admin/assets/"):
        return False
    if path == "/admin/chat" or path.startswith("/admin/chat/"):
        return False
    if path == "/admin" or path.startswith("/admin/"):
        return True
    return False


WS_ACCEPT_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def _ws_accept(sec_key: str) -> str:
    digest = hashlib.sha1((sec_key + WS_ACCEPT_GUID).encode("ascii")).digest()
    return base64.b64encode(digest).decode("ascii")


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("socket closed")
        buf.extend(chunk)
    return bytes(buf)


def _read_ws_frame(sock: socket.socket) -> tuple[int, bytes] | None:
    try:
        header = _recv_exact(sock, 2)
    except (ConnectionError, OSError):
        return None
    opcode = header[0] & 0x0F
    masked = (header[1] & 0x80) != 0
    length = header[1] & 0x7F
    if length == 126:
        length = struct.unpack("!H", _recv_exact(sock, 2))[0]
    elif length == 127:
        length = struct.unpack("!Q", _recv_exact(sock, 8))[0]
    mask = _recv_exact(sock, 4) if masked else None
    payload = _recv_exact(sock, length) if length else b""
    if mask:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    if opcode == 0x8:
        return None
    if opcode == 0x9:
        _write_ws_frame(sock, 0xA, payload, mask_out=True)
        return _read_ws_frame(sock)
    return opcode, payload


def _write_ws_frame(sock: socket.socket, opcode: int, payload: bytes, *, mask_out: bool) -> None:
    fin_opcode = 0x80 | (opcode & 0x0F)
    n = len(payload)
    header = bytearray([fin_opcode])
    if mask_out:
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", n))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", n))
        mask = os.urandom(4)
        header.extend(mask)
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        sock.sendall(header + payload)
        return
    if n < 126:
        header.append(n)
    elif n < 65536:
        header.append(126)
        header.extend(struct.pack("!H", n))
    else:
        header.append(127)
        header.extend(struct.pack("!Q", n))
    sock.sendall(header + payload)


def _connect_upstream_ws(upstream_http_url: str) -> socket.socket:
    parsed = urlparse(upstream_http_url)
    host = parsed.hostname or "127.0.0.1"
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    path = parsed.path or "/"
    if parsed.query:
        path = f"{path}?{parsed.query}"
    sock = socket.create_connection((host, port), timeout=30)
    key = base64.b64encode(os.urandom(16)).decode("ascii")
    req = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        f"Upgrade: websocket\r\n"
        f"Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        f"Sec-WebSocket-Version: 13\r\n"
        f"Sec-WebSocket-Protocol: tty\r\n"
        f"\r\n"
    )
    sock.sendall(req.encode("ascii"))
    resp = b""
    while b"\r\n\r\n" not in resp:
        chunk = sock.recv(4096)
        if not chunk:
            raise ConnectionError("upstream closed during websocket handshake")
        resp += chunk
    status_line = resp.split(b"\r\n", 1)[0]
    if b" 101 " not in status_line:
        raise ConnectionError(f"upstream websocket handshake failed: {status_line!r}")
    return sock


def _relay_ws(client: socket.socket, upstream: socket.socket) -> None:
    def pump(src: socket.socket, dst: socket.socket, mask_out: bool) -> None:
        try:
            while True:
                frame = _read_ws_frame(src)
                if frame is None:
                    break
                opcode, payload = frame
                if opcode in (0x1, 0x2):
                    _write_ws_frame(dst, opcode, payload, mask_out=mask_out)
        except (ConnectionError, OSError):
            pass
        finally:
            try:
                dst.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    t1 = threading.Thread(target=pump, args=(client, upstream, True), daemon=True)
    t2 = threading.Thread(target=pump, args=(upstream, client, False), daemon=True)
    t1.start()
    t2.start()
    t1.join()
    t2.join()


def ovs_workspace_folder(proj_id: str) -> str:
    pid = str(proj_id).strip() or "1"
    return f"{OVS_WORKSPACE_ROOT}/proj_{pid}/home"


def fetch_ovs_workspace_from_gateway(proj_id: str) -> tuple[dict | None, int | None, str | None]:
    """Gateway OVS workspace contract (`ovsFolderUrl`, `ovsBackend`, …). Returns (data, http_status, error)."""
    if not UPSTREAM_GATEWAY_BASE:
        return None, None, "gateway upstream not configured"
    url = (
        f"{UPSTREAM_GATEWAY_BASE.rstrip('/')}/v1/projects/"
        f"{urllib.parse.quote(str(proj_id).strip() or '1', safe='')}/ovs/workspace"
    )
    try:
        with urllib.request.urlopen(url, timeout=120) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            if isinstance(data, dict):
                return data, resp.status, None
            return None, resp.status, "invalid ovs/workspace JSON"
    except urllib.error.HTTPError as e:
        raw = e.read().decode("utf-8", errors="replace")
        err = raw
        try:
            parsed = json.loads(raw)
            if isinstance(parsed, dict) and parsed.get("error"):
                err = str(parsed["error"])
        except (json.JSONDecodeError, ValueError, TypeError):
            pass
        return None, e.code, err
    except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError, ValueError) as e:
        return None, None, str(e)


def fetch_ovs_folder_url_from_gateway(proj_id: str) -> str | None:
    """FC mode: Gateway returns direct e2b traffic ovsFolderUrl (WebSocket-native; no gateway proxy)."""
    data, _status, _err = fetch_ovs_workspace_from_gateway(proj_id)
    if not data:
        return None
    raw = data.get("ovsFolderUrl") or data.get("ovs_folder_url")
    return str(raw).strip() if raw else None


def send_ovs_strict_page(handler: BaseHTTPRequestHandler, proj_id: str) -> None:
    """Strict worker profile — OVS not available for this project. Author: kejiqing"""
    body = (
        f"<!DOCTYPE html><html><head><meta charset=utf-8>"
        f"<title>OVS 不可用</title></head><body>"
        f"<h1>该项目为 Strict Worker</h1>"
        f"<p>projId={proj_id} 的 worker profile 为 strict，无 Web IDE / OVS。</p>"
        f"<p>请在 Admin → Worker profile 将项目设为 <code>relaxed</code> 后重试。</p>"
        f"</body></html>"
    ).encode("utf-8")
    send_html_bytes(handler, 403, body)


def send_ovs_wait_page(handler: BaseHTTPRequestHandler, proj_id: str, detail: str) -> None:
    """FC OVS not ready — never redirect to dead :13000. Author: kejiqing"""
    gw = PUBLIC_GATEWAY_BASE.rstrip("/")
    retry = f"/ovs/?projId={urllib.parse.quote(str(proj_id).strip() or '1', safe='')}"
    body = (
        f"<!DOCTYPE html><html><head><meta charset=utf-8>"
        f"<title>OVS 启动中</title>"
        f'<meta http-equiv="refresh" content="5;url={retry}">'
        f"</head><body>"
        f"<h1>OVS 尚未就绪</h1><p>{detail}</p>"
        f"<p>projId={proj_id} · 5 秒后自动重试，或 "
        f'<a href="{retry}">点此重试</a>。</p>'
        f"<p>Gateway: <a href=\"{gw}/healthz\">{gw}/healthz</a></p>"
        f"</body></html>"
    ).encode("utf-8")
    send_html_bytes(handler, 503, body)


_ovs_workspace_materialized: set[str] = set()
_ovs_workspace_materialize_lock = threading.Lock()


def materialize_ovs_workspace_via_gateway(proj_id: str) -> None:
    """Gateway writes claw.projId into proj_N/home/.vscode/settings.json (OVS contract)."""
    pid = str(proj_id).strip() or "1"
    with _ovs_workspace_materialize_lock:
        if pid in _ovs_workspace_materialized:
            return
    url = f"{UPSTREAM_GATEWAY_BASE.rstrip('/')}/v1/projects/{urllib.parse.quote(pid, safe='')}/ovs/workspace"
    try:
        with urllib.request.urlopen(url, timeout=60) as resp:
            if 200 <= resp.status < 300:
                with _ovs_workspace_materialize_lock:
                    _ovs_workspace_materialized.add(pid)
    except (urllib.error.URLError, TimeoutError, OSError):
        pass


def ovs_upstream_url(rel_path: str, qs: dict[str, list[str]], proj_id: str) -> str:
    folder = ovs_workspace_folder(proj_id)
    base = OVS_UPSTREAM_BASE.rstrip("/")
    path = rel_path if rel_path.startswith("/") else f"/{rel_path}"
    if not path.startswith("/ovs"):
        if path == "/":
            path = "/ovs"
        else:
            path = "/ovs" + path
    merged = dict(qs)
    # Workspace bootstrap only; static assets under /stable-* must not get folder=.
    if rel_path in ("", "/"):
        merged["folder"] = [folder]
    query = urllib.parse.urlencode(merged, doseq=True)
    return f"{base}{path}?{query}" if query else f"{base}{path}"


_HOP_BY_HOP = frozenset(
    {
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "transfer-encoding",
        "upgrade",
    }
)


def proxy_ovs_http(
    handler: BaseHTTPRequestHandler, rel_path: str, qs: dict[str, list[str]], proj_id: str
) -> None:
    if not read_session_user(handler):
        handler.send_error(401, "login required")
        return
    # Browser must load OVS directly with ?folder=proj_N/home (proxy HTML loses workspace root). kejiqing
    if rel_path in ("", "/"):
        materialize_ovs_workspace_via_gateway(proj_id)
        data, status, err = (
            fetch_ovs_workspace_from_gateway(proj_id) if UPSTREAM_GATEWAY_BASE else (None, None, None)
        )
        if status == 403:
            send_ovs_strict_page(handler, proj_id)
            return
        if data:
            folder_url = data.get("ovsFolderUrl") or data.get("ovs_folder_url")
            if folder_url:
                send_redirect(handler, str(folder_url).strip())
                return
            backend = str(data.get("ovsBackend") or data.get("ovs_backend") or "").strip().lower()
            if backend == "e2b" or OVS_FROM_GATEWAY:
                send_ovs_wait_page(
                    handler,
                    proj_id,
                    err
                    or "relaxed worker OVS 仍在 warmup。就绪后请用 API 返回的 ovsFolderUrl 打开。",
                )
                return
        if OVS_FROM_GATEWAY:
            send_ovs_wait_page(
                handler,
                proj_id,
                "无法从 Gateway 读取 OVS 工作区（请确认 gateway-rs 已 up）。",
            )
            return
        folder = ovs_workspace_folder(proj_id)
        q = urllib.parse.urlencode({"folder": folder})
        send_redirect(handler, f"{OVS_PUBLIC_BASE}/ovs/?{q}")
        return
    url = ovs_upstream_url(rel_path, qs, proj_id)
    if not is_allowed_upstream(url.split("?", 1)[0]):
        handler.send_error(400, "upstream not allowed")
        return
    method = handler.command.upper()
    length = handler.headers.get("Content-Length")
    body: bytes | None = None
    if length:
        try:
            n = int(length)
        except ValueError:
            handler.send_error(400, "bad Content-Length")
            return
        if n > 0:
            body = handler.rfile.read(n)
    headers = {
        k: v
        for k, v in handler.headers.items()
        if k.lower() not in _HOP_BY_HOP and k.lower() not in ("host", "accept-encoding")
    }
    try:
        req = urllib.request.Request(url, data=body, method=method, headers=headers)
        req.add_header("Accept-Encoding", "identity")
        upstream = urllib.request.urlopen(req, timeout=120)
    except urllib.error.HTTPError as e:
        handler.send_response(e.code)
        for k, v in e.headers.items():
            if k.lower() not in _HOP_BY_HOP:
                handler.send_header(k, v)
        handler.end_headers()
        handler.wfile.write(e.read())
        return
    except urllib.error.URLError as e:
        handler.send_error(502, str(e.reason if hasattr(e, "reason") else e))
        return
    try:
        payload = upstream.read()
    finally:
        upstream.close()
    handler.send_response(upstream.status)
    for k, v in upstream.headers.items():
        lk = k.lower()
        if lk in _HOP_BY_HOP or lk in ("content-encoding", "content-length", "transfer-encoding"):
            continue
        handler.send_header(k, v)
    handler.send_header("Content-Length", str(len(payload)))
    handler.send_header("Connection", "close")
    handler.end_headers()
    if method != "HEAD":
        handler.wfile.write(payload)


def _gateway_public_upstream_url(request_path: str, query: str) -> str | None:
    """Map ``/gateway/...`` to upstream URL (strip prefix). Author: kejiqing"""
    if request_path == "/gateway":
        sub = "/"
    elif request_path.startswith("/gateway/"):
        sub = request_path[len("/gateway") :] or "/"
    else:
        return None
    if not sub.startswith("/"):
        sub = "/" + sub
    base = _effective_proxy_base(PUBLIC_GATEWAY_BASE).rstrip("/")
    q = f"?{query}" if query else ""
    return f"{base}{sub}{q}"


def proxy_gateway_public(handler: BaseHTTPRequestHandler) -> None:
    """Transparent public reverse proxy: ``/gateway/*`` → gateway (no login). Author: kejiqing"""
    parsed = urllib.parse.urlparse(handler.path)
    url = _gateway_public_upstream_url(parsed.path, parsed.query)
    if not url:
        handler.send_error(404, "not found")
        return
    check = url.split("?", 1)[0]
    if not is_allowed_upstream(check):
        handler.send_error(400, "upstream host/port not allowed")
        return

    method = handler.command.upper()
    length = handler.headers.get("Content-Length")
    body: bytes | None = None
    if length and method not in ("GET", "HEAD"):
        try:
            n = int(length)
        except ValueError:
            handler.send_error(400, "bad Content-Length")
            return
        if n > 0:
            body = handler.rfile.read(n)

    headers = {
        k: v
        for k, v in handler.headers.items()
        if k.lower() not in _HOP_BY_HOP and k.lower() not in ("host", "accept-encoding")
    }
    try:
        req = urllib.request.Request(url, data=body, method=method, headers=headers)
        req.add_header("Accept-Encoding", "identity")
        upstream = urllib.request.urlopen(req, timeout=600)
    except urllib.error.HTTPError as e:
        handler.send_response(e.code)
        for k, v in e.headers.items():
            if k.lower() not in _HOP_BY_HOP:
                handler.send_header(k, v)
        handler.end_headers()
        if method != "HEAD":
            handler.wfile.write(e.read())
        return
    except urllib.error.URLError as e:
        handler.send_error(502, str(e.reason if hasattr(e, "reason") else e))
        return

    ct = (upstream.headers.get("Content-Type") or "") if upstream.headers else ""
    is_sse = "text/event-stream" in ct.lower()
    try:
        if is_sse:
            handler.send_response(upstream.status)
            for k, v in upstream.headers.items():
                lk = k.lower()
                if lk in _HOP_BY_HOP or lk in (
                    "content-encoding",
                    "content-length",
                    "transfer-encoding",
                ):
                    continue
                handler.send_header(k, v)
            handler.send_header("Cache-Control", "no-cache")
            handler.send_header("Connection", "close")
            handler.send_header("X-Accel-Buffering", "no")
            handler.end_headers()
            try:
                while True:
                    chunk = upstream.read(4096)
                    if not chunk:
                        break
                    handler.wfile.write(chunk)
                    handler.wfile.flush()
            finally:
                upstream.close()
            return

        try:
            payload = upstream.read()
        finally:
            upstream.close()
        handler.send_response(upstream.status)
        for k, v in upstream.headers.items():
            lk = k.lower()
            if lk in _HOP_BY_HOP or lk in (
                "content-encoding",
                "content-length",
                "transfer-encoding",
            ):
                continue
            handler.send_header(k, v)
        handler.send_header("Content-Length", str(len(payload)))
        handler.send_header("Connection", "close")
        handler.end_headers()
        if method != "HEAD":
            handler.wfile.write(payload)
    except BrokenPipeError:
        try:
            upstream.close()
        except Exception:
            pass


def proxy_ovs_agent_ws(
    handler: BaseHTTPRequestHandler, proj_id: str, session_id: str, chat_session_id: str = ""
) -> None:
    if not read_session_user(handler):
        handler.send_error(401, "login required")
        return
    client_key = handler.headers.get("Sec-WebSocket-Key")
    if not client_key:
        handler.send_error(400, "missing Sec-WebSocket-Key")
        return
    sid = session_id.strip() or f"ovs-{proj_id.strip() or '1'}"
    pid = proj_id.strip() or "1"
    subpath = (
        f"/v1/sessions/{urllib.parse.quote(sid, safe='')}"
        f"/agent/ws?projId={urllib.parse.quote(pid, safe='')}"
    )
    chat_sid = chat_session_id.strip()
    if chat_sid:
        subpath += f"&chatSessionId={urllib.parse.quote(chat_sid, safe='')}"
    upstream_url = _effective_proxy_url(PUBLIC_GATEWAY_BASE, subpath)
    if not is_allowed_upstream(upstream_url):
        handler.send_error(400, "upstream host/port not allowed")
        return
    try:
        upstream = _connect_upstream_ws(upstream_url)
    except (ConnectionError, OSError, ValueError) as e:
        handler.send_error(502, f"upstream websocket: {e}")
        return
    accept = _ws_accept(client_key)
    handler.connection.sendall(
        b"HTTP/1.1 101 Switching Protocols\r\n"
        b"Upgrade: websocket\r\n"
        b"Connection: Upgrade\r\n"
        + b"Sec-WebSocket-Accept: "
        + accept.encode("ascii")
        + b"\r\n\r\n"
    )
    try:
        _relay_ws(handler.connection, upstream)
    finally:
        for s in (handler.connection, upstream):
            try:
                s.close()
            except OSError:
                pass


def proxy_ovs_vscode_ws(
    handler: BaseHTTPRequestHandler, rel_path: str, qs: dict[str, list[str]], proj_id: str
) -> None:
    if not read_session_user(handler):
        handler.send_error(401, "login required")
        return
    client_key = handler.headers.get("Sec-WebSocket-Key")
    if not client_key:
        handler.send_error(400, "missing Sec-WebSocket-Key")
        return
    http_url = ovs_upstream_url(rel_path, qs, proj_id)
    parsed = urlparse(http_url)
    if parsed.scheme not in ("http", "https"):
        handler.send_error(400, "bad ovs upstream")
        return
    ws_scheme = "wss" if parsed.scheme == "https" else "ws"
    ws_url = f"{ws_scheme}://{parsed.netloc}{parsed.path}"
    if parsed.query:
        ws_url += f"?{parsed.query}"
    try:
        upstream = _connect_upstream_ws(ws_url)
    except (ConnectionError, OSError, ValueError) as e:
        handler.send_error(502, f"upstream ovs websocket: {e}")
        return
    accept = _ws_accept(client_key)
    proto = handler.headers.get("Sec-WebSocket-Protocol", "")
    hdr = (
        b"HTTP/1.1 101 Switching Protocols\r\n"
        b"Upgrade: websocket\r\n"
        b"Connection: Upgrade\r\n"
        + b"Sec-WebSocket-Accept: "
        + accept.encode("ascii")
        + b"\r\n"
    )
    if proto.strip():
        hdr += b"Sec-WebSocket-Protocol: " + proto.encode("ascii") + b"\r\n"
    hdr += b"\r\n"
    handler.connection.sendall(hdr)
    try:
        _relay_ws(handler.connection, upstream)
    finally:
        for s in (handler.connection, upstream):
            try:
                s.close()
            except OSError:
                pass


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


def _is_private_lan_host(host: str | None) -> bool:
    """RFC1918 + link-local — cluster pool/gateway peers on LAN. Author: kejiqing"""
    if not host:
        return False
    parts = host.split(".")
    if len(parts) != 4:
        return False
    try:
        octets = [int(x) for x in parts]
    except ValueError:
        return False
    if octets[0] == 10:
        return True
    if octets[0] == 172 and 16 <= octets[1] <= 31:
        return True
    if octets[0] == 192 and octets[1] == 168:
        return True
    return False


def is_allowed_upstream(url: str) -> bool:
    try:
        p = urlparse(url)
    except ValueError:
        return False
    if p.scheme not in ("http", "https"):
        return False
    return _upstream_host_port_allowed(url)


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


def _looks_json_content_type(content_type: str) -> bool:
    ct = content_type.lower()
    return "application/json" in ct or ct.endswith("+json")


def _proxy_upstream_envelope(
    *,
    ok: bool,
    status: int,
    headers: dict[str, str],
    raw: bytes,
    content_type: str = "",
) -> dict:
    """Wrap upstream HTTP for /__proxy__; JSON bodies use ``body`` object for DevTools."""
    text = raw.decode("utf-8", errors="replace")
    ct = content_type or headers.get("Content-Type") or headers.get("content-type") or ""
    out: dict = {"ok": ok, "status": status, "headers": headers, "contentType": ct}
    if _looks_json_content_type(ct) and text.strip():
        try:
            out["body"] = json.loads(text)
            return out
        except json.JSONDecodeError:
            pass
    out["bodyText"] = text
    return out


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
    """Serve Vite SPA: static assets or index.html fallback (routes only, not missing assets)."""
    if not ADMIN_INDEX.is_file():
        send_html_bytes(
            handler,
            503,
            (
                "<h1>admin dist missing</h1>"
                "<p>Rebuild playground image (<code>gateway.sh build</code> / <code>quick</code>) "
                "or bind a full <code>web/gateway-admin/dist</code> "
                "(<code>CLAW_GATEWAY_ADMIN_BIND=1</code>).</p>"
            ).encode("utf-8"),
        )
        return True
    hit = _admin_dist_safe_path(subpath)
    if hit is not None:
        mime = _ADMIN_MIME.get(hit.suffix.lower(), "application/octet-stream")
        send_static_bytes(handler, 200, hit.read_bytes(), mime)
        return True
    # Missing hashed bundles must 404 — SPA fallback here caused silent black screens. kejiqing
    if subpath.startswith("assets/"):
        handler.send_error(404, "admin asset not found")
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
        "gatewayPublicPath": "/gateway",
        "adminLoginRequired": True,
        "adminChatPublic": True,
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path
        qs = urllib.parse.parse_qs(parsed.query)

        if self.headers.get("Upgrade", "").lower() == "websocket":
            if path == "/ovs/agent/ws":
                proj_id = (qs.get("projId") or qs.get("proj_id") or ["1"])[0]
                session_id = (qs.get("sessionId") or qs.get("session_id") or [""])[0]
                chat_session_id = (qs.get("chatSessionId") or qs.get("chat_session_id") or [""])[0]
                proxy_ovs_agent_ws(self, proj_id, session_id, chat_session_id)
                return
            if path == "/ovs" or path.startswith("/ovs/"):
                rel = path[len("/ovs") :] or "/"
                proj_id = (qs.get("projId") or qs.get("proj_id") or ["1"])[0]
                proxy_ovs_vscode_ws(self, rel, qs, proj_id)
                return
            self.send_error(400, "unsupported websocket path")
            return

        if path == "/gateway" or path.startswith("/gateway/"):
            proxy_gateway_public(self)
            return

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
            if _admin_requires_login(path) and not read_session_user(self):
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
            upstream_bytes = 0
            delta_frames = 0
            parse_buf = b""
            try:
                while True:
                    chunk = upstream.read(4096)
                    if not chunk:
                        break
                    upstream_bytes += len(chunk)
                    parse_buf += chunk
                    while b"\n\n" in parse_buf:
                        frame, parse_buf = parse_buf.split(b"\n\n", 1)
                        if b"event: biz.report.delta" in frame:
                            delta_frames += 1
                    self.wfile.write(chunk)
                    self.wfile.flush()
            finally:
                upstream.close()
                sys.stderr.write(
                    f"claw_proxy_sse_end target={target!r} "
                    f"bytes={upstream_bytes} delta_frames={delta_frames}\n"
                )
                sys.stderr.flush()
            return

        if path == "/ovs" or path.startswith("/ovs/"):
            if path == "/ovs/agent/ws":
                self.send_error(405, "use websocket")
                return
            if not read_session_user(self):
                nxt = urllib.parse.quote(path + ("?" + parsed.query if parsed.query else ""), safe="")
                send_redirect(self, f"/admin/login?next={nxt}")
                return
            proj_id = (qs.get("projId") or qs.get("proj_id") or ["1"])[0]
            rel = path[len("/ovs") :] or "/"
            proxy_ovs_http(self, rel, qs, proj_id)
            return

        if path in ("/", "/index.html"):
            send_redirect(self, "/admin")
            return

        self.send_error(404, "not found")

    def do_POST(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        if path == "/gateway" or path.startswith("/gateway/"):
            proxy_gateway_public(self)
            return

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
            rh = dict(e.headers.items()) if e.headers else {}
            ct = e.headers.get("Content-Type", "") if e.headers else ""
            send_json(
                self,
                e.code,
                _proxy_upstream_envelope(
                    ok=False,
                    status=e.code,
                    headers=rh,
                    raw=raw,
                    content_type=ct,
                ),
            )
            return
        except urllib.error.URLError as e:
            reason = getattr(e, "reason", e)
            hint = (
                "playground 进程连不上该地址：容器内请用 compose 的 PLAYGROUND_GATEWAY_BASE；"
                "宿主机请 gateway.sh up 并 curl 同端口 /healthz"
            )
            send_json(
                self,
                502,
                {
                    "ok": False,
                    "error": str(reason),
                    "upstream": url,
                    "browserBase": base,
                    "hint": hint,
                },
            )
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
            envelope = _proxy_upstream_envelope(
                ok=200 <= resp.status < 300,
                status=resp.status,
                headers=rh,
                raw=out,
                content_type=ct,
            )
            # Browsers discard response bodies on HTTP 204; envelope always carries JSON.
            proxy_status = 200 if 200 <= resp.status < 300 else resp.status
            send_json(self, proxy_status, envelope)
        finally:
            resp.close()

    def do_PUT(self) -> None:
        path = urllib.parse.urlparse(self.path).path
        if path == "/gateway" or path.startswith("/gateway/"):
            proxy_gateway_public(self)
            return
        self.send_error(404, "not found")

    def do_PATCH(self) -> None:
        path = urllib.parse.urlparse(self.path).path
        if path == "/gateway" or path.startswith("/gateway/"):
            proxy_gateway_public(self)
            return
        self.send_error(404, "not found")

    def do_DELETE(self) -> None:
        path = urllib.parse.urlparse(self.path).path
        if path == "/gateway" or path.startswith("/gateway/"):
            proxy_gateway_public(self)
            return
        self.send_error(404, "not found")


def main(argv: list[str]) -> int:
    host = LISTEN_HOST
    port = LISTEN_PORT
    if len(argv) >= 2:
        port = int(argv[1])
    httpd = ThreadingHTTPServer((host, port), Handler)
    print(f"gateway-async-playground: http://{host}:{port}/", flush=True)
    print(f"  default gateway: {PUBLIC_GATEWAY_BASE}", flush=True)
    print(f"  public API: /gateway → {UPSTREAM_GATEWAY_BASE or PUBLIC_GATEWAY_BASE}", flush=True)
    print(f"  admin user: {ADMIN_USER}", flush=True)
    httpd.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
