"""Content fingerprint for e2b template publish. Author: kejiqing

Same digest plus an existing PG buildId means Template.build is skipped.
"""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path


def digest_parts(parts: list[tuple[str, bytes]]) -> str:
    """Stable sha256 over named blobs. Order of `parts` does not matter."""
    hasher = hashlib.sha256()
    for name, data in sorted(parts, key=lambda item: item[0]):
        hasher.update(name.encode("utf-8"))
        hasher.update(b"\0")
        hasher.update(data)
        hasher.update(b"\0")
    return hasher.hexdigest()


def digest_tree(root: Path, extras: list[tuple[str, bytes]] | None = None) -> str:
    """Hash every file under `root` plus optional labeled extras."""
    parts: list[tuple[str, bytes]] = []
    if root.is_dir():
        for path in sorted(root.rglob("*")):
            if path.is_file() and not path.is_symlink():
                rel = path.relative_to(root).as_posix()
                parts.append((rel, path.read_bytes()))
    if extras:
        parts.extend(extras)
    return digest_parts(parts)


def should_skip_publish(stored_hash: str, stored_build_id: str, digest: str) -> bool:
    """Skip only when a previous publish recorded this exact digest and a buildId."""
    stored = stored_hash.strip()
    build_id = stored_build_id.strip()
    return bool(stored) and bool(build_id) and stored == digest


def try_skip_unchanged(settings_key: str, digest: str) -> bool:
    """True when PG contentHash matches and buildId is set. Lookup failure builds."""
    try:
        from e2b_pg_settings import load_settings_json_key

        row = load_settings_json_key(settings_key)
    except Exception as exc:  # noqa: BLE001 — missing PG must not block a first publish
        print(
            f"warn: content hash lookup for {settings_key} failed ({exc}); building",
            file=sys.stderr,
        )
        return False
    stored_hash = str(row.get("contentHash") or "")
    stored_build = str(row.get("buildId") or "")
    if should_skip_publish(stored_hash, stored_build, digest):
        print(
            f"==> skip Template.build {settings_key}: content unchanged "
            f"buildId={stored_build}"
        )
        return True
    return False
