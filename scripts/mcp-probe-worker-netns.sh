#!/usr/bin/env bash
# Probe SQLBot MCP from pool worker network namespace. Author: kejiqing
set -euo pipefail
WORKER="${1:-$(podman ps --format '{{.Names}}' | rg '^claw-worker' | head -1)}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
echo "worker=$WORKER"
podman run --rm --network "container:${WORKER}" \
  -v "${SCRIPT_DIR}/probe-sqlbot-chat-id-parallel.py:/probe.py:ro" \
  -v "${SCRIPT_DIR}/mcp-probe-worker-inline.py:/run.py:ro" \
  docker.1ms.run/library/python:3.12-slim \
  python3 /run.py
