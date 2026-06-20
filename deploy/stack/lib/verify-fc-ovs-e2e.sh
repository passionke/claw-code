#!/usr/bin/env bash
# E2E checks for FC interactive + FC OVS singleton. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-8088}"
PROJ_ID="${CLAW_FC_E2E_PROJ_ID:-1}"
SESSION_ID="ovs-${PROJ_ID}"
BACKEND="${CLAW_INTERACTIVE_BACKEND:-podman}"
OVS_BACKEND="${CLAW_OVS_BACKEND:-compose}"

fail() { echo "verify-fc-ovs-e2e: $*" >&2; exit 1; }

fc_sandbox_count() {
  local api="${CLAW_FC_API_URL:-http://10.8.0.9:3000}"
  local key="${CLAW_FC_API_KEY:-${ALIYUN_E2B_TOKEN:-}}"
  [[ -n "${key}" ]] || { echo 0; return; }
  curl -sS -m 10 "${api%/}/sandboxes" -H "X-API-Key: ${key}" \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d) if isinstance(d,list) else 0)" 2>/dev/null \
    || echo 0
}

echo "==> backend=${BACKEND} ovsBackend=${OVS_BACKEND} projId=${PROJ_ID} session=${SESSION_ID}"

if [[ "${BACKEND}" == "fc" && "${CLAW_FC_E2E_CLEANUP:-1}" == "1" ]]; then
  echo "==> fc sandbox cleanup (orphans from prior runs)"
  bash "${ROOT_DIR}/deploy/stack/lib/fc-sandbox-cleanup.sh"
  if podman container exists claw-gateway-rs >/dev/null 2>&1; then
    echo "==> restart gateway-rs (clear in-memory fc pool after cleanup)"
    podman restart claw-gateway-rs >/dev/null
    for _ in $(seq 1 60); do
      if curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/healthz" 2>/dev/null | grep -q '"ok":true'; then
        break
      fi
      sleep 1
    done
  fi
fi

before_count="$(fc_sandbox_count)"
echo "==> e2b sandboxes before E2E: ${before_count}"

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

if [[ "${OVS_BACKEND}" == "fc" ]]; then
  echo "==> GET /v1/projects/${PROJ_ID}/ovs/workspace (fc OVS singleton)"
  ws="$(curl -sS "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${PROJ_ID}/ovs/workspace")"
  echo "${ws}"
  echo "${ws}" | grep -q '"ovsBackend":"fc"' || fail "ovs/workspace not fc backend: ${ws}"
  echo "${ws}" | grep -q '"ovsUrl"' || fail "missing ovsUrl: ${ws}"
  echo "${ws}" | grep -q '"ovsFolderUrl"' || fail "missing ovsFolderUrl: ${ws}"
  ovs_url="$(python3 -c "import json,sys; print(json.load(sys.stdin).get('ovsUrl',''))" <<<"${ws}")"
  [[ -n "${ovs_url}" ]] || fail "empty ovsUrl"
  fc_domain="${CLAW_FC_DOMAIN:-10.8.0.9}"
  ovs_host="$(python3 -c "from urllib.parse import urlparse; print(urlparse('${ovs_url}').hostname or '')")"
  if [[ "${ovs_host}" == *-*.* ]]; then
    # Self-hosted e2b uses IP as domain; {port}-{sandboxId}.{ip} does not resolve via DNS.
    body="$(/usr/bin/curl -sS -m 15 --resolve "${ovs_host}:80:${fc_domain}" "${ovs_url}/" || true)"
    code="$(/usr/bin/curl -sS -o /dev/null -w '%{http_code}' -m 15 --resolve "${ovs_host}:80:${fc_domain}" "${ovs_url}/" || true)"
  else
    body="$(/usr/bin/curl -sS -m 15 "${ovs_url}/" || true)"
    code="$(/usr/bin/curl -sS -o /dev/null -w '%{http_code}' -m 15 "${ovs_url}/" || true)"
  fi
  [[ "${code}" == "200" ]] || fail "OVS singleton HTTP ${code} at ${ovs_url}/"
  if echo "${body}" | grep -q '君子慎独'; then
    fail "OVS URL hit e2b default site (F14: nginx port-prefix routing missing on ${fc_domain})"
  fi
  if echo "${body}" | grep -qiE 'invalid traffic access token|401 Unauthorized'; then
    fail "OVS URL requires traffic token (rebuild gateway with secure:false self-hosted or ?token=)"
  fi
  echo "${body}" | grep -qiE 'openvscode|vscode|workbench' \
    || fail "OVS response does not look like openvscode-server (F14?)"
  echo "==> fc OVS singleton reachable (${code})"
elif podman container exists claw-gateway-rs >/dev/null 2>&1; then
  podman exec claw-gateway-rs sh -c 'echo fc-e2e > /var/lib/claw/workspace/.fc_e2e_probe'
  podman exec claw-gateway-rs test -f /var/lib/claw/workspace/.fc_e2e_probe \
    || fail "gateway cannot write workspace"
  if podman container exists claw-openvscode-server >/dev/null 2>&1; then
    podman exec claw-openvscode-server test -f /home/workspace/.fc_e2e_probe \
      || fail "OVS cannot see workspace probe"
    echo "==> workspace probe visible in gateway + compose OVS"
  fi
  podman exec claw-gateway-rs rm -f /var/lib/claw/workspace/.fc_e2e_probe
else
  echo "==> skip workspace probe (claw-gateway-rs not running)" >&2
fi

echo "==> terminal/start (FC worker warm pool)"
curl -sS -X POST "http://127.0.0.1:${GATEWAY_PORT}/v1/sessions/${SESSION_ID}/terminal/stop?projId=${PROJ_ID}" \
  >/dev/null 2>&1 || true
resp="$(curl -sS -X POST "http://127.0.0.1:${GATEWAY_PORT}/v1/sessions/${SESSION_ID}/terminal/start" \
  -H 'Content-Type: application/json' \
  -d "{\"projId\":${PROJ_ID},\"sessionId\":\"${SESSION_ID}\"}")"
echo "${resp}" | grep -q '"sessionId"' || fail "terminal/start failed: ${resp}"
echo "${resp}"

if [[ "${OVS_BACKEND}" == "fc" ]]; then
  echo "==> agent/ws via gateway (fc worker ttyd)"
  if [[ "${CLAW_FC_E2E_SKIP_AGENT_WS:-0}" == "1" ]]; then
    echo "skip agent WS (CLAW_FC_E2E_SKIP_AGENT_WS=1; blocked on F14 e2b traffic routing)" >&2
  elif [[ -x "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" ]]; then
    CLAW_OVS_E2E_SKIP_CONTAINER=1 CLAW_OVS_E2E_PROJ_ID="${PROJ_ID}" \
      CLAW_OVS_E2E_PROMPT="${CLAW_FC_E2E_PROMPT:-ping}" \
      CLAW_OVS_E2E_FAST=1 \
      "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" || fail "OVS agent WS failed"
  fi
elif [[ -x "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" ]]; then
  echo "==> OVS agent WS (compose OVS path)"
  CLAW_OVS_E2E_PROJ_ID="${PROJ_ID}" CLAW_OVS_E2E_PROMPT="${CLAW_FC_E2E_PROMPT:-ping}" \
    "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" || fail "OVS agent WS failed"
fi

echo "==> terminal/stop (release warm slot)"
stop_resp="$(curl -sS -X POST "http://127.0.0.1:${GATEWAY_PORT}/v1/sessions/${SESSION_ID}/terminal/stop?projId=${PROJ_ID}")"
echo "${stop_resp}" | grep -q '"ok":true' || fail "terminal/stop failed: ${stop_resp}"

after_count="$(fc_sandbox_count)"
echo "==> e2b sandboxes after E2E: ${after_count} (before=${before_count})"

echo "verify-fc-ovs-e2e: OK"
