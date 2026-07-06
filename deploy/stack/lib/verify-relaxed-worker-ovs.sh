#!/usr/bin/env bash
# L1 acceptance: relaxed worker built-in OVS contract (INV-1..INV-9 subset). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-8088}"
RELAXED_PROJ_ID="${CLAW_OVS_E2E_PROJ_ID:-2}"
STRICT_PROJ_ID="${CLAW_STRICT_E2E_PROJ_ID:-1}"
E2B_API="${CLAW_E2B_API_URL:-http://10.8.0.1:3000}"
E2B_KEY="${CLAW_E2B_API_KEY:-${ALIYUN_E2B_TOKEN:-}}"

fail() { echo "verify-relaxed-worker-ovs [INV-${1:-?}]: $2" >&2; exit 1; }

curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/healthz" | grep -q '"ok":true' \
  || fail G1 "gateway :${GATEWAY_PORT} not healthy"

strict_code="$(curl -sS -o /dev/null -w '%{http_code}' \
  "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${STRICT_PROJ_ID}/ovs/workspace" || true)"
[[ "${strict_code}" == "403" ]] || fail 1 "strict proj ${STRICT_PROJ_ID} ovs/workspace expected 403 got ${strict_code}"

ws="$(curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${RELAXED_PROJ_ID}/ovs/workspace")"
echo "${ws}"

echo "${ws}" | grep -q '"workerProfile":"relaxed"' || fail 2 "missing workerProfile=relaxed"
echo "${ws}" | grep -q '"workspaceFolder":"/claw_ds"' || fail 2 "workspaceFolder must be /claw_ds"
echo "${ws}" | grep -q '"clusterId"' || fail 5 "missing clusterId"

folder_url="$(python3 -c "import json,sys; print(json.load(sys.stdin).get('ovsFolderUrl',''))" <<<"${ws}")"
[[ -n "${folder_url}" ]] || fail 4 "empty ovsFolderUrl"
[[ "${folder_url}" == *"folder=/claw_ds"* ]] || fail 2 "ovsFolderUrl must contain folder=/claw_ds: ${folder_url}"
[[ "${folder_url}" != *"/claw_ws/proj_"* ]] || fail 9 "ovsFolderUrl must not use legacy /claw_ws/proj_: ${folder_url}"
[[ "${folder_url}" == *"-sbx_"* ]] || fail 4 "ovsFolderUrl must use e2b traffic host: ${folder_url}"

sandbox_id="$(python3 -c "import json,sys; print(json.load(sys.stdin).get('sandboxId',''))" <<<"${ws}")"
[[ -n "${sandbox_id}" ]] || fail 3 "missing sandboxId in ovs/workspace response"
[[ "${folder_url}" == *"${sandbox_id}"* ]] || fail 3 "ovsFolderUrl sandbox must match sandboxId (${sandbox_id})"

code="$(curl -sS -o /dev/null -w '%{http_code}' -m 20 "${folder_url}" || true)"
[[ "${code}" == "200" ]] || fail 6 "OVS HTTP ${code} at ${folder_url}"

if [[ -n "${E2B_KEY}" ]]; then
  ovs_singleton_count="$(curl -sS -m 15 "${E2B_API%/}/sandboxes" -H "X-API-Key: ${E2B_KEY}" \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(sum(1 for s in d if (s.get('metadata') or {}).get('clawRole')=='ovs-singleton'))" 2>/dev/null || echo 0)"
  [[ "${ovs_singleton_count}" == "0" ]] || fail 4 "found ${ovs_singleton_count} legacy ovs-singleton sandbox(es)"
fi

echo "verify-relaxed-worker-ovs: OK"
