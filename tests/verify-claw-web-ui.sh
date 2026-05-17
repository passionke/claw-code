#!/usr/bin/env bash
# Claw Web UI verifier (Playwright + prerequisites). Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UI_DIR="${ROOT}/web/claw-web-ui"
WEB_PORT="${CLAW_WEB_UI_PORT:-4100}"
BASE_URL="${CLAW_WEB_UI_BASE_URL:-http://127.0.0.1:${WEB_PORT}}"

if [[ -f "${ROOT}/.env" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ROOT}/.env"
  set +a
fi

BRIDGE_PORT="${CLAW_AGUI_BRIDGE_HOST_PORT:-8090}"
GW_PORT="${GATEWAY_HOST_PORT:-8088}"
export CLAW_GATEWAY_BASE_URL="${CLAW_GATEWAY_BASE_URL:-http://127.0.0.1:${GW_PORT}}"
BRIDGE_URL="${CLAW_AGUI_BRIDGE_URL:-http://127.0.0.1:${BRIDGE_PORT}}"

pass() { echo "  ok: $*"; }
fail() { echo "  FAIL: $*" >&2; exit 1; }

section() { echo ""; echo "== $* =="; }

require_ui_deps() {
  if [[ ! -d "${UI_DIR}/node_modules" ]] || [[ ! -f "${UI_DIR}/package-lock.json" ]]; then
    fail "missing ${UI_DIR}/node_modules — run: cd ${UI_DIR} && npm install"
  fi
}

section "A. prerequisites"
curl -sf "${BRIDGE_URL}/healthz" >/dev/null || fail "bridge not up at ${BRIDGE_URL}"
pass "bridge healthz"
curl -sf "${CLAW_GATEWAY_BASE_URL}/healthz" >/dev/null || fail "gateway not up"
pass "gateway healthz"
require_ui_deps

section "B. curl stack (full tier)"
"${ROOT}/tests/verify-claw-web.sh" --tier full
pass "verify-claw-web full"

section "C. web UI reachable"
if ! curl -sf "${BASE_URL}" >/dev/null 2>&1; then
  fail "web UI not listening at ${BASE_URL} — run: ./deploy/stack/gateway.sh web-ui"
fi
pass "web UI HTTP ${BASE_URL}"

section "D. Playwright E2E (system Chrome, no browser download)"
(
  cd "${UI_DIR}"
  export CLAW_WEB_UI_PORT="${WEB_PORT}"
  export CLAW_WEB_UI_BASE_URL="${BASE_URL}"
  export CLAW_GATEWAY_BASE_URL
  export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1
  unset PLAYWRIGHT_DOWNLOAD_HOST
  npm run test:e2e
)
pass "playwright"

echo ""
echo "verify-claw-web-ui: all checks passed"
