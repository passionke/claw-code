#!/usr/bin/env python3
# Minimal NAS layout + project file API for e2b singleton. Author: kejiqing
"""claw-nas-api — HTTP service bound to NAS export root at /claw_ws."""

from __future__ import annotations

import os
import secrets
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote

NAS_ROOT = Path(os.environ.get("CLAW_NAS_API_ROOT", "/claw_ws"))
LISTEN_HOST = os.environ.get("CLAW_NAS_API_LISTEN_HOST", "0.0.0.0")
LISTEN_PORT = int(os.environ.get("CLAW_NAS_API_LISTEN_PORT", "8090"))
INTERNAL_TOKEN = os.environ.get("CLAW_GATEWAY_INTERNAL_TOKEN", "").strip()


def _authorized(headers) -> bool:
    if not INTERNAL_TOKEN:
        return True
    auth = headers.get("Authorization", "")
    if auth.startswith("Bearer "):
        return secrets.compare_digest(auth[7:].strip(), INTERNAL_TOKEN)
    return secrets.compare_digest(headers.get("X-Claw-Internal-Token", "").strip(), INTERNAL_TOKEN)


def _safe_rel(path: str) -> Path:
    rel = unquote(path.lstrip("/"))
    if ".." in rel.split("/"):
        raise ValueError("path traversal")
    return NAS_ROOT / rel


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):  # noqa: D401
        return

    def _json(self, code: int, body: dict) -> None:
        import json

        raw = json.dumps(body).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/healthz":
            ok = NAS_ROOT.is_dir() and os.access(NAS_ROOT, os.R_OK | os.W_OK)
            self._json(200 if ok else 503, {"ok": ok, "nasRoot": str(NAS_ROOT)})
            return
        if not _authorized(self.headers):
            self._json(401, {"error": "unauthorized"})
            return
        self._json(404, {"error": "not found"})

    def do_PUT(self) -> None:  # noqa: N802
        if not _authorized(self.headers):
            self._json(401, {"error": "unauthorized"})
            return
        prefix = "/v1/proj/"
        if not self.path.startswith(prefix):
            self._json(404, {"error": "not found"})
            return
        rest = self.path[len(prefix) :]
        parts = rest.split("/", 2)
        if len(parts) < 3 or parts[1] != "home":
            self._json(400, {"error": "expected /v1/proj/{id}/home/{path}"})
            return
        proj_id, _, rel = parts[0], parts[1], parts[2]
        try:
            target = _safe_rel(f"proj_{proj_id}/home/{rel}")
        except ValueError:
            self._json(400, {"error": "invalid path"})
            return
        length = int(self.headers.get("Content-Length", "0"))
        data = self.rfile.read(length)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        self._json(200, {"written": str(target.relative_to(NAS_ROOT))})


def main() -> None:
    server = ThreadingHTTPServer((LISTEN_HOST, LISTEN_PORT), Handler)
    print(f"claw-nas-api listening http://{LISTEN_HOST}:{LISTEN_PORT}/", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
