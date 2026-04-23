#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""HTTP gateway for claw prompt with datasource binding. Author: kejiqing"""

from __future__ import annotations

import json
import hashlib
import logging
import logging.handlers
import os
import selectors
import subprocess
import threading
import time
import uuid
import urllib.request
from base64 import b64decode
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional

import yaml
from fastapi import FastAPI, HTTPException, Query
from fastapi.responses import HTMLResponse
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
DEFAULT_ALLOWED_TOOLS_RAW = os.getenv("CLAW_ALLOWED_TOOLS", "").strip()
DEFAULT_TIMEOUT_SECONDS = int(os.getenv("CLAW_HTTP_TIMEOUT_SECONDS", "240"))
DEFAULT_DS_SOURCE = os.getenv("CLAW_DS_SOURCE", "auto").strip().lower()  # auto|sqlbot_api|sqlbot_pg|yaml
DEFAULT_MODEL_RAW = os.getenv("CLAW_DEFAULT_MODEL", "").strip()
LEGACY_MODEL_RAW = os.getenv("ANTHROPIC_MODEL", "").strip()
DEFAULT_OPENAI_FALLBACK_MODEL = os.getenv("CLAW_OPENAI_FALLBACK_MODEL", "openai/inclusionai/ling-2.6-flash:free").strip()
DEFAULT_MODEL_ALIASES_RAW = os.getenv("CLAW_MODEL_ALIASES_JSON", "").strip()
PREFER_OPENAI_PREFIX = os.getenv("CLAW_PREFER_OPENAI_PREFIX", "1").strip().lower() in ("1", "true", "yes", "on")
DISABLE_ANTHROPIC_ROUTING = os.getenv("CLAW_DISABLE_ANTHROPIC_ROUTING", "1").strip().lower() in (
    "1",
    "true",
    "yes",
    "on",
)
INIT_ON_FIRST_DSID = os.getenv("CLAW_INIT_ON_FIRST_DSID", "1").strip().lower() in ("1", "true", "yes", "on")
INIT_TIMEOUT_SECONDS = int(os.getenv("CLAW_INIT_TIMEOUT_SECONDS", "30"))
DEFAULT_LOG_PREVIEW_CHARS = int(os.getenv("CLAW_HTTP_LOG_PREVIEW_CHARS", "400"))
DEFAULT_LOG_FULL_STDERR = os.getenv("CLAW_HTTP_LOG_FULL_STDERR", "0").strip().lower() in (
    "1",
    "true",
    "yes",
    "on",
)
DEFAULT_LOG_FULL_STDERR_ON_ERROR = os.getenv("CLAW_HTTP_LOG_FULL_STDERR_ON_ERROR", "1").strip().lower() in (
    "1",
    "true",
    "yes",
    "on",
)
DEFAULT_LOG_FULL_STDERR_MAX_CHARS = int(os.getenv("CLAW_HTTP_LOG_FULL_STDERR_MAX_CHARS", "20000"))
DEFAULT_LOG_MODEL_RESPONSE = os.getenv("CLAW_HTTP_LOG_MODEL_RESPONSE", "0").strip().lower() in (
    "1",
    "true",
    "yes",
    "on",
)
DEFAULT_LOG_MODEL_RESPONSE_MAX_CHARS = int(os.getenv("CLAW_HTTP_LOG_MODEL_RESPONSE_MAX_CHARS", "12000"))
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
SQLBOT_MCP_RAW = os.getenv("SQLBOT_MCP", "").strip()
SQLBOT_MCP_HOST = os.getenv("SQLBOT_MCP_HOST", "").strip()
SQLBOT_MCP_PORT = os.getenv("SQLBOT_MCP_PORT", "").strip()
SQLBOT_MCP_SCHEME = os.getenv("SQLBOT_MCP_SCHEME", "http").strip() or "http"
SQLBOT_MCP_PATH = os.getenv("SQLBOT_MCP_PATH", "").strip()
SQLBOT_MCP_SERVER_NAME = os.getenv("SQLBOT_MCP_SERVER_NAME", "sqlbot").strip() or "sqlbot"
SQLBOT_MCP_AK = os.getenv("SQLBOT_MCP_AK", "").strip()
SQLBOT_MCP_SK = os.getenv("SQLBOT_MCP_SK", "").strip() or os.getenv("SQLBOT_MCT_SK", "").strip()
TRACE_ENABLED = os.getenv("CLAW_TRACE_ENABLED", "0").strip().lower() in ("1", "true", "yes", "on")
TRACE_SAMPLE_RATE = max(0.0, min(1.0, float(os.getenv("CLAW_TRACE_SAMPLE_RATE", "1.0"))))
TRACE_FILE_RAW = os.getenv("CLAW_TRACE_FILE", "").strip()
TRACE_DIR = Path(os.getenv("CLAW_TRACE_DIR", str(WORK_ROOT / "logs")).strip() or str(WORK_ROOT / "logs"))
TRACE_FILE_PATH = Path(TRACE_FILE_RAW) if TRACE_FILE_RAW else TRACE_DIR / "agent-trace.ndjson"
TRACE_WRITE_LOCK = threading.Lock()
TRACE_FULL_CHILD_LOG_ON_END = os.getenv("CLAW_TRACE_FULL_CHILD_LOG_ON_END", "1").strip().lower() in (
    "1",
    "true",
    "yes",
    "on",
)
TRACE_FULL_CHILD_LOG_MAX_CHARS = int(os.getenv("CLAW_TRACE_FULL_CHILD_LOG_MAX_CHARS", "20000"))


def _parse_allowed_tools(raw: str) -> list[str]:
    values: list[str] = []
    for chunk in raw.replace(",", " ").split():
        tool = chunk.strip()
        if tool and tool not in values:
            values.append(tool)
    return values


DEFAULT_ALLOWED_TOOLS = _parse_allowed_tools(DEFAULT_ALLOWED_TOOLS_RAW)


def _parse_model_aliases(raw: str) -> dict[str, str]:
    if not raw:
        return {}
    try:
        parsed = json.loads(raw)
    except Exception as exc:
        LOGGER.warning("invalid CLAW_MODEL_ALIASES_JSON; expected object string->string: %s", exc)
        return {}
    if not isinstance(parsed, dict):
        LOGGER.warning("invalid CLAW_MODEL_ALIASES_JSON; expected JSON object, got: %s", type(parsed).__name__)
        return {}
    aliases: dict[str, str] = {}
    for key, value in parsed.items():
        k = str(key).strip()
        v = str(value).strip()
        if k and v:
            aliases[k] = v
    return aliases


MODEL_ALIAS_MAP = _parse_model_aliases(DEFAULT_MODEL_ALIASES_RAW)

LOGGER.info(
    "gateway startup service_mode=%s ds_source=%s claw_bin=%s registry=%s work_root=%s log_file=%s default_model=%s disable_anthropic=%s allowed_tools=%s model_aliases=%s trace_enabled=%s trace_file=%s trace_sample_rate=%s",
    os.getenv("CLAW_SERVICE_MODE", "mcp"),
    DEFAULT_DS_SOURCE,
    CLAW_BIN,
    DS_REGISTRY_PATH,
    WORK_ROOT,
    DEFAULT_LOG_FILE or "(stdout-only)",
    DEFAULT_MODEL_RAW or LEGACY_MODEL_RAW or DEFAULT_OPENAI_FALLBACK_MODEL,
    DISABLE_ANTHROPIC_ROUTING,
    DEFAULT_ALLOWED_TOOLS if DEFAULT_ALLOWED_TOOLS else "(all)",
    len(MODEL_ALIAS_MAP),
    TRACE_ENABLED,
    TRACE_FILE_PATH,
    TRACE_SAMPLE_RATE,
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


class SolveAsyncResponse(BaseModel):
    taskId: str
    requestId: str
    status: str
    pollUrl: str
    traceUrl: str


class SolveTaskResponse(BaseModel):
    taskId: str
    requestId: str
    status: str
    createdAtMs: int
    startedAtMs: Optional[int] = None
    finishedAtMs: Optional[int] = None
    result: Optional[SolveResponse] = None
    error: Optional[dict[str, Any]] = None


class InjectMcpRequest(BaseModel):
    dsId: int = Field(..., ge=1, description="Datasource ID bound to runtime workspace.")
    mcpServers: dict[str, Any] = Field(..., description="MCP server configs in claw settings JSON style.")
    replace: bool = Field(default=False, description="Replace existing injected servers for this dsId.")
    probeTimeoutSeconds: int = Field(default=15, ge=1, le=120)


class InjectMcpResponse(BaseModel):
    requestId: str
    dsId: int
    injectedServerNames: list[str]
    loaded: bool
    missingServers: list[str]
    configuredServers: int
    status: str
    mcpReport: Optional[dict[str, Any]] = None


class GetInjectedMcpResponse(BaseModel):
    requestId: str
    dsId: int
    injectedServerNames: list[str]
    loaded: bool
    missingServers: list[str]
    configuredServers: int
    status: str
    mcpReport: Optional[dict[str, Any]] = None


class DeleteInjectedMcpResponse(BaseModel):
    requestId: str
    dsId: int
    removedServerNames: list[str]
    injectedServerNames: list[str]
    loaded: bool
    missingServers: list[str]
    configuredServers: int
    status: str
    mcpReport: Optional[dict[str, Any]] = None


TASKS_LOCK = threading.Lock()
TASKS: dict[str, dict[str, Any]] = {}
INJECTED_MCP_LOCK = threading.Lock()
INJECTED_MCP_BY_DS: dict[int, dict[str, Any]] = {}


def _now_ms() -> int:
    return int(time.time() * 1000)


def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _trace_sampled(trace_id: str) -> bool:
    if not TRACE_ENABLED:
        return False
    if TRACE_SAMPLE_RATE >= 1.0:
        return True
    digest = hashlib.sha1(trace_id.encode("utf-8")).hexdigest()
    bucket = int(digest[:8], 16) / float(0xFFFFFFFF)
    return bucket <= TRACE_SAMPLE_RATE


def _trace_emit(
    *,
    trace_id: str,
    event_kind: str,
    attributes: dict[str, Any],
    turn_id: Optional[str] = None,
    tool_call_id: Optional[str] = None,
    data_node_id: Optional[str] = None,
) -> None:
    if not _trace_sampled(trace_id):
        return
    event = {
        "type": "agent_trace_event",
        "event_version": 1,
        "event_kind": event_kind,
        "trace_id": trace_id,
        "timestamp_ms": _now_ms(),
        "@timestamp": _now_iso(),
        "trace.id": trace_id,
        "service.name": "claw-http-gateway",
        "event.action": event_kind,
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "data_node_id": data_node_id,
        "attributes": attributes,
    }
    try:
        TRACE_FILE_PATH.parent.mkdir(parents=True, exist_ok=True)
        with TRACE_WRITE_LOCK:
            with TRACE_FILE_PATH.open("a", encoding="utf-8") as fp:
                fp.write(json.dumps(event, ensure_ascii=False) + "\n")
    except Exception as exc:
        LOGGER.warning("trace write failed trace_id=%s event=%s error=%s", trace_id, event_kind, exc)


def _parse_kv_tokens(text: str) -> dict[str, str]:
    attrs: dict[str, str] = {}
    for token in text.split():
        if "=" not in token:
            continue
        key, value = token.split("=", 1)
        key = key.strip()
        value = value.strip()
        if key:
            attrs[key] = value
    return attrs


def _emit_child_stderr_trace(trace_id: str, stderr_text: str) -> None:
    if not stderr_text:
        return
    lines = [line.strip() for line in stderr_text.splitlines() if line.strip()]
    if not lines:
        return
    # Limit ingestion to keep trace files bounded during noisy runs.
    stream_stage_whitelist = {
        "stream_request_start",
        "stream_http_connected",
        "stream_completed",
        "stream_error",
        "stream_request_failed",
    }
    seen_stream_keys: set[str] = set()
    for line in lines[:400]:
        if line.startswith("[runtime-boundary]"):
            payload = line[len("[runtime-boundary]") :].strip()
            attrs = _parse_kv_tokens(payload)
            attrs["raw_line"] = line[:2000]
            _trace_emit(
                trace_id=trace_id,
                event_kind="runtime_boundary_event",
                turn_id=attrs.get("turn_id"),
                attributes=attrs,
            )
        elif line.startswith("[boundary-out]") or line.startswith("[boundary-in]"):
            payload = line.split("]", 1)[1].strip() if "]" in line else line
            attrs = _parse_kv_tokens(payload)
            attrs["direction"] = "out" if line.startswith("[boundary-out]") else "in"
            attrs["raw_line"] = line[:2000]
            _trace_emit(
                trace_id=trace_id,
                event_kind="provider_boundary_event",
                turn_id=attrs.get("turn_id"),
                attributes=attrs,
            )
        elif line.startswith("[sse-debug]") or line.startswith("[boundary-stream-first]") or line.startswith("[boundary-stream-eof]"):
            payload = line.split("]", 1)[1].strip() if "]" in line else line
            attrs = _parse_kv_tokens(payload)
            if line.startswith("[sse-debug]"):
                stage = attrs.get("stage", "")
                # Streaming debug can be extremely noisy; keep only key milestones.
                if stage and stage not in stream_stage_whitelist:
                    continue
                stream_key = f"sse:{stage}:{attrs.get('request_id','-')}"
                if stream_key in seen_stream_keys:
                    continue
                seen_stream_keys.add(stream_key)
            elif line.startswith("[boundary-stream-first]"):
                stream_key = f"boundary-first:{attrs.get('request_id','-')}"
                if stream_key in seen_stream_keys:
                    continue
                seen_stream_keys.add(stream_key)
            elif line.startswith("[boundary-stream-eof]"):
                stream_key = f"boundary-eof:{attrs.get('request_id','-')}"
                if stream_key in seen_stream_keys:
                    continue
                seen_stream_keys.add(stream_key)
            attrs["raw_line"] = line[:2000]
            _trace_emit(
                trace_id=trace_id,
                event_kind="stream_debug_event",
                turn_id=attrs.get("turn_id"),
                attributes=attrs,
            )


def _emit_child_stderr_full(trace_id: str, stderr_text: str, *, source: str) -> None:
    if not TRACE_FULL_CHILD_LOG_ON_END or not stderr_text:
        return
    max_chars = max(TRACE_FULL_CHILD_LOG_MAX_CHARS, 1)
    truncated = len(stderr_text) > max_chars
    payload = stderr_text[:max_chars]
    _trace_emit(
        trace_id=trace_id,
        event_kind="child_log_full",
        attributes={
            "source": source,
            "total_chars": len(stderr_text),
            "truncated": truncated,
            "content": payload,
        },
    )


def _stable_id(prefix: str, payload: str) -> str:
    digest = hashlib.sha1(payload.encode("utf-8")).hexdigest()[:16]
    return f"{prefix}_{digest}"


def _extract_data_summary(output_value: Any) -> tuple[Optional[int], list[str], list[str]]:
    if isinstance(output_value, dict):
        schema = list(output_value.keys())[:50]
        metric_keys = [k for k in schema if any(x in k.lower() for x in ("count", "sum", "avg", "rate", "amount", "total"))][:20]
        rows = None
        for key in ("rows", "data", "records", "items"):
            value = output_value.get(key)
            if isinstance(value, list):
                rows = len(value)
                if value and isinstance(value[0], dict):
                    schema = list(value[0].keys())[:50]
                break
        return rows, schema, metric_keys
    if isinstance(output_value, list):
        rows = len(output_value)
        schema: list[str] = []
        metric_keys: list[str] = []
        if output_value and isinstance(output_value[0], dict):
            schema = list(output_value[0].keys())[:50]
            metric_keys = [k for k in schema if any(x in k.lower() for x in ("count", "sum", "avg", "rate", "amount", "total"))][:20]
        return rows, schema, metric_keys
    return None, [], []


def _extract_assertions(message: str) -> list[str]:
    if not message:
        return []
    lines = [line.strip() for line in message.splitlines() if line.strip()]
    assertions: list[str] = []
    for line in lines:
        normalized = line.lstrip("-•*0123456789. ").strip()
        if len(normalized) >= 12:
            assertions.append(normalized[:500])
        if len(assertions) >= 24:
            break
    return assertions


def _emit_model_trace_events(trace_id: str, parsed: Optional[dict[str, Any]]) -> None:
    if not isinstance(parsed, dict):
        return
    tool_uses = parsed.get("tool_uses") if isinstance(parsed.get("tool_uses"), list) else []
    tool_results = parsed.get("tool_results") if isinstance(parsed.get("tool_results"), list) else []
    usage = parsed.get("usage") if isinstance(parsed.get("usage"), dict) else {}
    iterations = parsed.get("iterations")
    _trace_emit(
        trace_id=trace_id,
        event_kind="decision_event",
        turn_id="turn_final",
        attributes={
            "iterations": iterations,
            "tool_use_count": len(tool_uses),
            "usage_input_tokens": usage.get("input_tokens"),
            "usage_output_tokens": usage.get("output_tokens"),
            "estimated_cost": parsed.get("estimated_cost"),
        },
    )

    results_by_id: dict[str, dict[str, Any]] = {}
    for item in tool_results:
        if not isinstance(item, dict):
            continue
        tool_use_id = item.get("tool_use_id")
        if isinstance(tool_use_id, str) and tool_use_id:
            results_by_id[tool_use_id] = item

    data_node_ids: list[str] = []
    for index, tool in enumerate(tool_uses):
        if not isinstance(tool, dict):
            continue
        tool_id = str(tool.get("id") or f"tool_{index + 1}")
        tool_name = str(tool.get("name") or "unknown")
        result = results_by_id.get(tool_id, {})
        output = result.get("output")
        output_len = len(output) if isinstance(output, str) else 0
        is_error = bool(result.get("is_error", False))
        _trace_emit(
            trace_id=trace_id,
            event_kind="tool_call_event",
            turn_id="turn_final",
            tool_call_id=tool_id,
            attributes={
                "tool_name": tool_name,
                "input_size": len(json.dumps(tool.get("input", {}), ensure_ascii=False)),
                "output_size": output_len,
                "is_error": is_error,
                "status": "error" if is_error else "ok",
            },
        )

        parsed_output: Any = None
        if isinstance(output, str) and output:
            try:
                parsed_output = json.loads(output)
            except Exception:
                parsed_output = None
        rows, schema, metric_keys = _extract_data_summary(parsed_output)
        if rows is None and not schema and not metric_keys:
            continue
        data_node_id = _stable_id("data", f"{trace_id}:{tool_id}:{tool_name}:{rows}:{','.join(schema[:10])}")
        data_node_ids.append(data_node_id)
        _trace_emit(
            trace_id=trace_id,
            event_kind="data_event",
            turn_id="turn_final",
            tool_call_id=tool_id,
            data_node_id=data_node_id,
            attributes={
                "tool_name": tool_name,
                "row_count": rows,
                "schema_keys": schema,
                "metric_keys": metric_keys,
                "link_mode": "tool_output",
            },
        )

    assertions = _extract_assertions(str(parsed.get("message") or ""))
    for idx, assertion in enumerate(assertions, start=1):
        _trace_emit(
            trace_id=trace_id,
            event_kind="assertion_event",
            turn_id="turn_final",
            data_node_id=data_node_ids[0] if data_node_ids else None,
            attributes={
                "assertion_index": idx,
                "text": assertion,
                "linked_data_node_ids": data_node_ids[:5],
                "link_mode": "heuristic",
            },
        )


def _normalize_model_name(raw_model: str) -> str:
    model = raw_model.strip()
    if not model:
        return model
    if PREFER_OPENAI_PREFIX and "/" not in model:
        model = f"openai/{model}"
    return MODEL_ALIAS_MAP.get(model, model)


def _resolve_effective_model(request_model: Optional[str]) -> tuple[str, str]:
    source_model_raw = ""
    if request_model and request_model.strip():
        source = "request"
        source_model_raw = request_model.strip()
        model = _normalize_model_name(source_model_raw)
    elif DEFAULT_MODEL_RAW:
        source = "CLAW_DEFAULT_MODEL"
        source_model_raw = DEFAULT_MODEL_RAW
        model = _normalize_model_name(source_model_raw)
    elif LEGACY_MODEL_RAW:
        source = "ANTHROPIC_MODEL"
        source_model_raw = LEGACY_MODEL_RAW
        model = _normalize_model_name(source_model_raw)
    else:
        source = "CLAW_OPENAI_FALLBACK_MODEL"
        source_model_raw = DEFAULT_OPENAI_FALLBACK_MODEL
        model = _normalize_model_name(source_model_raw)
    if source_model_raw and source_model_raw != model:
        LOGGER.info("model normalized source=%s raw=%s normalized=%s", source, source_model_raw, model)

    if DISABLE_ANTHROPIC_ROUTING and model.startswith("anthropic/"):
        fallback = _normalize_model_name(DEFAULT_MODEL_RAW or DEFAULT_OPENAI_FALLBACK_MODEL)
        if fallback.startswith("anthropic/"):
            fallback = _normalize_model_name("openai/inclusionai/ling-2.6-flash:free")
        LOGGER.warning(
            "anthropic model blocked by policy requested=%s source=%s fallback=%s",
            model,
            source,
            fallback,
        )
        return fallback, f"{source}->fallback"

    return model, source


def _ensure_claw_workspace_initialized(work_dir: Path, request_id: str, ds_id: int) -> None:
    if not INIT_ON_FIRST_DSID:
        return
    claw_dir = work_dir / ".claw"
    marker = claw_dir / ".gateway_init_done"
    if marker.exists():
        return

    work_dir.mkdir(parents=True, exist_ok=True)
    started = time.time()
    cmd = [CLAW_BIN, "init"]
    LOGGER.info("request=%s workspace init start dsId=%s work_dir=%s", request_id, ds_id, work_dir)
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(work_dir),
            text=True,
            capture_output=True,
            timeout=INIT_TIMEOUT_SECONDS,
            check=False,
        )
    except FileNotFoundError as exc:
        raise HTTPException(status_code=500, detail=f"claw binary not found for init: {CLAW_BIN}") from exc
    except subprocess.TimeoutExpired as exc:
        raise HTTPException(status_code=500, detail=f"claw init timeout: {INIT_TIMEOUT_SECONDS}s") from exc

    elapsed_ms = int((time.time() - started) * 1000)
    out = (proc.stdout or "").strip()
    err = (proc.stderr or "").strip()
    if proc.returncode != 0:
        preview = err or out or "(empty output)"
        if len(preview) > 800:
            preview = preview[:800] + "..."
        LOGGER.warning(
            "request=%s workspace init failed dsId=%s exit_code=%s duration_ms=%s output=%s",
            request_id,
            ds_id,
            proc.returncode,
            elapsed_ms,
            preview,
        )
        raise HTTPException(status_code=500, detail=f"Failed to initialize claw workspace dsId={ds_id}: {preview}")

    claw_dir.mkdir(parents=True, exist_ok=True)
    marker.write_text(str(int(time.time())), encoding="utf-8")
    LOGGER.info(
        "request=%s workspace init done dsId=%s duration_ms=%s stdout_chars=%s stderr_chars=%s",
        request_id,
        ds_id,
        elapsed_ms,
        len(out),
        len(err),
    )


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


def _normalize_mcp_servers_payload(raw: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(raw, dict) or not raw:
        raise HTTPException(status_code=400, detail="mcpServers must be a non-empty JSON object.")
    normalized: dict[str, Any] = {}
    for raw_name, raw_config in raw.items():
        name = str(raw_name).strip()
        if not name:
            raise HTTPException(status_code=400, detail="mcpServers contains an empty server name.")
        if not isinstance(raw_config, dict):
            raise HTTPException(status_code=400, detail=f"mcpServers.{name} must be a JSON object.")
        has_shape_key = any(k in raw_config for k in ("type", "command", "url", "name"))
        if not has_shape_key:
            raise HTTPException(
                status_code=400,
                detail=f"mcpServers.{name} must include one of: type, command, url, name.",
            )
        try:
            normalized[name] = json.loads(json.dumps(raw_config, ensure_ascii=False))
        except Exception as exc:
            raise HTTPException(
                status_code=400,
                detail=f"mcpServers.{name} is not JSON-serializable: {exc}",
            ) from exc
    return normalized


def _extract_mcp_servers_value(raw: Any) -> dict[str, Any]:
    value = raw
    if value is None:
        return {}
    if isinstance(value, str):
        text = value.strip()
        if not text:
            return {}
        value = json.loads(text)
    if not isinstance(value, dict):
        raise ValueError("MCP config must be a JSON object.")
    if isinstance(value.get("mcpServers"), dict):
        value = value["mcpServers"]
    if any(key in value for key in ("type", "command", "url", "name")):
        value = {SQLBOT_MCP_SERVER_NAME: value}
    return _normalize_mcp_servers_payload(value)


def _sqlbot_mcp_servers_from_env() -> dict[str, Any]:
    if SQLBOT_MCP_RAW:
        try:
            return _extract_mcp_servers_value(SQLBOT_MCP_RAW)
        except Exception as exc:
            LOGGER.warning("invalid SQLBOT_MCP, ignored: %s", exc)
            return {}
    if not (SQLBOT_MCP_HOST and SQLBOT_MCP_PORT):
        return {}
    base_url = f"{SQLBOT_MCP_SCHEME}://{SQLBOT_MCP_HOST}:{SQLBOT_MCP_PORT}"
    url = f"{base_url}{SQLBOT_MCP_PATH}" if SQLBOT_MCP_PATH else base_url
    headers: dict[str, str] = {}
    if SQLBOT_MCP_AK:
        headers["x-ak"] = SQLBOT_MCP_AK
    if SQLBOT_MCP_SK:
        headers["x-sk"] = SQLBOT_MCP_SK
    server_config: dict[str, Any] = {
        "type": "http",
        "url": url,
    }
    if headers:
        server_config["headers"] = headers
    return {SQLBOT_MCP_SERVER_NAME: server_config}


def _builtin_mcp_server_names() -> list[str]:
    names = ["doris"]
    names.extend(sorted(_sqlbot_mcp_servers_from_env().keys()))
    # Keep stable output and avoid duplicates if names overlap.
    return sorted(set(names))


def _merge_mcp_servers(ds_id: int, base_servers: dict[str, Any]) -> dict[str, Any]:
    merged = dict(base_servers)
    with INJECTED_MCP_LOCK:
        injected = INJECTED_MCP_BY_DS.get(ds_id, {})
        for name, config in injected.items():
            merged[name] = config
    return merged


def _stable_doris_config_path(work_dir: Path, ds_id: int, config_payload: dict[str, Any]) -> tuple[Path, str]:
    canonical_json = json.dumps(
        config_payload,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    digest = hashlib.sha256(canonical_json.encode("utf-8")).hexdigest()[:16]
    file_path = work_dir / f"doris_{ds_id}_{digest}.yaml"
    yaml_text = yaml.safe_dump(
        config_payload,
        allow_unicode=True,
        sort_keys=False,
    )
    return file_path, yaml_text


def _build_claw_settings(
    doris_cfg_path: Path,
    ds_id: int,
) -> dict[str, Any]:
    mcp_args = [x.strip() for x in DEFAULT_DORIS_MCP_ARGS.split(" ") if x.strip()]
    base_servers = {
        "doris": {
            "type": "stdio",
            "command": DEFAULT_DORIS_MCP_COMMAND,
            "args": mcp_args,
            "env": {"DORIS_CONFIG": str(doris_cfg_path)},
        }
    }
    sqlbot_env_servers = _sqlbot_mcp_servers_from_env()
    merged_base = dict(base_servers)
    merged_base.update(sqlbot_env_servers)
    merged_servers = _merge_mcp_servers(ds_id, merged_base)
    return {
        "mcpServers": merged_servers
    }


def _probe_mcp_load(work_dir: Path, timeout_seconds: int) -> tuple[dict[str, Any], list[str], int, str]:
    cmd = [CLAW_BIN, "mcp", "--output-format", "json"]
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
        raise HTTPException(status_code=500, detail=f"claw binary not found for mcp probe: {CLAW_BIN}") from exc
    except subprocess.TimeoutExpired as exc:
        raise HTTPException(status_code=504, detail=f"claw mcp probe timeout: {timeout_seconds}s") from exc

    raw = (proc.stdout or "").strip() or (proc.stderr or "").strip()
    parsed: dict[str, Any] = {}
    if raw:
        try:
            value = json.loads(raw)
            if isinstance(value, dict):
                parsed = value
        except Exception:
            parsed = {"raw": raw}
    servers = parsed.get("servers") if isinstance(parsed.get("servers"), list) else []
    loaded_names: list[str] = []
    for item in servers:
        if isinstance(item, dict):
            name = item.get("name")
            if isinstance(name, str) and name:
                loaded_names.append(name)
    configured_servers = int(parsed.get("configured_servers") or len(loaded_names) or 0)
    status = str(parsed.get("status") or ("ok" if proc.returncode == 0 else "error"))
    parsed["exitCode"] = proc.returncode
    return parsed, loaded_names, configured_servers, status


def _apply_settings_and_probe(
    ds_id: int,
    request_id: str,
    probe_timeout_seconds: int,
) -> tuple[dict[str, Any], list[str], int, str]:
    ds_cfg = _resolve_ds_config(ds_id, request_id)
    ds_work_dir = WORK_ROOT / f"ds_{ds_id}"
    ds_work_dir.mkdir(parents=True, exist_ok=True)
    _ensure_claw_workspace_initialized(ds_work_dir, request_id, ds_id)

    cluster_config = _build_doris_cluster_config(ds_id, ds_cfg)
    doris_config_path, doris_yaml = _stable_doris_config_path(ds_work_dir, ds_id, cluster_config)
    if not doris_config_path.exists():
        doris_config_path.write_text(doris_yaml, encoding="utf-8")

    claw_dir = ds_work_dir / ".claw"
    claw_dir.mkdir(parents=True, exist_ok=True)
    settings = _build_claw_settings(doris_config_path, ds_id)
    (claw_dir / "settings.json").write_text(
        json.dumps(settings, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    return _probe_mcp_load(ds_work_dir, probe_timeout_seconds)


def _run_claw_prompt(
    *,
    work_dir: Path,
    user_prompt: str,
    model: Optional[str],
    timeout_seconds: int,
    request_id: str,
) -> tuple[int, str, Optional[dict[str, Any]], int, Optional[int], Optional[int], Optional[int]]:
    cmd = [CLAW_BIN, "--output-format", "json"]
    if model and model.strip():
        cmd.extend(["--model", model.strip()])
    if DEFAULT_ALLOWED_TOOLS:
        cmd.extend(["--allowedTools", ",".join(DEFAULT_ALLOWED_TOOLS)])
    cmd.extend(["prompt", user_prompt])
    LOGGER.info(
        "claw exec model=%s timeout=%ss prompt_chars=%s work_dir=%s allowed_tools=%s",
        model or "(default)",
        timeout_seconds,
        len(user_prompt),
        work_dir,
        DEFAULT_ALLOWED_TOOLS if DEFAULT_ALLOWED_TOOLS else "(all)",
    )
    _trace_emit(
        trace_id=request_id,
        event_kind="claw_exec_start",
        attributes={
            "model": model or "(default)",
            "timeout_seconds": timeout_seconds,
            "prompt_chars": len(user_prompt),
            "work_dir": str(work_dir),
            "allowed_tools": DEFAULT_ALLOWED_TOOLS if DEFAULT_ALLOWED_TOOLS else ["*"],
        },
    )
    started = time.time()
    child_env = os.environ.copy()
    if _trace_sampled(request_id):
        child_env["CLAW_TRACE_ENABLED"] = "1"
        child_env["CLAW_TRACE_ID"] = request_id
        child_env["CLAW_TRACE_FILE"] = str(TRACE_FILE_PATH)
    try:
        proc = subprocess.Popen(
            cmd,
            cwd=str(work_dir),
            env=child_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=False,
        )
    except FileNotFoundError as exc:
        raise HTTPException(status_code=500, detail=f"claw binary not found: {CLAW_BIN}") from exc
    selector = selectors.DefaultSelector()
    stdout_buffer = bytearray()
    stderr_buffer = bytearray()
    first_io_ms: Optional[int] = None
    first_stdout_ms: Optional[int] = None
    first_stderr_ms: Optional[int] = None
    deadline = started + timeout_seconds
    try:
        if proc.stdout:
            selector.register(proc.stdout, selectors.EVENT_READ, "stdout")
        if proc.stderr:
            selector.register(proc.stderr, selectors.EVENT_READ, "stderr")
        while selector.get_map():
            remaining = deadline - time.time()
            if remaining <= 0:
                proc.kill()
                try:
                    proc.wait(timeout=2)
                except Exception:
                    pass
                timeout_stderr = stderr_buffer.decode("utf-8", errors="replace")
                _emit_child_stderr_trace(request_id, timeout_stderr)
                _emit_child_stderr_full(request_id, timeout_stderr, source="timeout")
                _trace_emit(
                    trace_id=request_id,
                    event_kind="claw_exec_timeout",
                    attributes={"timeout_seconds": timeout_seconds, "work_dir": str(work_dir)},
                )
                raise HTTPException(status_code=504, detail=f"claw request timeout: {timeout_seconds}s")
            events = selector.select(timeout=remaining)
            if not events:
                continue
            for key, _ in events:
                stream_name = key.data
                chunk = os.read(key.fd, 65536)
                if chunk:
                    elapsed = int((time.time() - started) * 1000)
                    if first_io_ms is None:
                        first_io_ms = elapsed
                    if stream_name == "stdout":
                        if first_stdout_ms is None:
                            first_stdout_ms = elapsed
                        stdout_buffer.extend(chunk)
                    else:
                        if first_stderr_ms is None:
                            first_stderr_ms = elapsed
                        stderr_buffer.extend(chunk)
                else:
                    selector.unregister(key.fileobj)
        proc.wait(timeout=2)
    finally:
        try:
            selector.close()
        except Exception:
            pass
    exec_ms = int((time.time() - started) * 1000)
    stdout_text = stdout_buffer.decode("utf-8", errors="replace").strip()
    stderr_text = stderr_buffer.decode("utf-8", errors="replace").strip()
    LOGGER.info(
        "claw subprocess done exit_code=%s exec_ms=%s first_io_ms=%s first_stdout_ms=%s first_stderr_ms=%s stdout_chars=%s stderr_chars=%s",
        proc.returncode if proc.returncode is not None else -1,
        exec_ms,
        first_io_ms,
        first_stdout_ms,
        first_stderr_ms,
        len(stdout_text),
        len(stderr_text),
    )
    _trace_emit(
        trace_id=request_id,
        event_kind="claw_exec_done",
        attributes={
            "exit_code": proc.returncode if proc.returncode is not None else -1,
            "exec_ms": exec_ms,
            "first_io_ms": first_io_ms,
            "first_stdout_ms": first_stdout_ms,
            "first_stderr_ms": first_stderr_ms,
            "stdout_chars": len(stdout_text),
            "stderr_chars": len(stderr_text),
        },
    )
    if stderr_text:
        _emit_child_stderr_trace(request_id, stderr_text)
        _emit_child_stderr_full(request_id, stderr_text, source="process_end")
        preview = (
            stderr_text[:DEFAULT_LOG_PREVIEW_CHARS] + "..."
            if len(stderr_text) > DEFAULT_LOG_PREVIEW_CHARS
            else stderr_text
        )
        LOGGER.warning("claw stderr preview=%s", preview)
        should_log_full_stderr = DEFAULT_LOG_FULL_STDERR or (
            proc.returncode != 0 and DEFAULT_LOG_FULL_STDERR_ON_ERROR
        )
        if should_log_full_stderr:
            max_chars = max(DEFAULT_LOG_FULL_STDERR_MAX_CHARS, 1)
            truncated = len(stderr_text) > max_chars
            body = stderr_text[:max_chars] + ("\n... [truncated]" if truncated else "")
            LOGGER.warning(
                "claw stderr full exit_code=%s total_chars=%s truncated=%s begin\n%s\nclaw stderr full end",
                proc.returncode,
                len(stderr_text),
                truncated,
                body,
            )
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
    if DEFAULT_LOG_MODEL_RESPONSE and raw:
        max_chars = max(DEFAULT_LOG_MODEL_RESPONSE_MAX_CHARS, 1)
        truncated = len(raw) > max_chars
        body = raw[:max_chars] + ("\n... [truncated]" if truncated else "")
        LOGGER.info(
            "claw model response detail exit_code=%s total_chars=%s truncated=%s begin\n%s\nclaw model response detail end",
            proc.returncode,
            len(raw),
            truncated,
            body,
        )
    return (proc.returncode if proc.returncode is not None else -1), raw, parsed, exec_ms, first_io_ms, first_stdout_ms, first_stderr_ms


@APP.get("/healthz")
def healthz() -> dict[str, Any]:
    return {
        "ok": True,
        "serviceMode": os.getenv("CLAW_SERVICE_MODE", "mcp"),
        "dsSource": DEFAULT_DS_SOURCE,
        "clawBin": CLAW_BIN,
        "registryPath": str(DS_REGISTRY_PATH),
        "workRoot": str(WORK_ROOT),
        "builtinMcpServers": _builtin_mcp_server_names(),
        "dorisMcpCommand": DEFAULT_DORIS_MCP_COMMAND,
        "dorisMcpArgs": DEFAULT_DORIS_MCP_ARGS,
        "dorisMcpImageCompat": DORIS_MCP_IMAGE,
        "logFile": DEFAULT_LOG_FILE or None,
        "defaultModel": DEFAULT_MODEL_RAW or LEGACY_MODEL_RAW or DEFAULT_OPENAI_FALLBACK_MODEL,
        "disableAnthropicRouting": DISABLE_ANTHROPIC_ROUTING,
        "initOnFirstDsId": INIT_ON_FIRST_DSID,
        "initTimeoutSeconds": INIT_TIMEOUT_SECONDS,
        "traceEnabled": TRACE_ENABLED,
        "traceFile": str(TRACE_FILE_PATH),
        "traceSampleRate": TRACE_SAMPLE_RATE,
    }


@APP.get("/v1/traces/{request_id}")
def get_trace_events(request_id: str, limit: int = 1000) -> dict[str, Any]:
    if not TRACE_FILE_PATH.exists():
        raise HTTPException(status_code=404, detail=f"trace log not found: {TRACE_FILE_PATH}")
    items: list[dict[str, Any]] = []
    try:
        with TRACE_FILE_PATH.open("r", encoding="utf-8") as fp:
            for line in fp:
                line = line.strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except Exception:
                    continue
                trace_id = record.get("trace_id") or record.get("trace.id") or record.get("session_id")
                if trace_id != request_id:
                    continue
                items.append(record)
                if len(items) >= max(limit, 1):
                    break
    except Exception as exc:
        raise HTTPException(status_code=500, detail=f"failed reading trace log: {exc}") from exc
    return {"traceId": request_id, "count": len(items), "events": items}


@APP.get("/v1/traces")
def list_traces(limit: int = 50) -> dict[str, Any]:
    if not TRACE_FILE_PATH.exists():
        return {"traceFile": str(TRACE_FILE_PATH), "traces": []}
    traces: dict[str, dict[str, Any]] = {}
    try:
        with TRACE_FILE_PATH.open("r", encoding="utf-8") as fp:
            for line in fp:
                line = line.strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except Exception:
                    continue
                trace_id = str(record.get("trace_id") or record.get("trace.id") or record.get("session_id") or "")
                if not trace_id:
                    continue
                item = traces.get(trace_id)
                if item is None:
                    traces[trace_id] = {
                        "traceId": trace_id,
                        "firstTimestamp": record.get("timestamp_ms"),
                        "lastTimestamp": record.get("timestamp_ms"),
                        "eventCount": 1,
                    }
                else:
                    item["lastTimestamp"] = record.get("timestamp_ms")
                    item["eventCount"] = int(item["eventCount"]) + 1
    except Exception as exc:
        raise HTTPException(status_code=500, detail=f"failed reading trace log: {exc}") from exc
    values = sorted(
        traces.values(),
        key=lambda x: int(x.get("lastTimestamp") or 0),
        reverse=True,
    )[: max(limit, 1)]
    return {"traceFile": str(TRACE_FILE_PATH), "count": len(values), "traces": values}


@APP.get("/trace-viewer", response_class=HTMLResponse)
def trace_viewer() -> str:
    return """<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>Agent Trace Viewer</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 20px; background: #0b1220; color: #dbe5ff; }
    .row { display: flex; gap: 8px; margin-bottom: 12px; }
    input, button { padding: 8px; border-radius: 6px; border: 1px solid #32405f; background: #101a2e; color: #dbe5ff; }
    button { cursor: pointer; }
    .card { border: 1px solid #32405f; border-radius: 8px; padding: 10px; margin: 8px 0; background: #101a2e; }
    .muted { color: #94a3c4; font-size: 12px; }
    pre { white-space: pre-wrap; word-break: break-word; }
  </style>
</head>
<body>
  <h2>Agent Trace Viewer</h2>
  <div class="row">
    <input id="traceId" placeholder="requestId / trace_id" style="min-width: 360px;" />
    <button onclick="loadTrace()">Load Trace</button>
    <button onclick="loadLatest()">Latest</button>
  </div>
  <div id="meta" class="muted"></div>
  <div id="timeline"></div>
  <script>
    function esc(v) {
      return String(v ?? '').replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
    }
    function fmtTs(ms) {
      const n = Number(ms);
      if (!Number.isFinite(n) || n <= 0) return '-';
      const d = new Date(n);
      const yyyy = d.getFullYear();
      const mon = String(d.getMonth() + 1).padStart(2, '0');
      const day = String(d.getDate()).padStart(2, '0');
      const hh = String(d.getHours()).padStart(2, '0');
      const mm = String(d.getMinutes()).padStart(2, '0');
      const ss = String(d.getSeconds()).padStart(2, '0');
      const mmm = String(d.getMilliseconds()).padStart(3, '0');
      return `${yyyy}-${mon}-${day} ${hh}:${mm}:${ss}.${mmm}`;
    }
    function inferScope(e, action) {
      if ((e['service.name'] || e.service?.name) === 'claw-http-gateway') return 'gateway';
      const gatewayEvents = new Set([
        'solve_start', 'ds_resolve_done', 'workspace_init_done',
        'claw_exec_start', 'claw_exec_done', 'claw_exec_timeout',
        'solve_done', 'solve_error'
      ]);
      if (gatewayEvents.has(action)) return 'gateway';
      if (action === 'decision_event' || action === 'tool_call_event' || action === 'data_event' || action === 'assertion_event') return 'trace';
      if (action === 'turn_started' || action === 'assistant_iteration_completed' || action === 'tool_execution_started' || action === 'tool_execution_finished' || action === 'turn_completed' || action === 'turn_failed') return 'runtime';
      if (e.type === 'session_trace') return 'runtime';
      return 'unknown';
    }
    async function loadLatest() {
      const res = await fetch('/v1/traces?limit=1');
      const json = await res.json();
      if (!json.traces || !json.traces.length) return;
      const id = json.traces[0].traceId;
      document.getElementById('traceId').value = id;
      await loadTrace();
    }
    async function loadTrace() {
      const id = document.getElementById('traceId').value.trim();
      if (!id) return;
      const res = await fetch('/v1/traces/' + encodeURIComponent(id) + '?limit=2000');
      const json = await res.json();
      const events = json.events || [];
      document.getElementById('meta').textContent = `trace_id=${id} events=${events.length}`;
      const root = document.getElementById('timeline');
      root.innerHTML = '';
      events.sort((a, b) => (a.timestamp_ms || 0) - (b.timestamp_ms || 0));
      for (const e of events) {
        const action = e.event_kind || e.name || e.event?.action || e.type;
        const turnId = e.turn_id || e.turnId || '-';
        const toolId = e.tool_call_id || '-';
        const nodeId = e.data_node_id || '-';
        const scope = inferScope(e, action);
        const attrs = JSON.stringify(e.attributes || {}, null, 2);
        const ts = e.timestamp_ms || '-';
        const div = document.createElement('div');
        div.className = 'card';
        div.innerHTML = `<div><b>${esc(action)}</b></div>
          <div class="muted">scope=${esc(scope)} time=${esc(fmtTs(ts))} ts=${esc(ts)} turn=${esc(turnId)} tool=${esc(toolId)} data=${esc(nodeId)}</div>
          <pre>${esc(attrs)}</pre>`;
        root.appendChild(div);
      }
    }
  </script>
</body>
</html>"""


def _run_solve_request(req: SolveRequest, request_id: str) -> SolveResponse:
    effective_model, model_source = _resolve_effective_model(req.model)
    started = time.time()
    LOGGER.info(
        "request=%s solve start dsId=%s model=%s model_source=%s timeout=%s prompt_chars=%s",
        request_id,
        req.dsId,
        effective_model,
        model_source,
        req.timeoutSeconds or DEFAULT_TIMEOUT_SECONDS,
        len(req.userPrompt),
    )
    _trace_emit(
        trace_id=request_id,
        event_kind="solve_start",
        attributes={
            "ds_id": req.dsId,
            "model": effective_model,
            "model_source": model_source,
            "timeout_seconds": req.timeoutSeconds or DEFAULT_TIMEOUT_SECONDS,
            "prompt_chars": len(req.userPrompt),
        },
    )
    ds_resolve_started = time.time()
    ds_cfg = _resolve_ds_config(req.dsId, request_id)
    ds_resolve_ms = int((time.time() - ds_resolve_started) * 1000)
    _trace_emit(
        trace_id=request_id,
        event_kind="ds_resolve_done",
        attributes={"ds_id": req.dsId, "elapsed_ms": ds_resolve_ms},
    )
    timeout_seconds = req.timeoutSeconds or DEFAULT_TIMEOUT_SECONDS

    ds_work_dir = WORK_ROOT / f"ds_{req.dsId}"
    ds_work_dir.mkdir(parents=True, exist_ok=True)
    init_started = time.time()
    _ensure_claw_workspace_initialized(ds_work_dir, request_id, req.dsId)
    init_ms = int((time.time() - init_started) * 1000)
    _trace_emit(
        trace_id=request_id,
        event_kind="workspace_init_done",
        attributes={"ds_id": req.dsId, "elapsed_ms": init_ms},
    )
    claw_dir = ds_work_dir / ".claw"
    claw_dir.mkdir(parents=True, exist_ok=True)

    prepare_started = time.time()
    cluster_config = _build_doris_cluster_config(req.dsId, ds_cfg)
    doris_config_path, doris_yaml = _stable_doris_config_path(
        ds_work_dir,
        req.dsId,
        cluster_config,
    )
    if not doris_config_path.exists():
        doris_config_path.write_text(doris_yaml, encoding="utf-8")

    settings = _build_claw_settings(doris_config_path, req.dsId)
    (claw_dir / "settings.json").write_text(
        json.dumps(settings, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    prepare_ms = int((time.time() - prepare_started) * 1000)

    try:
        code, out_text, out_json, claw_exec_ms, first_io_ms, first_stdout_ms, first_stderr_ms = _run_claw_prompt(
            work_dir=ds_work_dir,
            user_prompt=req.userPrompt.strip(),
            model=effective_model,
            timeout_seconds=timeout_seconds,
            request_id=request_id,
        )
    except HTTPException as exc:
        _trace_emit(
            trace_id=request_id,
            event_kind="solve_error",
            attributes={
                "status_code": exc.status_code,
                "detail": exc.detail,
                "ds_id": req.dsId,
                "model": effective_model,
            },
        )
        raise
    duration_ms = int((time.time() - started) * 1000)
    LOGGER.info(
        "request=%s solve done dsId=%s exit_code=%s duration_ms=%s ds_resolve_ms=%s init_ms=%s prepare_ms=%s claw_exec_ms=%s claw_first_io_ms=%s claw_first_stdout_ms=%s claw_first_stderr_ms=%s output_chars=%s output_json=%s",
        request_id,
        req.dsId,
        code,
        duration_ms,
        ds_resolve_ms,
        init_ms,
        prepare_ms,
        claw_exec_ms,
        first_io_ms,
        first_stdout_ms,
        first_stderr_ms,
        len(out_text),
        out_json is not None,
    )
    _trace_emit(
        trace_id=request_id,
        event_kind="solve_done",
        attributes={
            "ds_id": req.dsId,
            "exit_code": code,
            "duration_ms": duration_ms,
            "ds_resolve_ms": ds_resolve_ms,
            "init_ms": init_ms,
            "prepare_ms": prepare_ms,
            "claw_exec_ms": claw_exec_ms,
            "claw_first_io_ms": first_io_ms,
            "claw_first_stdout_ms": first_stdout_ms,
            "claw_first_stderr_ms": first_stderr_ms,
            "output_chars": len(out_text),
            "output_json": out_json is not None,
        },
    )
    _emit_model_trace_events(request_id, out_json)
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


def _run_solve_task(task_id: str, req: SolveRequest) -> None:
    with TASKS_LOCK:
        task = TASKS.get(task_id)
        if task is None:
            return
        task["status"] = "running"
        task["startedAtMs"] = _now_ms()
    try:
        result = _run_solve_request(req, request_id=task_id)
        with TASKS_LOCK:
            task = TASKS.get(task_id)
            if task is None:
                return
            task["status"] = "succeeded"
            task["finishedAtMs"] = _now_ms()
            task["result"] = result.model_dump()
    except HTTPException as exc:
        with TASKS_LOCK:
            task = TASKS.get(task_id)
            if task is None:
                return
            task["status"] = "failed"
            task["finishedAtMs"] = _now_ms()
            task["error"] = {"status_code": exc.status_code, "detail": exc.detail}
    except Exception as exc:
        LOGGER.exception("task=%s unexpected async solve error: %s", task_id, exc)
        with TASKS_LOCK:
            task = TASKS.get(task_id)
            if task is None:
                return
            task["status"] = "failed"
            task["finishedAtMs"] = _now_ms()
            task["error"] = {"status_code": 500, "detail": str(exc)}


@APP.post("/v1/solve", response_model=SolveResponse)
def solve(req: SolveRequest) -> SolveResponse:
    request_id = uuid.uuid4().hex
    return _run_solve_request(req, request_id=request_id)


@APP.post("/v1/solve_async", response_model=SolveAsyncResponse)
def solve_async(req: SolveRequest) -> SolveAsyncResponse:
    task_id = uuid.uuid4().hex
    task_record = {
        "taskId": task_id,
        "requestId": task_id,
        "status": "queued",
        "createdAtMs": _now_ms(),
        "startedAtMs": None,
        "finishedAtMs": None,
        "result": None,
        "error": None,
    }
    with TASKS_LOCK:
        TASKS[task_id] = task_record
    worker = threading.Thread(target=_run_solve_task, args=(task_id, req), daemon=True)
    worker.start()
    return SolveAsyncResponse(
        taskId=task_id,
        requestId=task_id,
        status="queued",
        pollUrl=f"/v1/tasks/{task_id}",
        traceUrl=f"/v1/traces/{task_id}",
    )


@APP.post("/v1/mcp/inject", response_model=InjectMcpResponse)
def inject_mcp(req: InjectMcpRequest) -> InjectMcpResponse:
    request_id = uuid.uuid4().hex
    normalized = _normalize_mcp_servers_payload(req.mcpServers)

    if req.replace:
        with INJECTED_MCP_LOCK:
            INJECTED_MCP_BY_DS[req.dsId] = normalized
    else:
        with INJECTED_MCP_LOCK:
            current = dict(INJECTED_MCP_BY_DS.get(req.dsId, {}))
            current.update(normalized)
            INJECTED_MCP_BY_DS[req.dsId] = current

    report, loaded_names, configured_servers, status = _apply_settings_and_probe(
        req.dsId,
        request_id,
        req.probeTimeoutSeconds,
    )
    injected_names = sorted(normalized.keys())
    missing = sorted([name for name in injected_names if name not in set(loaded_names)])
    loaded = not missing and status == "ok"
    return InjectMcpResponse(
        requestId=request_id,
        dsId=req.dsId,
        injectedServerNames=injected_names,
        loaded=loaded,
        missingServers=missing,
        configuredServers=configured_servers,
        status=status,
        mcpReport=report,
    )


@APP.get("/v1/mcp/injected/{ds_id}", response_model=GetInjectedMcpResponse)
def get_injected_mcp(
    ds_id: int,
    probe_timeout_seconds: int = Query(default=15, ge=1, le=120),
) -> GetInjectedMcpResponse:
    request_id = uuid.uuid4().hex
    with INJECTED_MCP_LOCK:
        injected = dict(INJECTED_MCP_BY_DS.get(ds_id, {}))
    injected_names = sorted(injected.keys())

    report, loaded_names, configured_servers, status = _apply_settings_and_probe(
        ds_id,
        request_id,
        probe_timeout_seconds,
    )
    missing = sorted([name for name in injected_names if name not in set(loaded_names)])
    loaded = not missing and status == "ok"
    return GetInjectedMcpResponse(
        requestId=request_id,
        dsId=ds_id,
        injectedServerNames=injected_names,
        loaded=loaded,
        missingServers=missing,
        configuredServers=configured_servers,
        status=status,
        mcpReport=report,
    )


@APP.delete("/v1/mcp/injected/{ds_id}", response_model=DeleteInjectedMcpResponse)
def delete_injected_mcp(
    ds_id: int,
    server_names: Optional[str] = Query(
        default=None,
        description="Comma-separated injected server names to remove. Omit to clear all injected servers for the dsId.",
    ),
    probe_timeout_seconds: int = Query(default=15, ge=1, le=120),
) -> DeleteInjectedMcpResponse:
    request_id = uuid.uuid4().hex
    with INJECTED_MCP_LOCK:
        current = dict(INJECTED_MCP_BY_DS.get(ds_id, {}))
        if server_names is None or not server_names.strip():
            removed = sorted(current.keys())
            INJECTED_MCP_BY_DS.pop(ds_id, None)
            remaining = {}
        else:
            targets = {
                token.strip()
                for token in server_names.split(",")
                if token.strip()
            }
            if not targets:
                raise HTTPException(status_code=400, detail="server_names is empty after parsing.")
            missing_targets = sorted([name for name in targets if name not in current])
            if missing_targets:
                raise HTTPException(
                    status_code=404,
                    detail=f"injected server(s) not found for dsId={ds_id}: {', '.join(missing_targets)}",
                )
            removed = sorted(targets)
            for name in removed:
                current.pop(name, None)
            if current:
                INJECTED_MCP_BY_DS[ds_id] = current
            else:
                INJECTED_MCP_BY_DS.pop(ds_id, None)
            remaining = current

    report, loaded_names, configured_servers, status = _apply_settings_and_probe(
        ds_id,
        request_id,
        probe_timeout_seconds,
    )
    remaining_names = sorted(remaining.keys())
    missing = sorted([name for name in remaining_names if name not in set(loaded_names)])
    loaded = not missing and status == "ok"
    return DeleteInjectedMcpResponse(
        requestId=request_id,
        dsId=ds_id,
        removedServerNames=removed,
        injectedServerNames=remaining_names,
        loaded=loaded,
        missingServers=missing,
        configuredServers=configured_servers,
        status=status,
        mcpReport=report,
    )


@APP.get("/v1/tasks/{task_id}", response_model=SolveTaskResponse)
def get_task(task_id: str) -> SolveTaskResponse:
    with TASKS_LOCK:
        task = TASKS.get(task_id)
        if task is None:
            raise HTTPException(status_code=404, detail=f"task not found: {task_id}")
        payload = dict(task)
    return SolveTaskResponse(**payload)
