#!/usr/bin/env python3
"""e2b self-hosted NAS bind nasConfig helpers. Author: kejiqing"""
from __future__ import annotations

import json
import urllib.error
import urllib.request
from typing import Any, Callable


def e2b_host_mount_root(
    *,
    env_get: Callable[[str, str], str],
    api_url: str,
    api_key: str,
    self_hosted: bool,
    http_json: Callable[..., Any],
) -> str:
    """Resolve e2b host bind root: CLAW_E2B_NAS_HOST_MOUNT → GET /health nas.hostMountRoot."""
    root = env_get("CLAW_E2B_NAS_HOST_MOUNT", "")
    if root:
        return root
    try:
        health = http_json("GET", f"{api_url.rstrip('/')}/health", api_key, self_hosted)
        nas = health.get("nas") or {}
        r = (nas.get("hostMountRoot") or "").strip()
        if r:
            return r
    except Exception:  # noqa: BLE001 — health is best-effort
        pass
    raise RuntimeError(
        "e2b NAS hostMountRoot not configured: set CLAW_E2B_NAS_HOST_MOUNT in .env "
        "(e2b host directory, e.g. /data/claw-nas) and ensure it exists on the e2b server; "
        "or set [nas].host_mount_root in e2bserver config.toml"
    )


def nas_bind_config_body(
    *,
    host_mount_root: str,
    mount_points: list[tuple[str, str]],
    user_id: int,
    group_id: int,
) -> dict[str, Any]:
    """Build nasConfig for e2b bind inject (hostMountRoot per mountPoint)."""
    root = host_mount_root.rstrip("/")
    return {
        "userId": user_id,
        "groupId": group_id,
        "mountPoints": [
            {"relPath": rel, "mountDir": mount_dir, "hostMountRoot": root}
            for rel, mount_dir in mount_points
        ],
    }


def http_json_selfhosted(
    method: str,
    url: str,
    api_key: str,
    self_hosted: bool,
    body: dict[str, Any] | None = None,
) -> Any:
    headers = (
        {"X-API-Key": api_key, "Content-Type": "application/json"}
        if self_hosted
        else {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}
    )
    data = None if body is None else json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    with urllib.request.urlopen(req, timeout=120) as resp:
        raw = resp.read().decode("utf-8")
        return json.loads(raw) if raw.strip() else {}
