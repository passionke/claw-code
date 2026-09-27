#!/usr/bin/env bash
# L1 acceptance: relaxed worker ensure (no OVS). Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
# shellcheck source=/dev/null
[[ -f "${ROOT_DIR}/.env" ]] && source "${ROOT_DIR}/.env"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-18088}"
RELAXED_PROJ_ID="${CLAW_RELAXED_E2E_PROJ_ID:-${CLAW_OVS_E2E_PROJ_ID:-2}}"
E2B_API="${CLAW_E2B_API_URL:-http://10.8.0.1:3000}"
E2B_KEY="${CLAW_E2B_API_KEY:-${ALIYUN_E2B_TOKEN:-}}"

fail() { echo "verify-relaxed-worker [INV-${1:-?}]: $2" >&2; exit 1; }

curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/healthz" | grep -q '"ok":true' \
  || fail G1 "gateway :${GATEWAY_PORT} not healthy"

# OVS exit: workspace route must be gone.
ovs_code="$(curl -sS -o /dev/null -w '%{http_code}' \
  "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${RELAXED_PROJ_ID}/ovs/workspace" || true)"
[[ "${ovs_code}" == "404" ]] || fail 1 "ovs/workspace expected 404 got ${ovs_code}"

# ensure_worker via force reset → at least one running worker with sandboxId.
reset_json="$(curl -fsS -X POST \
  "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${RELAXED_PROJ_ID}/e2b-worker/reset" \
  -H "Content-Type: application/json" \
  -d '{}')"
echo "${reset_json}"

echo "${reset_json}" | grep -q '"ok":true' || fail 2 "e2b-worker/reset ok!=true"
echo "${reset_json}" | grep -q '"sandboxId"' || fail 3 "missing sandboxId after ensure/reset"
worker_count="$(python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d.get('workers') or []))" <<<"${reset_json}")"
[[ "${worker_count}" -ge 1 ]] || fail 3 "expected >=1 worker after ensure, got ${worker_count}"

status_json="$(curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/projects/${RELAXED_PROJ_ID}/e2b-worker")"
echo "${status_json}" | grep -q '"workerProfile":"relaxed"' \
  || fail 4 "workerProfile must be relaxed for proj ${RELAXED_PROJ_ID}"

if [[ -n "${E2B_KEY}" ]]; then
  ovs_singleton_count="$(curl -sS -m 15 "${E2B_API%/}/sandboxes" -H "X-API-Key: ${E2B_KEY}" \
    | python3 -c "import json,sys; d=json.load(sys.stdin); print(sum(1 for s in d if (s.get('metadata') or {}).get('clawRole')=='ovs-singleton'))" 2>/dev/null || echo 0)"
  [[ "${ovs_singleton_count}" == "0" ]] || fail 5 "found ${ovs_singleton_count} legacy ovs-singleton sandbox(es)"
fi

echo "verify-relaxed-worker: OK"
