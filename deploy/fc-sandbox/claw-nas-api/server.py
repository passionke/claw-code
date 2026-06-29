#!/usr/bin/env python3
# Gateway-dedicated NAS layout + file API (e2b singleton at /claw_ws). Author: kejiqing
"""claw-nas-api — HTTP service bound to NAS export root at /claw_ws."""

from __future__ import annotations

import json
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
    if not rel or rel.startswith("/"):
        raise ValueError("empty path")
    parts = rel.split("/")
    if ".." in parts:
        raise ValueError("path traversal")
    return NAS_ROOT / rel


def _json(handler: BaseHTTPRequestHandler, code: int, body: dict) -> None:
    raw = json.dumps(body).encode("utf-8")
    handler.send_response(code)
    handler.send_header("Content-Type", "application/json; charset=utf-8")
    handler.send_header("Content-Length", str(len(raw)))
    handler.end_headers()
    handler.wfile.write(raw)


def _read_json(handler: BaseHTTPRequestHandler) -> dict:
    length = int(handler.headers.get("Content-Length", "0"))
    if length <= 0:
        return {}
    raw = handler.rfile.read(length)
    try:
        parsed = json.loads(raw.decode("utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"invalid json: {exc}") from exc
    if not isinstance(parsed, dict):
        raise ValueError("json body must be an object")
    return parsed


def _stat_rel(rel_path: str) -> dict:
    target = _safe_rel(rel_path)
    if not target.exists() and not target.is_symlink():
        return {"relPath": rel_path, "exists": False}
    meta = target.lstat()
    return {
        "relPath": rel_path,
        "exists": True,
        "isDir": target.is_dir(),
        "isSymlink": target.is_symlink(),
        "size": meta.st_size,
    }


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):  # noqa: D401
        return

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/healthz":
            ok = NAS_ROOT.is_dir() and os.access(NAS_ROOT, os.R_OK | os.W_OK)
            _json(self, 200 if ok else 503, {"ok": ok, "nasRoot": str(NAS_ROOT)})
            return
        if self.path.startswith("/v1/stat/"):
            if not _authorized(self.headers):
                _json(self, 401, {"error": "unauthorized"})
                return
            rel = self.path[len("/v1/stat/") :]
            try:
                _json(self, 200, _stat_rel(rel))
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
            return
        if self.path.startswith("/v1/files/"):
            if not _authorized(self.headers):
                _json(self, 401, {"error": "unauthorized"})
                return
            rel = self.path[len("/v1/files/") :]
            try:
                target = _safe_rel(rel)
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
                return
            if not target.is_file():
                _json(self, 404, {"error": "not found"})
                return
            try:
                data = target.read_bytes()
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if not _authorized(self.headers):
            _json(self, 401, {"error": "unauthorized"})
            return
        _json(self, 404, {"error": "not found"})

    def do_POST(self) -> None:  # noqa: N802
        if not _authorized(self.headers):
            _json(self, 401, {"error": "unauthorized"})
            return
        if self.path == "/v1/mkdir":
            try:
                body = _read_json(self)
                rel = str(body.get("relPath", "")).strip()
                parents = bool(body.get("parents", True))
                target = _safe_rel(rel)
                if parents:
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.mkdir(exist_ok=True)
                _json(self, 200, {"relPath": rel, "created": True})
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
            return
        if self.path == "/v1/symlink":
            try:
                body = _read_json(self)
                rel = str(body.get("relPath", "")).strip()
                link_target = str(body.get("target", "")).strip()
                if not link_target:
                    raise ValueError("target is required")
                link_path = _safe_rel(rel)
                if link_path.exists() or link_path.is_symlink():
                    if link_path.is_symlink() or link_path.is_file():
                        link_path.unlink()
                    elif link_path.is_dir():
                        raise ValueError("refusing to replace directory with symlink")
                link_path.parent.mkdir(parents=True, exist_ok=True)
                link_path.symlink_to(link_target)
                _json(self, 200, {"relPath": rel, "target": link_target, "linked": True})
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
            return
        _json(self, 404, {"error": "not found"})

    def do_PUT(self) -> None:  # noqa: N802
        if not _authorized(self.headers):
            _json(self, 401, {"error": "unauthorized"})
            return
        prefix = "/v1/proj/"
        if self.path.startswith(prefix):
            rest = self.path[len(prefix) :]
            parts = rest.split("/", 2)
            if len(parts) < 3 or parts[1] != "home":
                _json(self, 400, {"error": "expected /v1/proj/{id}/home/{path}"})
                return
            proj_id, _, rel = parts[0], parts[1], parts[2]
            try:
                target = _safe_rel(f"proj_{proj_id}/home/{rel}")
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
                return
            length = int(self.headers.get("Content-Length", "0"))
            data = self.rfile.read(length)
            try:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
                return
            _json(
                self,
                200,
                {"written": str(target.relative_to(NAS_ROOT))},
            )
            return
        if self.path.startswith("/v1/files/"):
            rel = self.path[len("/v1/files/") :]
            try:
                target = _safe_rel(rel)
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
                return
            length = int(self.headers.get("Content-Length", "0"))
            data = self.rfile.read(length)
            try:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
                return
            _json(self, 200, {"written": str(target.relative_to(NAS_ROOT))})
            return
        _json(self, 404, {"error": "not found"})

    def do_DELETE(self) -> None:  # noqa: N802
        if not _authorized(self.headers):
            _json(self, 401, {"error": "unauthorized"})
            return
        if not self.path.startswith("/v1/path/"):
            _json(self, 404, {"error": "not found"})
            return
        rel = self.path[len("/v1/path/") :]
        try:
            target = _safe_rel(rel)
        except ValueError as exc:
            _json(self, 400, {"error": str(exc)})
            return
        if not target.exists() and not target.is_symlink():
            _json(self, 404, {"error": "not found"})
            return
        try:
            if target.is_dir() and not target.is_symlink():
                _json(self, 400, {"error": "refusing to delete directory; use explicit API"})
                return
            target.unlink()
        except OSError as exc:
            _json(self, 500, {"error": str(exc)})
            return
        _json(self, 200, {"removed": rel})


def main() -> None:
    server = ThreadingHTTPServer((LISTEN_HOST, LISTEN_PORT), Handler)
    print(f"claw-nas-api listening http://{LISTEN_HOST}:{LISTEN_PORT}/", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
