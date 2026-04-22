#!/usr/bin/env bash
set -euo pipefail

# Runtime mode selector:
# - mcp  (default): start doris MCP stdio server
# - http: start claw HTTP gateway
# Author: kejiqing

MODE="${CLAW_SERVICE_MODE:-mcp}"

if [[ "${MODE}" == "http" ]]; then
  PORT="${CLAW_HTTP_PORT:-18080}"
  exec python3 -m uvicorn http_gateway.app:APP --app-dir /app --host 0.0.0.0 --port "${PORT}"
fi

exec node dist/index.js
