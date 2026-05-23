#!/usr/bin/env bash
# Mock HTTP calls for global LLM API (no versions). Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASE="${GATEWAY_BASE_URL:-http://127.0.0.1:18088}"
MOCK_URL="https://api.example.com/v1"
MOCK_MODEL="mock-model-$(date +%s)"
MOCK_KEY="sk-mock-global-llm-$(date +%s)"

json_get() {
  python3 -c "import json,sys; d=json.load(sys.stdin); print(d$1)" 2>/dev/null
}

echo "==> [1/6] cargo integration test (DB + file sync)"
cd "${ROOT}/rust"
cargo test -p http-gateway-rs --test global_llm_api -- --nocapture

echo "==> [2/6] healthz ${BASE}"
curl -fsS "${BASE}/healthz" | head -c 120
echo ""

echo "==> [3/6] GET /v1/gateway/global-settings (before)"
BEFORE="$(curl -fsS "${BASE}/v1/gateway/global-settings")"
echo "${BEFORE}" | python3 -m json.tool | head -30

if echo "${BEFORE}" | python3 -c "import json,sys; d=json.load(sys.stdin); sys.exit(0 if 'activeLlmConfig' in d else 1)"; then
  echo "    OK: response has activeLlmConfig (new gateway build)"
else
  echo "    WARN: activeLlmConfig missing — gateway image may be old; run: ./deploy/stack/gateway.sh pack-deploy"
fi

echo "==> [4/6] PUT /v1/gateway/global-settings/active-llm-config"
PUT_BODY="$(python3 - <<PY
import json
print(json.dumps({
  "name": "mock-http-test",
  "baseModelUrl": "${MOCK_URL}",
  "modelName": "${MOCK_MODEL}",
  "apiKey": "${MOCK_KEY}",
}))
PY
)"
PUT_RESP="$(curl -fsS -X PUT "${BASE}/v1/gateway/global-settings/active-llm-config" \
  -H 'Content-Type: application/json' \
  -d "${PUT_BODY}")"
echo "${PUT_RESP}" | python3 -m json.tool

BASE_URL_BACK="$(echo "${PUT_RESP}" | json_get "['baseModelUrl']")"
if [[ "${BASE_URL_BACK}" != "${MOCK_URL}" ]]; then
  echo "FAIL: PUT response baseModelUrl=${BASE_URL_BACK} expected ${MOCK_URL}" >&2
  exit 1
fi

echo "==> [5/6] GET again (verify roundtrip)"
AFTER="$(curl -fsS "${BASE}/v1/gateway/global-settings")"
AFTER_URL="$(echo "${AFTER}" | python3 -c "
import json,sys
d=json.load(sys.stdin)
c=d.get('activeLlmConfig') or {}
print(c.get('baseModelUrl',''))
")"
if [[ "${AFTER_URL}" != "${MOCK_URL}" ]]; then
  echo "FAIL: GET activeLlmConfig.baseModelUrl=${AFTER_URL} expected ${MOCK_URL}" >&2
  exit 1
fi
echo "    OK: activeLlmConfig.baseModelUrl=${AFTER_URL}"

VERS="$(curl -fsS "${BASE}/v1/gateway/global-settings/llm-models/global/versions" 2>/dev/null || true)"
if [[ -n "${VERS}" ]]; then
  VCNT="$(echo "${VERS}" | python3 -c "import json,sys; print(len(json.load(sys.stdin).get('versions',[])))")"
  if [[ "${VCNT}" != "0" ]]; then
    echo "FAIL: versions list should be empty, got ${VCNT}" >&2
    exit 1
  fi
  echo "    OK: versions=[]"
fi

echo "==> [6/6] check host files (if gateway mounts repo .env)"
UP="${ROOT}/.claw/claw-tap-upstream.json"
if [[ -f "${UP}" ]]; then
  if grep -q "${MOCK_URL}" "${UP}"; then
    echo "    OK: ${UP} contains ${MOCK_URL}"
  else
    echo "    WARN: ${UP} does not contain ${MOCK_URL} (gateway may not mount host repo)"
    cat "${UP}"
  fi
fi

echo "==> all checks passed"
