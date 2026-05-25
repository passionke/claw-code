#!/usr/bin/env bash
# Host-side gateway-async-playground (solve_async + /admin). Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${LIB_DIR}/../../.." && pwd)"
PG="${ROOT_DIR}/web/gateway-async-playground"

cd "${ROOT_DIR}"

if [[ ! -f .env ]]; then
  echo "缺少 .env：cp .env.example .env 并填写" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "${ROOT_DIR}/.env"
set +a

export PLAYGROUND_PUBLIC_GATEWAY_BASE="${PLAYGROUND_PUBLIC_GATEWAY_BASE:-http://127.0.0.1:${GATEWAY_HOST_PORT:-8088}}"
"${LIB_DIR}/build-gateway-admin.sh"
port="${GATEWAY_PLAYGROUND_HOST_PORT:-18765}"
export PLAYGROUND_LISTEN_PORT="${port}"
lsof -ti ":${port}" 2>/dev/null | xargs kill -9 2>/dev/null || true
# Legacy default 18765 when host port was remapped (e.g. .env 18675) but old process still on 18765.
lsof -ti ":18765" 2>/dev/null | xargs kill -9 2>/dev/null || true
echo "playground http://127.0.0.1:${port}/"
exec python3 "${PG}/server.py"
