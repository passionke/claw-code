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

claw_assert_gateway_pool_http_reachable "${PODMAN_DIR}"
claw_wait_gateway_claw_tap_ready || exit 1
claw_ensure_default_project_ds "${DS_ID}" || exit 1

BODY="$(GATEWAY_PORT="${GATEWAY_PORT}" DS_ID="${DS_ID}" USER_PROMPT="${PROMPT}" python3 <<'PY'
import json, os, urllib.request
port = os.environ["GATEWAY_PORT"]
ds = int(os.environ["DS_ID"])
prompt = os.environ["USER_PROMPT"]
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
print(json.dumps({"dsId": ds, "userPrompt": prompt, "extraSession": extra}, ensure_ascii=False))
PY
)"

echo "POST /v1/solve_async"
echo "${BODY}"
TASK_JSON="$(curl -fsS -X POST "http://127.0.0.1:${GATEWAY_PORT}/v1/solve_async" \
  -H "Content-Type: application/json" -d "${BODY}")"
echo "${TASK_JSON}"
TASK_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["taskId"])')"
TURN_ID="$(printf '%s' "${TASK_JSON}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["turnId"])')"

for _ in $(seq 1 120); do
  sleep 2
  R="$(curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/v1/tasks/${TASK_ID}")"
  ST="$(printf '%s' "${R}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["status"])')"
  echo "poll status=${ST}"
  if [[ "${ST}" == "succeeded" || "${ST}" == "failed" ]]; then
    printf '%s\n' "${R}" | python3 -m json.tool
    if [[ "${ST}" == "succeeded" ]]; then
      exit 0
    fi
    exit 1
  fi
done
echo "timeout waiting task ${TASK_ID} turn ${TURN_ID}" >&2
exit 1
