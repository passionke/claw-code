#!/usr/bin/env bash
# E2E checks for FC interactive mode + NAS workspace. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-8088}"
PROJ_ID="${CLAW_FC_E2E_PROJ_ID:-1}"
SESSION_ID="ovs-${PROJ_ID}"
BACKEND="${CLAW_INTERACTIVE_BACKEND:-podman}"

fail() { echo "verify-fc-ovs-e2e: $*" >&2; exit 1; }

echo "==> backend=${BACKEND} projId=${PROJ_ID} session=${SESSION_ID}"

curl -sS "http://127.0.0.1:${GATEWAY_PORT}/healthz" | grep -q '"ok":true' \
  || fail "gateway :${GATEWAY_PORT} not healthy"

if [[ "${BACKEND}" == "fc" ]]; then
  [[ -n "${CLAW_FC_API_KEY:-${ALIYUN_E2B_TOKEN:-}}" ]] \
    || fail "CLAW_FC_API_KEY or ALIYUN_E2B_TOKEN required for fc mode"
  [[ -f "${ROOT_DIR}/deploy/fc-sandbox/fc_exec.py" ]] \
    || fail "missing deploy/fc-sandbox/fc_exec.py"
else
  echo "hint: set CLAW_INTERACTIVE_BACKEND=fc for full FC E2E" >&2
fi

if podman container exists claw-gateway-rs >/dev/null 2>&1; then
  podman exec claw-gateway-rs sh -c 'echo fc-e2e > /var/lib/claw/workspace/.fc_e2e_probe'
  podman exec claw-gateway-rs test -f /var/lib/claw/workspace/.fc_e2e_probe \
    || fail "gateway cannot write workspace (NFS compose volume?)"
  podman exec claw-openvscode-server test -f /home/workspace/.fc_e2e_probe \
    || fail "OVS cannot see NAS workspace probe"
  echo "==> workspace probe visible in gateway + OVS"
  podman exec claw-gateway-rs rm -f /var/lib/claw/workspace/.fc_e2e_probe
else
  echo "==> skip workspace probe (claw-gateway-rs not running)" >&2
fi

echo "==> terminal/start (creates FC sandbox or podman worker)"
resp="$(curl -sS -X POST "http://127.0.0.1:${GATEWAY_PORT}/v1/sessions/${SESSION_ID}/terminal/start" \
  -H 'Content-Type: application/json' \
  -d "{\"projId\":${PROJ_ID},\"sessionId\":\"${SESSION_ID}\"}")"
echo "${resp}" | grep -q '"sessionId"' || fail "terminal/start failed: ${resp}"
echo "${resp}"

if [[ -x "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" ]]; then
  echo "==> OVS agent WS (optional full chat path)"
  CLAW_OVS_E2E_PROJ_ID="${PROJ_ID}" CLAW_OVS_E2E_PROMPT="${CLAW_FC_E2E_PROMPT:-ping}" \
    "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" || fail "OVS agent WS failed"
fi

echo "verify-fc-ovs-e2e: OK"
