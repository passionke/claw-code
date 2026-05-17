#!/usr/bin/env bash
# Start Claw Web UI (Next + CopilotKit). Author: kejiqing
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STACK_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${STACK_DIR}/../.." && pwd)"
UI_DIR="${REPO_ROOT}/web/claw-web-ui"

set -a
# shellcheck disable=SC1090
[[ -f "${REPO_ROOT}/.env" ]] && source "${REPO_ROOT}/.env"
set +a

BRIDGE_PORT="${CLAW_AGUI_BRIDGE_HOST_PORT:-8090}"
GW_PORT="${GATEWAY_HOST_PORT:-8088}"
WEB_PORT="${CLAW_WEB_UI_PORT:-4100}"
BRIDGE_URL="${CLAW_AGUI_BRIDGE_URL:-http://127.0.0.1:${BRIDGE_PORT}}"
GW_URL="${CLAW_GATEWAY_BASE_URL:-http://127.0.0.1:${GW_PORT}}"

check_health() {
  local url="$1"
  local name="$2"
  if ! curl -sf "${url}/healthz" >/dev/null; then
    echo "claw-web-ui: ${name} not healthy at ${url}/healthz" >&2
    echo "  Run: ./deploy/stack/gateway.sh tap-up" >&2
    exit 1
  fi
}

require_ui_deps() {
  if [[ ! -d "${UI_DIR}/node_modules" ]] || [[ ! -f "${UI_DIR}/package-lock.json" ]]; then
    echo "claw-web-ui: dependencies missing. Run in your terminal:" >&2
    echo "  cd ${UI_DIR} && npm install" >&2
    exit 1
  fi
}

check_health "${BRIDGE_URL}" "AG-UI bridge"
check_health "${GW_URL}" "gateway"
require_ui_deps

export CLAW_AGUI_BRIDGE_URL="${BRIDGE_URL}"
export CLAW_GATEWAY_BASE_URL="${GW_URL}"
export NEXT_PUBLIC_CLAW_AGUI_BRIDGE_URL="${BRIDGE_URL}"
export NEXT_PUBLIC_CLAW_GATEWAY_BASE_URL="${GW_URL}"
export NEXT_PUBLIC_CLAW_TAP_URL="${CLAW_TAP_URL:-http://127.0.0.1:3000}"
if [[ "${CLAW_CODE_SERVER_ENABLED:-0}" == "1" ]]; then
  export NEXT_PUBLIC_CLAW_CODE_SERVER_ENABLED=1
  export NEXT_PUBLIC_CLAW_CODE_SERVER_PORT="${CLAW_CODE_SERVER_PORT:-4101}"
fi

echo "claw-web-ui: http://127.0.0.1:${WEB_PORT} (bridge ${BRIDGE_URL})"
exec bash -lc "cd '${UI_DIR}' && npm run dev -- -p ${WEB_PORT}"
