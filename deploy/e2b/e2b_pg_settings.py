#!/usr/bin/env python3
"""Persist gateway_global_settings from e2b scripts via CLAW_GATEWAY_DATABASE_URL. Author: kejiqing"""
from __future__ import annotations

import json
import os
import time
from typing import Any


def database_url() -> str:
    for key in ("CLAW_E2B_WORKER_DATABASE_URL", "CLAW_GATEWAY_DATABASE_URL"):
        val = os.environ.get(key, "").strip()
        if val:
            return val
    raise RuntimeError("set CLAW_GATEWAY_DATABASE_URL (or CLAW_E2B_WORKER_DATABASE_URL) in .env")


def merge_settings_json_key(key: str, patch: dict[str, Any], *, now_ms: int | None = None) -> None:
    """Merge `patch` into settings_json[key] on gateway_global_settings singleton row."""
    try:
        import psycopg
    except ImportError as exc:
        raise RuntimeError(
            "psycopg missing in e2b venv; re-run script (venv install) or: pip install 'psycopg[binary]'"
        ) from exc

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
WHERE singleton_id = 1
"""
    with psycopg.connect(database_url()) as conn:
        with conn.cursor() as cur:
            cur.execute(sql, (patch_json, ts))
            if cur.rowcount == 0:
                raise RuntimeError("gateway_global_settings row missing (singleton_id=1)")
        conn.commit()
