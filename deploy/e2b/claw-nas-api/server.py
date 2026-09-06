#!/usr/bin/env python3
# Gateway-dedicated NAS layout + file API (e2b singleton at /claw_ws). Author: kejiqing
"""claw-nas-api — HTTP service bound to NAS export root at /claw_ws."""

from __future__ import annotations

import json
import os
import secrets
import subprocess
import tarfile
import tempfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import shutil
from pathlib import Path
from urllib.parse import unquote

NAS_ROOT = Path(os.environ.get("CLAW_NAS_API_ROOT", "/claw_ws"))
LISTEN_HOST = os.environ.get("CLAW_NAS_API_LISTEN_HOST", "0.0.0.0")
LISTEN_PORT = int(os.environ.get("CLAW_NAS_API_LISTEN_PORT", "8090"))
INTERNAL_TOKEN = os.environ.get("CLAW_GATEWAY_INTERNAL_TOKEN", "").strip()
# Git import limits (gateway packs clone tree → chunked tar.gz upload). Author: kejiqing
MAX_GIT_IMPORT_FILE_BYTES = int(
    os.environ.get("CLAW_NAS_API_MAX_GIT_IMPORT_FILE_BYTES", str(64 * 1024 * 1024))
)
MAX_GIT_IMPORT_UPLOAD_PART_BYTES = int(
    os.environ.get(
        "CLAW_NAS_API_MAX_GIT_IMPORT_UPLOAD_PART_BYTES", str(64 * 1024 * 1024)
    )
)


def _authorized(headers) -> bool:
    if not INTERNAL_TOKEN:
        return True
    auth = headers.get("Authorization", "")
    if auth.startswith("Bearer "):
        return secrets.compare_digest(auth[7:].strip(), INTERNAL_TOKEN)
    return secrets.compare_digest(headers.get("X-Claw-Internal-Token", "").strip(), INTERNAL_TOKEN)


_RESERVED_HOME_NAMES = {"project_home_def", ".claw", ".vscode", ".git", "home"}


def _git_import_dest_or_raise(rel: str) -> Path:
    """Allow recursive rmdir only for `{cluster}/proj_N/home/{destRel}` (one dest segment). Author: kejiqing"""
    parts = [p for p in rel.strip("/").split("/") if p]
    if len(parts) != 4:
        raise ValueError("rmdir only allowed for {cluster}/proj_N/home/{destRel}")
    _cluster, proj, home, dest = parts
    if not proj.startswith("proj_") or home != "home":
        raise ValueError("rmdir path must be {cluster}/proj_N/home/{destRel}")
    if dest in _RESERVED_HOME_NAMES or dest in {".", ".."}:
        raise ValueError(f"refusing to delete reserved home path: {dest}")
    return _safe_rel(rel)


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


def _atomic_symlink(link_path: Path, link_target: str) -> None:
    """Create or replace a symlink atomically (tmp link + os.replace). Author: kejiqing"""
    if link_path.exists() or link_path.is_symlink():
        if link_path.is_dir() and not link_path.is_symlink():
            raise ValueError("refusing to replace directory with symlink")
    link_path.parent.mkdir(parents=True, exist_ok=True)
    tmp_link = link_path.with_name(f".{link_path.name}.tmp-{secrets.token_hex(8)}")
    try:
        tmp_link.symlink_to(link_target)
        os.replace(tmp_link, link_path)
    finally:
        if tmp_link.exists() or tmp_link.is_symlink():
            tmp_link.unlink()


def _git_import_upload_id_or_raise(upload_id: str) -> str:
    token = upload_id.strip()
    if not token or not all(c.isalnum() or c in "-_" for c in token):
        raise ValueError("invalid uploadId")
    if len(token) > 128:
        raise ValueError("uploadId too long")
    return token


def _git_import_upload_staging(upload_id: str) -> Path:
    uid = _git_import_upload_id_or_raise(upload_id)
    return NAS_ROOT / ".claw" / "git-import-upload" / uid


def _safe_tar_member_name(name: str) -> str:
    """Reject absolute paths and `..` segments in tar entries. Author: kejiqing"""
    raw = name.replace("\\", "/").strip()
    while raw.startswith("./"):
        raw = raw[2:]
    raw = raw.lstrip("/")
    parts = [p for p in raw.split("/") if p and p != "."]
    if not parts or any(part == ".." for part in parts):
        raise ValueError(f"unsafe tar member path: {name!r}")
    return "/".join(parts)


def _extract_git_import_tar_from_path(dest_dir: Path, tar_gz_path: Path) -> int:
    """Replace `{cluster}/proj_N/home/{destRel}/` via system `tar -xzf`. Author: kejiqing"""
    list_proc = subprocess.run(
        ["tar", "-tzf", str(tar_gz_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    if list_proc.returncode != 0:
        raise ValueError(
            f"tar list failed: {(list_proc.stderr or list_proc.stdout).strip()}"
        )
    for raw in list_proc.stdout.splitlines():
        name = raw.rstrip("/")
        if not name:
            continue
        _safe_tar_member_name(name)
    if dest_dir.exists() or dest_dir.is_symlink():
        if dest_dir.is_dir() and not dest_dir.is_symlink():
            shutil.rmtree(dest_dir)
        else:
            dest_dir.unlink()
    dest_dir.parent.mkdir(parents=True, exist_ok=True)
    dest_dir.mkdir(parents=True, exist_ok=True)
    extract_proc = subprocess.run(
        ["tar", "-xzf", str(tar_gz_path), "-C", str(dest_dir)],
        capture_output=True,
        text=True,
        check=False,
    )
    if extract_proc.returncode != 0:
        raise ValueError(
            f"tar extract failed: {(extract_proc.stderr or extract_proc.stdout).strip()}"
        )
    dest_resolved = dest_dir.resolve()
    written = 0
    for path in dest_dir.rglob("*"):
        if not path.is_file():
            continue
        if not path.resolve().is_relative_to(dest_resolved):
            shutil.rmtree(dest_dir, ignore_errors=True)
            raise ValueError(f"unsafe tar member escaped dest: {path}")
        if path.stat().st_size > MAX_GIT_IMPORT_FILE_BYTES:
            shutil.rmtree(dest_dir, ignore_errors=True)
            raise ValueError(f"tar member too large: {path}")
        written += 1
    return written


def _extract_git_import_tar(dest_dir: Path, tar_gz: bytes) -> int:
    """Replace dest from in-memory gzip tar (small uploads / tests). Author: kejiqing"""
    with tempfile.NamedTemporaryFile(prefix="git-import-", suffix=".tar.gz", delete=False) as tmp:
        tmp.write(tar_gz)
        tmp_path = Path(tmp.name)
    try:
        return _extract_git_import_tar_from_path(dest_dir, tmp_path)
    finally:
        tmp_path.unlink(missing_ok=True)


def _finish_git_import_parts(dest_rel: str, upload_id: str, parts: int) -> dict:
    if parts < 1:
        raise ValueError("parts must be >= 1")
    dest = _git_import_dest_or_raise(dest_rel)
    staging = _git_import_upload_staging(upload_id)
    if not staging.is_dir():
        raise ValueError(f"upload staging not found: {upload_id}")
    bundle = staging / "bundle.tar.gz"
    try:
        with bundle.open("wb") as out:
            for i in range(parts):
                part = staging / f"part-{i:04d}"
                if not part.is_file():
                    raise ValueError(f"missing upload part {i}")
                with part.open("rb") as src:
                    shutil.copyfileobj(src, out)
        extracted = _extract_git_import_tar_from_path(dest, bundle)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    return {
        "relPath": dest_rel,
        "uploadId": upload_id,
        "parts": parts,
        "extracted": extracted,
        "written": str(dest.relative_to(NAS_ROOT)),
    }


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
        if self.path == "/v1/rmdir":
            try:
                body = _read_json(self)
                rel = str(body.get("relPath", "")).strip()
                recursive = bool(body.get("recursive", False))
                target = _git_import_dest_or_raise(rel)
                if not target.exists() and not target.is_symlink():
                    _json(self, 200, {"relPath": rel, "removed": False})
                    return
                if target.is_dir() and not target.is_symlink():
                    if not recursive:
                        raise ValueError("directory delete requires recursive=true")
                    shutil.rmtree(target)
                else:
                    target.unlink()
                _json(self, 200, {"relPath": rel, "removed": True})
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
                _atomic_symlink(link_path, link_target)
                _json(self, 200, {"relPath": rel, "target": link_target, "linked": True})
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
            return
        if self.path == "/v1/git-import-finish":
            try:
                body = _read_json(self)
                dest_rel = str(body.get("destRelPath", "")).strip()
                upload_id = str(body.get("uploadId", "")).strip()
                parts = int(body.get("parts", 0))
                result = _finish_git_import_parts(dest_rel, upload_id, parts)
                _json(self, 200, result)
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
        parts_prefix = "/v1/git-import-parts/"
        if self.path.startswith(parts_prefix):
            rest = self.path[len(parts_prefix) :]
            segments = rest.split("/", 1)
            if len(segments) != 2:
                _json(self, 400, {"error": "expected /v1/git-import-parts/{uploadId}/{partIndex}"})
                return
            upload_id, part_raw = segments[0], segments[1]
            try:
                part_index = int(part_raw)
                if part_index < 0:
                    raise ValueError("partIndex must be >= 0")
                uid = _git_import_upload_id_or_raise(upload_id)
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
                return
            length = int(self.headers.get("Content-Length", "0"))
            if length <= 0:
                _json(self, 400, {"error": "empty upload part body"})
                return
            if length > MAX_GIT_IMPORT_UPLOAD_PART_BYTES:
                _json(
                    self,
                    400,
                    {"error": f"upload part exceeds {MAX_GIT_IMPORT_UPLOAD_PART_BYTES} bytes"},
                )
                return
            staging = _git_import_upload_staging(uid)
            staging.mkdir(parents=True, exist_ok=True)
            part_path = staging / f"part-{part_index:04d}"
            try:
                data = self.rfile.read(length)
                if len(data) != length:
                    raise ValueError(
                        f"upload part body truncated: expected {length}, got {len(data)}"
                    )
                part_path.write_bytes(data)
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
                return
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
                return
            _json(
                self,
                200,
                {
                    "uploadId": uid,
                    "partIndex": part_index,
                    "bytes": length,
                    "written": str(part_path.relative_to(NAS_ROOT)),
                },
            )
            return
        if self.path.startswith("/v1/extract-tar/"):
            rel = self.path[len("/v1/extract-tar/") :]
            try:
                dest = _git_import_dest_or_raise(rel)
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
                return
            length = int(self.headers.get("Content-Length", "0"))
            if length <= 0:
                _json(self, 400, {"error": "empty tar.gz body"})
                return
            fd, tmp_name = tempfile.mkstemp(prefix="git-import-", suffix=".tar.gz")
            os.close(fd)
            tmp_path = Path(tmp_name)
            try:
                data = self.rfile.read(length)
                if len(data) != length:
                    raise ValueError(
                        f"tar.gz body truncated: expected {length}, got {len(data)}"
                    )
                tmp_path.write_bytes(data)
                files = _extract_git_import_tar_from_path(dest, tmp_path)
            except ValueError as exc:
                _json(self, 400, {"error": str(exc)})
                return
            except OSError as exc:
                _json(self, 500, {"error": str(exc)})
                return
            finally:
                tmp_path.unlink(missing_ok=True)
            _json(
                self,
                200,
                {
                    "relPath": rel,
                    "extracted": files,
                    "written": str(dest.relative_to(NAS_ROOT)),
                },
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
