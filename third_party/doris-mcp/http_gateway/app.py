#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""HTTP gateway for claw prompt with datasource binding. Author: kejiqing"""

from __future__ import annotations

import json
import logging
import logging.handlers
import os
import subprocess
import tempfile
import time
import uuid
import urllib.request
from base64 import b64decode
from pathlib import Path
from typing import Any, Optional

import yaml
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

APP = FastAPI(title="claw-code HTTP Gateway", version="0.1.0")

DEFAULT_LOG_FILE = os.getenv("CLAW_HTTP_LOG_FILE", "").strip()
DEFAULT_LOG_ROTATE_BYTES = int(os.getenv("CLAW_HTTP_LOG_ROTATE_BYTES", "10485760"))
DEFAULT_LOG_BACKUP_COUNT = int(os.getenv("CLAW_HTTP_LOG_BACKUP_COUNT", "5"))


def _build_logger() -> logging.Logger:
    level_name = os.getenv("CLAW_HTTP_LOG_LEVEL", "INFO").upper()
    level = getattr(logging, level_name, logging.INFO)
    logger = logging.getLogger("claw-http-gateway")
    if not logger.handlers:
        handler = logging.StreamHandler()
        formatter = logging.Formatter(
            "%(asctime)s | %(levelname)s | %(name)s | %(message)s",
            "%Y-%m-%d %H:%M:%S",
        )
        handler.setFormatter(formatter)
        logger.addHandler(handler)
        if DEFAULT_LOG_FILE:
            try:
                log_path = Path(DEFAULT_LOG_FILE)
                log_path.parent.mkdir(parents=True, exist_ok=True)
                file_handler = logging.handlers.RotatingFileHandler(
                    filename=str(log_path),
                    maxBytes=max(DEFAULT_LOG_ROTATE_BYTES, 1),
                    backupCount=max(DEFAULT_LOG_BACKUP_COUNT, 1),
                    encoding="utf-8",
                )
                file_handler.setFormatter(formatter)
                logger.addHandler(file_handler)
            except Exception as exc:
                # Keep service available even when file logging setup fails.
                logger.error("failed to initialize file logger path=%s error=%s", DEFAULT_LOG_FILE, exc)
    logger.setLevel(level)
    logger.propagate = False
    return logger


LOGGER = _build_logger()

CLAW_BIN = os.getenv("CLAW_BIN", "claw")
DS_REGISTRY_PATH = Path(os.getenv("CLAW_DS_REGISTRY", "/app/http_gateway/config/datasources.yaml"))
WORK_ROOT = Path(os.getenv("CLAW_WORK_ROOT", "/var/lib/claw-runs"))
DORIS_MCP_IMAGE = os.getenv("DORIS_MCP_IMAGE", "ghcr.io/passionke/claw-code:latest")
DEFAULT_DORIS_MCP_COMMAND = os.getenv("DORIS_MCP_COMMAND", "node")
DEFAULT_DORIS_MCP_ARGS = os.getenv("DORIS_MCP_ARGS", "/app/dist/index.js")
DEFAULT_TIMEOUT_SECONDS = int(os.getenv("CLAW_HTTP_TIMEOUT_SECONDS", "240"))
DEFAULT_DS_SOURCE = os.getenv("CLAW_DS_SOURCE", "auto").strip().lower()  # auto|sqlbot_api|sqlbot_pg|yaml
DEFAULT_LOG_PREVIEW_CHARS = int(os.getenv("CLAW_HTTP_LOG_PREVIEW_CHARS", "400"))
SQLBOT_BASE_URL = os.getenv("SQLBOT_BASE_URL", "").strip().rstrip("/")
SQLBOT_TIMEOUT_SECONDS = int(os.getenv("SQLBOT_TIMEOUT_SECONDS", "10"))
SQLBOT_API_TOKEN = os.getenv("SQLBOT_API_TOKEN", "").strip()
SQLBOT_API_COOKIE = os.getenv("SQLBOT_API_COOKIE", "").strip()
SQLBOT_PG_HOST = os.getenv("SQLBOT_PG_HOST", "").strip()
SQLBOT_PG_PORT = int(os.getenv("SQLBOT_PG_PORT", "5432"))
SQLBOT_PG_USER = os.getenv("SQLBOT_PG_USER", "").strip()
SQLBOT_PG_PASSWORD = os.getenv("SQLBOT_PG_PASSWORD", "")
SQLBOT_PG_DB = os.getenv("SQLBOT_PG_DB", "").strip()
SQLBOT_AES_KEY = os.getenv("SQLBOT_AES_KEY", "SQLBot1234567890")

LOGGER.info(
    "gateway startup service_mode=%s ds_source=%s claw_bin=%s registry=%s work_root=%s log_file=%s",
    os.getenv("CLAW_SERVICE_MODE", "mcp"),
    DEFAULT_DS_SOURCE,
    CLAW_BIN,
    DS_REGISTRY_PATH,
    WORK_ROOT,
    DEFAULT_LOG_FILE or "(stdout-only)",
)


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


def _resolve_ds_config_from_yaml(ds_id: int) -> dict[str, Any]:
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


def _aes_decrypt_sqlbot_config(encrypted: str) -> dict[str, Any]:
    try:
        from Crypto.Cipher import AES
        from Crypto.Util.Padding import unpad
    except Exception as exc:
        raise RuntimeError("pycryptodome is required for SQLBot config decryption") from exc
    cipher = AES.new(SQLBOT_AES_KEY.encode("utf-8"), AES.MODE_ECB)
    plain = unpad(cipher.decrypt(b64decode(encrypted)), AES.block_size).decode("utf-8")
    data = json.loads(plain)
    if not isinstance(data, dict):
        raise ValueError("decrypted datasource configuration is not a JSON object")
    return data


def _from_sqlbot_record(record: dict[str, Any], tables: list[str]) -> dict[str, Any]:
    conf = _aes_decrypt_sqlbot_config(str(record.get("configuration") or ""))
    user = str(conf.get("username") or conf.get("user") or "").strip()
    database = str(conf.get("dbSchema") or conf.get("database") or "").strip()
    ds_cfg = {
        "host": conf.get("host"),
        "port": int(conf.get("port") or 0),
        "user": user,
        "password": conf.get("password"),
        "default_database": database,
        "ssl": bool(conf.get("ssl", False)),
        "allowed_tables": [t for t in tables if str(t).strip()],
    }
    required = ("host", "port", "user", "password", "default_database")
    missing = [k for k in required if ds_cfg.get(k) in (None, "", 0)]
    if missing:
        raise RuntimeError(f"sqlbot record missing required config fields: {', '.join(missing)}")
    return ds_cfg


def _sqlbot_headers() -> dict[str, str]:
    headers = {"Content-Type": "application/json"}
    if SQLBOT_API_TOKEN:
        headers["Authorization"] = f"Bearer {SQLBOT_API_TOKEN}"
    if SQLBOT_API_COOKIE:
        headers["Cookie"] = SQLBOT_API_COOKIE
    return headers


def _fetch_ds_config_from_sqlbot_api(ds_id: int) -> dict[str, Any]:
    if not SQLBOT_BASE_URL:
        raise RuntimeError("SQLBOT_BASE_URL is empty")
    req_ds = urllib.request.Request(
        f"{SQLBOT_BASE_URL}/datasource/get/{ds_id}",
        method="POST",
        headers=_sqlbot_headers(),
        data=b"{}",
    )
    with urllib.request.urlopen(req_ds, timeout=SQLBOT_TIMEOUT_SECONDS) as resp:
        ds_record = json.loads(resp.read().decode("utf-8"))
    req_tables = urllib.request.Request(
        f"{SQLBOT_BASE_URL}/datasource/tableList/{ds_id}",
        method="POST",
        headers=_sqlbot_headers(),
        data=b"{}",
    )
    with urllib.request.urlopen(req_tables, timeout=SQLBOT_TIMEOUT_SECONDS) as resp:
        table_records = json.loads(resp.read().decode("utf-8"))
    table_names = [str(x.get("table_name") or "").strip() for x in (table_records or []) if x.get("checked")]
    return _from_sqlbot_record(ds_record, table_names)


def _fetch_ds_config_from_sqlbot_pg(ds_id: int) -> dict[str, Any]:
    if not (SQLBOT_PG_HOST and SQLBOT_PG_USER and SQLBOT_PG_DB):
        raise RuntimeError("SQLBOT_PG_* environment variables are incomplete")
    try:
        import psycopg2
    except Exception as exc:
        raise RuntimeError("psycopg2-binary is required for SQLBot PG fallback") from exc
    conn = psycopg2.connect(
        host=SQLBOT_PG_HOST,
        port=SQLBOT_PG_PORT,
        user=SQLBOT_PG_USER,
        password=SQLBOT_PG_PASSWORD,
        dbname=SQLBOT_PG_DB,
        connect_timeout=SQLBOT_TIMEOUT_SECONDS,
    )
    try:
        with conn.cursor() as cur:
            cur.execute("SELECT id, configuration FROM core_datasource WHERE id = %s", (ds_id,))
            row = cur.fetchone()
            if not row:
                raise RuntimeError(f"datasource id={ds_id} not found in sqlbot pg")
            cur.execute(
                "SELECT table_name FROM core_table WHERE ds_id = %s AND checked = true ORDER BY id ASC",
                (ds_id,),
            )
            tables = [r[0] for r in cur.fetchall()]
        return _from_sqlbot_record({"id": row[0], "configuration": row[1]}, tables)
    finally:
        conn.close()


def _resolve_ds_config(ds_id: int, request_id: str) -> dict[str, Any]:
    source = DEFAULT_DS_SOURCE
    errors: list[str] = []
    LOGGER.info("request=%s datasource resolve start dsId=%s source=%s", request_id, ds_id, source)
    if source in ("auto", "sqlbot_api"):
        try:
            ds_cfg = _fetch_ds_config_from_sqlbot_api(ds_id)
            LOGGER.info("request=%s datasource resolved via sqlbot_api dsId=%s", request_id, ds_id)
            return ds_cfg
        except Exception as exc:
            errors.append(f"sqlbot_api: {exc}")
            LOGGER.warning(
                "request=%s datasource resolve failed via sqlbot_api dsId=%s error=%s",
                request_id,
                ds_id,
                exc,
            )
            if source == "sqlbot_api":
                raise HTTPException(status_code=500, detail=f"Failed to load datasource from SQLBot API: {exc}") from exc
    if source in ("auto", "sqlbot_pg"):
        try:
            ds_cfg = _fetch_ds_config_from_sqlbot_pg(ds_id)
            LOGGER.info("request=%s datasource resolved via sqlbot_pg dsId=%s", request_id, ds_id)
            return ds_cfg
        except Exception as exc:
            errors.append(f"sqlbot_pg: {exc}")
            LOGGER.warning(
                "request=%s datasource resolve failed via sqlbot_pg dsId=%s error=%s",
                request_id,
                ds_id,
                exc,
            )
            if source == "sqlbot_pg":
                raise HTTPException(status_code=500, detail=f"Failed to load datasource from SQLBot PG: {exc}") from exc
    if source in ("auto", "yaml"):
        try:
            ds_cfg = _resolve_ds_config_from_yaml(ds_id)
            LOGGER.info("request=%s datasource resolved via yaml dsId=%s", request_id, ds_id)
            return ds_cfg
        except Exception as exc:
            errors.append(f"yaml: {exc}")
            LOGGER.warning(
                "request=%s datasource resolve failed via yaml dsId=%s error=%s",
                request_id,
                ds_id,
                exc,
            )
            if source == "yaml":
                raise
    raise HTTPException(status_code=500, detail=f"Failed to resolve datasource dsId={ds_id}. Tried: {' | '.join(errors)}")


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
    LOGGER.info(
        "claw exec model=%s timeout=%ss prompt_chars=%s work_dir=%s",
        model or "(default)",
        timeout_seconds,
        len(user_prompt),
        work_dir,
    )
    started = time.time()
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
    exec_ms = int((time.time() - started) * 1000)
    stderr_text = (proc.stderr or "").strip()
    stdout_text = (proc.stdout or "").strip()
    LOGGER.info(
        "claw subprocess done exit_code=%s exec_ms=%s stdout_chars=%s stderr_chars=%s",
        proc.returncode,
        exec_ms,
        len(stdout_text),
        len(stderr_text),
    )
    if stderr_text:
        preview = (
            stderr_text[:DEFAULT_LOG_PREVIEW_CHARS] + "..."
            if len(stderr_text) > DEFAULT_LOG_PREVIEW_CHARS
            else stderr_text
        )
        LOGGER.warning("claw stderr preview=%s", preview)
    raw = stdout_text
    if not raw and stderr_text:
        raw = stderr_text
    parsed = None
    if raw:
        try:
            parsed = json.loads(raw)
        except Exception:
            parsed = None
    if isinstance(parsed, dict):
        usage = parsed.get("usage") if isinstance(parsed.get("usage"), dict) else {}
        tool_uses = parsed.get("tool_uses") if isinstance(parsed.get("tool_uses"), list) else []
        tool_names: list[str] = []
        for item in tool_uses:
            if isinstance(item, dict):
                name = item.get("name")
                if isinstance(name, str) and name:
                    tool_names.append(name)
        LOGGER.info(
            "claw result summary iterations=%s usage_in=%s usage_out=%s tool_uses=%s tool_names=%s estimated_cost=%s",
            parsed.get("iterations"),
            usage.get("input_tokens"),
            usage.get("output_tokens"),
            len(tool_uses),
            tool_names[:8],
            parsed.get("estimated_cost"),
        )
    return proc.returncode, raw, parsed, exec_ms


@APP.get("/healthz")
def healthz() -> dict[str, Any]:
    return {
        "ok": True,
        "serviceMode": os.getenv("CLAW_SERVICE_MODE", "mcp"),
        "dsSource": DEFAULT_DS_SOURCE,
        "clawBin": CLAW_BIN,
        "registryPath": str(DS_REGISTRY_PATH),
        "workRoot": str(WORK_ROOT),
        "dorisMcpCommand": DEFAULT_DORIS_MCP_COMMAND,
        "dorisMcpArgs": DEFAULT_DORIS_MCP_ARGS,
        "dorisMcpImageCompat": DORIS_MCP_IMAGE,
        "logFile": DEFAULT_LOG_FILE or None,
    }


@APP.post("/v1/solve", response_model=SolveResponse)
def solve(req: SolveRequest) -> SolveResponse:
    request_id = uuid.uuid4().hex
    LOGGER.info(
        "request=%s solve start dsId=%s model=%s timeout=%s prompt_chars=%s",
        request_id,
        req.dsId,
        req.model or "(default)",
        req.timeoutSeconds or DEFAULT_TIMEOUT_SECONDS,
        len(req.userPrompt),
    )
    ds_cfg = _resolve_ds_config(req.dsId, request_id)
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

    code, out_text, out_json, claw_exec_ms = _run_claw_prompt(
        work_dir=ds_work_dir,
        user_prompt=req.userPrompt.strip(),
        model=req.model,
        timeout_seconds=timeout_seconds,
    )
    duration_ms = int((time.time() - started) * 1000)
    LOGGER.info(
        "request=%s solve done dsId=%s exit_code=%s duration_ms=%s claw_exec_ms=%s output_chars=%s output_json=%s",
        request_id,
        req.dsId,
        code,
        duration_ms,
        claw_exec_ms,
        len(out_text),
        out_json is not None,
    )
    if code != 0:
        LOGGER.warning(
            "request=%s claw non-zero exit output_preview=%s",
            request_id,
            (out_text[:400] + "...") if len(out_text) > 400 else out_text,
        )

    return SolveResponse(
        requestId=request_id,
        dsId=req.dsId,
        workDir=str(ds_work_dir),
        durationMs=duration_ms,
        clawExitCode=code,
        outputText=out_text,
        outputJson=out_json,
    )
