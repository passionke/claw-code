#!/usr/bin/env bash
# Local router/delegate test project bootstrap — 127.0.0.1 only. Author: kejiqing
# ⚠️ 禁止对预发 252 / 271 执行。仅本机 gateway + router-test-local cluster。
set -euo pipefail

GW="${GW:-http://127.0.0.1:18088}"
: "${CLAW_ADMIN_TOKEN:?set CLAW_ADMIN_TOKEN (local Admin MCP)}"

ROUTER_PROJ="${ROUTER_PROJ:-99010}"
KB_PROJ="${KB_PROJ:-99011}"
OPS_PROJ="${OPS_PROJ:-99012}"

AUTH="Authorization: Bearer ${CLAW_ADMIN_TOKEN}"
JSON='Content-Type: application/json'

curl_json() {
  local method=$1 path=$2 body=${3:-}
  if [[ -n "$body" ]]; then
    curl -fsS -X "$method" "${GW}${path}" -H "$AUTH" -H "$JSON" -d "$body"
  else
    curl -fsS -X "$method" "${GW}${path}" -H "$AUTH" -H "$JSON"
  fi
}

ensure_project() {
  local id=$1 code=$2 desc=$3
  if curl -fsS -H "$AUTH" "${GW}/v1/project/config/${id}" >/dev/null 2>&1; then
    echo "==> project ${id} (${code}) exists"
    return 0
  fi
  echo "==> create project ${id} (${code})"
  curl_json POST /v1/projects "{\"projId\":${id},\"projectCode\":\"${code}\",\"projectDescription\":\"${desc}\"}" >/dev/null
}

ensure_project "$ROUTER_PROJ" "gpos-router-local" "local router test"
ensure_project "$KB_PROJ" "gpos-kb-local" "local kb-qa test"
ensure_project "$OPS_PROJ" "gpos-ops-local" "local ops test"

echo "==> Set router role (seed CLAUDE + specialist-registry)"
curl_json PUT "/v1/projects/${ROUTER_PROJ}/role" '{"projectRole":"router"}' >/dev/null

echo "==> Router delegate-targets"
curl_json PUT "/v1/projects/${ROUTER_PROJ}/delegate-targets" \
  "{\"targets\":[{\"targetProjId\":${KB_PROJ},\"label\":\"kb-qa\",\"capabilityHint\":\"product manual\"},{\"targetProjId\":${OPS_PROJ},\"label\":\"ops-analysis\",\"capabilityHint\":\"business analytics\"}]}" >/dev/null

echo "==> Activate router (materialize registry appendix)"
REV=$(curl -fsS -H "$AUTH" "${GW}/v1/project/config/${ROUTER_PROJ}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('stableContentRev',''))")
if [[ -n "$REV" ]]; then
  curl -fsS -X POST "${GW}/v1/project/config/${ROUTER_PROJ}/versions/${REV}/activate" -H "$AUTH" >/dev/null || true
else
  echo "warn: no stableContentRev; skip activate"
fi

cat <<EOF
Done (local only).
  export GW=${GW}
  export GPOS_PROJ_ID=${ROUTER_PROJ}
  export ROUTER_PROJ=${ROUTER_PROJ} KB_PROJ=${KB_PROJ} OPS_PROJ=${OPS_PROJ}
  python3 scripts/gpos-router-split/acceptance_smoke.py --check-config
EOF
