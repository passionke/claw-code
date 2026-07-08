#!/usr/bin/env bash
# E2E checks for e2b interactive + relaxed worker built-in OVS. Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-8088}"
PROJ_ID="${CLAW_E2B_E2E_PROJ_ID:-2}"
SESSION_ID="ovs-${PROJ_ID}"

fail() { echo "verify-e2b-ovs-e2e: $*" >&2; exit 1; }

e2b_sandbox_count() {
  local api="${CLAW_E2B_API_URL:-http://10.8.0.1:3000}"
  local key="${CLAW_E2B_API_KEY:-${ALIYUN_E2B_TOKEN:-}}"
  [[ -n "${key}" ]] || { echo 0; return; }
  curl -sS -m 10 "${api%/}/sandboxes" -H "X-API-Key: ${key}" \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d) if isinstance(d,list) else 0)" 2>/dev/null \
    || echo 0
}

echo "==> e2b-only backend projId=${PROJ_ID} session=${SESSION_ID}"

# Disabled by default — singletons + proj workers must survive gateway restart/E2E.
# Opt-in only: CLAW_E2B_E2E_CLEANUP=1 ./deploy/stack/lib/verify-e2b-ovs-e2e.sh
if [[ "${CLAW_E2B_E2E_CLEANUP:-0}" == "1" ]]; then
  echo "==> e2b sandbox cleanup (CLAW_E2B_E2E_CLEANUP=1 — kills ALL sandboxes on e2b API)"
  bash "${ROOT_DIR}/deploy/stack/lib/e2b-sandbox-cleanup.sh"
  if podman container exists claw-gateway-rs >/dev/null 2>&1; then
    echo "==> restart gateway-rs (clear in-memory e2b pool after cleanup)"
    podman restart claw-gateway-rs >/dev/null
    for _ in $(seq 1 60); do
      if curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/healthz" 2>/dev/null | grep -q '"ok":true'; then
        break
      fi
      sleep 1
    done
  fi
fi

before_count="$(e2b_sandbox_count)"
echo "==> e2b sandboxes before E2E: ${before_count}"

curl -sS "http://127.0.0.1:${GATEWAY_PORT}/healthz" | grep -q '"ok":true' \
  || fail "gateway :${GATEWAY_PORT} not healthy"

[[ -n "${CLAW_E2B_API_KEY:-${ALIYUN_E2B_TOKEN:-}}" ]] \
  || fail "CLAW_E2B_API_KEY or ALIYUN_E2B_TOKEN required for e2b mode"
[[ -f "${ROOT_DIR}/deploy/e2b/e2b_exec.py" ]] \
  || fail "missing deploy/e2b/e2b_exec.py"

echo "==> GET /v1/projects/${PROJ_ID}/ovs/workspace (relaxed worker built-in OVS)"
ws="$(curl -sS "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${PROJ_ID}/ovs/workspace")"
echo "${ws}"
echo "${ws}" | grep -q '"ovsBackend":"e2b"' || fail "ovs/workspace not e2b backend: ${ws}"
echo "${ws}" | grep -q '"workerProfile":"relaxed"' || fail "missing workerProfile=relaxed: ${ws}"
echo "${ws}" | grep -q '"workspaceFolder":"/claw_ds"' || fail "workspaceFolder must be /claw_ds: ${ws}"
echo "${ws}" | grep -q '"sandboxId"' || fail "missing sandboxId: ${ws}"
echo "${ws}" | grep -q '"ovsFolderUrl"' || fail "missing ovsFolderUrl: ${ws}"
fc_domain="${CLAW_E2B_DOMAIN:-supone.top}"
traffic_port="${CLAW_E2B_TRAFFIC_PORT:-3001}"
folder_url="$(python3 -c "import json,sys; print(json.load(sys.stdin).get('ovsFolderUrl',''))" <<<"${ws}")"
sandbox_id="$(python3 -c "import json,sys; print(json.load(sys.stdin).get('sandboxId',''))" <<<"${ws}")"
hosts_line="$(python3 -c "import json,sys; print(json.load(sys.stdin).get('ovsBrowserHostsLine',''))" <<<"${ws}")"
[[ -n "${folder_url}" ]] || fail "empty ovsFolderUrl"
[[ "${folder_url}" == *"folder=/claw_ds"* ]] || fail "ovsFolderUrl must contain folder=/claw_ds: ${folder_url}"
[[ "${folder_url}" != *"/claw_ws/proj_"* ]] || fail "ovsFolderUrl must not use legacy /claw_ws/proj_: ${folder_url}"
[[ "${folder_url}" != *"/v1/fc-ovs"* ]] || fail "ovsFolderUrl must not use gateway proxy: ${folder_url}"
[[ "${folder_url}" == *"-sbx_"* ]] || fail "ovsFolderUrl must use e2b Host traffic URL: ${folder_url}"
[[ "${folder_url}" == *"${sandbox_id}"* ]] || fail "ovsFolderUrl sandbox must match sandboxId (${sandbox_id})"
[[ "${folder_url}" != *"/e2b/"* ]] || fail "ovsFolderUrl must not use legacy /e2b/ path: ${folder_url}"
body="$(/usr/bin/curl -sS -m 15 "${folder_url}" || true)"
code="$(/usr/bin/curl -sS -o /dev/null -w '%{http_code}' -m 15 "${folder_url}" || true)"
[[ "${code}" == "200" ]] || fail "OVS browser URL HTTP ${code} at ${folder_url}"
if echo "${body}" | grep -q '君子慎独'; then
  fail "OVS URL hit e2b default site (F14: nginx port-prefix routing missing on ${fc_domain})"
fi
if echo "${body}" | grep -qiE 'invalid traffic access token|401 Unauthorized'; then
  fail "OVS URL requires traffic token (rebuild gateway with secure:false self-hosted or ?token=)"
fi
echo "${body}" | grep -qiE 'openvscode|vscode|workbench' \
  || fail "OVS response does not look like openvscode-server (F14?)"
echo "==> fc OVS browser URL reachable (${code})"
echo "    ovsFolderUrl=${folder_url}"
echo "==> GET /v1/gateway/global-settings (fc observe Live URLs)"
gs="$(curl -sS "http://127.0.0.1:${GATEWAY_PORT}/v1/gateway/global-settings")"
live_base="$(python3 -c "import json,sys; print((json.load(sys.stdin).get('clawTap') or {}).get('liveBaseUrl',''))" <<<"${gs}")"
[[ -n "${live_base}" ]] || fail "missing clawTap.liveBaseUrl after observe ensure: ${gs}"
echo "${live_base}" | grep -qE 'https?://[0-9]+-sbx_[^./]+\\.' \
  || fail "liveBaseUrl must be e2b Host traffic URL, not gateway proxy: ${live_base}"
echo "${live_base}" | grep -q '/e2b/' \
  && fail "liveBaseUrl must not use legacy /e2b/ path: ${live_base}"
echo "==> fc observe Live base: ${live_base}"

if podman container exists claw-gateway-rs >/dev/null 2>&1; then
  podman exec claw-gateway-rs sh -c 'echo fc-e2e > /var/lib/claw/workspace/.fc_e2e_probe'
  podman exec claw-gateway-rs test -f /var/lib/claw/workspace/.fc_e2e_probe \
    || fail "gateway cannot write workspace"
  podman exec claw-gateway-rs rm -f /var/lib/claw/workspace/.fc_e2e_probe
else
  echo "==> skip workspace probe (claw-gateway-rs not running)" >&2
fi

echo "==> agent/ws via gateway (fc worker ttyd)"
if [[ "${CLAW_E2B_E2E_SKIP_AGENT_WS:-0}" == "1" ]]; then
  echo "skip agent WS (CLAW_E2B_E2E_SKIP_AGENT_WS=1; blocked on F14 e2b traffic routing)" >&2
elif [[ -x "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" ]]; then
  CLAW_OVS_E2E_SKIP_CONTAINER=1 CLAW_OVS_E2E_PROJ_ID="${PROJ_ID}" \
    CLAW_OVS_E2E_PROMPT="${CLAW_E2B_E2E_PROMPT:-ping}" \
    CLAW_OVS_E2E_FAST=1 \
    "${ROOT_DIR}/deploy/stack/lib/verify-ovs-claw-e2e.sh" || fail "OVS agent WS failed"
fi

after_count="$(e2b_sandbox_count)"
echo "==> e2b sandboxes after E2E: ${after_count} (before=${before_count})"

echo "verify-e2b-ovs-e2e: OK"
