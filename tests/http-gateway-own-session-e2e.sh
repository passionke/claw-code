#!/usr/bin/env bash
# E2E: own_session — extraSessionFieldsJson, solve validation, entry_params_json, session filter, admin feedback guard.
# Runs against live gateway (default :18088) + host pool. Author: kejiqing
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIB_DIR="${REPO_ROOT}/deploy/stack/lib"
PODMAN_DIR="${REPO_ROOT}/deploy/stack"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-18088}"
BASE="http://127.0.0.1:${GATEWAY_PORT}"
DS_ID="${DS_ID:-1}"
CURL_MAX=30
WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/claw-own-session-e2e.XXXXXX")"
ORIG_CFG="${WORKDIR}/orig_config.json"
RESTORE_CFG="${WORKDIR}/restore_put.json"
FAIL=0
PASS=0
SKIP=0

pass() { PASS=$((PASS + 1)); echo "[PASS] $*"; }
fail() { FAIL=$((FAIL + 1)); echo "[FAIL] $*" >&2; }
skip() { SKIP=$((SKIP + 1)); echo "[SKIP] $*"; }

http_code() {
  local out="$1"
  shift
  curl -sS -o "$out" -w '%{http_code}' --max-time "$CURL_MAX" "$@"
}

json_get() {
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d[sys.argv[2]])' "$@"
}

assert_http() {
  local name="$1" expect="$2" got="$3" body="$4"
  if [[ "$got" == "$expect" ]]; then
    pass "$name (HTTP $got)"
  else
    fail "$name expected HTTP $expect, got $got body=$(tr -d '\n' <"$body" | head -c 400)"
  fi
}

assert_py() {
  local name="$1" script="$2" file="$3"
  if python3 -c "$script" "$file"; then
    pass "$name"
  else
    fail "$name"
  fi
}

build_put_from_get() {
  local src="$1" dst="$2" extra_fields="${3:-}"
  EXTRA_FIELDS_JSON="${extra_fields}" python3 - "$src" "$dst" <<'PY'
import json, os, sys
src, dst = sys.argv[1], sys.argv[2]
cfg = json.load(open(src))
extra = os.environ.get("EXTRA_FIELDS_JSON", "")
if extra:
    cfg["extraSessionFieldsJson"] = json.loads(extra)
body = {
    "contentRev": cfg.get("contentRev") or "",
    "rulesJson": cfg.get("rulesJson") or [],
    "mcpServersJson": cfg.get("mcpServersJson") or {},
    "skillsSourcesJson": cfg.get("skillsSourcesJson") or [],
    "skillsJson": cfg.get("skillsJson") or [],
    "allowedToolsJson": cfg.get("allowedToolsJson") or [],
    "claudeMd": cfg.get("claudeMd"),
    "gitSyncJson": cfg.get("gitSyncJson"),
    "solvePreflightJson": cfg.get("solvePreflightJson"),
    "solveOrchestrationJson": cfg.get("solveOrchestrationJson"),
    "extraSessionFieldsJson": cfg.get("extraSessionFieldsJson") or [],
}
json.dump(body, open(dst, "w"), ensure_ascii=False)
PY
}

admin_extra_body() {
  local prompt="$1" store="${2:-E2E_STORE}" org="${3:-}" client_origin="${4:-gateway-admin}"
  DS_ID="$DS_ID" STORE="$store" ORG="$org" PROMPT="$prompt" CLIENT_ORIGIN="$client_origin" python3 - <<'PY'
import json, os
extra = {
    "tenant_code": "GPOS",
    "solution_code": "restaurant",
    "biz_type": "BOSS_REPORT",
    "_claw_client_origin": os.environ["CLIENT_ORIGIN"],
    "store_id": os.environ["STORE"],
    "org_id": os.environ["ORG"],
}
print(json.dumps({
    "projId": int(os.environ.get("DS_ID", "1")),
    "userPrompt": os.environ["PROMPT"],
    "extraSession": extra,
}, ensure_ascii=False))
PY
}

restore_config() {
  if [[ -f "$RESTORE_CFG" ]]; then
    curl -sf --max-time "$CURL_MAX" -X PUT "${BASE}/v1/project/config/${DS_ID}" \
      -H 'Content-Type: application/json' -d @"$RESTORE_CFG" >/dev/null 2>&1 || true
  fi
}
trap 'restore_config; rm -rf "$WORKDIR"' EXIT

echo "=== own_session E2E gateway=${BASE} ds=${DS_ID} ==="

if ! curl -sf --max-time "$CURL_MAX" "${BASE}/healthz" >/dev/null; then
  echo "gateway not reachable at ${BASE}" >&2
  exit 1
fi
pass "gateway healthz"

POOL_OK=1
if ! claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}" 2>/dev/null; then
  POOL_OK=0
  skip "host pool not ready — enqueue / full solve tests will be skipped"
fi

# Save original project config for restore
curl -sf --max-time "$CURL_MAX" "${BASE}/v1/project/config/${DS_ID}" -o "$ORIG_CFG"
build_put_from_get "$ORIG_CFG" "$RESTORE_CFG"
pass "saved original project config"

# --- 1. project config extraSessionFieldsJson roundtrip ---
build_put_from_get "$ORIG_CFG" "${WORKDIR}/put_fields.json" '["store_id","org_id"]'
CODE="$(http_code "${WORKDIR}/put_resp.json" -X PUT "${BASE}/v1/project/config/${DS_ID}" \
  -H 'Content-Type: application/json' -d @"${WORKDIR}/put_fields.json")"
assert_http "PUT extraSessionFieldsJson" "200" "$CODE" "${WORKDIR}/put_resp.json"

curl -sf --max-time "$CURL_MAX" "${BASE}/v1/project/config/${DS_ID}" -o "${WORKDIR}/cfg_after.json"
assert_py "GET reflects extraSessionFieldsJson" \
  'import json,sys; c=json.load(open(sys.argv[1])); f=c.get("extraSessionFieldsJson") or []; assert f==["store_id","org_id"], f' \
  "${WORKDIR}/cfg_after.json"

# --- 2. reject _claw_ prefix in field definitions ---
build_put_from_get "$ORIG_CFG" "${WORKDIR}/put_bad_field.json" '["_claw_secret"]'
CODE="$(http_code "${WORKDIR}/bad_field.json" -X PUT "${BASE}/v1/project/config/${DS_ID}" \
  -H 'Content-Type: application/json' -d @"${WORKDIR}/put_bad_field.json")"
assert_http "PUT rejects _claw_ field name" "400" "$CODE" "${WORKDIR}/bad_field.json"
# restore fields for solve tests
curl -sf --max-time "$CURL_MAX" -X PUT "${BASE}/v1/project/config/${DS_ID}" \
  -H 'Content-Type: application/json' -d @"${WORKDIR}/put_fields.json" >/dev/null

# --- 3. solve validation (no pool needed) ---
DS_ID="$DS_ID" BODY="$(admin_extra_body 'e2e missing org' 'S1' '__MISSING__' )"
# remove org_id key entirely
BODY="$(printf '%s' "$BODY" | python3 -c 'import json,sys; d=json.load(sys.stdin); d["extraSession"].pop("org_id",None); print(json.dumps(d))')"
CODE="$(http_code "${WORKDIR}/v_miss_org.json" -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' -d "$BODY")"
assert_http "solve_async missing org_id -> 400" "400" "$CODE" "${WORKDIR}/v_miss_org.json"
assert_py "missing org_id message" \
  'import json,sys; d=json.load(open(sys.argv[1])); m=str(d.get("detail") or d.get("message") or ""); assert "org_id" in m, m' \
  "${WORKDIR}/v_miss_org.json"

DS_ID="$DS_ID" BODY="$(admin_extra_body 'e2e non-string org' 'S1' '123')"
BODY="$(printf '%s' "$BODY" | python3 -c 'import json,sys; d=json.load(sys.stdin); d["extraSession"]["org_id"]=123; print(json.dumps(d))')"
CODE="$(http_code "${WORKDIR}/v_bad_type.json" -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' -d "$BODY")"
assert_http "solve_async non-string org_id -> 400" "400" "$CODE" "${WORKDIR}/v_bad_type.json"

CODE="$(http_code "${WORKDIR}/v_no_extra.json" -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' \
  -d "{\"projId\":${DS_ID},\"userPrompt\":\"no extra\"}")"
assert_http "solve_async no extraSession object -> 400" "400" "$CODE" "${WORKDIR}/v_no_extra.json"

# sync /v1/solve shares validate_solve_request
CODE="$(http_code "${WORKDIR}/v_sync_miss.json" -X POST "${BASE}/v1/solve" -H 'Content-Type: application/json' -d "$BODY")"
assert_http "solve sync missing org_id -> 400" "400" "$CODE" "${WORKDIR}/v_sync_miss.json"

# --- 4. empty org_id enqueue + turns extraSession snapshot (needs pool) ---
if [[ "$POOL_OK" -eq 1 ]]; then
  UNIQUE_STORE="E2E_$(date +%s)_$$"
  DS_ID="$DS_ID" BODY="$(DS_ID="$DS_ID" admin_extra_body 'own_session e2e empty org_id' "$UNIQUE_STORE" '')"
  CODE="$(http_code "${WORKDIR}/enqueue.json" -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' -d "$BODY")"
  assert_http "solve_async empty org_id enqueue" "200" "$CODE" "${WORKDIR}/enqueue.json"

  if [[ "$CODE" == "200" ]]; then
    SESSION_ID="$(json_get "${WORKDIR}/enqueue.json" sessionId)"
    TURN_ID="$(json_get "${WORKDIR}/enqueue.json" turnId)"
    pass "enqueued session=${SESSION_ID} turn=${TURN_ID}"

    curl -sf --max-time "$CURL_MAX" \
      "${BASE}/v1/sessions/${SESSION_ID}/turns?projId=${DS_ID}" -o "${WORKDIR}/turns.json"
    assert_py "turns list includes extraSession snapshot" \
      "import json,sys; d=json.load(open(sys.argv[1])); turns=d.get('turns') or []; assert turns, turns; t=next((x for x in turns if x.get('turnId')=='${TURN_ID}'), None); assert t, turns; es=t.get('extraSession') or {}; assert es.get('store_id')=='${UNIQUE_STORE}', es; assert es.get('org_id')=='', es" \
      "${WORKDIR}/turns.json"

    # session list filter by extraSession store_id
    FILTER="$(python3 -c "import json,urllib.parse; print(urllib.parse.quote(json.dumps({'store_id':'${UNIQUE_STORE}'})))")"
    curl -sf --max-time "$CURL_MAX" \
      "${BASE}/v1/projects/${DS_ID}/sessions?limit=50&extraSession=${FILTER}" -o "${WORKDIR}/sessions_filter.json"
    assert_py "session list extraSession filter hits session" \
      "import json,sys; d=json.load(open(sys.argv[1])); ids=[s.get('sessionId') for s in (d.get('sessions') or [])]; assert '${SESSION_ID}' in ids, ids" \
      "${WORKDIR}/sessions_filter.json"

    # reject filter on undefined key
    BAD_FILTER="$(python3 -c 'import json,urllib.parse; print(urllib.parse.quote(json.dumps({"rogue_key":"x"})))')"
    CODE="$(http_code "${WORKDIR}/bad_filter.json" "${BASE}/v1/projects/${DS_ID}/sessions?limit=5&extraSession=${BAD_FILTER}")"
    assert_http "session list rejects undefined extraSession filter key" "400" "$CODE" "${WORKDIR}/bad_filter.json"
  fi

  # --- 5. external turn: admin feedback forbidden ---
  EXT_STORE="EXT_$(date +%s)_$$"
  DS_ID="$DS_ID" BODY="$(DS_ID="$DS_ID" admin_extra_body 'external origin e2e' "$EXT_STORE" 'ext-org' 'dingtalk-bot')"
  CODE="$(http_code "${WORKDIR}/ext_enqueue.json" -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' -d "$BODY")"
  if [[ "$CODE" == "200" ]]; then
    EXT_SID="$(json_get "${WORKDIR}/ext_enqueue.json" sessionId)"
    EXT_TID="$(json_get "${WORKDIR}/ext_enqueue.json" turnId)"

    curl -sf --max-time "$CURL_MAX" \
      "${BASE}/v1/sessions/${EXT_SID}/turns?projId=${DS_ID}" -o "${WORKDIR}/ext_turns.json"
    assert_py "external turn clientOrigin persisted" \
      "import json,sys; d=json.load(open(sys.argv[1])); t=next(x for x in d['turns'] if x['turnId']=='${EXT_TID}'); assert t.get('clientOrigin')=='dingtalk-bot', t" \
      "${WORKDIR}/ext_turns.json"

    CODE="$(http_code "${WORKDIR}/fb_forbidden.json" -X POST "${BASE}/v1/agent/feedback" \
      -H 'Content-Type: application/json' \
      -H 'X-Claw-Client-Origin: gateway-admin' \
      -d "{\"projId\":${DS_ID},\"sessionId\":\"${EXT_SID}\",\"turnId\":\"${EXT_TID}\",\"feedback\":\"good\"}")"
    assert_http "admin feedback on external turn -> 403" "403" "$CODE" "${WORKDIR}/fb_forbidden.json"

    # admin-origin turn feedback should succeed
    CODE="$(http_code "${WORKDIR}/fb_ok.json" -X POST "${BASE}/v1/agent/feedback" \
      -H 'Content-Type: application/json' \
      -H 'X-Claw-Client-Origin: gateway-admin' \
      -d "{\"projId\":${DS_ID},\"sessionId\":\"${SESSION_ID}\",\"turnId\":\"${TURN_ID}\",\"feedback\":\"good\"}")"
    assert_http "admin feedback on admin turn -> 200" "200" "$CODE" "${WORKDIR}/fb_ok.json"
  else
    fail "external enqueue for feedback test (HTTP $CODE)"
  fi
else
  skip "enqueue / turns / filter / feedback tests (pool down)"
fi

# --- 6. no field defs: validation skipped ---
build_put_from_get "$ORIG_CFG" "${WORKDIR}/put_clear_fields.json" '[]'
curl -sf --max-time "$CURL_MAX" -X PUT "${BASE}/v1/project/config/${DS_ID}" \
  -H 'Content-Type: application/json' -d @"${WORKDIR}/put_clear_fields.json" >/dev/null
CODE="$(http_code "${WORKDIR}/v_minimal.json" -X POST "${BASE}/v1/solve_async" -H 'Content-Type: application/json' \
  -d "{\"projId\":${DS_ID},\"userPrompt\":\"minimal extra\",\"extraSession\":{\"tenant_code\":\"GPOS\"}}")"
if [[ "$POOL_OK" -eq 1 ]]; then
  assert_http "empty field defs: minimal extraSession enqueue" "200" "$CODE" "${WORKDIR}/v_minimal.json"
else
  # without pool, 503 is acceptable; 400 would be a defect
  if [[ "$CODE" == "400" ]]; then
    fail "empty field defs should not 400 on minimal extraSession (got 400)"
  else
    skip "empty field defs enqueue (pool down, HTTP $CODE)"
  fi
fi

# restore original extraSessionFieldsJson
curl -sf --max-time "$CURL_MAX" -X PUT "${BASE}/v1/project/config/${DS_ID}" \
  -H 'Content-Type: application/json' -d @"$RESTORE_CFG" >/dev/null
pass "restored original project config"

echo ""
echo "=== own_session E2E summary: pass=${PASS} fail=${FAIL} skip=${SKIP} ==="
if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
echo "OK — http-gateway-own-session-e2e"
