#!/usr/bin/env python3
"""Extract files/trees from a registry image (no local podman/docker). Author: kejiqing"""
from __future__ import annotations

import base64
import gzip
import io
import json
import os
import tarfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Literal, Tuple, Union

# Staged entry: file bytes, symlink target, or directory marker. Author: kejiqing
_Entry = Union[
    Tuple[Literal["file"], bytes, int],
    Tuple[Literal["lnk"], str],
    Tuple[Literal["dir"]],
]


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def parse_image_ref(image_ref: str) -> tuple[str, str, str]:
    """Return (registry_host, repository, tag_or_digest)."""
    ref = image_ref.strip()
    if not ref:
        raise ValueError("empty image ref")
    if "@" in ref:
        name, digest = ref.rsplit("@", 1)
        tag = f"@{digest}"
    elif ":" in ref.rsplit("/", 1)[-1]:
        name, tag = ref.rsplit(":", 1)
    else:
        name, tag = ref, "latest"
    if "/" not in name:
        return "docker.io", f"library/{name}", tag
    registry, repo = name.split("/", 1)
    if "." not in registry and ":" not in registry and registry != "localhost":
        return "docker.io", name, tag
    return registry, repo, tag


def _docker_config_paths() -> list[Path]:
    paths: list[Path] = []
    for key in ("CLAW_DOCKER_CONFIG", "DOCKER_CONFIG"):
        raw = _env(key)
        if not raw:
            continue
        p = Path(raw)
        paths.append(p if p.name == "config.json" else p / "config.json")
    paths.extend(
        [
            Path("/run/claw/docker-config.json"),
            Path("/run/claw/claw/docker-config.json"),
            Path.home() / ".docker" / "config.json",
            Path.home() / ".config" / "containers" / "auth.json",
        ]
    )
    return paths


def registry_basic_auth(registry: str) -> tuple[str, str] | None:
    user = _env("ACR_USERNAME") or _env("ACR_USER")
    password = _env("ACR_PASSWORD") or _env("ACR_PASSWORK")
    if user and password:
        return user, password
    for path in _docker_config_paths():
        if not path.is_file():
            continue
        try:
            cfg = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        auths = cfg.get("auths") or {}
        for host in (registry, f"https://{registry}", f"http://{registry}"):
            entry = auths.get(host) or auths.get(host.rstrip("/"))
            if not entry:
                continue
            auth = entry.get("auth")
            if not auth:
                continue
            try:
                decoded = base64.b64decode(auth).decode("utf-8")
            except (ValueError, UnicodeDecodeError):
                continue
            if ":" not in decoded:
                continue
            u, _, p = decoded.partition(":")
            if u and p:
                return u, p
    return None


def _http_json(url: str, headers: dict[str, str]) -> tuple[dict | list, dict[str, str]]:
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            body = resp.read()
            return json.loads(body.decode("utf-8")), {k.lower(): v for k, v in resp.headers.items()}
    except urllib.error.HTTPError as exc:
        err_body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {exc.code} for {url}: {err_body[:400]}") from exc


def _http_bytes(url: str, headers: dict[str, str]) -> bytes:
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=300) as resp:
            return resp.read()
    except urllib.error.HTTPError as exc:
        err_body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {exc.code} for {url}: {err_body[:400]}") from exc


def _bearer_token(registry: str, repository: str, www_auth: str, basic: tuple[str, str] | None) -> str:
    # Bearer realm="...",service="...",scope="..."
    raw = www_auth
    if raw.lower().startswith("bearer "):
        raw = raw[7:]
    parts: dict[str, str] = {}
    for item in raw.split(","):
        if "=" not in item:
            continue
        k, v = item.split("=", 1)
        parts[k.strip()] = v.strip().strip('"')
    realm = parts.get("realm")
    if not realm:
        raise RuntimeError(f"missing realm in Www-Authenticate: {www_auth!r}")
    query = {
        "service": parts.get("service", ""),
        "scope": parts.get("scope", f"repository:{repository}:pull"),
    }
    url = f"{realm}?{urllib.parse.urlencode(query)}"
    headers: dict[str, str] = {}
    if basic:
        token = base64.b64encode(f"{basic[0]}:{basic[1]}".encode()).decode()
        headers["Authorization"] = f"Basic {token}"
    data, _ = _http_json(url, headers)
    out = data.get("token") or data.get("access_token")
    if not out:
        raise RuntimeError(f"token endpoint returned no token: {data!r}")
    return str(out)


def _auth_headers(registry: str, repository: str) -> dict[str, str]:
    basic = registry_basic_auth(registry)
    # Probe for challenge
    url = f"https://{registry}/v2/{repository}/manifests/latest"
    req = urllib.request.Request(url)
    try:
        urllib.request.urlopen(req, timeout=30)
        return {}
    except urllib.error.HTTPError as exc:
        www = exc.headers.get("Www-Authenticate") or ""
        if exc.code in (401, 403) and www.lower().startswith("bearer"):
            token = _bearer_token(registry, repository, www, basic)
            return {"Authorization": f"Bearer {token}"}
        if basic:
            token = base64.b64encode(f"{basic[0]}:{basic[1]}".encode()).decode()
            return {"Authorization": f"Basic {token}"}
        raise RuntimeError(f"registry auth failed for {registry}/{repository}: HTTP {exc.code}") from exc


def _pick_platform_manifest(index: dict, platform: str) -> str:
    want_os, _, want_arch = platform.partition("/")
    want_os = want_os or "linux"
    want_arch = want_arch or "amd64"
    for m in index.get("manifests") or []:
        plat = m.get("platform") or {}
        if plat.get("os") == want_os and plat.get("architecture") == want_arch:
            digest = m.get("digest")
            if digest:
                return str(digest)
    raise RuntimeError(f"no manifest for platform={platform!r} in index")


def _manifest_for_image(registry: str, repository: str, tag: str, platform: str, headers: dict[str, str]) -> dict:
    ref = tag[1:] if tag.startswith("@") else tag
    path = f"https://{registry}/v2/{repository}/manifests/{urllib.parse.quote(ref, safe=':@')}"
    accept = (
        "application/vnd.oci.image.index.v1+json,"
        "application/vnd.docker.distribution.manifest.list.v2+json,"
        "application/vnd.oci.image.manifest.v1+json,"
        "application/vnd.docker.distribution.manifest.v2+json"
    )
    hdrs = {**headers, "Accept": accept}
    manifest, _ = _http_json(path, hdrs)
    media = str(manifest.get("mediaType") or "")
    if "manifest.list" in media or "image.index" in media or "manifests" in manifest:
        digest = _pick_platform_manifest(manifest, platform)
        path2 = f"https://{registry}/v2/{repository}/manifests/{digest}"
        manifest, _ = _http_json(path2, hdrs)
    return manifest


def _ungzip_if_needed(blob: bytes) -> bytes:
    if len(blob) >= 2 and blob[0] == 0x1F and blob[1] == 0x8B:
        return gzip.decompress(blob)
    return blob


def _normalize_tar_name(name: str) -> str:
    return name.lstrip("./").lstrip("/")


def _iter_image_layers(
    image_ref: str,
    *,
    platform: str = "linux/amd64",
) -> tuple[str, str, str, dict[str, str], list[dict]]:
    registry, repository, tag = parse_image_ref(image_ref)
    headers = _auth_headers(registry, repository)
    manifest = _manifest_for_image(registry, repository, tag, platform, headers)
    layers = list(manifest.get("layers") or [])
    if not layers:
        raise RuntimeError(f"manifest has no layers for {image_ref!r}")
    return registry, repository, tag, headers, layers


def _rel_under_prefix(norm: str, prefix: str) -> str | None:
    if not prefix:
        return norm
    if norm == prefix:
        return ""
    if norm.startswith(prefix + "/"):
        return norm[len(prefix) + 1 :]
    return None


def _drop_entry_tree(entries: dict[str, _Entry], rel: str) -> None:
    """Remove rel and any children from a staged tree. Author: kejiqing"""
    if rel == "":
        entries.clear()
        return
    entries.pop(rel, None)
    child = rel + "/"
    for key in list(entries):
        if key.startswith(child):
            del entries[key]


def _apply_whiteout_to_tree(entries: dict[str, _Entry], prefix: str, norm: str) -> bool:
    """Apply AUFS whiteout into staged tree keyed by paths relative to prefix. Author: kejiqing"""
    base = Path(norm).name
    if not base.startswith(".wh."):
        return False
    parent_norm = str(Path(norm).parent).replace("\\", "/")
    if parent_norm in (".", ""):
        parent_norm = ""
    parent_rel = _rel_under_prefix(parent_norm, prefix) if parent_norm else (
        "" if not prefix else None
    )
    # Whiteout file itself must live under prefix (parent is under or is prefix).
    if parent_rel is None:
        if parent_norm == prefix:
            parent_rel = ""
        else:
            return False

    if base == ".wh..wh..opq":
        _drop_entry_tree(entries, parent_rel)
        return True

    deleted = base[len(".wh.") :]
    target_rel = deleted if parent_rel == "" else f"{parent_rel}/{deleted}"
    _drop_entry_tree(entries, target_rel)
    return True


def extract_file_from_image(
    image_ref: str,
    container_path: str,
    dest: Path,
    *,
    platform: str = "linux/amd64",
) -> Path:
    """Download layers until container_path is found; write to dest. Author: kejiqing"""
    registry, repository, tag, headers, layers = _iter_image_layers(
        image_ref, platform=platform
    )
    print(
        f"==> registry extract {container_path!r} from {registry}/{repository}:{tag} ({platform})",
        flush=True,
    )

    target = _normalize_tar_name(container_path)
    found: bytes | None = None
    for layer in layers:
        digest = layer.get("digest")
        if not digest:
            continue
        blob_url = f"https://{registry}/v2/{repository}/blobs/{digest}"
        raw = _http_bytes(blob_url, headers)
        data = _ungzip_if_needed(raw)
        try:
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:*") as tar:
                try:
                    member = tar.getmember(target)
                except KeyError:
                    alt = None
                    for name in tar.getnames():
                        if _normalize_tar_name(name) == target:
                            alt = name
                            break
                    if alt is None:
                        continue
                    member = tar.getmember(alt)
                extracted = tar.extractfile(member)
                if extracted is None:
                    continue
                found = extracted.read()
        except tarfile.TarError:
            continue
        if found is not None:
            break

    if found is None:
        raise RuntimeError(f"file {container_path!r} not found in {image_ref!r}")

    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(found)
    dest.chmod(0o755)
    print(f"==> wrote {dest} ({len(found)} bytes)", flush=True)
    return dest


def extract_paths_from_image(
    image_ref: str,
    path_map: dict[str, Path],
    *,
    platform: str = "linux/amd64",
    required_prefixes: set[str] | None = None,
) -> dict[str, int]:
    """Extract one or more directory trees from image layers into dest dirs.

    path_map keys are absolute container paths (e.g. /home/.openvscode-server).
    Later layers override earlier; AUFS whiteouts are applied. Author: kejiqing
    """
    import shutil

    if not path_map:
        raise ValueError("path_map is empty")
    registry, repository, tag, headers, layers = _iter_image_layers(
        image_ref, platform=platform
    )
    prefixes = {src.strip("/"): dest for src, dest in path_map.items()}
    print(
        f"==> registry extract trees {sorted(prefixes)} from "
        f"{registry}/{repository}:{tag} ({platform})",
        flush=True,
    )

    trees: dict[str, dict[str, _Entry]] = {p: {} for p in prefixes}

    for layer in layers:
        digest = layer.get("digest")
        if not digest:
            continue
        blob_url = f"https://{registry}/v2/{repository}/blobs/{digest}"
        raw = _http_bytes(blob_url, headers)
        data = _ungzip_if_needed(raw)
        try:
            with tarfile.open(fileobj=io.BytesIO(data), mode="r:*") as tar:
                for member in tar.getmembers():
                    norm = _normalize_tar_name(member.name)
                    if not norm:
                        continue
                    for prefix, entries in trees.items():
                        if Path(norm).name.startswith(".wh."):
                            _apply_whiteout_to_tree(entries, prefix, norm)
                            continue
                        rel = _rel_under_prefix(norm, prefix)
                        if rel is None:
                            continue
                        if member.isdir():
                            if rel:
                                entries[rel] = ("dir",)
                            continue
                        if member.issym():
                            entries[rel] = ("lnk", member.linkname)
                            continue
                        if member.islnk() or not member.isfile():
                            continue
                        extracted = tar.extractfile(member)
                        if extracted is None:
                            continue
                        mode = member.mode & 0o777 if member.mode else 0o644
                        entries[rel] = ("file", extracted.read(), mode)
        except tarfile.TarError as exc:
            print(f"warn: skip corrupt layer {digest}: {exc}", flush=True)
            continue

    must = required_prefixes if required_prefixes is not None else set(prefixes)
    counts: dict[str, int] = {}
    for prefix, dest in prefixes.items():
        entries = trees[prefix]
        file_count = sum(1 for v in entries.values() if v[0] == "file")
        counts[prefix] = file_count
        if prefix in must and file_count == 0:
            raise RuntimeError(
                f"tree {prefix!r} not found (or empty) in {image_ref!r}"
            )
        if dest.exists():
            shutil.rmtree(dest)
        dest.mkdir(parents=True, exist_ok=True)
        for rel in sorted(entries.keys(), key=lambda s: (s.count("/"), s)):
            entry = entries[rel]
            path = dest / rel if rel else dest
            if entry[0] == "dir":
                path.mkdir(parents=True, exist_ok=True)
            elif entry[0] == "lnk":
                path.parent.mkdir(parents=True, exist_ok=True)
                if path.exists() or path.is_symlink():
                    path.unlink()
                path.symlink_to(entry[1])
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(entry[1])
                path.chmod(entry[2] or 0o644)
        print(
            f"==> wrote tree {prefix!r} → {dest} ({file_count} files)",
            flush=True,
        )
    return counts


def extract_tree_from_image(
    image_ref: str,
    container_dir: str,
    dest: Path,
    *,
    platform: str = "linux/amd64",
    required: bool = True,
) -> Path:
    """Extract a directory tree from image into dest. Author: kejiqing"""
    prefix = container_dir.strip("/")
    extract_paths_from_image(
        image_ref,
        {container_dir: dest},
        platform=platform,
        required_prefixes={prefix} if required else set(),
    )
    return dest


if __name__ == "__main__":
    import sys

    if len(sys.argv) == 4:
        extract_file_from_image(sys.argv[1], sys.argv[2], Path(sys.argv[3]))
    elif len(sys.argv) == 5 and sys.argv[1] == "--tree":
        extract_tree_from_image(sys.argv[2], sys.argv[3], Path(sys.argv[4]))
    else:
        print(
            f"usage: {sys.argv[0]} <image> <container-path> <dest-file>\n"
            f"       {sys.argv[0]} --tree <image> <container-dir> <dest-dir>",
            file=sys.stderr,
        )
        raise SystemExit(2)
