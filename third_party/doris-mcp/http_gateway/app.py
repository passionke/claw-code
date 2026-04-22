#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""HTTP gateway for claw prompt with datasource binding. Author: kejiqing"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
import uuid
from pathlib import Path
from typing import Any, Optional

import yaml
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

APP = FastAPI(title="claw-code HTTP Gateway", version="0.1.0")

CLAW_BIN = os.getenv("CLAW_BIN", "claw")
DS_REGISTRY_PATH = Path(os.getenv("CLAW_DS_REGISTRY", "/app/http_gateway/config/datasources.yaml"))
WORK_ROOT = Path(os.getenv("CLAW_WORK_ROOT", "/var/lib/claw-runs"))
DORIS_MCP_IMAGE = os.getenv("DORIS_MCP_IMAGE", "ghcr.io/passionke/claw-code:latest")
DEFAULT_DORIS_MCP_COMMAND = os.getenv("DORIS_MCP_COMMAND", "node")
DEFAULT_DORIS_MCP_ARGS = os.getenv("DORIS_MCP_ARGS", "/app/dist/index.js")
DEFAULT_TIMEOUT_SECONDS = int(os.getenv("CLAW_HTTP_TIMEOUT_SECONDS", "240"))


class SolveRequest(BaseModel):
    dsId: int = Field(..., ge=1, description="Datasource ID")
    userPrompt: str = Field(..., min_length=1, description="User natural-language prompt")
    model: Optional[str] = Field(default=None, description="Optional claw model override")
    timeoutSeconds: Optional[int] = Field(default=None, ge=10, le=1800)


class SolveResponse(BaseModel):
    requestId: str
    dsId: int
    workDir: str
    durationMs: int
    clawExitCode: int
    outputText: str
    outputJson: Optional[dict[str, Any]] = None


def _load_registry() -> dict[str, Any]:
    if not DS_REGISTRY_PATH.exists():
        raise HTTPException(
            status_code=500,
            detail=f"Datasource registry not found: {DS_REGISTRY_PATH}",
        )
    try:
        data = yaml.safe_load(DS_REGISTRY_PATH.read_text(encoding="utf-8")) or {}
    except Exception as exc:
        raise HTTPException(status_code=500, detail=f"Invalid registry yaml: {exc}") from exc
    if not isinstance(data, dict) or not isinstance(data.get("datasources"), dict):
        raise HTTPException(
            status_code=500,
            detail="Registry yaml must contain top-level 'datasources' mapping.",
        )
    return data["datasources"]


def _resolve_ds_config(ds_id: int) -> dict[str, Any]:
    datasources = _load_registry()
    raw = datasources.get(str(ds_id))
    if raw is None:
        raw = datasources.get(ds_id)
    if not isinstance(raw, dict):
        raise HTTPException(status_code=404, detail=f"Datasource dsId={ds_id} not configured.")
    required = ("host", "port", "user", "password", "default_database")
    missing = [k for k in required if raw.get(k) in (None, "")]
    if missing:
        raise HTTPException(
            status_code=400,
            detail=f"Datasource dsId={ds_id} missing required fields: {', '.join(missing)}",
        )
    return raw


def _build_doris_cluster_config(ds_id: int, ds_cfg: dict[str, Any]) -> dict[str, Any]:
    cluster_name = f"ds_{ds_id}"
    cluster = {
        "host": ds_cfg["host"],
        "port": int(ds_cfg["port"]),
        "user": ds_cfg["user"],
        "password": ds_cfg["password"],
        "default_database": ds_cfg["default_database"],
        "ssl": bool(ds_cfg.get("ssl", False)),
    }
    allowed_tables = ds_cfg.get("allowed_tables")
    if isinstance(allowed_tables, list):
        cluster["allowed_tables"] = [str(x).strip() for x in allowed_tables if str(x).strip()]
    env_map = ds_cfg.get("env")
    if isinstance(env_map, dict):
        cluster["env"] = {str(k): str(v) for k, v in env_map.items()}
    return {"clusters": {cluster_name: cluster}}


def _build_claw_settings(doris_cfg_path: Path) -> dict[str, Any]:
    mcp_args = [x.strip() for x in DEFAULT_DORIS_MCP_ARGS.split(" ") if x.strip()]
    return {
        "mcpServers": {
            "doris": {
                "type": "stdio",
                "command": DEFAULT_DORIS_MCP_COMMAND,
                "args": mcp_args,
                "env": {"DORIS_CONFIG": str(doris_cfg_path)},
            }
        }
    }


def _run_claw_prompt(
    *,
    work_dir: Path,
    user_prompt: str,
    model: Optional[str],
    timeout_seconds: int,
) -> tuple[int, str, Optional[dict[str, Any]]]:
    cmd = [CLAW_BIN, "--output-format", "json"]
    if model and model.strip():
        cmd.extend(["--model", model.strip()])
    cmd.extend(["prompt", user_prompt])
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(work_dir),
            text=True,
            capture_output=True,
            timeout=timeout_seconds,
            check=False,
        )
    except FileNotFoundError as exc:
        raise HTTPException(status_code=500, detail=f"claw binary not found: {CLAW_BIN}") from exc
    except subprocess.TimeoutExpired as exc:
        raise HTTPException(status_code=504, detail=f"claw request timeout: {timeout_seconds}s") from exc
    raw = (proc.stdout or "").strip()
    if not raw and proc.stderr:
        raw = proc.stderr.strip()
    parsed = None
    if raw:
        try:
            parsed = json.loads(raw)
        except Exception:
            parsed = None
    return proc.returncode, raw, parsed


@APP.get("/healthz")
def healthz() -> dict[str, Any]:
    return {
        "ok": True,
        "serviceMode": os.getenv("CLAW_SERVICE_MODE", "mcp"),
        "clawBin": CLAW_BIN,
        "registryPath": str(DS_REGISTRY_PATH),
        "workRoot": str(WORK_ROOT),
        "dorisMcpCommand": DEFAULT_DORIS_MCP_COMMAND,
        "dorisMcpArgs": DEFAULT_DORIS_MCP_ARGS,
        "dorisMcpImageCompat": DORIS_MCP_IMAGE,
    }


@APP.post("/v1/solve", response_model=SolveResponse)
def solve(req: SolveRequest) -> SolveResponse:
    ds_cfg = _resolve_ds_config(req.dsId)
    request_id = uuid.uuid4().hex
    timeout_seconds = req.timeoutSeconds or DEFAULT_TIMEOUT_SECONDS
    started = time.time()

    ds_work_dir = WORK_ROOT / f"ds_{req.dsId}"
    ds_work_dir.mkdir(parents=True, exist_ok=True)
    claw_dir = ds_work_dir / ".claw"
    claw_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.NamedTemporaryFile(
        mode="w",
        suffix=".yaml",
        prefix=f"doris_{req.dsId}_",
        dir=str(ds_work_dir),
        encoding="utf-8",
        delete=False,
    ) as fp:
        doris_config_path = Path(fp.name)
        yaml.safe_dump(
            _build_doris_cluster_config(req.dsId, ds_cfg),
            fp,
            allow_unicode=True,
            sort_keys=False,
        )

    settings = _build_claw_settings(doris_config_path)
    (claw_dir / "settings.json").write_text(
        json.dumps(settings, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )

    code, out_text, out_json = _run_claw_prompt(
        work_dir=ds_work_dir,
        user_prompt=req.userPrompt.strip(),
        model=req.model,
        timeout_seconds=timeout_seconds,
    )
    duration_ms = int((time.time() - started) * 1000)

    return SolveResponse(
        requestId=request_id,
        dsId=req.dsId,
        workDir=str(ds_work_dir),
        durationMs=duration_ms,
        clawExitCode=code,
        outputText=out_text,
        outputJson=out_json,
    )
