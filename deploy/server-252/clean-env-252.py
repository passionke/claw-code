#!/usr/bin/env python3
"""Rewrite 192.168.9.252 repo-root .env: drop legacy Python/macOS keys, keep business secrets.

Usage on server: python3 deploy/server-252/clean-env-252.py /home/admin/claw-code/.env
Author: kejiqing
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

# Removed: Python http_gateway, macOS paths, SQLBot PG bridge, duplicate image :local, worker-openai env-file.
DROP_KEYS = frozenset(
    {
        "CLAW_SERVICE_MODE",
        "CLAW_DS_SOURCE",
        "CLAW_WORK_ROOT",
        "CLAW_BIN",
        "DORIS_MCP_COMMAND",
        "DORIS_MCP_ARGS",
        "MCP_HTTP_BRIDGE_COMMAND",
        "MCP_HTTP_BRIDGE_SCRIPT",
        "SQLBOT_PG_HOST",
        "SQLBOT_PG_PORT",
        "SQLBOT_PG_USER",
        "SQLBOT_PG_PASSWORD",
        "SQLBOT_PG_DB",
        "SQLBOT_MCP_HOST",
        "SQLBOT_MCP_PORT",
        "SQLBOT_MCP_PATH",
        "SQLBOT_MCP_TRANSPORT",
        "SQLBOT_MCP_AK",
        "SQLBOT_MCT_SK",
        "SQLBOT_MCP_SK",
        "SQLBOT_MCP_USERNAME",
        "SQLBOT_MCP_PASSWORD",
        "CLAW_DEFAULT_HTTP_MCP_NAME",
        "CLAW_DEFAULT_HTTP_MCP_URL",
        "CLAW_DEFAULT_HTTP_MCP_TRANSPORT",
        "CLAW_GATEWAY_SESSION_DB",
        "GATEWAY_IMAGE",
        "CLAW_DOCKER_IMAGE",
        "CLAW_PODMAN_IMAGE",
        "CLAW_DOCKER_EXTRA_ARGS",
        "CLAW_PODMAN_EXTRA_ARGS",
        "OPENAI_BASE_URL",
        "CLAW_POOL_DAEMON_TCP_HOST",
        "PODMAN_HOST_SOCK",
        "CLAW_USE_DOCKER",
        "CLAW_CONTAINER_SOCKET",
        "CLAW_POOL_DAEMON_TCP",
        "CLAW_POOL_HTTP_BASE",
        "CLAW_POOL_RPC_HOST_WORK_ROOT",
        "CLAW_POOL_WORK_ROOT_HOST",
    }
)

KEEP_KEYS = frozenset(
    {
        "CLAW_DEPLOY_PROFILE",
        "CLAW_CLUSTER_ID",
        "UPSTREAM_OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "CLAW_DEFAULT_MODEL",
        "ANTHROPIC_MODEL",
        "CLAW_OPENAI_FALLBACK_MODEL",
        "DEEPSEEK_API_KEY",
        "REPORT_LLM_PROVIDER",
        "GATEWAY_HOST_PORT",
        "CLAW_TIMEOUT_SECONDS",
        "CLAW_SOLVE_ISOLATION",
        "CLAW_CONTAINER_RUNTIME",
        "CLAW_POOL_HOST_DAEMON",
        "CLAW_POOL_DAEMON_SKIP_BUILD",
        "CLAW_POOL_DAEMON_BIN",
        "CLAW_DOCKER_POOL_SIZE",
        "CLAW_DOCKER_POOL_MIN_IDLE",
        "CLAW_POOL_ADVERTISE_HOST",
        "CLAW_POOL_ID",
        "CLAW_IMAGE_REGISTRY",
        "CLAW_IMAGE_PREFIX",
        "CLAW_MCP_TOOL_CALL_TIMEOUT_MS",
        "CLAW_MCP_MAX_CONCURRENT",
        "CLAW_MCP_PARALLEL_FANOUT",
        "CLAUDE_TAP_MODE",
        "CLAUDE_TAP_IMAGE",
        "CLAUDE_TAP_HOST_PORT",
        "CLAUDE_TAP_LIVE_PORT",
        "INTERNAL_CLAUDE_TAP_HOST",
        "CLAW_HOST_LOG_DIR",
        "CLAW_HTTP_LOG_LEVEL",
        "CLAW_HTTP_LOG_FILE",
        "CLAW_HTTP_LOG_ROTATE_BYTES",
        "CLAW_HTTP_LOG_BACKUP_COUNT",
        "CLAW_HTTP_LOG_PREVIEW_CHARS",
        "CLAW_HTTP_LOG_FULL_STDERR_ON_ERROR",
        "CLAW_HTTP_LOG_FULL_STDERR_MAX_CHARS",
        "CLAW_HTTP_LOG_MODEL_RESPONSE",
        "CLAW_HTTP_LOG_MODEL_RESPONSE_MAX_CHARS",
        "CLAW_SSE_DEBUG",
        "CLAW_SSE_DEBUG_PREVIEW_CHARS",
        "CLAW_PREFER_OPENAI_PREFIX",
        "CLAW_DISABLE_ANTHROPIC_ROUTING",
        "CLAW_TRACE_ENABLED",
        "CLAW_TRACE_SAMPLE_RATE",
        "CLAW_ALLOWED_TOOLS",
        "CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL",
        "CLAW_GATEWAY_SQLBOT_PREFLIGHT",
        "CLAW_GATEWAY_ASSISTANT_STREAM_SPILL",
        "CLAW_INSTRUCTION_FILE_MAX_CHARS",
        "CLAW_INSTRUCTION_TOTAL_MAX_CHARS",
        "DEFAULT_PROGRESS_MESSAGE_MAX_CHARS",
        "CLAW_PROJECTS_GIT_URL",
        "CLAW_PROJECTS_GIT_BRANCH",
        "CLAW_PROJECTS_GIT_AUTHOR",
        "CLAW_PROJECTS_GIT_TOKEN",
        "CLAW_PROJECTS_GIT_DS_HOME_POLL_INTERVAL_SECS",
        "PLAYGROUND_ADMIN_USER",
        "PLAYGROUND_ADMIN_PASSWORD",
        "CLAW_GATEWAY_DATABASE_URL",
        "CLAW_GATEWAY_INTERNAL_BASE_URL",
        "CLAW_GATEWAY_INTERNAL_TOKEN",
        "CLAW_LLM_PROXY",
        "CLAW_TAP_PROXY_URL",
    }
)


def parse_env(path: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        if s.startswith("export "):
            s = s[7:].strip()
        if "=" not in s:
            continue
        k, _, v = s.partition("=")
        k = k.strip()
        v = v.strip()
        if k in DROP_KEYS:
            continue
        if (v.startswith("'") and v.endswith("'")) or (v.startswith('"') and v.endswith('"')):
            v = v[1:-1]
        out[k] = v
    return out


def fmt(k: str, v: str) -> str:
    if re.search(r"[\s#'\"]", v) or v.startswith("-"):
        q = "'" + v.replace("'", "'\"'\"'") + "'"
        return f"{k}={q}\n"
    return f"{k}={v}\n"


def main() -> None:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} /path/to/.env", file=sys.stderr)
        sys.exit(2)
    path = Path(sys.argv[1])
    if not path.is_file():
        print(f"error: missing {path}", file=sys.stderr)
        sys.exit(1)

    old = parse_env(path)
    kept = {k: old[k] for k in KEEP_KEYS if k in old and old[k]}

    # Align models to last explicit default in file (qwen3-max won on 252).
    model = old.get("CLAW_DEFAULT_MODEL") or "openai/qwen3-max"
    kept["CLAW_DEFAULT_MODEL"] = model
    kept["ANTHROPIC_MODEL"] = model
    kept["CLAW_OPENAI_FALLBACK_MODEL"] = model

    defaults: dict[str, str] = {
        "CLAW_DEPLOY_PROFILE": "production",
        "CLAW_CLUSTER_ID": kept.get("CLAW_CLUSTER_ID", "prod-claw-252"),
        "CLAW_CONTAINER_RUNTIME": "docker",
        "CLAW_SOLVE_ISOLATION": "e2b",
        "CLAW_POOL_HOST_DAEMON": "1",
        "CLAW_POOL_DAEMON_SKIP_BUILD": "1",
        "CLAW_POOL_DAEMON_BIN": "/home/admin/.local/bin/claw-pool-daemon",
        "GATEWAY_HOST_PORT": kept.get("GATEWAY_HOST_PORT", "18088"),
        "CLAW_TIMEOUT_SECONDS": kept.get("CLAW_TIMEOUT_SECONDS", "1800"),
        "CLAW_DOCKER_POOL_SIZE": kept.get("CLAW_DOCKER_POOL_SIZE", "4"),
        "CLAW_DOCKER_POOL_MIN_IDLE": kept.get("CLAW_DOCKER_POOL_MIN_IDLE", "1"),
        "CLAW_POOL_ADVERTISE_HOST": kept.get("CLAW_POOL_ADVERTISE_HOST", "192.168.9.252"),
        "CLAW_HOST_LOG_DIR": "./deploy/stack/claw-logs",
        "CLAUDE_TAP_MODE": "docker",
        "CLAUDE_TAP_IMAGE": "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-tap:latest",
        "CLAUDE_TAP_HOST_PORT": "8080",
        "INTERNAL_CLAUDE_TAP_HOST": "http://host.docker.internal:8080",
        "CLAW_IMAGE_REGISTRY": "acr",
    }
    for k, v in defaults.items():
        kept.setdefault(k, v)

    header = """# 192.168.9.252 — human .env (Rust http-gateway-rs + e2b + host pool)
# Author: kejiqing
# Images: ./deploy/stack/gateway.sh up --release release-vX.Y.Z  → .claw-image-release.env
# Do NOT set GATEWAY_IMAGE=:local here. Do NOT set CLAW_POOL_DAEMON_TCP_HOST to LAN IP.
# LLM keys: prefer Admin → PG; OPENAI_* / UPSTREAM_* here are bootstrap only.
# Generated keys: deploy/stack/.claw-pool-rpc/gateway.env, .claw-worker-runtime.env

"""
    order = [
        "CLAW_DEPLOY_PROFILE",
        "CLAW_CLUSTER_ID",
        "CLAW_IMAGE_REGISTRY",
        "CLAW_IMAGE_PREFIX",
        "",
        "UPSTREAM_OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "CLAW_DEFAULT_MODEL",
        "ANTHROPIC_MODEL",
        "CLAW_OPENAI_FALLBACK_MODEL",
        "DEEPSEEK_API_KEY",
        "REPORT_LLM_PROVIDER",
        "",
        "GATEWAY_HOST_PORT",
        "CLAW_TIMEOUT_SECONDS",
        "CLAW_CONTAINER_RUNTIME",
        "CLAW_SOLVE_ISOLATION",
        "CLAW_POOL_HOST_DAEMON",
        "CLAW_POOL_DAEMON_SKIP_BUILD",
        "CLAW_POOL_DAEMON_BIN",
        "CLAW_POOL_ADVERTISE_HOST",
        "CLAW_POOL_ID",
        "CLAW_DOCKER_POOL_SIZE",
        "CLAW_DOCKER_POOL_MIN_IDLE",
        "CLAW_MCP_TOOL_CALL_TIMEOUT_MS",
        "CLAW_MCP_MAX_CONCURRENT",
        "CLAW_MCP_PARALLEL_FANOUT",
        "",
        "CLAUDE_TAP_MODE",
        "CLAUDE_TAP_IMAGE",
        "CLAUDE_TAP_HOST_PORT",
        "INTERNAL_CLAUDE_TAP_HOST",
        "CLAW_LLM_PROXY",
        "CLAW_TAP_PROXY_URL",
        "",
        "CLAW_HOST_LOG_DIR",
        "CLAW_HTTP_LOG_LEVEL",
        "CLAW_HTTP_LOG_FILE",
        "CLAW_HTTP_LOG_ROTATE_BYTES",
        "CLAW_HTTP_LOG_BACKUP_COUNT",
        "CLAW_HTTP_LOG_PREVIEW_CHARS",
        "CLAW_HTTP_LOG_FULL_STDERR_ON_ERROR",
        "CLAW_HTTP_LOG_FULL_STDERR_MAX_CHARS",
        "CLAW_HTTP_LOG_MODEL_RESPONSE",
        "CLAW_HTTP_LOG_MODEL_RESPONSE_MAX_CHARS",
        "CLAW_SSE_DEBUG",
        "CLAW_SSE_DEBUG_PREVIEW_CHARS",
        "CLAW_PREFER_OPENAI_PREFIX",
        "CLAW_DISABLE_ANTHROPIC_ROUTING",
        "CLAW_TRACE_ENABLED",
        "CLAW_TRACE_SAMPLE_RATE",
        "CLAW_ALLOWED_TOOLS",
        "CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL",
        "CLAW_GATEWAY_SQLBOT_PREFLIGHT",
        "CLAW_GATEWAY_ASSISTANT_STREAM_SPILL",
        "CLAW_INSTRUCTION_FILE_MAX_CHARS",
        "CLAW_INSTRUCTION_TOTAL_MAX_CHARS",
        "DEFAULT_PROGRESS_MESSAGE_MAX_CHARS",
        "",
        "CLAW_PROJECTS_GIT_URL",
        "CLAW_PROJECTS_GIT_BRANCH",
        "CLAW_PROJECTS_GIT_AUTHOR",
        "CLAW_PROJECTS_GIT_TOKEN",
        "CLAW_PROJECTS_GIT_DS_HOME_POLL_INTERVAL_SECS",
        "CLAW_GATEWAY_DATABASE_URL",
        "CLAW_GATEWAY_INTERNAL_BASE_URL",
        "CLAW_GATEWAY_INTERNAL_TOKEN",
        "PLAYGROUND_ADMIN_USER",
        "PLAYGROUND_ADMIN_PASSWORD",
    ]

    lines = [header]
    written: set[str] = set()
    for key in order:
        if key == "":
            lines.append("\n")
            continue
        if key not in kept:
            continue
        lines.append(fmt(key, kept[key]))
        written.add(key)
    extra = sorted(k for k in kept if k not in written)
    if extra:
        lines.append("\n# --- retained extras ---\n")
        for k in extra:
            lines.append(fmt(k, kept[k]))

    backup = path.with_suffix(path.suffix + ".bak-cleanup")
    backup.write_text(path.read_text(encoding="utf-8"), encoding="utf-8")
    path.write_text("".join(lines), encoding="utf-8")
    print(f"OK: wrote {path} (backup {backup})")
    print(f"removed {len(DROP_KEYS)} deprecated key names; kept {len(kept)} keys")


if __name__ == "__main__":
    main()
