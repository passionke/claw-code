#!/usr/bin/env python3
"""Persist gateway_global_settings from e2b scripts via CLAW_GATEWAY_DATABASE_URL. Author: kejiqing"""
from __future__ import annotations

import json
import os
import time
from typing import Any
from urllib.parse import quote, urlparse, urlunparse


def database_url() -> str:
    for key in ("CLAW_E2B_WORKER_DATABASE_URL", "CLAW_GATEWAY_DATABASE_URL"):
        val = os.environ.get(key, "").strip()
        if val:
            return val
    raise RuntimeError("set CLAW_GATEWAY_DATABASE_URL (or CLAW_E2B_WORKER_DATABASE_URL) in .env")


def sandbox_database_url() -> str:
    """PG URL injected into e2b sandboxes (remote host cannot use Mac 127.0.0.1). Author: kejiqing"""
    explicit = os.environ.get("CLAW_E2B_SANDBOX_DATABASE_URL", "").strip()
    if explicit:
        return explicit

    url = database_url()
    parsed = urlparse(url)
    host = (parsed.hostname or "").lower()
    if host not in ("127.0.0.1", "localhost", "::1"):
        return url

    sandbox_host = (
        os.environ.get("CLAW_E2B_SANDBOX_PG_HOST", "").strip()
        or os.environ.get("CLAW_POOL_ADVERTISE_HOST", "").strip()
    )
    if not sandbox_host:
        raise RuntimeError(
            "CLAW_E2B_WORKER_DATABASE_URL uses 127.0.0.1 but e2b sandboxes run on another host; "
            "set CLAW_E2B_SANDBOX_DATABASE_URL or CLAW_E2B_SANDBOX_PG_HOST (Mac LAN IP, e.g. 10.22.11.185)"
        )

    user = quote(parsed.username or "", safe="")
    password = quote(parsed.password or "", safe="")
    port = parsed.port or 5432
    auth = f"{user}:{password}@" if parsed.username else ""
    netloc = f"{auth}{sandbox_host}:{port}"
    return urlunparse(parsed._replace(netloc=netloc))


def cluster_id() -> str:
    """Gateway cluster scope (PK on gateway_global_settings after phase-2 migration). Author: kejiqing"""
    return os.environ.get("CLAW_CLUSTER_ID", "").strip() or "default"


def _ensure_settings_row(cur, cid: str) -> None:
    cur.execute(
        """
        INSERT INTO gateway_global_settings (cluster_id, settings_json, git_pat_tokens_json, updated_at_ms)
        VALUES (%s, '{"gitPats":[]}'::jsonb, '{}'::jsonb, 0)
        ON CONFLICT (cluster_id) DO NOTHING
        """,
        (cid,),
    )


def load_settings_json_key(key: str) -> dict[str, Any]:
    """Read settings_json[key] for CLAW_CLUSTER_ID from gateway_global_settings."""
    try:
        import psycopg
    except ImportError as exc:
        raise RuntimeError(
            "psycopg missing in e2b venv; re-run script (venv install) or: pip install 'psycopg[binary]'"
        ) from exc

    cid = cluster_id()
    sql = f"""
SELECT COALESCE(settings_json->'{key}', '{{}}'::jsonb)
FROM gateway_global_settings
WHERE cluster_id = %s
"""
    with psycopg.connect(database_url()) as conn:
        with conn.cursor() as cur:
            cur.execute(sql, (cid,))
            row = cur.fetchone()
            if not row:
                raise RuntimeError(f"gateway_global_settings row missing (cluster_id={cid!r})")
            val = row[0]
            if isinstance(val, dict):
                return val
            if isinstance(val, str):
                return json.loads(val) if val.strip() else {}
            return {}


def merge_settings_json_key(key: str, patch: dict[str, Any], *, now_ms: int | None = None) -> None:
    """Merge `patch` into settings_json[key] for CLAW_CLUSTER_ID."""
    try:
        import psycopg
    except ImportError as exc:
        raise RuntimeError(
            "psycopg missing in e2b venv; re-run script (venv install) or: pip install 'psycopg[binary]'"
        ) from exc

    cid = cluster_id()
    ts = now_ms if now_ms is not None else int(time.time() * 1000)
    patch_json = json.dumps(patch, ensure_ascii=False)
    sql = f"""
UPDATE gateway_global_settings SET
  settings_json = jsonb_set(
    COALESCE(settings_json, '{{}}'::jsonb),
    '{{{key}}}',
    COALESCE(settings_json->'{key}', '{{}}'::jsonb) || %s::jsonb,
    true
  ),
  updated_at_ms = %s
WHERE cluster_id = %s
"""
    with psycopg.connect(database_url()) as conn:
        with conn.cursor() as cur:
            _ensure_settings_row(cur, cid)
            cur.execute(sql, (patch_json, ts, cid))
            if cur.rowcount == 0:
                raise RuntimeError(f"gateway_global_settings row missing (cluster_id={cid!r})")
        conn.commit()
