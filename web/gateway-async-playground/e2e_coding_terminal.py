#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
E2E: playground /coding → __proxy__ terminal/start → WS /coding/terminal/* → terminal/stop.
Stdlib only. Author: kejiqing

Usage:
  python3 web/gateway-async-playground/e2e_coding_terminal.py
  PLAYGROUND_BASE=http://127.0.0.1:18765 GATEWAY_BASE=http://127.0.0.1:8088 \\
    python3 web/gateway-async-playground/e2e_coding_terminal.py
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import secrets
import socket
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from http import cookies
from http.cookiejar import CookieJar
from pathlib import Path

PLAYGROUND_BASE = os.environ.get("PLAYGROUND_BASE", "http://127.0.0.1:18765").rstrip("/")
GATEWAY_BASE = os.environ.get("GATEWAY_BASE", "http://127.0.0.1:8088").rstrip("/")
ADMIN_USER = os.environ.get("PLAYGROUND_ADMIN_USER", "admin")
ADMIN_PASSWORD = os.environ.get("PLAYGROUND_ADMIN_PASSWORD", "sunmi123")
PROJ_ID = int(os.environ.get("E2E_PROJ_ID", "1"))
SESSION_ID = os.environ.get("E2E_SESSION_ID", f"e2e-{int(time.time())}")
START_TIMEOUT_SEC = int(os.environ.get("E2E_START_TIMEOUT_SEC", "90"))
WS_READ_TIMEOUT_SEC = float(os.environ.get("E2E_WS_READ_TIMEOUT_SEC", "8"))
E2E_POOL_RESET = os.environ.get("E2E_POOL_RESET", "1").strip() not in ("0", "false", "no")
REPO_ROOT = Path(__file__).resolve().parents[2]
GATEWAY_SH = REPO_ROOT / "deploy" / "stack" / "gateway.sh"


class E2eError(Exception):
    pass


def _jar_opener(jar: CookieJar) -> urllib.request.OpenerDirector:
    return urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def _json_req(
    opener: urllib.request.OpenerDirector,
    url: str,
    *,
    method: str = "GET",
    body: dict | None = None,
    timeout: float = 120,
) -> tuple[int, dict]:
    data = None
    headers = {"Accept": "application/json"}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers["Content-Type"] = "application/json; charset=utf-8"
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with opener.open(req, timeout=timeout) as resp:
            raw = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        raw = e.read()
        status = e.code
    try:
        return status, json.loads(raw.decode("utf-8", errors="replace") or "{}")
    except json.JSONDecodeError as e:
        raise E2eError(f"non-JSON from {url}: {raw[:200]!r}") from e


def login(opener: urllib.request.OpenerDirector) -> None:
    status, data = _json_req(
        opener,
        f"{PLAYGROUND_BASE}/__admin_login__",
        method="POST",
        body={"user": ADMIN_USER, "password": ADMIN_PASSWORD, "next": "/coding"},
    )
    if status != 200 or not data.get("ok"):
        raise E2eError(f"login failed status={status} body={data}")
    print(f"[ok] login user={data.get('user')}")


def proxy_envelope_body(data: dict) -> dict:
    body = data.get("body")
    if isinstance(body, dict):
        return body
    if data.get("bodyJson") is not None:
        return data["bodyJson"]
    text = data.get("bodyText") or ""
    if text.strip():
        return json.loads(text)
    return {}


def proxy_terminal_start(opener: urllib.request.OpenerDirector, session_id: str) -> dict:
    status, data = _json_req(
        opener,
        f"{PLAYGROUND_BASE}/__proxy__",
        method="POST",
        body={
            "baseUrl": GATEWAY_BASE,
            "method": "POST",
            "path": f"/v1/sessions/{urllib.parse.quote(session_id, safe='')}/terminal/start",
            "body": {"projId": PROJ_ID, "sessionId": session_id},
        },
        timeout=START_TIMEOUT_SEC,
    )
    inner = proxy_envelope_body(data)
    if not data.get("ok"):
        msg = data.get("error") or inner.get("error") or status
        if data.get("status") == 409 or "already active" in str(msg).lower():
            ws_path = (
                f"/v1/sessions/{urllib.parse.quote(session_id, safe='')}"
                f"/terminal/ws?projId={PROJ_ID}"
            )
            print(f"[ok] terminal already active, reuse wsPath {ws_path}")
            return {"sessionId": session_id, "projId": PROJ_ID, "wsPath": ws_path}
        raise E2eError(f"terminal/start via __proxy__ failed status={status} body={data}")
    ws_path = inner.get("wsPath") or inner.get("ws_path")
    if not ws_path:
        raise E2eError(f"terminal/start missing wsPath in proxy body={data}")
    print(f"[ok] terminal/start wsPath={ws_path} slot={inner.get('slotIndex')} ttyd={inner.get('ttydHostPort')}")
    return inner


def proxy_terminal_stop(opener: urllib.request.OpenerDirector, session_id: str) -> None:
    q = urllib.parse.urlencode({"projId": PROJ_ID})
    status, data = _json_req(
        opener,
        f"{PLAYGROUND_BASE}/__proxy__",
        method="POST",
        body={
            "baseUrl": GATEWAY_BASE,
            "method": "POST",
            "path": f"/v1/sessions/{urllib.parse.quote(session_id, safe='')}/terminal/stop?{q}",
            "body": None,
        },
        timeout=30,
    )
    inner = proxy_envelope_body(data)
    if not data.get("ok"):
        raise E2eError(f"terminal/stop failed status={status} body={data}")
    print(f"[ok] terminal/stop {inner}")


def coding_html_has_fix(opener: urllib.request.OpenerDirector) -> None:
    req = urllib.request.Request(f"{PLAYGROUND_BASE}/coding", method="GET")
    with opener.open(req, timeout=15) as resp:
        html = resp.read().decode("utf-8", errors="replace")
    if "proxyEnvelopeBody" not in html or "ttydSendInput" not in html:
        raise E2eError("coding.html missing ttyd wire protocol (stale playground image?)")
    print("[ok] coding.html has ttyd wire protocol")


def ws_path_to_playground(ws_path: str) -> str:
    m = __import__("re").match(
        r"/v1/sessions/([^/?]+)/terminal/ws\?projId=(\d+)", ws_path
    )
    if not m:
        raise E2eError(f"cannot parse wsPath: {ws_path!r}")
    sid = urllib.parse.quote(m.group(1), safe="")
    return f"/coding/terminal/{sid}?projId={m.group(2)}"


def _ws_accept(key: str) -> str:
    digest = hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
    return base64.b64encode(digest).decode("ascii")


def _read_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise E2eError("websocket closed while reading")
        buf += chunk
    return buf


def _ws_recv_payload(sock: socket.socket) -> tuple[int, bytes]:
    hdr = _read_exact(sock, 2)
    b1, b2 = hdr[0], hdr[1]
    opcode = b1 & 0x0F
    masked = (b2 & 0x80) != 0
    length = b2 & 0x7F
    if length == 126:
        length = struct.unpack("!H", _read_exact(sock, 2))[0]
    elif length == 127:
        length = struct.unpack("!Q", _read_exact(sock, 8))[0]
    mask = _read_exact(sock, 4) if masked else b""
    payload = _read_exact(sock, length)
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return opcode, payload


def _ws_send_text(sock: socket.socket, text: bytes) -> None:
    mask = secrets.token_bytes(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(text))
    n = len(text)
    if n < 126:
        sock.sendall(bytes([0x81, 0x80 | n]) + mask + masked)
    else:
        sock.sendall(bytes([0x81, 0x80 | 126]) + struct.pack("!H", n) + mask + masked)


def _ws_send_binary(sock: socket.socket, payload: bytes) -> None:
    mask = secrets.token_bytes(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    n = len(payload)
    if n < 126:
        sock.sendall(bytes([0x82, 0x80 | n]) + mask + masked)
    else:
        sock.sendall(bytes([0x82, 0x80 | 126]) + struct.pack("!H", n) + mask + masked)


def _ws_recv_ttyd(sock: socket.socket) -> str:
    opcode, payload = _ws_recv_payload(sock)
    if opcode == 0x8:
        raise E2eError("websocket closed by server")
    if opcode == 0x9:
        # ping -> ignore and continue
        return _ws_recv_ttyd(sock)
    if opcode in (0x1, 0x2) and payload[:1] == b"0":
        return payload[1:].decode("utf-8", errors="replace")
    if opcode in (0x1, 0x2):
        return payload.decode("utf-8", errors="replace")
    return ""


def ws_probe(opener: urllib.request.OpenerDirector, pg_path: str) -> str:
    parsed = urllib.parse.urlparse(PLAYGROUND_BASE)
    host = parsed.hostname or "127.0.0.1"
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    key = base64.b64encode(secrets.token_bytes(16)).decode("ascii")
    cookie_hdr = ""
    for c in opener.handlers:
        if isinstance(c, urllib.request.HTTPCookieProcessor):
            jar = c.cookiejar
            cookie_hdr = "; ".join(
                f"{x.name}={x.value}" for x in jar if x.name
            )
    req = (
        f"GET {pg_path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "Sec-WebSocket-Protocol: tty\r\n"
        f"Cookie: {cookie_hdr}\r\n"
        "\r\n"
    ).encode("utf-8")
    sock = socket.create_connection((host, port), timeout=15)
    try:
        sock.sendall(req)
        resp = b""
        while b"\r\n\r\n" not in resp:
            chunk = sock.recv(4096)
            if not chunk:
                raise E2eError("no websocket handshake response")
            resp += chunk
        status_line = resp.split(b"\r\n", 1)[0].decode()
        if " 101 " not in status_line:
            raise E2eError(f"websocket upgrade failed: {status_line} {resp[:300]!r}")
        # ttyd 1.7: subprotocol tty + first message plain JSON spawns claw.
        spawn = json.dumps({"columns": 120, "rows": 30}).encode("utf-8")
        _ws_send_text(sock, spawn)
        sock.settimeout(WS_READ_TIMEOUT_SEC)
        sample = ""
        deadline = time.time() + WS_READ_TIMEOUT_SEC
        while time.time() < deadline:
            try:
                piece = _ws_recv_ttyd(sock)
            except socket.timeout:
                break
            except E2eError:
                break
            if piece:
                sample += piece
                if "Model" in sample or ">" in sample or "CLAW" in sample.upper():
                    break
        if not sample:
            _ws_send_binary(sock, b"0\r")
            deadline = time.time() + 5
            while time.time() < deadline:
                try:
                    piece = _ws_recv_ttyd(sock)
                except (socket.timeout, E2eError):
                    break
                if piece:
                    sample += piece
                    if len(sample) >= 4:
                        break
        if sample:
            print(f"[ok] websocket ttyd output ({len(sample)} bytes): {sample[:120]!r}")
        else:
            raise E2eError("websocket 101 but no ttyd/claw output (check ttyd protocol or worker)")
        return sample
    finally:
        sock.close()


def ensure_pool_idle() -> None:
    """Gateway restart drops terminal registry while pool slots stay leased — reset before E2E."""
    if not E2E_POOL_RESET:
        print("[skip] E2E_POOL_RESET=0, not resetting pool")
        return
    if not GATEWAY_SH.is_file():
        raise E2eError(f"gateway.sh not found: {GATEWAY_SH}")
    print("[..] pool-reset + pool-up (orphan slot cleanup)")
    subprocess.run([str(GATEWAY_SH), "pool-reset"], cwd=REPO_ROOT, check=True, timeout=180)
    subprocess.run([str(GATEWAY_SH), "pool-up"], cwd=REPO_ROOT, check=True, timeout=60)
    print("[ok] pool ready")


def main() -> int:
    print(f"==> E2E coding terminal playground={PLAYGROUND_BASE} gateway={GATEWAY_BASE} session={SESSION_ID}")
    ensure_pool_idle()
    jar = CookieJar()
    opener = _jar_opener(jar)

    login(opener)
    coding_html_has_fix(opener)
    start = proxy_terminal_start(opener, SESSION_ID)
    ws_path = start.get("wsPath") or start.get("ws_path")
    if not ws_path:
        raise E2eError("missing wsPath after start")
    pg_ws = ws_path_to_playground(ws_path)
    ws_probe(opener, pg_ws)
    proxy_terminal_stop(opener, SESSION_ID)
    print("==> E2E PASS")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except E2eError as e:
        print(f"==> E2E FAIL: {e}", file=sys.stderr)
        raise SystemExit(1)
    except urllib.error.URLError as e:
        print(f"==> E2E FAIL: network {e}", file=sys.stderr)
        raise SystemExit(1)
