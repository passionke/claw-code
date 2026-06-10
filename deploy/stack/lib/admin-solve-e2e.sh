#!/usr/bin/env bash
# Admin ChatPage 同等 solve_async：POST + 轮询到终态。Author: kejiqing
set -euo pipefail
LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PODMAN_DIR="$(cd "${LIB_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${PODMAN_DIR}/../.." && pwd)"
# shellcheck disable=SC1091
source "${LIB_DIR}/pool-health.sh"
# shellcheck disable=SC1091
source "${LIB_DIR}/bootstrap-runtime.sh"

GATEWAY_PORT="${GATEWAY_HOST_PORT:-18088}"
DS_ID="${1:-1}"
PROMPT="${2:-connectivity check}"
CAPTURE_SESSION="${CLAW_E2E_CAPTURE_SESSION_ID:-0}"
if [[ "${CAPTURE_SESSION}" == "1" ]]; then
  exec 3>&1
  exec 1>&2
fi

_claw_e2e_log() {
  echo "$@"
}

# shellcheck disable=SC1091
source "${LIB_DIR}/e2e-project-isolation.sh"

if [[ -n "${CLAW_E2E_WORKER_ISOLATION:-}" ]]; then
  claw_e2e_set_project_worker_isolation "${GATEWAY_PORT}" "${DS_ID}" "${CLAW_E2E_WORKER_ISOLATION}"
fi

claw_e2e_assert_solve_task() {
  local json="$1" label="$2"
  if [[ -n "${CLAW_E2E_EXPECT_POOL_ID:-}" ]]; then
    local got_pool
    got_pool="$(printf '%s' "${json}" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("poolId") or "")')"
    if [[ "${got_pool}" != "${CLAW_E2E_EXPECT_POOL_ID}" ]]; then
      echo "error: ${label} poolId=${got_pool!r} expected ${CLAW_E2E_EXPECT_POOL_ID!r}" >&2
      exit 1
    fi
  fi
  if [[ -n "${CLAW_E2E_EXPECT_WORKER_ISOLATION:-}" ]]; then
    local got_iso
    got_iso="$(printf '%s' "${json}" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("workerIsolation") or "")')"
    if [[ "${got_iso}" != "${CLAW_E2E_EXPECT_WORKER_ISOLATION}" ]]; then
      echo "error: ${label} workerIsolation=${got_iso!r} expected ${CLAW_E2E_EXPECT_WORKER_ISOLATION!r}" >&2
      exit 1
    fi
  fi
}

claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}"
claw_wait_gateway_claw_tap_ready || exit 1
claw_ensure_default_project_ds "${DS_ID}" || exit 1

BODY="$(GATEWAY_PORT="${GATEWAY_PORT}" DS_ID="${DS_ID}" USER_PROMPT="${PROMPT}" CLAW_E2E_SESSION_ID="${CLAW_E2E_SESSION_ID:-}" python3 <<'PY'
import json, os, urllib.request
port = os.environ["GATEWAY_PORT"]
ds = int(os.environ["DS_ID"])
prompt = os.environ["USER_PROMPT"]
sid = os.environ.get("CLAW_E2E_SESSION_ID", "").strip()
cfg = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/v1/project/config/{ds}", timeout=15))
extra = {
    "tenant_code": "GPOS",
    "solution_code": "restaurant",
    "biz_type": "BOSS_REPORT",
    "client_origin": "gateway-admin",
}
for f in (cfg.get("extraSessionFieldsJson") or []):
    if isinstance(f, str) and f.strip():
        extra[f.strip()] = ""
body = {"projId": ds, "userPrompt": prompt, "extraSession": extra}
if sid:
    body["sessionId"] = sid
print(json.dumps(body, ensure_ascii=False))
PY
)"

_claw_e2e_log "POST /v1/solve_async"
_claw_e2e_log "${BODY}"
TASK_JSON="$(curl -fsS -X POST "http://127.0.0.1:${GATEWAY_PORT}/v1/solve_async" \
  -H "Content-Type: application/json" -d "${BODY}")"
_claw_e2e_log "${TASK_JSON}"
claw_e2e_assert_solve_task "${TASK_JSON}" "solve_async"
TASK_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["taskId"])')"
SESSION_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["sessionId"])')"
TURN_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["turnId"])')"

for _ in $(seq 1 120); do
  sleep 2
  R="$(curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/tasks/${TASK_ID}")"
  ST="$(printf '%s' "${R}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["status"])')"
  echo "poll status=${ST}"
  if [[ "${ST}" == "succeeded" || "${ST}" == "failed" ]]; then
    if [[ "${CAPTURE_SESSION}" != "1" ]]; then
      printf '%s\n' "${R}" | python3 -m json.tool
    fi
    if [[ "${ST}" == "succeeded" ]]; then
      claw_e2e_assert_solve_task "${R}" "task poll"
      if [[ "${CAPTURE_SESSION}" == "1" ]]; then
        printf '%s\n' "${SESSION_ID}" >&3
      fi
      exit 0
    fi
    exit 1
  fi
done
echo "timeout waiting task ${TASK_ID} turn ${TURN_ID}" >&2
exit 1
